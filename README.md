# mint.run

Turn an agent tool call into a safe, recoverable, verifiable transaction.

Frameworks run the agent. Identity systems authorize access. Mint safely commits the side effect.

**Status:** experimental proof of concept. Not production-ready.

The GitHub repository may still be named `agentmint`. The product, crate, and binary are `mint.run` / `mint-run` / `mint`.

## What problem it solves

The agent or framework proposes an action. Identity and policy systems determine who is acting and whether the action is permitted. Mint binds that permission to the exact action, safely executes it, reconciles uncertain outcomes, and emits a signed provider-linked receipt.

> The agent proposes a tool call. Mint makes sure the exact authorized call executes safely once and records what actually happened.

Mint is approximately:

```text
database transaction
+ idempotency coordinator
+ exact approval binding
+ provider-specific recovery
+ signed execution receipt
```

This MVP implements one vertical slice: a support agent proposing a partial Stripe refund associated with a support ticket.

## Five-minute local fake-provider demo

No Stripe credentials are required. The deterministic fake provider never moves money.

```bash
git clone https://github.com/aniketh-maddipati/agentmint
cd agentmint
cargo build
./scripts/demo-local.sh
```

What the demo does:

1. Generates a local Ed25519 key with `mint init` (never implicit on startup).
2. Starts `mint serve` with `MINT_PROVIDER=fake`.
3. Proposes a 4,200-cent refund for `ticket_982`.
4. Executes it once.
5. Verifies the signed receipt with `mint verify`.
6. Exercises the TypeScript client against the same API.

Equivalent TypeScript:

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
  provider: "fake",
  operation: "refund.create",
  resource: { type: "charge", id: "ch_123" },
  arguments: { amount: 4200, currency: "usd", reason: "duplicate" },
  context: { supportTicketId: "ticket_982" },
});

const result = await mint.actions.execute(action.id);
```

Manual server:

```bash
mint init --key-file mint.ed25519.pem
export MINT_SIGNING_KEY_FILE=mint.ed25519.pem
export MINT_PROVIDER=fake
export MINT_MODE=development
mint serve
```

## Stripe sandbox demo

Stripe sandbox transactions move no real funds. This path is opt-in and never runs in default CI.

```bash
export MINT_STRIPE_TEST_SECRET_KEY=sk_test_...
export MINT_RUN_STRIPE_E2E=1
./scripts/test-stripe-sandbox.sh
```

The sandbox test:

1. Refuses a live Stripe key.
2. Creates a Stripe sandbox charge.
3. Proposes and authorizes a 4,200-cent partial refund.
4. Executes it through Mint.
5. Verifies the refund through Stripe.
6. Re-executes concurrently and checks that one refund exists.
7. Exercises the after-provider failpoint.
8. Reconciles to the original Stripe refund ID.
9. Verifies the signed Mint receipt.

Live keys (`sk_live_…`, `rk_live_…`, or anything that is not clearly test mode) are refused. This MVP cannot execute a live-money transaction.

## Architecture and trust boundary

```text
agent / framework
        │  proposes exact tool call
        ▼
identity (Keycard / Oasis / OIDC)     optional policy (Onyx / OPA / HTTP PDP)
        │                                      │
        └──────────────► mint.run ◄────────────┘
                            │
                            │ binds hash, CAS Authorized → Executing,
                            │ durable attempt, provider idempotency key
                            ▼
                         Stripe (system of record)
                            │
                            ▼
                     signed receipt + stdout events
                            │
                            ▼
                     logs / OTel / SIEM
