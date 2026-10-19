#!/usr/bin/env bash
# One-time setup on a fresh clone (macOS or Linux). Safe to re-run.
# - makes a local venv and installs parallel-web, stripe, agentmint (from GitHub)
# - asks for your Parallel and Stripe test keys if .env is missing (input hidden, saved to gitignored .env)
# - if this machine has no AgentMint signing key: archives the committed receipts with the public key
#   that signed them (they still verify), then creates a fresh key and receipt store for your recording
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -x .venv/bin/python ]; then
  echo "Creating .venv and installing dependencies…"
  python3 -m venv .venv
  .venv/bin/python -m pip install --quiet --upgrade pip
  .venv/bin/python -m pip install --quiet parallel-web stripe "git+https://github.com/aerf-spec/agentmint-python"
fi

if [ ! -f .env ]; then
  read -rsp "PARALLEL_API_KEY: " PK; echo
  read -rsp "STRIPE_KEY (rk_test_… or sk_test_…): " SK; echo
  case "$SK" in rk_test_*|sk_test_*) ;; *) echo "Stripe key must be a test-mode key (rk_test_ or sk_test_)."; exit 1;; esac
  ( umask 077; printf 'PARALLEL_API_KEY=%s\nSTRIPE_KEY=%s\n' "$PK" "$SK" > .env )
  echo "Saved keys to .env (gitignored)."
fi

if [ ! -f .agentmint/keys/signing_key.bin ]; then
  ARCH="archive/$(date +%Y%m%d-%H%M%S)"
  echo "No local signing key. Archiving committed receipts to $ARCH/ and starting a fresh receipt store."
  mkdir -p "$ARCH"
  [ -d receipts ] && mv receipts "$ARCH/receipts"
  [ -f .agentmint/keys/public_key.pem ] && cp .agentmint/keys/public_key.pem "$ARCH/public_key.pem"
  rm -rf .agentmint
  cp .gitignore .gitignore.bak
  .venv/bin/agentmint init -y . >/dev/null
  mv .gitignore.bak .gitignore          # init appends lines; keep ours
  mkdir -p receipts
  echo "Archived receipts still verify:  .venv/bin/agentmint verify $ARCH/receipts"
fi

echo "Ready. Parallel Task runs used so far: $(cat runs/.task_count 2>/dev/null || echo 0)/10"
