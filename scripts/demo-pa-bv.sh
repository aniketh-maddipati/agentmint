#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

export MINT_LAB_DIR="$TMP/lab-data"
export MINT_LAB_FIXTURES="$ROOT/fixtures/pa_bv"

echo "PA/BV lab demo — approval path (synthetic CPT 72148)"
echo "data dir: $MINT_LAB_DIR"

cargo build --quiet

OUT="$(cargo run --quiet -- lab run approval --dir "$MINT_LAB_DIR" --json)"
echo "$OUT"

STAGE="$(printf '%s' "$OUT" | sed -n 's/.*"stage"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
DISP="$(printf '%s' "$OUT" | sed -n 's/.*"disposition"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"

if [[ "$STAGE" != "handoff" ]]; then
  echo "FAIL — expected stage handoff, got: $STAGE"
  exit 1
fi

if [[ "$DISP" != "approved_handoff" ]]; then
  echo "FAIL — expected disposition approved_handoff, got: $DISP"
  exit 1
fi

RUN_ID="$(printf '%s' "$OUT" | sed -n 's/.*"run_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
cargo run --quiet -- lab check "$RUN_ID" --dir "$MINT_LAB_DIR"

echo "PASS — PA/BV approval path: BV→docs→review→submit→decision→handoff (synthetic only)"
