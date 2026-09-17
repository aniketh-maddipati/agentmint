#!/usr/bin/env bash
# Black-box adversarial probes: invalid config, process restart, concurrent HTTP.
# Fake provider only. No Stripe. Temporary dirs only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

unset MINT_STRIPE_TEST_SECRET_KEY MINT_RUN_STRIPE_E2E MINT_PROVIDER MINT_MODE \
  MINT_IDENTITY MINT_POLICY MINT_SIGNING_KEY MINT_SIGNING_KEY_FILE \
  MINT_DATABASE_PATH MINT_BIND_ADDR MINT_HTTP_TIMEOUT_MS \
  MINT_POLICY_AUTO_CENTS MINT_POLICY_APPROVAL_CENTS 2>/dev/null || true

cargo build -q
BIN="$ROOT/target/debug/mint"
TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

pass=0
fail=0
note() { echo "NOTE  $*"; }
ok() { echo "PASS  $*"; pass=$((pass + 1)); }
bad() { echo "FAIL  $*"; fail=$((fail + 1)); }

"$BIN" init --key-file "$TMP/ok.pem" >/dev/null

probe_exit() {
  local name="$1"; shift
  set +e
  env -i PATH="$PATH" HOME="$HOME" "$@" "$BIN" serve >"$TMP/${name}.log" 2>&1 &
  local pid=$!
  sleep 0.25
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    bad "$name still serving"
  else
    wait "$pid"
    local code=$?
    if [[ "$code" -ne 0 ]]; then
      ok "$name exits $code"
    else
      bad "$name exited 0"
    fi
  fi
  set -e
}

echo "==> invalid configuration must exit before serving"
probe_exit typo_provider MINT_PROVIDER=strpie MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/a.db"
probe_exit bad_mode MINT_MODE=staging MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/b.db"
probe_exit bad_identity MINT_IDENTITY=webauthn MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/c.db"
probe_exit bad_policy MINT_POLICY=opa MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/d.db"
probe_exit prod_local MINT_MODE=production MINT_PROVIDER=fake MINT_IDENTITY=local MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/e.db"
probe_exit thresh_order MINT_POLICY_AUTO_CENTS=6000 MINT_POLICY_APPROVAL_CENTS=5000 MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/f.db"
probe_exit neg_thresh MINT_POLICY_AUTO_CENTS=-1 MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/g.db"
probe_exit zero_timeout MINT_HTTP_TIMEOUT_MS=0 MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/h.db"
probe_exit missing_key MINT_DATABASE_PATH="$TMP/i.db"
echo 'not-a-pem' >"$TMP/bad.pem"
probe_exit malformed_key MINT_SIGNING_KEY_FILE="$TMP/bad.pem" MINT_DATABASE_PATH="$TMP/j.db"
probe_exit unwritable_db MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="/root/nope/mint.db"
probe_exit sk_live MINT_PROVIDER=stripe MINT_STRIPE_TEST_SECRET_KEY=sk_live_NOT_A_REAL_KEY MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/k.db"
probe_exit rk_live MINT_PROVIDER=stripe MINT_STRIPE_TEST_SECRET_KEY=rk_live_NOT_A_REAL_KEY MINT_SIGNING_KEY_FILE="$TMP/ok.pem" MINT_DATABASE_PATH="$TMP/l.db"

echo "==> process restart preserves Authorized action"
export MINT_MODE=development MINT_PROVIDER=fake MINT_IDENTITY=local
export MINT_DATABASE_PATH="$TMP/restart.db" MINT_SIGNING_KEY_FILE="$TMP/restart.pem"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
export MINT_BIND_ADDR="127.0.0.1:${PORT}"
"$BIN" init --key-file "$MINT_SIGNING_KEY_FILE" >/dev/null
"$BIN" serve >"$TMP/s1.log" 2>&1 &
PID=$!
for _ in $(seq 1 80); do curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null && break; sleep 0.05; done
TOKEN="$(python3 - <<'PY'
import json, base64
payload={"tenant_id":"acme","subject":"user_123","agent_id":"support-agent-7","issuer":"https://identity.example.com","delegated_by":None}
print("dev."+base64.urlsafe_b64encode(json.dumps(payload,separators=(",",":")).encode()).decode().rstrip("="))
PY
)"
PROPOSE="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
  -d '{"tenantId":"acme","actor":{"subject":"user_123","agentId":"support-agent-7","issuer":"https://identity.example.com"},"provider":"fake","operation":"refund.create","resource":{"type":"charge","id":"ch_restart"},"arguments":{"amount":4217,"currency":"usd","reason":"duplicate"},"context":{"supportTicketId":"ticket_982"}}')"
ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$PROPOSE")"
kill "$PID"; wait "$PID" 2>/dev/null || true
"$BIN" serve >"$TMP/s2.log" 2>&1 &
PID=$!
for _ in $(seq 1 80); do curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null && break; sleep 0.05; done
GET="$(curl -sf "http://127.0.0.1:${PORT}/v1/actions/${ID}" -H "Authorization: Bearer ${TOKEN}")"
STATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$GET")"
EXECUTE="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions/${ID}/execute" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" -d '{}')"
ESTATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$EXECUTE")"
PROVIDER="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["providerResourceId"])' "$EXECUTE")"
curl -sf "http://127.0.0.1:${PORT}/v1/actions/${ID}/receipt" -H "Authorization: Bearer ${TOKEN}" >"$TMP/receipt.json"
"$BIN" verify --receipt "$TMP/receipt.json" --key-file "$MINT_SIGNING_KEY_FILE" >/dev/null
if [[ "$STATUS" == "Authorized" && "$ESTATUS" == "Succeeded" ]]; then
  ok "restart Authorized -> execute Succeeded ($PROVIDER)"
