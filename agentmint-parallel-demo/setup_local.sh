#!/usr/bin/env bash
# One-time setup on a fresh clone (macOS or Linux). Safe to re-run.
# - makes a local venv and installs parallel-web, stripe, agentmint (from GitHub)
# - asks for your Parallel and Stripe test keys if .env is missing (input hidden, saved to gitignored .env)
# - if this machine has no AgentMint signing key: archives the committed receipts with the public key
#   that signed them (they still verify), then creates a fresh key and receipt store for your recording
set -euo pipefail
cd "$(dirname "$0")"

# The agent needs anthropic>=2 and mcp>=2, which need Python 3.10+ (macOS's default python3 is often 3.9).
PY=""
for p in python3.13 python3.12 python3.11 python3.10 python3; do
  if command -v "$p" >/dev/null && "$p" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then PY="$p"; break; fi
done
[ -n "$PY" ] || { echo "Python 3.10+ is required. On macOS: brew install python@3.12  (then re-run)"; exit 1; }
if [ -x .venv/bin/python ] && ! .venv/bin/python -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)'; then
  echo "Existing .venv uses Python < 3.10; recreating it with $PY."; rm -rf .venv
fi
if [ ! -x .venv/bin/python ] || ! .venv/bin/python -c "import anthropic, mcp; from mcp import Client; from mcp.server.mcpserver import MCPServer" 2>/dev/null; then
  echo "Creating .venv ($($PY --version)) and installing dependencies…"
  [ -x .venv/bin/python ] || "$PY" -m venv .venv
  .venv/bin/python -m pip install --quiet --upgrade pip
  .venv/bin/python -m pip install --quiet parallel-web stripe "anthropic>=2" "mcp>=2" "git+https://github.com/aerf-spec/agentmint-python"
fi

command -v cargo >/dev/null || { echo "Rust is required for mint.run. Install it from https://rustup.rs, then re-run."; exit 1; }
echo "Building mint.run…"; ( cd .. && cargo build -q --bin mint )

if [ ! -f .env ]; then
  read -rsp "PARALLEL_API_KEY: " PK; echo
  read -rsp "STRIPE_KEY (rk_test_… or sk_test_…): " SK; echo
  case "$SK" in rk_test_*|sk_test_*) ;; *) echo "Stripe key must be a test-mode key (rk_test_ or sk_test_)."; exit 1;; esac
  ( umask 077; printf 'PARALLEL_API_KEY=%s\nSTRIPE_KEY=%s\n' "$PK" "$SK" > .env )
  echo "Saved keys to .env (gitignored)."
fi
if ! grep -q '^ANTHROPIC_API_KEY=' .env && [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  read -rsp "ANTHROPIC_API_KEY (for the Claude agent): " AK; echo
  ( umask 077; printf 'ANTHROPIC_API_KEY=%s\n' "$AK" >> .env ); echo "Added ANTHROPIC_API_KEY to .env."
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
