#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

run() {
  echo "==> $*"
  if ! "$@"; then
    echo "FAIL — $*"
    exit 1
  fi
}

run cargo fmt --check
run cargo clippy --all-targets --all-features -- -D warnings
run cargo test
run npm test --prefix sdk/typescript
run cargo build --release
run ./scripts/demo-local.sh

echo "PASS — launch-check"