else
  bad "restart path status=$STATUS execute=$ESTATUS"
fi

echo "==> Unknown survives restart; reconcile recovers deterministic fake id"
PROPOSE2="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
  -d '{"tenantId":"acme","actor":{"subject":"user_123","agentId":"support-agent-7","issuer":"https://identity.example.com"},"provider":"fake","operation":"refund.create","resource":{"type":"charge","id":"ch_unknown"},"arguments":{"amount":4217,"currency":"usd","reason":"duplicate"},"context":{"supportTicketId":"ticket_982"}}')"
ID2="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$PROPOSE2")"
curl -s -o "$TMP/unk.json" -X POST "http://127.0.0.1:${PORT}/v1/actions/${ID2}/execute" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
  -H "x-mint-failpoint: after_provider_success" -d '{}' >/dev/null
UNK="$(curl -sf "http://127.0.0.1:${PORT}/v1/actions/${ID2}" -H "Authorization: Bearer ${TOKEN}")"
USTATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$UNK")"
kill "$PID"; wait "$PID" 2>/dev/null || true
"$BIN" serve >"$TMP/s3.log" 2>&1 &
PID=$!
for _ in $(seq 1 80); do curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null && break; sleep 0.05; done
AFTER="$(curl -sf "http://127.0.0.1:${PORT}/v1/actions/${ID2}" -H "Authorization: Bearer ${TOKEN}")"
ASTATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$AFTER")"
REC="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions/${ID2}/reconcile" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" -d '{}')"
RSTATUS="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$REC")"
RID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["providerResourceId"])' "$REC")"
EXPECTED="re_fake_$(echo "$ID2" | tr -d '-')"
if [[ "$USTATUS" == "Unknown" && "$ASTATUS" == "Unknown" && "$RSTATUS" == "Succeeded" && "$RID" == "$EXPECTED" ]]; then
  ok "Unknown restart reconcile -> $RID"
  note "fake provider re-derives deterministic id after process restart (in-memory ledger not durable); Stripe path lists provider objects"
else
  bad "Unknown restart u=$USTATUS a=$ASTATUS r=$RSTATUS id=$RID expected=$EXPECTED"
fi
kill "$PID"; wait "$PID" 2>/dev/null || true

echo "==> release build ignores failpoints"
cargo build -q --release
RBIN="$ROOT/target/release/mint"
export MINT_DATABASE_PATH="$TMP/rel.db" MINT_SIGNING_KEY_FILE="$TMP/rel.pem"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
export MINT_BIND_ADDR="127.0.0.1:${PORT}"
"$RBIN" init --key-file "$MINT_SIGNING_KEY_FILE" >/dev/null
"$RBIN" serve >"$TMP/rel.log" 2>&1 &
PID=$!
for _ in $(seq 1 80); do curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null && break; sleep 0.05; done
PROP="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
  -d '{"tenantId":"acme","actor":{"subject":"user_123","agentId":"support-agent-7","issuer":"https://identity.example.com"},"provider":"fake","operation":"refund.create","resource":{"type":"charge","id":"ch_rel"},"arguments":{"amount":100,"currency":"usd","reason":"duplicate"},"context":{"supportTicketId":"t"}}')"
RID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$PROP")"
BODY="$(curl -sf -X POST "http://127.0.0.1:${PORT}/v1/actions/${RID}/execute" \
  -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
  -H "x-mint-failpoint: after_provider_success" -d '{}')"
RSTAT="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$BODY")"
if [[ "$RSTAT" == "Succeeded" ]]; then
  ok "release ignores x-mint-failpoint (status=Succeeded)"
else
  bad "release failpoint leaked status=$RSTAT"
fi
kill "$PID"; wait "$PID" 2>/dev/null || true

echo "adversarial-local: $pass passed, $fail failed"
if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "PASS — adversarial-local"
