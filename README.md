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

## Stripe sandbox testing

Stripe sandbox transactions move **no real funds**. This path is opt-in and never runs in default CI. Live keys (`sk_live_…`, `rk_live_…`, or anything that is not clearly test mode) are refused. This MVP cannot execute a live-money transaction.

### Prerequisites

1. A Stripe account with **test mode** enabled.
2. A secret key that starts with `sk_test_` (Dashboard → Developers → API keys → Reveal test key).
3. Network access to `api.stripe.com`.
4. A local Rust toolchain that can build this repo (`cargo test`).

Optional but useful: keep the Stripe Dashboard open on **Payments → Refunds (test mode)** while you run the steps below.

### Automated end-to-end test (recommended)

This is the complete Stripe proof. One command runs the opt-in integration test in `tests/stripe_sandbox.rs`.

```bash
# From the repo root
export MINT_STRIPE_TEST_SECRET_KEY=sk_test_...   # must be test mode
export MINT_RUN_STRIPE_E2E=1                     # required opt-in flag

./scripts/test-stripe-sandbox.sh
```

Equivalent direct cargo invocation:

```bash
export MINT_STRIPE_TEST_SECRET_KEY=sk_test_...
export MINT_RUN_STRIPE_E2E=1
cargo test --test stripe_sandbox -- --nocapture
```

Without both variables set, `cargo test` skips the Stripe test cleanly and `./scripts/test-stripe-sandbox.sh` exits with instructions.

#### What the automated test does, in order

1. **Refuse live keys** — asserts `sk_live_…` is rejected before any network call.
2. **Create a sandbox charge** — `POST https://api.stripe.com/v1/charges` for 5,000 USD cents with `source=tok_visa` (test token). No real money moves.
3. **Start Mint** with `MINT_PROVIDER=stripe` against a temporary SQLite DB and a temporary Ed25519 key.
4. **Propose** a 4,200-cent partial refund for `ticket_982` on that charge (`refund.create`).
5. **Authorize automatically** — 4,200 ≤ 5,000 cents, so status becomes `Authorized` with no human approval.
6. **Execute** through Mint — Mint derives a stable Stripe idempotency key, records an attempt, then calls Stripe Refunds.
7. **Verify in Stripe** — `GET /v1/refunds/{re_…}` and assert `amount == 4200` and `metadata.mint_action_id` matches the Mint action ID.
8. **Concurrent re-execute** — eight parallel `POST /v1/actions/{id}/execute` calls all return `Succeeded` with the **same** `providerResourceId`; only one Stripe refund exists.
9. **Failpoint after provider success** — on a second charge/action, inject `after_provider_success` so Stripe succeeds but Mint records `Unknown`.
10. **Reconcile** — `POST /v1/actions/{id}/reconcile` recovers the original Stripe refund ID (same idempotency key / metadata match).
11. **Verify the signed Mint receipt** — Ed25519 verification succeeds and the receipt’s `provider_resource_id` matches the Stripe refund.

Expected result: the test prints Stripe IDs and ends with `ok` for `stripe_sandbox_partial_refund_round_trip`.

### Manual Stripe sandbox walkthrough

Use this when you want to click through the API yourself (or debug a failing automated run).

#### 1. Init keys and start Mint against Stripe

```bash
cargo build
./target/debug/mint init --key-file mint.ed25519.pem

export MINT_MODE=development
export MINT_IDENTITY=local
export MINT_PROVIDER=stripe
export MINT_SIGNING_KEY_FILE=mint.ed25519.pem
export MINT_DATABASE_PATH=mint-stripe.db
export MINT_BIND_ADDR=127.0.0.1:8787
export MINT_KID=mint-local-1
export MINT_STRIPE_TEST_SECRET_KEY=sk_test_...

./target/debug/mint serve
```

Confirm health:

```bash
curl -s http://127.0.0.1:8787/health
# {"status":"ok"}
```

#### 2. Create a local-dev bearer token

```bash
TOKEN="$(python3 - <<'PY'
import json, base64
payload = {
  "tenant_id": "acme",
  "subject": "user_123",
  "agent_id": "support-agent-7",
  "issuer": "https://identity.example.com",
  "delegated_by": None,
}
raw = json.dumps(payload, separators=(",", ":")).encode()
print("dev." + base64.urlsafe_b64encode(raw).decode().rstrip("="))
PY
)"
```

#### 3. Create a Stripe sandbox charge (5,000 cents)

```bash
CHARGE="$(curl -s https://api.stripe.com/v1/charges \
  -u "${MINT_STRIPE_TEST_SECRET_KEY}:" \
  -d amount=5000 \
  -d currency=usd \
  -d source=tok_visa)"
echo "$CHARGE" | python3 -m json.tool
CHARGE_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$CHARGE")"
echo "charge=$CHARGE_ID"
```

