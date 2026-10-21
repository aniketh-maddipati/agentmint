# AgentMint × Parallel: evidence-bound refund demo

An agent may refund a $12.99 shipping fee only when Parallel Task API research finds a
carrier disruption **and** the cited evidence meets a fixed policy. Every step (research,
authorization, each refund attempt) is signed into an AgentMint receipt. The refund itself is
executed by **mint.run** (the Rust server in this repo), with the AgentMint receipt ID and binding
inside Mint's intent hash. Mint signs its own receipt for the refund.

## What it proves

1. **Evidence gate.** `policy()` allows the refund only if `content.disruption` is `true`,
   the `disruption` field's basis has confidence `high` or `medium`, and its citations come from
   at least **2 distinct domains** (`www.` stripped). The thresholds are fixed. A refusal is signed like an approval.
2. **Binding.** `binding = sha256(payment_intent | amount | sha256(run_id, content, basis))`.
   The authorization receipt signs the binding, the policy inputs and the full Parallel basis.
3. **Signed, tamper-evident record.** `@notarise` signs each call's args and return value
   (Ed25519, hash-chained per plan). Editing one character of a receipt breaks verification.
4. **AgentMint is the gate.** `refund()` takes only the path to a signed authorization receipt.
   It verifies the signature against the project's pinned public key (`.agentmint/keys/public_key.pem`,
   not a key file next to the receipt), checks the action and `allowed`, and recomputes the binding.
   Only then does it hand the refund to mint.run. A tampered or re-signed receipt is blocked before
   anything reaches Stripe (`runs/gate_test.txt`).
5. **mint.run executes it once.** `refund()` proposes a Stripe `refund.create` to a local mint.run server
   (`supportTicketId = agentmint:<receipt id>:<binding>`, part of the intent hash) and executes it. Mint allows one
   execution winner, records the attempt before calling Stripe, and uses `mint:<action id>:refund.create:v1` as the
   Stripe idempotency key. Each binding maps to one Mint action (`runs/mint_actions.json`), so a retry re-executes
   or reconciles the same action instead of proposing a new one.

## The agent

`agent.py` is a minimal Claude support agent (`claude-opus-5-5`, low effort, manual tool loop). It reads one customer
message and talks to `refund_desk_mcp.py`, a stdio MCP server with two tools:

- `check_delivery_disruption(order_id)`: the order's carrier, city and date go to a Parallel Task (base, JSON schema).
  It returns the verdict, confidence and cited sites, and signs an AgentMint research receipt.
- `refund_shipping(order_id)`: policy, then a signed AgentMint authorization, then the gate, then mint.run, then Stripe.
  The amount ($12.99) is fixed in code.

The model only supplies an order ID, so it can't change what gets researched or how much is refunded.
It needs `ANTHROPIC_API_KEY` in `.env`.

## Who guarantees what

- **AgentMint (Python):** whether the refund may happen at all. It checks that the authorization is signed,
  allowed and bound to the exact Parallel evidence. It does not track receipt use.
- **mint.run (Rust):** that it happens at most once. Mint picks one execution winner, keeps a durable attempt,
  sets the provider idempotency key and reconciles unknown outcomes. It also signs a receipt for the result.
- **Stripe:** the money, plus a human approval step on this restricted key.

**Human approval at Stripe.** This `rk_test_` key makes Stripe hold refunds for approval with a 403 `approval_required`.
mint.run (`src/packs/stripe.rs`) now treats that as **Unknown**, not Failed, because Stripe runs the refund if a human
approves it. After approval, `reconcile` finds the refund by its `metadata.mint_action_id` and the action becomes
**Succeeded** with a signed receipt (`runs/mint_refund_after_approval.log`, `mint_receipts/`).

If a decorated function raises, no AgentMint receipt is written, so `refund()` returns errors instead of raising.

## How to run

```bash
./setup_local.sh                         # venv, deps, builds mint.run (needs Rust), asks for keys
./mint_up.sh                             # local mint.run server (provider=stripe, test mode)
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

## Recording a Loom

```bash
./loom_demo.sh --replay   # rehearse: reuses signed Parallel results, 0 Task runs
./loom_demo.sh            # record: 2 live Parallel runs with progress streamed on screen
```

Press Enter to move to each beat. Say-this-now notes for each beat are in `LOOM_SCRIPT.md`.

## Files

- `demo.py`: the demo. AgentMint-only lines are tagged `# [agentmint]`. The gate in `refund()` adds about 10 more.
- `agent.py`, `refund_desk_mcp.py`: the Claude agent and its MCP server.
- `loom_demo.sh`, `LOOM_SCRIPT.md`: Loom walkthrough (one scenario, 5 beats) and talk track.
- `mint_up.sh`: starts and stops the local mint.run server (`.mint/` holds its key and DB, gitignored).
- `mint_receipts/`: mint.run's signed refund receipts, plus its public key (`keys.json`).
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
