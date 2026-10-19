# Results

Parallel `base` processor; 4 of 10 Task runs used. Stripe test mode (restricted key).

## Runs

| Run | Claim | Parallel run | disruption | confidence | citation domains | Policy | Refunds |
|---|---|---|---|---|---|---|---|
| Step 3 (raw) | FedEx, Dallas TX, 2021-02-16 | `trun_…c55b0bdb` | true | medium | cbsnews.com, northlandnewsradio.com | n/a (parser check) | n/a |
| A, try 1 | FedEx, Dallas TX, 2021-02-16 | `trun_…eb95b7e0` | true | medium | cbsnews.com | **REFUSED** (1 domain) | 0 |
| **A** | FedEx, Fort Myers FL, 2022-09-28 (Hurricane Ian) | `trun_…e9916d64` | true | medium | nytimes.com, supplychaindive.com | **AUTHORIZED** | **0** (see below) |
| **B** | FedEx, Phoenix AZ, 2024-05-14 | `trun_…e9210584` | false | medium | developer.fedex.com | **REFUSED** (no disruption) | 0 |

**Run A refund.** The policy authorized $12.99 on `pi_3USOI9RilGVAxTyU08z2KNDu` (binding `71472a0c2f77…`).
Stripe rejected `Refund.create` with HTTP 403 `PermissionError`: *"This action requires
human approval… An approval request has been created… If approved, it will automatically execute."*
The first attempt raised before the `refund()` fix and left no receipt. Two retries from the signed
authorization (`retry_refund.py`) used the same idempotency key, and both returned the **same** approval
request `apreq_test_61VbmGwii5xdFf…`. Refunds listed on the payment: 0 ($0.00). At most one refund is pending
(awaiting human approval in Stripe), and none has executed.

**Run B.** Parallel answered "no documented disruption". The only May 14 FedEx notice was a test-environment API interruption.
That "no" answer was still cited (developer.fedex.com). Both refund attempts returned `refused by policy`.

## Verification

| Check | Result |
|---|---|
| `agentmint verify receipts/` (12 receipts) | 12/12 valid, exit 0 (`runs/verify.txt`) |
| `agentmint chain verify` | ✗ "first receipt has non-null chain hash". CLI sorts by filename (UUID), not chain order |
| `verify_chain_ordered.py receipts` | **chain valid**, 12/12 linked across 4 processes |
| Tamper: `evidence.result.amount` 1299→9299 in a copy | ✗ "signature mismatch", exit 1 (`runs/tamper.txt`). The original still verifies |

## Measurements (`runs/measurements.json`)

| Metric | Value |
|---|---|
| Parallel Task latency (base), 4 runs | 65.1 s, 71.9 s, 87.5 s, 109.6 s (median 79.7 s) |
| `@notarise` added time on `authorize`, as used (config mode: sign + write receipt to disk) | **1.70 ms** median (p90 1.93 ms), 50 iters |
| Same, in-memory `Notary()`, timestamp off | 0.50 ms median |
| Same, decorator default `enable_timestamp=True` (freetsa.org per call) | 629 ms median. Not used by the demo |
| Undecorated `authorize` | 6.7 µs |
| AgentMint integration lines in `demo.py` | 5 (1 import, 1 `Notary()`, 3 decorators) |
