# Loom script

## Before you hit record

- Stripe dashboard (test mode) open in a browser tab.
- Rust installed. `setup_local.sh` builds mint.run, and `loom_demo.sh` starts and stops it.
- Big terminal font, dark theme, about 110 columns.
- Rehearse: `./loom_demo.sh --replay` (no Parallel runs). Record: `./loom_demo.sh` (2 live runs, 2 takes left).
- Press **Enter** for each beat.

---

## 1 / 8 · Architecture

- "An agent wants to refund a customer. I don't want the model deciding that, so the refund only runs on cited evidence."
- "How I use Parallel: a Task run with a JSON schema. I read the answer plus its basis, meaning the citations and confidence for each field."
- "How I use AgentMint: it signs every step, and the refund function won't run without a valid signed authorization."
- "mint.run makes the Stripe call, so a retry can't refund twice."

## 2 / 8 · The policy

- "This is the whole rule: disruption true, confidence medium or high, and at least two different sites cited."
- "Why: it's plain code, so it's testable and the same input always gets the same answer."

## 3 / 8 · Case A: Hurricane Ian

- "Claim: FedEx, Fort Myers, September 28th, 2022."
- While Parallel runs: "That's Parallel's live event stream: its plan, its searches, sources found."
- If it goes quiet: "It's reading sources now, which doesn't send events." (Or cut this.)
- On the verdict: "True, medium, two sites. Authorized."
- "The binding hashes the payment, the amount and this exact Parallel result. Why: the approval can't be reused for a different refund."
- On the Stripe line: "mint.run sent it. My Stripe key requires human approval, so Stripe holds it. Mint marks it Unknown, not Failed, because it can still go through."

## 4 / 8 · Approve, then reconcile

- "I'll approve it in Stripe." (Approve, switch back, Enter.)
- "The agent retries with the same signed receipt. Mint sees Unknown, so it checks Stripe instead of sending a new refund."
- On `REFUND EXECUTED`: "It found the refund and marked it Succeeded. One refund, $12.99."
- "AgentMint decides if it's allowed. mint.run makes sure it happens once."

## 5 / 8 · Case B: Phoenix

- "Claim: FedEx, Phoenix, May 14th, 2024. No real outage."
- On the verdict: "False, still cited. The only FedEx notice that day was about their test API."
- "Refused and signed. Mint never gets a request, and neither does Stripe."

## 6 / 8 · What got signed

- "The AgentMint receipt has the decision, the payment, the amount and the hash of the Parallel result."
- "Each receipt includes the hash of the one before it. Why: you can't delete or reorder a step."
- "The mint.run receipt is for the refund itself, and its intent hash includes the AgentMint receipt. So the refund points back to the evidence."

## 7 / 8 · Verify

- "Anyone can check these with the public keys. No account needed."
- "All AgentMint receipts are valid, the chain is intact, and the mint.run receipt verifies."

## 8 / 8 · Tamper

- "I change one byte in a copy of the authorization: $12.99 becomes $92.99."
- "The signature breaks."
- "I pass it to the refund function, and it's blocked before mint.run or Stripe."
- "It checks against my own public key, not one sitting next to the file. Why: re-signing a fake with a different key doesn't work either."
- Close: "Parallel for evidence, a policy in code, AgentMint to gate and sign, mint.run to run it once."

---

## If something goes sideways

- **Case A refused live** (Parallel sometimes cites one site): "One site this time, so it's refused. That's the gate working." Re-record the refund half with `--replay`.
- **Skip the approval:** `SKIP_APPROVAL=1 ./loom_demo.sh`. Beat 4 shows it still Unknown, 0 refunds.
- **Hands-free test:** `AUTO=1 ./loom_demo.sh`.
