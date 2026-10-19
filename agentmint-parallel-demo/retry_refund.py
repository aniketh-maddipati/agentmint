"""Retry a refund from its signed authorization receipt. refund() verifies the receipt
before Stripe is called. No new Parallel run. Usage: python3 retry_refund.py <receipt.json>"""
import json, sys
import demo

path = sys.argv[1]
a1 = demo.refund(path); r1 = demo.refund.last_receipt.id; print("attempt 1:", demo.fmt(a1))
a2 = demo.refund(path); r2 = demo.refund.last_receipt.id; print("attempt 2 (retry):", demo.fmt(a2))
pi = json.load(open(path))["evidence"]["result"]["pi"]
refunds = demo.stripe.Refund.list(payment_intent=pi).data
print(f"refunds on {pi}: {len(refunds)} (${sum(r.amount for r in refunds)/100:.2f})")
print("RECEIPTS:", r1, r2)
