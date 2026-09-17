# mint.run Development Standards

## Product
mint.run is the transaction runtime for consequential AI-agent side effects.
The agent proposes a tool call. Mint binds authorization to that exact call,
executes it once internally, reconciles uncertain outcomes, and emits a signed receipt.

## Standards
- No .unwrap() in runtime code — use ?
- No duplication — extract shared logic
- One function does one thing
- Early returns, no deep nesting
- Names tell the story
- Fail closed on identity, policy, hash, tenant, state, or credential checks
- Do not hold a SQLite transaction or global mutex across a provider await
- Live Stripe secrets are refused

## Commands
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm test --prefix sdk/typescript
./scripts/demo-local.sh
./scripts/test-stripe-sandbox.sh
