"""Retry the refund for an already-authorized case, using the authorization stored
in its signed AgentMint receipt. No new Parallel run. Usage: python3 retry_refund.py <receipt.json>"""
import json, sys
from agentmint import verify as am_verify
import demo

path = sys.argv[1]
res = am_verify.verify(path)[0]
assert res.ok, f"authorization receipt does not verify: {res.reason}"
auth = json.load(open(path))["evidence"]["result"]
print(f"authorization receipt {res.receipt_id} verified; allowed={auth['allowed']} binding {auth['binding'][:12]}")
a1 = demo.refund(auth); r1 = demo.refund.last_receipt.id; print("attempt 1:", demo.fmt(a1))
a2 = demo.refund(auth); r2 = demo.refund.last_receipt.id; print("attempt 2 (retry):", demo.fmt(a2))
refunds = demo.stripe.Refund.list(payment_intent=auth["pi"]).data
print(f"refunds on {auth['pi']}: {len(refunds)} (${sum(r.amount for r in refunds)/100:.2f})")
print("RECEIPTS:", r1, r2)
