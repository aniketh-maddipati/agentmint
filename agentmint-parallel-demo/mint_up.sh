#!/usr/bin/env bash
# Start (or stop) a local mint.run server for the demo. Usage: ./mint_up.sh [stop]
# Builds the Rust binary from this repo, keeps its signing key + SQLite DB in .mint/ (gitignored),
# and passes STRIPE_KEY from .env as MINT_STRIPE_TEST_SECRET_KEY (test keys only; never printed).
set -euo pipefail
cd "$(dirname "$0")"
DIR="$PWD/.mint"; mkdir -p "$DIR"; PIDF="$DIR/server.pid"
PORT="${MINT_PORT:-8787}"

if [ "${1:-}" = "stop" ]; then
  [ -f "$PIDF" ] && kill "$(cat "$PIDF")" 2>/dev/null && echo "mint.run stopped" || echo "mint.run not running"
  rm -f "$PIDF"; exit 0
fi
if [ -f "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; then echo "mint.run already running on :$PORT"; exit 0; fi

command -v cargo >/dev/null || { echo "cargo not found. Install Rust: https://rustup.rs"; exit 1; }
( cd .. && cargo build -q --bin mint )
BIN="$(cd .. && pwd)/target/debug/mint"

SK="$(sed -n 's/^STRIPE_KEY=//p; s/^STRIPE_SECRET_KEY=//p' .env 2>/dev/null | head -1 | tr -d '[:space:]')"
SK="${SK:-${STRIPE_KEY:-${STRIPE_SECRET_KEY:-}}}"
case "$SK" in rk_test_*|sk_test_*) ;; *) echo "Need a Stripe test key (rk_test_/sk_test_) in .env"; exit 1;; esac

export MINT_MODE=development MINT_IDENTITY=local MINT_PROVIDER=stripe MINT_KID=mint-demo-1 \
       MINT_BIND_ADDR="127.0.0.1:$PORT" MINT_DATABASE_PATH="$DIR/mint.db" MINT_SIGNING_KEY_FILE="$DIR/mint.ed25519.pem" \
       MINT_STRIPE_TEST_SECRET_KEY="$SK"
[ -f "$MINT_SIGNING_KEY_FILE" ] || "$BIN" init --key-file "$MINT_SIGNING_KEY_FILE" >/dev/null
nohup "$BIN" serve >"$DIR/server.log" 2>&1 &
echo $! >"$PIDF"
for _ in $(seq 1 100); do curl -sf "http://127.0.0.1:$PORT/health" >/dev/null && break; sleep 0.1; done
curl -sf "http://127.0.0.1:$PORT/health" >/dev/null || { echo "mint.run failed to start; see .mint/server.log"; exit 1; }
curl -sf "http://127.0.0.1:$PORT/v1/keys" >"$DIR/keys.json"
echo "mint.run up on http://127.0.0.1:$PORT (provider=stripe, test mode)"
