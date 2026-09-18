#!/usr/bin/env bash
set -euo pipefail

# Synthetic MCP demo — no real PHI, payers, or model keys.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d)"
trap 'kill "$MCP_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

export MINT_LAB_DIR="$TMP/lab-data"
export MINT_LAB_FIXTURES="$ROOT/fixtures/pa_bv"
export MINT_LAB_MCP_TOKEN="lab-demo-token"
export MINT_LAB_AUTO_PAYER=1

PORT="${MINT_LAB_MCP_PORT:-18787}"
ERR="$TMP/mcp.err"

echo "PA/BV MCP demo — unclear_bv via loopback HTTP (synthetic CPT 72148)"
cargo build --quiet

cargo run --quiet -- lab mcp-http --scenario unclear_bv --bind "127.0.0.1:${PORT}" --dir "$MINT_LAB_DIR" \
  >"$TMP/mcp.out" 2>"$ERR" &
MCP_PID=$!

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

if ! curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null; then
  echo "FAIL — MCP HTTP did not become healthy"
  cat "$ERR" || true
  exit 1
fi

RUN_ID="$(sed -n 's/^MCP_RUN_ID=//p' "$ERR" | head -n1)"
TASK_ID="$(sed -n 's/^MCP_TASK_ID=//p' "$ERR" | head -n1)"
if [[ -z "$RUN_ID" || -z "$TASK_ID" ]]; then
  echo "FAIL — missing MCP_RUN_ID/MCP_TASK_ID"
  cat "$ERR" || true
  exit 1
fi

rpc() {
  curl -sf -X POST "http://127.0.0.1:${PORT}/mcp" \
    -H "Authorization: Bearer ${MINT_LAB_MCP_TOKEN}" \
    -H "Content-Type: application/json" \
    --data "$1"
}

echo "run_id=$RUN_ID task_id=$TASK_ID"

LIST="$(rpc '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}')"
echo "$LIST" | grep -q 'read_assigned_context'
echo "$LIST" | grep -q 'ask_payer'
echo "$LIST" | grep -qv 'set_stage'

rpc "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"read_assigned_context\",\"arguments\":{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\"}}}" >/dev/null

ASKED="$(rpc "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"ask_payer\",\"arguments\":{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"question\":\"Is prior authorization required for CPT 72148?\",\"evidence_hint\":\"payer_bv_response\"}}}")"
echo "$ASKED" | grep -q '"status":"answered"'
echo "$ASKED" | grep -q '"mode":"auto_payer"'

MSG_ID="$(printf '%s' "$ASKED" | sed -n 's/.*"msg_id":"\([^"]*\)".*/\1/p' | head -n1)"
EVIDENCE_ID="msg:${MSG_ID}"

rpc "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"read_permitted_evidence\",\"arguments\":{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"evidence_id\":\"$EVIDENCE_ID\"}}}" >/dev/null

REPORTED="$(rpc "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"report_observations\",\"arguments\":{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"needs_human_review\":true,\"observations\":[{\"kind\":\"pa_requirement\",\"statement\":\"PA requirement remains unclear from payer language.\",\"uncertainty\":\"unknown\",\"evidence_refs\":[\"$EVIDENCE_ID\"]}]}}}")"
echo "$REPORTED" | grep -q '"accepted":true'

CTX="$(rpc "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"resources/read\",\"params\":{\"uri\":\"mint-lab://run/${RUN_ID}/bv-task/${TASK_ID}/context\"}}")"
echo "$CTX" | grep -q '72148'

echo "PASS — MCP stdio/HTTP surface: five BV tools, auto-payer, resources (synthetic only)"
