#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "${MINT_RUN_STRIPE_E2E:-}" != "1" ]]; then
  echo "This script is opt-in and talks to Stripe's sandbox."
  echo "Stripe sandbox transactions move no real funds."
  echo
  echo "Run:"
  echo "  MINT_STRIPE_TEST_SECRET_KEY=sk_test_... MINT_RUN_STRIPE_E2E=1 $0"
  exit 1
fi

if [[ -z "${MINT_STRIPE_TEST_SECRET_KEY:-}" ]]; then
  echo "MINT_STRIPE_TEST_SECRET_KEY is required and must start with sk_test_ or rk_test_."
  exit 1
fi

if [[ "${MINT_STRIPE_TEST_SECRET_KEY}" == sk_live_* || "${MINT_STRIPE_TEST_SECRET_KEY}" == rk_live_* ]]; then
  echo "Refusing live Stripe credentials."
  exit 1
fi

cargo test --test stripe_sandbox -- --nocapture
