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

## 1 / 8 · Title card

On screen: the five-line overview.

- "This is a demo of an agent that can only issue a refund when the evidence holds up."
- "A customer says their package was late and wants shipping refunded."
- "The agent researches it with Parallel's Task API and checks the result against a fixed policy. Only then does mint.run make the Stripe refund."
- "Every step gets signed into an AgentMint receipt, and mint.run signs the refund itself."

## 2 / 8 · The policy

On screen: the `policy()` function.

- "This is the whole rule, and it's in code, not in a prompt."
- "Parallel has to say there was a disruption."
- "Confidence has to be medium or high."
- "The citations have to come from at least two different domains."
- "If any of those fail, there's no refund."
- "AgentMint signs every step, and the refund function won't run without a valid signed authorization."

## 3 / 8 · Case A: Hurricane Ian

On screen: Parallel's live progress (plan, searches, sources considered), then the verdict.

- "First claim: FedEx, Fort Myers, September 28, 2022. That's Hurricane Ian."
- "We charge a hundred dollars to a test card so there's something to refund."
- While Parallel runs: "This is Parallel working live. You can see the queries it's running and how many sources it's looking at."
- If it goes quiet for a minute: "It's reading sources now. That part doesn't send events, so give it a minute." (Or cut this in editing.)
- On the verdict: "Disruption true, medium confidence, two sources: Supply Chain Dive and the New York Times."
- "So it's authorized. The binding at the bottom is a hash of the payment, the amount and the exact Parallel result."
- On the Stripe line: "mint.run made the Stripe call. This key needs a human to approve refunds, so Stripe is holding it. Mint marks that Unknown, not failed, because the money could still move."
- "The retry hits the same approval request, so it doesn't create a second one."

## 4 / 8 · Approve in Stripe, then retry

On screen: the prompt to approve. Switch to the browser.

- "I'll approve it in Stripe." (Approve it, switch back, press Enter.)
- "Now the agent retries. All it passes in is the signed authorization receipt. AgentMint checks it, and mint.run reconciles the same action."
- On `REFUND EXECUTED via mint.run`: "Mint found the refund Stripe ran after my approval and moved it from Unknown to Succeeded. One refund, $12.99."
- "AgentMint decides whether a refund is allowed. mint.run makes sure it happens once."

## 5 / 8 · Case B: Phoenix

On screen: Parallel's live progress, then REFUSED.

- "Second claim: FedEx, Phoenix, May 14, 2024. There wasn't a real outage."
- While it runs: "Same thing, Parallel researching live."
- On the verdict: "Disruption false. It even found the one FedEx notice from that day, which was about their test API environment, not deliveries."
- "Refused. Stripe never gets called. Zero refunds."

## 6 / 8 · What got signed

On screen: the fields of the authorization receipt.

- "This is the receipt for the Ian authorization."
- "It holds the decision, the payment, the amount and the evidence hash, which ties it to that exact Parallel result."
- "Below it is mint.run's receipt for the refund. Its intent hash includes the AgentMint receipt, so the refund is tied to the evidence."
- "Previous receipt is a hash chain, so you can't drop or reorder a step."
- "And it's signed with Ed25519."

## 7 / 8 · Verify

On screen: `agentmint verify` and the chain check.

- "Anyone with the public key can check these. No account needed."
- "Every receipt is valid, and the chain is intact."
- On `valid receipt … status=Succeeded`: "And mint.run's refund receipt checks out against Mint's published key."

## 8 / 8 · Tamper

On screen: a diff changing 1299 to 9299, the verify failure, then `refund()` on the tampered copy.

- "Now I'll change one character in a copy of the authorization: $12.99 becomes $92.99."
- "Verification fails with a signature mismatch."
- "And this is the part that matters: I hand that receipt to the refund function."
- On `BLOCKED by AgentMint`: "AgentMint rejects it before mint.run or Stripe is ever called."
- "The refund function only takes a signed authorization. It checks the signature against our key, checks it was allowed, and recomputes the binding. Only then does it hand off to mint.run."
- Close: "That's it: Parallel for the research, a policy you can read, AgentMint as the gate, mint.run to execute it once, and Stripe for the money."

---

## If something goes sideways

- **Case A comes back REFUSED live** (Parallel sometimes returns one source): "And that's the policy working. One source isn't enough." Then re-record with `--replay` for the refund half.
- **You don't want to approve live:** `SKIP_APPROVAL=1 ./loom_demo.sh`. Beat 4 will show 0 refunds and the pending approval.
- **Hands-free run:** `AUTO=1 ./loom_demo.sh` moves forward every 2 seconds. Better for a test than for recording.
