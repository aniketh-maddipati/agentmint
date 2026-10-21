# Loom script

## Before you hit record

- Stripe dashboard (test mode) open in a browser tab.
- `./setup_local.sh` done once (it asks for the Parallel, Stripe and Anthropic keys, and builds mint.run).
- Big terminal font, dark theme, about 110 columns.
- Rehearse: `./loom_demo.sh --replay` (no Parallel run). Record: `./loom_demo.sh` (1 live Parallel run per take).
- Press **Enter** for each beat.

## The tools (refund-desk MCP server)

| Tool | Input | What it does |
|---|---|---|
| `check_delivery_disruption` | `order_id` | Looks up the order's carrier, city and delivery date, runs a Parallel Task, and returns the verdict, confidence and cited sites. Signed into an AgentMint receipt. |
| `refund_shipping` | `order_id` | Runs the policy on that evidence, signs the AgentMint authorization, then the gate passes it to mint.run, which calls Stripe. The amount is fixed at $12.99. |

The agent only passes an order ID. It can't change the city, the date or the amount.

---

## 1 / 5 · What this is

- "This is a support agent built with Claude. It can ask for a refund, but it can't grant one."
- "How I use Parallel: the agent's research tool runs a Parallel Task with a JSON schema. I use the answer plus its basis, meaning the citations and confidence."
- "How I use AgentMint: every step is signed, and the refund function only runs on a valid signed authorization."
- "mint.run makes the Stripe call, so retries can't double-refund."

## 2 / 5 · The agent run

- "A normal support ticket: order 1042 was late because of the January snowstorm, and the customer wants their $12.99 shipping back."
- On `agent → tool: check_delivery_disruption`: "The agent passes the order ID. The tool pulls the city and date from the order, not from the customer."
- While Parallel runs: "This is Parallel's live event stream: its plan, its searches, and the sources it finds."
- On the result: "Disruption true, the confidence level, and the sites it cited."
- On `refund_shipping`: "Now the agent asks for the refund. The policy decides: disruption, confidence medium or higher, at least two sites."
- If authorized: "mint.run sent it. My Stripe key requires human approval, so Stripe holds it. Mint marks it Unknown, not Failed."
- If refused: "Only one site this time, so it's refused and signed. The agent tells the customer no, and Stripe is never called."
- On `agent:`: "The agent reports what the refund system decided. It can't override it."

## 3 / 5 · Approve, then reconcile

- "I'll approve it in Stripe." (Approve, switch back, Enter.)
- "The retry uses the same signed authorization. Mint sees Unknown, so it checks Stripe instead of sending another refund."
- On `REFUND EXECUTED`: "Found it. Succeeded, one refund, $12.99."
- (If it was refused: "The retry checks the signed authorization, sees it was refused, and stops.")

## 4 / 5 · Signed and verified

- "The AgentMint receipt has the decision, the amount, and a hash of the exact Parallel result."
- "The mint.run receipt is for the refund itself, and it points back to that authorization."
- "Every receipt verifies with public keys, and they're hash-chained, so a step can't be dropped or reordered."

## 5 / 5 · Tamper

- "I change one byte in a copy of the authorization: $12.99 becomes $92.99."
- "Pass it to the refund function: blocked before mint.run or Stripe."
- Close: "Claude handles the conversation, Parallel brings the evidence, AgentMint decides if it's allowed, and mint.run makes it happen once."

---

## If something goes sideways

- **Skip the approval:** `SKIP_APPROVAL=1 ./loom_demo.sh`. Beat 3 shows it still Unknown.
- **Hands-free test:** `AUTO=1 ./loom_demo.sh`.
- **No captions needed?** Leave off `SILENT=1`. With it on, a one-line caption shows at the end of each beat.
