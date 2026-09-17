# mint.run Development Standards

mint.run is the transaction runtime for consequential AI-agent side effects.

See README.md for product scope, guarantees, and non-guarantees.

## Commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm test --prefix sdk/typescript
./scripts/demo-local.sh
./scripts/test-stripe-sandbox.sh
```
