# mint.run

Make consequential agent actions safe to retry.

Agents retry, race, and lose provider responses. Mint binds authorization to an exact action, chooses one execution winner, reconciles uncertain outcomes, and signs the result. The current proof of concept supports Stripe partial refunds.

## Two-minute demo

```bash
git clone https://github.com/aniketh-maddipati/agentmint.git
cd agentmint
cargo build
cargo run -- init
./scripts/demo-local.sh
```

```text
proposed -> authorized
executing -> succeeded (re_fake_…)
retry -> same provider result
receipt -> valid
PASS — one action, one provider effect, retry returned same result, receipt valid
```

No Stripe credentials required for the demo. Experimental — not production-ready.

## What Mint adds

| Problem                                  | Mint behavior                   |
| ---------------------------------------- | ------------------------------- |
| Concurrent callers                       | One execution winner            |
| Retry after timeout                      | Stable provider idempotency     |
| Arguments change after approval          | Hash mismatch; no provider call |
| Provider succeeded but response was lost | Unknown, then reconcile         |
| Audit question                           | Signed receipt                  |

Stripe supplies strong primitives; Mint assembles them into an agent-facing execution boundary.

## How it works

```text
agent -> exact intent -> identity/policy -> atomic claim -> Stripe -> signed receipt
                                                |
                                             reconcile
```

- Authorization is bound to a canonical intent hash (RFC 8785 + SHA-256).
- Only one process wins `Authorized → Executing`.
- A durable attempt is recorded before provider I/O.
- Ambiguous outcomes become `Unknown` and require reconcile.
- The result is an Ed25519-signed receipt.

## Stripe test-mode proof

Validated against Stripe test mode on September 17, 2026: concurrent callers converged on one $42.17 partial refund from a $100 test payment; simulated lost-response reconciliation recovered the same refund; the receipt verified.

Optional (uses your Stripe test secret; creates Dashboard-visible test charges/refunds):

```bash
read -s MINT_STRIPE_TEST_SECRET_KEY
export MINT_STRIPE_TEST_SECRET_KEY
export MINT_PROVIDER=stripe
export MINT_RUN_STRIPE_E2E=1

cargo run -- doctor
./scripts/test-stripe-sandbox.sh
```

Paste an `sk_test_...` key after `read` begins waiting.

> Never use a live Stripe key. This MVP intentionally refuses `sk_live_` and `rk_live_` credentials.

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
  provider: "fake",
  operation: "refund.create",
  resource: { type: "charge", id: "ch_123" },
  arguments: { amount: 4200, currency: "usd", reason: "duplicate" },
  context: { supportTicketId: "ticket_982" },
});

const result = await mint.actions.execute(action.id);
```

## Guarantees and limits

**Guarantees**

- authorization bound to canonical intent
- one internal execution winner
- durable attempt before provider I/O
- provider idempotency and reconciliation
- tenant isolation
- signed receipts
- live Stripe credentials refused

**Limits**

- experimental and not production-ready
- not universal exactly-once delivery
- receipt proves Mint’s recorded result, not Stripe’s internal correctness
- host or signing-key compromise is out of scope
- authenticated approval does not prove a human identity
- actions bypassing Mint are not protected
- OIDC and external policy integrations remain unverified

## Development

```bash
./scripts/launch-check.sh
```

## PA/BV lab (experimental)

Synthetic medical-benefit prior-authorization / benefits-verification case console for outpatient MRI lumbar spine (CPT 72148). Isolated under `src/lab/`; does not use real patient data, payers, or Stripe.

```bash
./scripts/demo-pa-bv.sh
./scripts/demo-pa-bv-mcp.sh
./scripts/demo-pa-bv-rest.sh
cargo test --test pa_bv_acceptance
mint lab start --scenario approval
mint lab console <run-id>
```

See `docs/pa-bv-lab.md` and `docs/pa-bv-domain.md`.

## Status

Experimental proof of concept.

- Fake-provider and offline safety suite: verified
- Stripe test-mode refund and reconciliation: verified
- Live Stripe operation: intentionally disabled
- OIDC and external policy integrations: implemented but unverified
- Production readiness: no

[MIT](LICENSE)
