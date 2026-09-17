#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "${MINT_RUN_STRIPE_E2E:-}" != "1" ]]; then
  echo "SKIPPED — credential not provided / opt-in flag missing"
  echo "Stripe sandbox transactions move no real funds."
  echo
  echo "Run:"
  echo "  export MINT_STRIPE_TEST_SECRET_KEY='sk_test_...'"
  echo "  export MINT_RUN_STRIPE_E2E=1"
  echo "  cargo run -- doctor"
  echo "  $0"
  exit 1
fi

if [[ -z "${MINT_STRIPE_TEST_SECRET_KEY:-}" ]]; then
  echo "SKIPPED — credential not provided"
  echo "MINT_STRIPE_TEST_SECRET_KEY is required and must start with sk_test_ or rk_test_."
  exit 1
fi

if [[ "${MINT_STRIPE_TEST_SECRET_KEY}" == sk_live_* || "${MINT_STRIPE_TEST_SECRET_KEY}" == rk_live_* ]]; then
  echo "FAIL — live Stripe credentials refused"
  exit 1
fi

export MINT_PROVIDER=stripe
export MINT_MODE=development
export MINT_IDENTITY=local

echo "Running mint doctor (Stripe readiness)..."
cargo run -q -- doctor

echo "Running ignored Stripe sandbox test..."
cargo test --test stripe_sandbox -- --ignored --nocapture
echo "PASS — Stripe sandbox test finished"
