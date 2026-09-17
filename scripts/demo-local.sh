#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo build -q

TMP="$(mktemp -d)"
cleanup() {
  if [[ -n "${PID:-}" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

export MINT_MODE=development
export MINT_PROVIDER=fake
export MINT_IDENTITY=local
export MINT_DATABASE_PATH="$TMP/mint.db"
export MINT_SIGNING_KEY_FILE="$TMP/mint.ed25519.pem"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
export MINT_BIND_ADDR="127.0.0.1:${PORT}"
export MINT_KID=mint-local-1

./target/debug/mint init --key-file "$MINT_SIGNING_KEY_FILE"
./target/debug/mint doctor
./target/debug/mint serve >"$TMP/server.log" 2>&1 &
PID=$!

TOKEN="$(python3 - <<'PY'
import json, base64
payload = {
  "tenant_id": "acme",
  "subject": "user_123",
  "agent_id": "support-agent-7",
  "issuer": "https://identity.example.com",
  "delegated_by": None,
}
raw = json.dumps(payload, separators=(",", ":")).encode()
print("dev." + base64.urlsafe_b64encode(raw).decode().rstrip("="))
PY
)"

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null; then
    break
  fi
  sleep 0.1
done
curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null

PROPOSE="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "tenantId": "acme",
    "actor": {
      "subject": "user_123",
      "agentId": "support-agent-7",
      "issuer": "https://identity.example.com"
    },
    "provider": "fake",
    "operation": "refund.create",
    "resource": { "type": "charge", "id": "ch_123" },
    "arguments": { "amount": 4200, "currency": "usd", "reason": "duplicate" },
    "context": { "supportTicketId": "ticket_982", "reason": "duplicate" }
  }')"

echo "$PROPOSE" | python3 -m json.tool
ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$PROPOSE")"
STATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$PROPOSE")"
test "$STATUS" = "Authorized"

EXECUTE="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions/${ID}/execute" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{}')"
echo "$EXECUTE" | python3 -m json.tool
test "$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$EXECUTE")" = "Succeeded"

curl -sf "http://127.0.0.1:${PORT}/v1/actions/${ID}/receipt" \
  -H "Authorization: Bearer ${TOKEN}" >"$TMP/receipt.json"
python3 -m json.tool <"$TMP/receipt.json" >/dev/null
./target/debug/mint verify --receipt "$TMP/receipt.json" --key-file "$MINT_SIGNING_KEY_FILE"

node --experimental-strip-types - <<PY
import { Mint, encodeDevToken } from "${ROOT}/sdk/typescript/src/index.ts";
const mint = new Mint({
  baseUrl: "http://127.0.0.1:${PORT}",
  token: encodeDevToken({
    tenantId: "acme",
    subject: "user_123",
    agentId: "support-agent-7",
    issuer: "https://identity.example.com",
  }),
});
const action = await mint.actions.propose({
  tenantId: "acme",
  actor: {
    subject: "user_123",
    agentId: "support-agent-7",
    issuer: "https://identity.example.com",
  },
  provider: "fake",
  operation: "refund.create",
  resource: { type: "charge", id: "ch_123" },
  arguments: { amount: 4200, currency: "usd", reason: "duplicate" },
  context: { supportTicketId: "ticket_982" },
});
const result = await mint.actions.execute(action.id);
if (result.status !== "Succeeded") {
  throw new Error("typescript client execute failed: " + result.status);
}
console.log("typescript client executed", result.id, result.status);
PY

echo "local fake-provider demo succeeded"
