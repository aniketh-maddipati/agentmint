# AgentMint × Parallel: evidence-bound refund demo

An agent may refund a $12.99 shipping fee only when Parallel Task API research finds a
carrier disruption **and** the cited evidence meets a fixed policy. Every step (research,
authorization, each refund attempt) is signed into an AgentMint receipt. The refund is
bound to the exact payment, amount and Parallel result, and is sent with an idempotency
key derived from that binding.

## What it proves

1. **Evidence gate.** `policy()` allows the refund only if `content.disruption` is `true`,
   the `disruption` field's basis has confidence `high` or `medium`, and its citations come from
   at least **2 distinct domains** (`www.` stripped). The thresholds are fixed. A refusal is signed like an approval.
2. **Binding.** `binding = sha256(payment_intent | amount | sha256(run_id, content, basis))`.
   The authorization receipt signs the binding, the policy inputs and the full Parallel basis.
3. **Signed, tamper-evident record.** `@notarise` signs each call's args and return value
   (Ed25519, hash-chained per plan). Editing one character of a receipt breaks verification.
4. **Retry safety.** A retried refund reuses `idempotency_key = agentmint-<binding>`.

## At-most-once comes from Stripe, not AgentMint

The at-most-once guarantee comes from **Stripe idempotency keyed on the binding**, not from
Mint's execution boundary. AgentMint's `@notarise` here is a notary: it runs the function
first and signs what happened afterwards. It does not gate, queue or deduplicate the Stripe call.
If the function raises an exception, no receipt is written (so `refund()` catches Stripe errors
and returns them, to keep blocked attempts on the record).

## How to run

```bash
python3 -m pip install --break-system-packages parallel-web stripe \
  "git+https://github.com/aerf-spec/agentmint-python"   # PyPI agentmint 0.1.0 lacks @notarise and the CLI
agentmint init -y .                      # keystore + ./receipts sink, timestamper = none
printf 'PARALLEL_API_KEY=...\nSTRIPE_KEY=rk_test_...\n' > .env   # test-mode Stripe key only; .env is gitignored
python3 demo.py holds | tee runs/A.log   # expect AUTHORIZED
python3 demo.py fails | tee runs/B.log   # expect REFUSED, 0 refunds
python3 retry_refund.py receipts/<authorize-receipt>.json   # retry a refund from its signed authorization
agentmint verify receipts/               # signature check of every receipt
python3 verify_chain_ordered.py receipts # hash-chain check in link order
python3 bench_overhead.py <scratch_dir> config   # @notarise overhead, no API calls
```

`demo.py` keeps a Parallel Task counter in `runs/.task_count` and stops at 10.

## Files

- `demo.py`: the demo. AgentMint-only lines are tagged `# [agentmint]` (5 code lines).
- `retry_refund.py`: verifies an authorization receipt, then retries its refund twice.
- `verify_chain_ordered.py`: works around `agentmint chain verify` sorting receipts by filename.
- `bench_overhead.py`: overhead benchmark.
- `receipts/`: all signed receipts. `.agentmint/keys/public_key.pem` is the verify key; the private key is gitignored.
- `runs/`: logs, raw Parallel result, show/verify/tamper output, measurements.
- `RESULTS.md`, `FINDINGS_FOR_PARALLEL.md`.

## Limits

- **Stripe human approval sat in the loop.** This restricted key requires approval for refunds.
  Before approval, Stripe returned 403 and all attempts mapped to one approval request. After approval, exactly one
  $12.99 refund executed. Later retries with the same idempotency key still returned the cached 403, not the
  refund ID, so the refund has to be found with `Refund.list`. AgentMint receipts record the 403s, not the refund ID.
- The 2-domain rule counts domains, not corroboration. In Run A, one of the two citations
  (NYT) documents the hurricane at Fort Myers, not a FedEx disruption.
- The same Parallel query returned 2 citation domains once and 1 the next time. Policy outcomes on the same claim can differ between runs.
- Receipts are signed with a local key and are not timestamped (`timestamper = none`). They prove integrity, not time, to a third party.
- `agentmint verify <dir>` checks signatures only. Chain linkage needs `verify_chain_ordered.py` (or a .zip export).
- One small run per case. This is not a benchmark of Parallel accuracy.
