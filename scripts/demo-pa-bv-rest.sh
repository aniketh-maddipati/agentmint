#!/usr/bin/env bash
set -euo pipefail

# Synthetic REST demo — same five BV tools as MCP, no real PHI, payers, or model keys.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d)"
trap 'kill "$MCP_PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

export MINT_LAB_DIR="$TMP/lab-data"
export MINT_LAB_FIXTURES="$ROOT/fixtures/pa_bv"
export MINT_LAB_MCP_TOKEN="lab-demo-token"
export MINT_LAB_AUTO_PAYER=1

PORT="${MINT_LAB_MCP_PORT:-18788}"
ERR="$TMP/rest.err"

echo "PA/BV REST demo — unclear_bv via loopback HTTP (synthetic CPT 72148)"
cargo build --quiet

cargo run --quiet -- lab mcp-http --scenario unclear_bv --bind "127.0.0.1:${PORT}" --dir "$MINT_LAB_DIR" \
  >"$TMP/rest.out" 2>"$ERR" &
MCP_PID=$!

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

if ! curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null; then
  echo "FAIL — lab HTTP did not become healthy"
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

rest() {
  local method="$1"
  local path="$2"
  shift 2
  curl -sf -X "$method" "http://127.0.0.1:${PORT}${path}" \
    -H "Authorization: Bearer ${MINT_LAB_MCP_TOKEN}" \
    -H "Content-Type: application/json" \
    "$@"
}

echo "run_id=$RUN_ID task_id=$TASK_ID"

SPEC="$(rest GET /lab/openapi.json)"
echo "$SPEC" | grep -q '/lab/bv/read_assigned_context'
echo "$SPEC" | grep -q '/lab/bv/ask_payer'
echo "$SPEC" | grep -qv '/lab/bv/set_stage'

rest POST /lab/bv/read_assigned_context \
  --data "{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\"}" >/dev/null

ASKED="$(rest POST /lab/bv/ask_payer --data "{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"question\":\"Is prior authorization required for CPT 72148?\",\"evidence_hint\":\"payer_bv_response\"}")"
echo "$ASKED" | grep -q '"status":"answered"'
echo "$ASKED" | grep -q '"mode":"auto_payer"'
echo "$ASKED" | grep -qv '"jsonrpc"'

MSG_ID="$(printf '%s' "$ASKED" | sed -n 's/.*"msg_id":"\([^"]*\)".*/\1/p' | head -n1)"
EVIDENCE_ID="msg:${MSG_ID}"

rest POST /lab/bv/read_permitted_evidence \
  --data "{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"evidence_id\":\"$EVIDENCE_ID\"}" >/dev/null

REPORTED="$(rest POST /lab/bv/report_observations --data "{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"needs_human_review\":true,\"observations\":[{\"kind\":\"pa_requirement\",\"statement\":\"PA requirement remains unclear from payer language.\",\"uncertainty\":\"unknown\",\"evidence_refs\":[\"$EVIDENCE_ID\"]}]}")"
echo "$REPORTED" | grep -q '"accepted":true'

INSPECT="$(rest GET "/lab/runs/${RUN_ID}/inspect")"
echo "$INSPECT" | grep -q '"recorded_fact"'
echo "$INSPECT" | grep -q '"agent_claim"'
echo "$INSPECT" | grep -qv '"hidden_facts"'

TRACE="$(rest GET "/lab/runs/${RUN_ID}/trace")"
echo "$TRACE" | grep -q '"trace_version":"bv-tools-v1"'
echo "$TRACE" | grep -q 'read_assigned_context'
echo "$TRACE" | grep -q 'ask_payer'

SET_STAGE_CODE="$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:${PORT}/lab/bv/set_stage" \
  -H "Authorization: Bearer ${MINT_LAB_MCP_TOKEN}" \
  -H "Content-Type: application/json" \
  --data "{\"run_id\":\"$RUN_ID\",\"task_id\":\"$TASK_ID\",\"stage\":\"handoff\"}")"
if [[ "$SET_STAGE_CODE" != 4* ]]; then
  echo "FAIL — set_stage should be 4xx, got $SET_STAGE_CODE"
  exit 1
fi

UNAUTH_CODE="$(curl -s -o /dev/null -w '%{http_code}' -X GET "http://127.0.0.1:${PORT}/lab/openapi.json")"
if [[ "$UNAUTH_CODE" != "401" ]]; then
  echo "FAIL — OpenAPI without bearer should be 401, got $UNAUTH_CODE"
  exit 1
fi

echo "PASS — REST dual: five BV tools, inspect/trace, OpenAPI 3.1 (synthetic only)"
