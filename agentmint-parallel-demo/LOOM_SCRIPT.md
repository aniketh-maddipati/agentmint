# Loom script

## Before you hit record

- Open the Stripe dashboard (test mode) in a browser tab, ready to approve a refund.
- Rust must be installed. `setup_local.sh` builds mint.run, and `loom_demo.sh` starts and stops it for you.
- Make the terminal font big and use a dark theme. Use a window around 110 columns wide.
- `cd agentmint-parallel-demo`
- Rehearse with `./loom_demo.sh --replay`. It uses no Parallel runs.
- Record with `./loom_demo.sh`. It makes 2 live Parallel runs (5 of 10 used so far, so you have 2 takes).
- Press **Enter** to move to the next beat. The screen clears each time.

---

## 1 / 8 · Architecture

On screen: the pipeline overview.

- "This is an evidence-gated side effect. An agent wants to issue a Stripe refund, and the authorization comes from cited research, not from the model's judgment."
- "There are four stages: a Parallel Task run with a JSON output schema, a deterministic policy over the result's basis, an AgentMint-signed authorization, and mint.run as the execution boundary for the Stripe call."
- "Each stage emits a signed artifact, and each one is bound by hash to the one before it."

## 2 / 8 · The policy

On screen: the `policy()` function.

- "The policy is a pure function over Parallel's FieldBasis for the `disruption` field. There's no LLM in the decision path."
- "Three predicates: `content.disruption` is true, the basis confidence is medium or high, and the citations span at least two distinct hostnames."
- "If any predicate fails, the authorization is signed with `allowed: false` and nothing downstream runs."
- "`refund()` doesn't take arguments from the agent. It takes a path to a signed authorization receipt and derives everything from that."

## 3 / 8 · Case A: Hurricane Ian

On screen: Parallel's live progress (plan, searches, sources considered), then the verdict.

- "First claim: FedEx, Fort Myers, September 28th, 2022. That's Hurricane Ian's landfall."
- "I seed a $100 PaymentIntent on a test card so there's a real charge to refund against."
- While Parallel runs: "This is the Task run's event stream on the base processor: the plan, the search queries, and source counts as it fans out."
- If it goes quiet for a minute: "This is the read phase. It doesn't emit progress events, so there's a gap here." (Or cut this in editing.)
- On the verdict: "`disruption: true`, medium confidence, citations from supplychaindive.com and nytimes.com. All three predicates pass."
- "The binding is SHA-256 over the PaymentIntent, the amount in cents, and a hash of the run ID, content and basis. Change any of those and the binding changes."
- On the Stripe line: "mint.run proposed the action, its threshold policy authorized it, and it claimed the single execution slot before calling Stripe."
- "This restricted key requires human approval, so Stripe returns 403 `approval_required`. Mint records that as Unknown, not Failed, because the refund runs later if a human approves it."
- "The retry hits the same Mint action and the same idempotency key, so Stripe returns the same held request."

## 4 / 8 · Approve in Stripe, then reconcile

On screen: the prompt to approve. Switch to the browser.

- "I'll approve the held request in the Stripe dashboard." (Approve it, switch back, press Enter.)
- "The agent retries with the same authorization receipt. AgentMint re-verifies it, and mint.run sees the action is Unknown, so it reconciles instead of executing again."
- On `REFUND EXECUTED via mint.run`: "Reconcile lists refunds on the PaymentIntent and matches on `metadata.mint_action_id`. It finds the refund Stripe ran after approval and moves the action from Unknown to Succeeded."
- "The second retry returns the stored terminal result. Stripe isn't called again."
- "So the split is clean. AgentMint answers whether this effect is allowed. mint.run makes sure it happens at most once, and reconciles when the provider's answer is ambiguous."

## 5 / 8 · Case B: Phoenix

On screen: Parallel's live progress, then REFUSED.

- "Second claim: FedEx, Phoenix, May 14th, 2024. There was no carrier outage."
- While it runs: "Same Task spec, same schema, different input."
- On the verdict: "`disruption: false`, and the basis still cites its source. The only FedEx notice that day was a test-environment API interruption, and Parallel separated that from delivery operations."
- "The authorization is signed with `allowed: false`. `refund()` stops at the policy check, so no Mint action is proposed and Stripe sees zero requests."

## 6 / 8 · What got signed

On screen: the AgentMint authorization receipt fields, then the mint.run receipt.

- "This is the AgentMint authorization receipt. It's Ed25519 over canonical JSON with sorted keys, and it carries the decision, the PaymentIntent, the amount, the evidence hash and the binding."
- "`previous_receipt_hash` is a SHA-256 of the prior signed receipt, so research, authorize and each refund attempt form a hash chain. You can't drop or reorder a step without breaking it."
- "Below it is mint.run's receipt. The intent hash is RFC 8785 canonical JSON plus SHA-256, and it includes `agentmint:<receipt id>:<binding>`. So Mint's signed result is bound to the exact Parallel evidence."
- "The Stripe idempotency key is `mint:<action id>:refund.create:v1`. It's derived from the Mint action, not supplied by the agent."
- "`reconciliation_required: true` records that this outcome went through Unknown before it was established."

## 7 / 8 · Verify

On screen: `agentmint verify`, the chain check, then `mint verify`.

- "Verification needs only the public keys. No account, and no call back to either service's state."
- "Every AgentMint receipt verifies, and walking `previous_receipt_hash` gives one unbroken chain."
- On `valid receipt … status=Succeeded`: "And the mint.run receipt verifies against the JWK Mint publishes at `/v1/keys`."

## 8 / 8 · Tamper

On screen: a diff changing 1299 to 9299, the verify failure, then `refund()` on the tampered copy.

- "I'll change one byte in a copy of the authorization. The signed amount goes from 1299 to 9299 cents."
- "`agentmint verify` reports a signature mismatch."
- "Now I pass that file to `refund()`."
- On `BLOCKED by AgentMint`: "It's rejected before mint.run or Stripe is ever called."
- "The gate verifies against a pinned project key, not whatever `public_key.pem` sits next to the receipt. So re-signing a forged receipt with your own key doesn't get through either."
- "Then it checks the action and `allowed`, and recomputes the binding from the signed fields. Only then does it propose to mint.run."
- Close: "Parallel produces cited evidence, the policy turns it into a signed decision, AgentMint enforces that decision at the call site, and mint.run executes the Stripe side effect at most once, reconciling when the outcome is ambiguous."

---

## If something goes sideways

- **Case A comes back REFUSED live** (Parallel sometimes returns one citation domain): "Same claim, one domain this time. The predicate fails, so the gate holds. That's the behavior you want." Then re-record with `--replay` for the refund half.
- **You don't want to approve live:** `SKIP_APPROVAL=1 ./loom_demo.sh`. Beat 4 shows the action still Unknown with 0 refunds.
- **Hands-free run:** `AUTO=1 ./loom_demo.sh` moves forward every 2 seconds. Better for a test than for recording.
