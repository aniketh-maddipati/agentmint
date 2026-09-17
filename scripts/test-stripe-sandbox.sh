#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HEAD_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
echo "Tree: ${HEAD_SHA}"

if grep -n 'stripe_refund\["livemode"\]' tests/stripe_sandbox.rs >/dev/null 2>&1; then
  echo "FAIL — stale tests/stripe_sandbox.rs still asserts Refund.livemode (Null)"
  echo "Stripe Refund has no livemode; test mode is asserted on Charge."
  echo "Your panic at Null == false is this stale assertion."
  echo "Pull the fixed tree, then rerun:"
  echo "  git fetch origin cursor/mint-run-poc-0c0c"
  echo "  git checkout cursor/mint-run-poc-0c0c"
  echo "  git pull --ff-only origin cursor/mint-run-poc-0c0c"
  echo "  $0"
  exit 1
fi

if ! grep -n 'retrieve_charge' tests/stripe_sandbox.rs >/dev/null 2>&1; then
  echo "FAIL — tests/stripe_sandbox.rs is missing Charge.livemode check via retrieve_charge"
  echo "Pull the fixed tree (need ff2cc48 / a2da7a8 or newer), then rerun."
  exit 1
fi

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
