#!/usr/bin/env bash
# Loom walkthrough: evidence-gated refund with AgentMint receipts.
#   ./loom_demo.sh            live: 2 fresh Parallel Task runs, progress streamed on screen (~1-2 min each)
#   ./loom_demo.sh --replay   rehearsal: reuse saved signed Parallel results (0 Task runs)
# Press Enter to advance each beat. Beat numbers match LOOM_SCRIPT.md.
# AUTO=1 advances on its own. SKIP_APPROVAL=1 skips the Stripe dashboard step.
# SILENT=1 shows the talking points on screen as captions (no voiceover needed).
set -uo pipefail
cd "$(dirname "$0")"
[ -x .venv/bin/python ] && export PATH="$PWD/.venv/bin:$PATH"   # created by setup_local.sh

LIVE=1; [ "${1:-}" = "--replay" ] && LIVE=0
USED=$(cat runs/.task_count 2>/dev/null || echo 0)
if [ "$LIVE" = 1 ] && [ $((10 - USED)) -lt 2 ]; then
  echo "Only $((10 - USED)) of 10 Parallel Task runs left; using --replay."; LIVE=0; sleep 2
fi
if [ "$LIVE" = 0 ] && ! grep -lq '"research:carrier_disruption"' receipts/*.json 2>/dev/null; then
  echo "Nothing to replay yet on this machine. Run ./loom_demo.sh once live first."; exit 1
fi
./mint_up.sh >/dev/null || exit 1
trap './mint_up.sh stop >/dev/null' EXIT
export LOOM=1 PYTHONUNBUFFERED=1
[ "$LIVE" = 1 ] || export DEMO_REPLAY=1
TOTAL=8
B=$'\033[1m'; D=$'\033[2m'; G=$'\033[32m'; R=$'\033[31m'; Y=$'\033[33m'; C=$'\033[36m'; N=$'\033[0m'

pause() { if [ "${AUTO:-0}" = 1 ]; then sleep 2; else read -rs _; fi; }
beat()  { clear; printf "\n  ${D}%s / %s${N}   ${B}%s${N}\n\n" "$1" "$TOTAL" "$2"; }
run()   { printf "  ${C}\$ %s${N}\n\n" "$*"; }
paint() { sed -e "s/AUTHORIZED/${G}${B}AUTHORIZED${N}/" -e "s/REFUSED/${R}${B}REFUSED${N}/" \
              -e "s/REFUND EXECUTED/${G}REFUND EXECUTED${N}/" -e "s/human approval required/${Y}human approval required${N}/" \
              -e "s/ valid/ ${G}valid${N}/" -e "s/ invalid/ ${R}invalid${N}/" -e "s/CHAIN VALID/${G}CHAIN VALID${N}/" \
              -e "s/^/  /"; }
# line-by-line coloring for live output (perl: same behaviour on macOS and Linux)
stream() { G="$G" B="$B" R="$R" Y="$Y" C="$C" N="$N" perl -ne 'BEGIN{$|=1} next if /^RECEIPTS/;
  s/AUTHORIZED/$ENV{G}$ENV{B}AUTHORIZED$ENV{N}/; s/REFUSED/$ENV{R}$ENV{B}REFUSED$ENV{N}/;
  s/REFUND EXECUTED/$ENV{G}REFUND EXECUTED$ENV{N}/; s/human approval required/$ENV{Y}human approval required$ENV{N}/;
  s/^(  parallel.*)/$ENV{C}$1$ENV{N}/; print "  $_";'; }
CAP=""
cap()    { CAP="$*"; }
endbeat() { if [ "${SILENT:-0}" = 1 ] && [ -n "$CAP" ]; then
              echo; printf '%s\n' "$CAP" | fold -s -w 92 | sed "s/^/  ${Y}${B}│${N} ${Y}/; s/\$/${N}/"; fi
            CAP=""; pause; }

# 1 ─ title
beat 1 "Evidence-gated side effects: Parallel → policy → AgentMint → mint.run → Stripe"
cat <<EOF
  1. Research   Parallel Task API (base), JSON output schema, per-field basis: citations + confidence
  2. Decide     pure policy over FieldBasis: disruption ∧ confidence∈{medium,high} ∧ ≥2 hostnames
  3. Authorize  AgentMint receipt: Ed25519, hash-chained, binding = H(pi | amount | H(run_id, content, basis))
  4. Gate       refund(receipt_path): pinned-key verify, allowed, recompute binding, else no call
  5. Execute    mint.run: intent hash (RFC 8785) includes the AgentMint receipt; one winner; reconcile
  6. Provider   Stripe test mode, \$12.99 partial refund on a \$100 PaymentIntent

  ${D}mode: $([ "$LIVE" = 1 ] && echo "live Parallel Task runs (used ${USED}/10 so far)" || echo "replaying signed Parallel results")${N}
EOF
cap "Refunds authorized by cited evidence, not by the model."
endbeat

# 2 ─ the policy
beat 2 "Policy: a pure function over Parallel's FieldBasis"
run "sed -n '/^def policy/,/^\$/p' demo.py"
sed -n '/^def policy/,/^$/p' demo.py | sed 's/^/  /'
echo; echo "  ${D}refund() only accepts a signed authorization receipt, verified before mint.run is called${N}"
cap "Disruption + confidence ≥ medium + 2 sites. Plain code, no LLM."
endbeat

# 3 ─ case A
beat 3 "Case A: Hurricane Ian, Fort Myers, 2022-09-28"
run "python3 demo.py holds"
LOG_A=$(mktemp); python3 demo.py holds 2>&1 | tee "$LOG_A" | stream
AUTH_A=$(awk '/^RECEIPTS:/{print $3}' "$LOG_A")
if grep -q AUTHORIZED "$LOG_A"; then
  cap "Authorized. mint.run sent it. Stripe holds it, so status is Unknown."
else
  cap "Not enough evidence this run. Signed refusal. No Mint action."
fi
endbeat

# 4 ─ human approval in Stripe, then retry
beat 4 "Human approval at Stripe, then mint.run reconciles"
if [ "${SKIP_APPROVAL:-0}" != 1 ] && [ -n "$AUTH_A" ]; then
  echo "  ${Y}Approve the pending \$12.99 refund in the Stripe dashboard now.${N}"
  echo "  ${D}Press Enter when it's approved.${N}"; pause; echo
fi
if [ -n "$AUTH_A" ]; then
  run "python3 retry_refund.py receipts/${AUTH_A:0:8}…json"
  LOG_R=$(mktemp); python3 retry_refund.py "receipts/$AUTH_A.json" 2>&1 | tee "$LOG_R" | grep -v '^RECEIPTS' | paint
else
  echo "  ${R}Case A produced no authorization receipt; skipping.${N}"
fi
if grep -q "refused by policy" "${LOG_R:-/dev/null}"; then
  cap "allowed=false, so refund() stops before mint.run."
elif grep -q ": 1 (\$12.99)" "${LOG_R:-/dev/null}"; then
  cap "Reconciled: Unknown → Succeeded. One \$12.99 refund."
else
  cap "Still held at Stripe. Status stays Unknown."
fi
endbeat

# 5 ─ case B
beat 5 "Case B: Phoenix, 2024-05-14 (no real disruption)"
run "python3 demo.py fails"
python3 demo.py fails 2>&1 | stream
cap "disruption=false, still cited. Signed refusal. Zero Stripe requests."
endbeat

# 6 ─ what got signed
beat 6 "Signed artifacts: AgentMint authorization + mint.run result"
run "cat receipts/${AUTH_A:0:8}…json"
python3 - "receipts/$AUTH_A.json" <<'PY' | sed 's/^/  /'
import json, sys
d = json.load(open(sys.argv[1])); r = d["evidence"]["result"]
print(f"action            {d['action']}")
print(f"allowed           {r['allowed']}")
print(f"payment           {r['pi']}")
print(f"amount            {r['amount']}  (cents)")
print(f"evidence_sha256   {r['evidence_sha256'][:32]}…   <- hash of the Parallel run_id + content + basis")
print(f"binding           {r['binding'][:32]}…   <- payment|amount|evidence; also the Stripe idempotency key")
print(f"sources           {', '.join(r['why']['independent_sources'])}  (confidence {r['why']['confidence']})")
print(f"previous_receipt  {str(d.get('previous_receipt_hash'))[:32]}…   <- hash chain")
print(f"signature         {d['signature'][:32]}…   (Ed25519)")
PY
MR=$(python3 -c "import json,sys; m=json.load(open('runs/mint_actions.json')); b=json.load(open(sys.argv[1]))['evidence']['result']['binding']; print(m.get(b,''))" "receipts/$AUTH_A.json" 2>/dev/null)
if [ -n "$MR" ] && [ -f "mint_receipts/$MR.json" ]; then
  echo; echo "  ${B}mint.run receipt for the refund${N}"
  python3 - "mint_receipts/$MR.json" <<'PY' | sed 's/^/  /'
import json, sys
p = json.load(open(sys.argv[1]))["payload"]
print(f"status            {p['status']}   refund {p.get('provider_resource_id')}")
print(f"intent_hash       {p['intent_hash'][:40]}…   <- includes the AgentMint receipt id + binding")
print(f"idempotency_key   {p['provider_idempotency_key']}")
print(f"reconciled        {p['reconciliation_required']}   (outcome was Unknown until Stripe approval)")
PY
fi
cap "AgentMint signs the decision. mint.run signs the refund, bound to it."
endbeat

# 7 ─ verify
beat 7 "Offline verification"
run "agentmint verify receipts/"
agentmint --no-color verify receipts/ 2>&1 | grep -E "valid|invalid" | sed 's/ \/.*$//' | tail -n 5 | paint
n=$(agentmint --no-color verify receipts/ 2>&1 | grep -c " valid"); echo "  ${D}… ${n} receipts checked${N}"; echo
run "python3 verify_chain_ordered.py receipts"
python3 verify_chain_ordered.py receipts 2>&1 | tail -n 1 | paint
if [ -n "${MR:-}" ] && [ -f "mint_receipts/$MR.json" ]; then
  echo; run "mint verify --receipt mint_receipts/${MR:0:8}…json --keys-url …/v1/keys"
  ../target/debug/mint verify --receipt "mint_receipts/$MR.json" --keys-url "http://127.0.0.1:${MINT_PORT:-8787}/v1/keys" 2>&1 | paint
fi
cap "Verified offline with public keys."
endbeat

# 8 ─ tamper
beat 8 "Tamper: one byte, rejected at the gate"
TMP=$(mktemp -d); cp "receipts/$AUTH_A.json" "$TMP/"
python3 - "$TMP/$AUTH_A.json" <<'PY'
import sys; p = sys.argv[1]; s = open(p).read()
open(p, "w").write(s.replace('"amount": 1299', '"amount": 9299', 1))
PY
run "diff original tampered"
diff "receipts/$AUTH_A.json" "$TMP/$AUTH_A.json" | sed 's/^/  /'; echo
run "agentmint verify tampered/${AUTH_A:0:8}…json"
agentmint --no-color verify "$TMP/$AUTH_A.json" 2>&1 | paint; echo
run "refund(tampered/${AUTH_A:0:8}…json)"
python3 -c "import sys, demo; print('attempt:', demo.fmt(demo.refund(sys.argv[1])))" "$TMP/$AUTH_A.json" 2>&1 \
  | sed -e "s/BLOCKED by AgentMint/${R}${B}BLOCKED by AgentMint${N}/" -e "s/^/  /"
rm -rf "$TMP"
echo; echo "  ${D}github.com/aerf-spec/agentmint-python  ·  docs.parallel.ai${N}"; echo
cap "One byte changed. Blocked before mint.run or Stripe."
[ "${SILENT:-0}" = 1 ] && endbeat