```

Mint's trust boundary is the runtime that stores the intent, makes the provider call, and signs the receipt. Stripe remains the provider and system of record. The receipt's provider refund ID (`re_…`) is the externally checkable evidence. The signature proves what this Mint runtime recorded; it does not independently prove Stripe's internal state.

## Comparison

| System | Role relative to Mint |
|---|---|
| Agent framework | Decides what it wants to do. Mint is not a framework. |
| Framework-native approvals | Can collect a human "yes". Mint binds that yes to the exact call and commits it. |
| Keycard | Can give an agent permission. Mint makes using that permission a recoverable transaction. |
| Oasis / OIDC / Okta / Entra / Auth0 | Supply identity. Mint verifies standard JWT/OIDC claims; it is not an IdP or OAuth server. |
| Onyx / OPA | Optional external policy decision. Mint ships only Stripe refund thresholds plus a fail-closed HTTP PDP adapter. |
| Temporal / job systems | Optional broader workflow. Mint commits the exact authorized side effect. |
| Stripe | Provider and ledger. Mint derives and persists the correct idempotency key, coordinates state, and links the result to the approved intent. |

Mint is not a general AI security platform, secret manager, prompt-injection detector, generic observability platform, or replacement for those systems.

## Threat model

In scope for this MVP:

- Replay of the same authorization as a second provider effect.
- Post-approval mutation of amount, currency, charge, tenant, actor, ticket, or reason.
- Cross-tenant reads and writes.
- Ambiguous provider timeouts being guessed as success.
- Silent live-mode Stripe execution.
- Leakage of provider secrets in API bodies, receipts, logs, or SQLite.
- Unauthenticated audit listings and permissive CORS.

Out of scope: making model reasoning correct, stopping prompt injection before a tool call is proposed, and proving Stripe's internals.

## Guarantees

- Authorization is bound to a SHA-256 hash of RFC 8785 canonical material fields.
- Changing tenant, actor, provider, operation, resource, amount, currency, reason, or ticket association changes the intent hash.
- The hash is recomputed immediately before provider execution.
- Only one process can win `Authorized → Executing`.
- A durable execution attempt is recorded before contacting the provider.
- A missing or ambiguous provider response becomes `Unknown`, not guessed success.
- Terminal results are returned to later callers.
- Tenant ownership is enforced on every read and mutation.
- Receipts verify with the configured key ID and reject unknown formats.

Accurate one-attempt wording:

> Mint allows one internal execution attempt to begin. External exactly-once behavior depends on provider idempotency or reliable provider reconciliation.

Stripe already supplies idempotency. Mint's job is to derive and persist the correct key, bind it to authorization, coordinate state, reconcile uncertain outcomes, and connect the provider result to the original approved intent.

## Non-guarantees

Mint does **not** claim:

- Universal exactly-once execution.
- Formal verification.
- Certified Keycard, Oasis, or Onyx integrations.
- Production readiness.
- Independent proof of Stripe internals.
- That Mint makes model reasoning correct.

## Receipt verification

```bash
mint verify --receipt receipt.json --keys-url http://127.0.0.1:8787/v1/keys
mint verify --receipt receipt.json --key-file mint.ed25519.pem
```

A receipt includes format version, action and tenant IDs, actor, intent hash, policy decision and version, approval identity when applicable, attempt ID, provider, operation, provider idempotency key, provider resource ID, final status, timestamps, reconciliation flag, and signing key ID.

It does not include Stripe secrets, raw identity tokens, unnecessary customer data, or conversation history.

## Integration interfaces

- **Identity:** local development identity (development mode only) or generic JWT/OIDC with issuer, audience, JWKS, subject, and agent claims. JWKS is warmed at startup and cached; it is not fetched on every action.
- **Policy:** built-in Stripe refund thresholds (`MINT_POLICY_AUTO_CENTS`, `MINT_POLICY_APPROVAL_CENTS`) or a fail-closed HTTP PDP. Defaults: ≤5000 cents automatic, 5001–50000 approval required, >50000 denied.
- **Credentials:** Stripe test key from the environment, or a generic HTTP credential hook. Credentials are never returned to the agent-facing API. This is not a vault.
- **Events:** structured tracing on stdout (`action_id`, `tenant_id`, state transition, provider, operation, latency, outcome, reconciliation flag). Collect with OpenTelemetry, Datadog, or a SIEM. There is no embedded telemetry platform.
- **Client:** HTTP/JSON under `/v1` and a zero-dependency TypeScript fetch wrapper. No framework adapters yet.

### API

```text
POST /v1/actions
GET  /v1/actions/{id}
POST /v1/actions/{id}/approve
POST /v1/actions/{id}/deny
POST /v1/actions/{id}/execute
POST /v1/actions/{id}/reconcile
GET  /v1/actions/{id}/receipt
GET  /v1/actions/{id}/approval
GET  /v1/keys
GET  /health
```

Authenticated tenant/action endpoints require `Authorization: Bearer …`. CORS is disabled unless `MINT_CORS_ORIGINS` is set. `/health` returns `{"status":"ok"}` and does not leak configuration.

Canonical material fields are documented in `fixtures/canonicalization/README.md`. Version: `jcs-rfc8785-v1`.

## Development and test commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm test --prefix sdk/typescript
./scripts/demo-local.sh
./scripts/test-stripe-sandbox.sh
cargo build --release
./target/release/mint bench
```

Default `cargo test` does not require network access. The Stripe sandbox test is skipped unless `MINT_RUN_STRIPE_E2E=1` and a test-mode secret are set.

Local overhead (excluding approval wait and Stripe network latency) is measured by `mint bench`. The design target is single-digit-millisecond Mint overhead under normal local conditions.

## Experimental / beta status

This is a pre-1.0 proof of concept. APIs, receipt formats, and storage schema may change. Do not use it to move live money. Do not treat receipts as a substitute for Stripe's own dashboard and API.
