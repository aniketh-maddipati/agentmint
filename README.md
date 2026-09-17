# mint.run

mint.run makes consequential agent actions safe to retry.

The agent proposes an exact refund. Mint binds authorization to that exact intent, allows only one execution winner, calls Stripe with a stable idempotency key, reconciles ambiguous results, and returns a signed receipt.

```text
agent
  -> exact refund intent
  -> identity + policy
  -> atomic execution claim
  -> Stripe
  -> signed receipt
```

**Status:** experimental proof of concept. Not production-ready.  
Repo name may still be `agentmint`. Product / crate / binary: `mint.run` / `mint-run` / `mint`.

## Why

Agents retry. Networks fail. Approvals get reused. Mint’s job is to make the *exact authorized refund* safe under those conditions: one internal execution winner, a durable attempt, a stable Stripe idempotency key, and a signed receipt of what Mint recorded.

## What this MVP supports

- Stripe `refund.create` against a Charge or PaymentIntent (test mode only)
- Threshold policy: ≤5000¢ automatic, 5001–50000¢ separate-principal approval, >50000¢ denied
- Local fake provider for offline demos
- Local-dev identity or generic JWT/OIDC
- SQLite storage, Ed25519 receipts, RFC 8785 intent hashing

It is not an agent framework, IdP, secret manager, policy language, or workflow engine.

## Quickstart with the fake provider

```bash
git clone https://github.com/aniketh-maddipati/agentmint.git
cd agentmint
git checkout cursor/mint-run-poc-0c0c

cargo build
cargo run -- init
export MINT_SIGNING_KEY_FILE=mint.ed25519.pem
export MINT_PROVIDER=fake
export MINT_MODE=development
cargo run -- doctor
./scripts/demo-local.sh
```

## Test it yourself

```bash
git clone https://github.com/aniketh-maddipati/agentmint.git
cd agentmint
git checkout cursor/mint-run-poc-0c0c

cargo build
cargo run -- init
cargo run -- doctor
./scripts/demo-local.sh

cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm test --prefix sdk/typescript
cargo build --release
```

Default `cargo test` stays offline. The Stripe sandbox test is `#[ignore]` and is not a silent pass when skipped.

## Stripe test-mode proof

Uses Stripe **test mode only**. Sandbox transactions move **no real money**.

> Never paste a live Stripe key into mint.run. This MVP intentionally refuses live credentials.

```bash
export MINT_STRIPE_TEST_SECRET_KEY='sk_test_...'
export MINT_RUN_STRIPE_E2E=1
export MINT_PROVIDER=stripe
export MINT_SIGNING_KEY_FILE=mint.ed25519.pem   # after `mint init`

cargo run -- doctor
./scripts/test-stripe-sandbox.sh
```

Successful output ends with `STRIPE_SANDBOX: PASS` and `PASS — Stripe sandbox test finished`.

Then verify in the Stripe Dashboard (test mode):

- one refund exists for the charge
- refund amount matches (sandbox uses a distinctive amount)
- test mode is active (`livemode=false`)
- expected charge was refunded
- Mint action ID is present in refund metadata
- retries did not create additional refunds
- reconciliation returned the same refund ID

**Stripe status:** Stripe integration is implemented but has not yet been validated against a real Stripe test account in this environment (`SKIPPED — credential not provided`).

Refund POST fields: `amount`, `reason`, `charge` or `payment_intent`, metadata. Currency is validated by retrieving the Charge/PaymentIntent first; it is **not** sent on create-refund (Stripe rejects it).

## API example

```typescript
import { Mint, encodeDevToken } from "./sdk/typescript/src/index.ts";

const mint = new Mint({
  baseUrl: "http://127.0.0.1:8787",
  token: encodeDevToken({
    tenantId: "acme",
    subject: "user_123",
    agentId: "support-agent-7",
    issuer: "https://identity.example.com",
  }),
});

const action = await mint.actions.propose({
  tenantId: "acme",
  actor: {
    subject: "user_123",
    agentId: "support-agent-7",
    issuer: "https://identity.example.com",
  },
  provider: "fake", // or "stripe" with test credentials
  operation: "refund.create",
  resource: { type: "charge", id: "ch_123" },
  arguments: { amount: 4200, currency: "usd", reason: "duplicate" },
  context: { supportTicketId: "ticket_982" },
});

const result = await mint.actions.execute(action.id);
```

Endpoints: `POST /v1/actions`, `GET /v1/actions/{id}`, `POST .../approve|deny|execute|reconcile`, `GET .../receipt`, `GET /v1/keys`, `GET /health`.

**Approval:** amounts in the approval band require a **separate authenticated principal** in the same tenant (approver subject ≠ originating actor subject). This is authenticated / separate-principal approval, not proof of a human identity.

Unknown `MINT_MODE` / `MINT_PROVIDER` / `MINT_IDENTITY` / `MINT_POLICY` values fail startup (no silent fake fallback).

## Safety model

- Material intent is RFC 8785–canonicalized and SHA-256 hashed; hash is rechecked before execution
- Only one process wins `Authorized → Executing`
- Durable attempt is written before contacting Stripe
- Provider idempotency key is `mint:{action_id}:refund.create:v1`
- Ambiguous provider outcomes become `Unknown` and require reconcile
- Live Stripe credentials (`sk_live_` / `rk_live_`) are refused
- Secrets never appear in logs, receipts, SQLite, or `mint doctor` output

Mint provides **one internal execution winner**, stable provider idempotency, and reconciliation. It does not claim universal exactly-once external effects.

## Threat model and limits

**Protects against:** retries and concurrent execution; post-authorization argument changes; process failure around provider calls; cross-tenant action lookup; receipt tampering; accidental live Stripe credentials.

**Does not prove or protect against:** Stripe’s internal correctness; a compromised host or signing key; a malicious/misconfigured external PDP; a human identity behind an authenticated approval; agent behavior outside Mint; mathematically perfect exactly-once delivery.

## Development checks

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm test --prefix sdk/typescript
cargo build --release
cargo run -- doctor
./scripts/demo-local.sh
```

GitHub Actions runs the offline checks above. Stripe is never called in default CI.

Docker: `Dockerfile` builds `mint`. Mount/pass signing key, SQLite path, and `MINT_PROVIDER=fake` for local use. Image build was **not executed** in the agent environment (`docker` unavailable).

## Status

| Area | Label |
|---|---|
| Offline tests, fake demo, fmt/clippy, TS client, release build | implemented and covered by offline tests |
| Stripe sandbox against a real test account | implemented but not yet independently verified |
| OIDC against a real issuer / external PDP | implemented but not yet independently verified |
| Docker image run | NOT RUN — environment unavailable |
| Production readiness | experimental — not production-ready |