#### 4. Propose the partial refund through Mint

```bash
PROPOSE="$(curl -s -X POST http://127.0.0.1:8787/v1/actions \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{
    \"tenantId\": \"acme\",
    \"actor\": {
      \"subject\": \"user_123\",
      \"agentId\": \"support-agent-7\",
      \"issuer\": \"https://identity.example.com\"
    },
    \"provider\": \"stripe\",
    \"operation\": \"refund.create\",
    \"resource\": { \"type\": \"charge\", \"id\": \"${CHARGE_ID}\" },
    \"arguments\": { \"amount\": 4200, \"currency\": \"usd\", \"reason\": \"duplicate\" },
    \"context\": { \"supportTicketId\": \"ticket_982\" }
  }")"
echo "$PROPOSE" | python3 -m json.tool
ACTION_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$PROPOSE")"
# status should be Authorized (4200 <= auto threshold 5000)
```

For an approval-required path, propose `amount: 12000` instead, open `GET /v1/actions/{id}/approval`, then:

```bash
INTENT_HASH="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["intentHash"])' "$PROPOSE")"
curl -s -X POST "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/approve" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"intentHash\": \"${INTENT_HASH}\"}" | python3 -m json.tool
```

#### 5. Execute and inspect the Stripe refund

```bash
EXECUTE="$(curl -s -X POST "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/execute" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{}')"
echo "$EXECUTE" | python3 -m json.tool
REFUND_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["providerResourceId"])' "$EXECUTE")"

curl -s "https://api.stripe.com/v1/refunds/${REFUND_ID}" \
  -u "${MINT_STRIPE_TEST_SECRET_KEY}:" | python3 -m json.tool
# expect amount=4200 and metadata.mint_action_id == ACTION_ID
```

#### 6. Prove concurrent re-execute does not double-refund

```bash
for i in $(seq 1 8); do
  curl -s -X POST "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/execute" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{}' &
done
wait
# Each response should be Succeeded with the same providerResourceId.
# In Stripe, only one refund for that charge/metadata should exist.
```

#### 7. Failpoint + reconcile (debug builds only)

Failpoints are available in debug builds and disabled/unavailable in release operation.

```bash
# New charge + propose another Authorized action, then:
curl -s -X POST "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/execute" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -H "x-mint-failpoint: after_provider_success" \
  -d '{}' | python3 -m json.tool
# expect unknown_outcome / Unknown status even though Stripe created the refund

curl -s -X POST "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/reconcile" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{}' | python3 -m json.tool
# expect Succeeded with the original Stripe refund id
```

#### 8. Fetch and verify the signed receipt

```bash
curl -s "http://127.0.0.1:8787/v1/actions/${ACTION_ID}/receipt" \
  -H "Authorization: Bearer ${TOKEN}" > receipt.json
python3 -m json.tool < receipt.json

./target/debug/mint verify --receipt receipt.json --key-file mint.ed25519.pem
# or:
./target/debug/mint verify --receipt receipt.json --keys-url http://127.0.0.1:8787/v1/keys
```

### Policy thresholds used by the Stripe path

| Refund amount (USD cents) | Decision |
|---|---|
| ≤ `MINT_POLICY_AUTO_CENTS` (default 5000) | Automatic `Authorized` |
| `auto+1` … `MINT_POLICY_APPROVAL_CENTS` (default 50000) | `PendingApproval` |
| > approval max | `Denied` |

Malformed, negative, overflowing, or non-integer amounts fail closed before any Stripe call.

### Safety checklist

- [ ] Key starts with `sk_test_` (never `sk_live_`).
- [ ] `MINT_RUN_STRIPE_E2E=1` only when you intend to hit Stripe.
- [ ] Default `cargo test` / CI stays offline (Stripe test skipped).
- [ ] Dashboard is in **test mode** when inspecting charges/refunds.
- [ ] Receipt verification succeeds locally after execute/reconcile.

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

Default `cargo test` does not require network access. The Stripe sandbox test is skipped unless `MINT_RUN_STRIPE_E2E=1` and a test-mode secret are set. Full Stripe testing steps (automated + manual) are in [Stripe sandbox testing](#stripe-sandbox-testing).

Local overhead (excluding approval wait and Stripe network latency) is measured by `mint bench`. The design target is single-digit-millisecond Mint overhead under normal local conditions.

## Experimental / beta status

This is a pre-1.0 proof of concept. APIs, receipt formats, and storage schema may change. Do not use it to move live money. Do not treat receipts as a substitute for Stripe's own dashboard and API.
