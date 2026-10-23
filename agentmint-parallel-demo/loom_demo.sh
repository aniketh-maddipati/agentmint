#!/usr/bin/env bash
# Loom walkthrough: a Claude support agent asks for a refund; evidence + AgentMint + mint.run decide.
#   ./loom_demo.sh            live: 1 fresh Parallel Task run, progress streamed on screen (~1 min)
#   ./loom_demo.sh --replay   rehearsal: reuse the saved signed Parallel result (0 Task runs)
# Press Enter to advance each beat. Beat numbers match LOOM_SCRIPT.md.
# AUTO=1 advances on its own. SKIP_APPROVAL=1 skips the Stripe dashboard step.
# SILENT=1 shows short captions on screen (no voiceover needed).
set -uo pipefail
cd "$(dirname "$0")"
[ -x .venv/bin/python ] && export PATH="$PWD/.venv/bin:$PATH"   # created by setup_local.sh

LIVE=1; [ "${1:-}" = "--replay" ] && LIVE=0
USED=$(cat runs/.task_count 2>/dev/null || echo 0)
if [ "$LIVE" = 1 ] && [ $((10 - USED)) -lt 1 ]; then
  echo "No Parallel Task runs left in the 10-run budget; using --replay."; LIVE=0; sleep 2
fi
if [ "$LIVE" = 0 ] && ! grep -lq 'Memphis, Tennessee' receipts/*.json 2>/dev/null; then
  echo "Nothing to replay yet on this machine. Run ./loom_demo.sh once live first."; exit 1
fi
grep -q '^ANTHROPIC_API_KEY=' .env 2>/dev/null || [ -n "${ANTHROPIC_API_KEY:-}" ] || { echo "Add ANTHROPIC_API_KEY to .env (or run ./setup_local.sh)."; exit 1; }
./mint_up.sh >/dev/null || exit 1
trap './mint_up.sh stop >/dev/null' EXIT
export LOOM=1 PYTHONUNBUFFERED=1
[ "$LIVE" = 1 ] || export DEMO_REPLAY=1
TOTAL=5
B=$'\033[1m'; D=$'\033[2m'; G=$'\033[32m'; R=$'\033[31m'; Y=$'\033[33m'; C=$'\033[36m'; M=$'\033[35m'; N=$'\033[0m'

pause() { if [ "${AUTO:-0}" = 1 ]; then sleep 2; else read -rs _; fi; }
beat()  { clear; printf "\n  ${D}%s / %s${N}   ${B}%s${N}\n\n" "$1" "$TOTAL" "$2"; }
run()   { printf "  ${C}\$ %s${N}\n\n" "$*"; }
# line-by-line coloring (perl: same behaviour on macOS and Linux)
stream() { G="$G" B="$B" R="$R" Y="$Y" C="$C" M="$M" N="$N" perl -ne 'BEGIN{$|=1} next if /^RECEIPTS/;
  s/^(customer:)/$ENV{B}$1$ENV{N}/; s/^(agent:)/$ENV{M}$ENV{B}$1$ENV{N}/; s/^(agent → tool:|tool → agent:)/$ENV{M}$1$ENV{N}/;
  s/AUTHORIZED/$ENV{G}$ENV{B}AUTHORIZED$ENV{N}/; s/REFUSED/$ENV{R}$ENV{B}REFUSED$ENV{N}/;
  s/REFUND EXECUTED/$ENV{G}REFUND EXECUTED$ENV{N}/; s/human approval required/$ENV{Y}human approval required$ENV{N}/;
  s/BLOCKED by AgentMint/$ENV{R}$ENV{B}BLOCKED by AgentMint$ENV{N}/;
  s/ valid/ $ENV{G}valid$ENV{N}/; s/ invalid/ $ENV{R}invalid$ENV{N}/; s/CHAIN VALID/$ENV{G}CHAIN VALID$ENV{N}/;
  s/^(  parallel.*)/$ENV{C}$1$ENV{N}/; print "  $_";'; }
CAP=""
cap()    { CAP="$*"; }
endbeat() { if [ "${SILENT:-0}" = 1 ] && [ -n "$CAP" ]; then
              echo; printf '%s\n' "$CAP" | fold -s -w 92 | sed "s/^/  ${Y}${B}│${N} ${Y}/; s/\$/${N}/"; fi
            CAP=""; pause; }

# 1 ─ title
beat 1 "A support agent that can ask for a refund, but can't grant one"
cat <<EOF
  Claude      reads the customer message, calls 2 MCP tools
  Parallel    finds and cites the evidence (Task API)
  Policy      decides in plain code
  AgentMint   signs it and gates the refund
  mint.run    runs the Stripe refund once

  ${D}mode: $([ "$LIVE" = 1 ] && echo "live Parallel Task run (used ${USED}/10 so far)" || echo "replaying the signed Parallel result")${N}
EOF
cap "The agent asks. Evidence and policy decide."
endbeat

# 2 ─ the agent run
beat 2 "Order #1042: late because of the snowstorm"
run "python3 agent.py"
LOG=$(mktemp); python3 agent.py 2>&1 | tee "$LOG" | stream
AUTH=$(awk '/^RECEIPTS:/{print $3}' "$LOG" | tail -1)
if [ -z "$AUTH" ]; then
  echo; echo "  ${R}${B}The agent run did not finish, so there is nothing to show in beats 3-5.${N}"
  echo "  ${D}See the error above. Full log: $LOG${N}"; exit 1
fi
if grep -q "AUTHORIZED" "$LOG"; then
  cap "Parallel found it, policy passed, mint.run sent it. Stripe holds it for a human: Unknown."
else
  cap "Not enough evidence this run. Signed refusal. Stripe never called."
fi
endbeat

# 3 ─ human approval in Stripe, then reconcile
beat 3 "Approve in Stripe, then the retry reconciles"
if grep -q "AUTHORIZED" "$LOG" && [ "${SKIP_APPROVAL:-0}" != 1 ]; then
  echo "  ${Y}Approve the pending \$12.99 refund in the Stripe dashboard now.${N}"
  echo "  ${D}Press Enter when it's approved.${N}"; pause; echo
fi
if [ -n "$AUTH" ]; then
  run "python3 retry_refund.py receipts/${AUTH:0:8}…json"
  LOG_R=$(mktemp); python3 retry_refund.py "receipts/$AUTH.json" 2>&1 | tee "$LOG_R" | grep -v '^RECEIPTS' | stream
fi
if grep -q "refused by policy" "${LOG_R:-/dev/null}"; then cap "Refused authorization: refund() stops before mint.run."
elif grep -q ": 1 (\$12.99)" "${LOG_R:-/dev/null}"; then cap "Unknown → Succeeded. One \$12.99 refund."
else cap "Still held at Stripe. Status stays Unknown."; fi
endbeat

# 4 ─ signed + verified
beat 4 "Signed, chained, verifiable"
[ -n "$AUTH" ] && python3 - "receipts/$AUTH.json" <<'PY' | sed 's/^/  /'
import json, sys
d = json.load(open(sys.argv[1])); r = d["evidence"]["result"]
print(f"AgentMint  {d['action']}  allowed={r['allowed']}  amount={r['amount']}")
print(f"           evidence {r['evidence_sha256'][:24]}…  binding {r['binding'][:24]}…  sig {d['signature'][:16]}…")
PY
MR=$(python3 -c "import json,sys; m=json.load(open('runs/mint_actions.json')); b=json.load(open(sys.argv[1]))['evidence']['result']['binding']; print(m.get(b,''))" "receipts/$AUTH.json" 2>/dev/null)
if [ -n "$MR" ] && [ -f "mint_receipts/$MR.json" ]; then
  python3 - "mint_receipts/$MR.json" <<'PY' | sed 's/^/  /'
import json, sys
p = json.load(open(sys.argv[1]))["payload"]
print(f"mint.run   {p['status']}  refund {p.get('provider_resource_id')}  intent {p['intent_hash'][:30]}…")
PY
fi
echo
n=$(agentmint --no-color verify receipts/ 2>&1 | grep -c " valid"); bad=$(agentmint --no-color verify receipts/ 2>&1 | grep -c " invalid")
echo "agentmint verify receipts/   ${n} valid, ${bad} invalid" | stream
python3 verify_chain_ordered.py receipts 2>&1 | tail -n 1 | stream
if [ -n "$MR" ] && [ -f "mint_receipts/$MR.json" ]; then
  ../target/debug/mint verify --receipt "mint_receipts/$MR.json" --keys-url "http://127.0.0.1:${MINT_PORT:-8787}/v1/keys" 2>&1 | stream
fi
cap "Every step signed and chained. Checked with public keys only."
endbeat

# 5 ─ tamper
beat 5 "Change one byte: blocked before mint.run or Stripe"
if [ -n "$AUTH" ]; then
  TMP=$(mktemp -d); cp "receipts/$AUTH.json" "$TMP/"
  python3 - "$TMP/$AUTH.json" <<'PY'
import sys; p = sys.argv[1]; s = open(p).read()
open(p, "w").write(s.replace('"amount": 1299', '"amount": 9299', 1))
PY
  diff "receipts/$AUTH.json" "$TMP/$AUTH.json" | sed 's/^/  /'; echo
  run "refund(tampered/${AUTH:0:8}…json)"
  python3 -c "import sys, demo; print('result:', demo.fmt(demo.refund(sys.argv[1])))" "$TMP/$AUTH.json" 2>&1 | stream
  rm -rf "$TMP"
fi
cap "One byte changed. AgentMint rejects it before mint.run or Stripe."
[ "${SILENT:-0}" = 1 ] && endbeat
echo
