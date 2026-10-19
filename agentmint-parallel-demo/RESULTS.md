# Results

Parallel `base` processor; 4 of 10 Task runs used. Stripe test mode (restricted key).

## Runs

| Run | Claim | Parallel run | disruption | confidence | citation domains | Policy | Refunds |
|---|---|---|---|---|---|---|---|
| Step 3 (raw) | FedEx, Dallas TX, 2021-02-16 | `trun_…c55b0bdb` | true | medium | cbsnews.com, northlandnewsradio.com | n/a (parser check) | n/a |
| A, try 1 | FedEx, Dallas TX, 2021-02-16 | `trun_…eb95b7e0` | true | medium | cbsnews.com | **REFUSED** (1 domain) | 0 |
| **A** | FedEx, Fort Myers FL, 2022-09-28 (Hurricane Ian) | `trun_…e9916d64` | true | medium | nytimes.com, supplychaindive.com | **AUTHORIZED** | **1 × $12.99** after human approval in Stripe (see below) |
| **B** | FedEx, Phoenix AZ, 2024-05-14 | `trun_…e9210584` | false | medium | developer.fedex.com | **REFUSED** (no disruption) | 0 |

**Run A refund.** The policy authorized $12.99 on `pi_3USOI9RilGVAxTyU08z2KNDu` (binding `71472a0c2f77…`).
This restricted Stripe key requires human approval for refunds. `Refund.create` returned HTTP 403 `PermissionError`
(*"An approval request has been created… If approved, it will automatically execute."*).
The first attempt raised before the `refund()` fix and left no receipt. Two retries from the signed authorization
(`retry_refund.py`) used the same idempotency key, and both returned the **same** approval request `apreq_test_61VbmGwii5xdFf…`.

After a human approved it in the Stripe dashboard, Stripe executed **exactly one refund**:
`re_3USOI9RilGVAxTyU0VTzIMgf`, $12.99, `succeeded` (2026-10-19 22:00 UTC). Two more retries with the same key
(`runs/A_refund_after_approval.log`) created no second refund. They still returned the cached 403 and the approval-request ID,
**not** the refund ID, so the agent learns the refund ID only by listing refunds.
Refunds on the payment: **1 ($12.99)**. Five refund attempts in total (three before approval, two after) led to one refund.

**Run B.** Parallel answered "no documented disruption". The only May 14 FedEx notice was a test-environment API interruption.
That "no" answer was still cited (developer.fedex.com). Both refund attempts returned `refused by policy`.

**Run A through mint.run** (replayed Parallel result `trun_…e9916d64`, so no new Task run). AgentMint authorized $12.99
on `pi_3USP5yRilGVAxTyU1O0upOQl` and the gate passed. mint.run proposed action `be15d989…`, which was Authorized by Mint's
threshold policy, and executed it. Stripe returned 403 `approval_required`, so Mint marked it **Unknown** (reconciliation required).
After human approval in Stripe, the retry reconciled the same action, found `re_3USP5yRilGVAxTyU16grdQ2H` by
`metadata.mint_action_id`, and moved **Unknown → Succeeded**. A second retry returned the stored result without calling Stripe.
Refunds on the payment: **1 ($12.99)**. `mint verify` on the receipt: valid. One changed character in a copy: `invalid signature`.

## Verification

| Check | Result |
|---|---|
| `agentmint verify receipts/` (14 receipts) | 14/14 valid, exit 0 (`runs/verify.txt`) |
| `agentmint chain verify` | ✗ "first receipt has non-null chain hash". CLI sorts by filename (UUID), not chain order |
| `verify_chain_ordered.py receipts` | **chain valid**, 14/14 linked across 5 processes |
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
