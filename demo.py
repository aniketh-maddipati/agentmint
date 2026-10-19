import hashlib, json, os, sys, time
from urllib.parse import urlparse
import stripe
from parallel import Parallel
from agentmint import Notary, notarise  # [agentmint]

for line in open(".env") if os.path.exists(".env") else []:
    if "=" in line and not line.startswith("#"):
        k, v = line.strip().split("=", 1); os.environ.setdefault(k.strip(), v.strip())

stripe.api_key = os.environ.get("STRIPE_KEY") or os.environ["STRIPE_SECRET_KEY"]
assert stripe.api_key.startswith(("sk_test_", "rk_test_")), "test mode only"
client = Parallel()
# [agentmint] With .agentmint/config.toml present (agentmint init), @notarise signs with the
# project keystore and writes each receipt to ./receipts/<id>.json; this Notary is the fallback.
notary = Notary()  # [agentmint]
SHIPPING_FEE = 1299
TASK_BUDGET = 10
TASK_COUNT_FILE = "runs/.task_count"

CLAIMS = {
    # Original (try 1, REFUSED: only 1 citation domain, see runs/A_try1_dallas.log):
    # "holds": dict(carrier="FedEx", region="Dallas, Texas", date="2021-02-16",
    #               text="My package was days late because of the winter storm. Please refund shipping."),
    "holds": dict(carrier="FedEx", region="Fort Myers, Florida", date="2022-09-28",
                  text="My package was days late because of Hurricane Ian. Please refund shipping."),
    "fails": dict(carrier="FedEx", region="Phoenix, Arizona", date="2024-05-14",
                  text="My package was late because of a carrier outage. Please refund shipping."),
}
SCHEMA = {"type": "json", "json_schema": {"type": "object", "properties": {
    "disruption": {"type": "boolean", "description": "True only if a carrier service disruption affecting deliveries in this region on or within two days of this date is documented by a reliable source."},
    "summary": {"type": "string", "description": "One sentence describing the disruption or why none was found."}},
    "required": ["disruption", "summary"]}}


def take_task_budget():
    """Count Parallel Task runs across all invocations; refuse past the budget."""
    used = int(open(TASK_COUNT_FILE).read().strip()) if os.path.exists(TASK_COUNT_FILE) else 0
    if used >= TASK_BUDGET:
        sys.exit(f"STOP: Parallel Task budget exhausted ({used}/{TASK_BUDGET})")
    used += 1
    open(TASK_COUNT_FILE, "w").write(str(used))
    print(f"TASK RUNS USED: {used}/{TASK_BUDGET}")


def seed_payment():
    return stripe.PaymentIntent.create(amount=10000, currency="usd", payment_method="pm_card_visa",
        confirm=True, automatic_payment_methods={"enabled": True, "allow_redirects": "never"})


def to_plain(x):
    # mode="json" so the receipt gets real JSON, not the decorator's repr() fallback
    return x.model_dump(mode="json") if hasattr(x, "model_dump") else x


def domain(url):
    host = urlparse(url).netloc.lower()
    return host[4:] if host.startswith("www.") else host  # www.x.com and x.com are one source


@notarise(notary, action="research:carrier_disruption")  # [agentmint]
def research(claim):
    q = f"Was there a {claim['carrier']} delivery disruption in {claim['region']} on or around {claim['date']}?"
    take_task_budget()
    t0 = time.time()
    run = client.task_run.create(input=q, task_spec={"output_schema": SCHEMA}, processor="base")
    res = client.task_run.result(run.run_id, api_timeout=1800)
    out = to_plain(res.output)
    content = out["content"] if isinstance(out["content"], dict) else json.loads(out["content"])
    basis = [to_plain(b) for b in (out.get("basis") or [])]
    ev = {"run_id": run.run_id, "query": q, "content": content, "basis": basis, "latency_s": round(time.time() - t0, 1)}
    ev["sha256"] = hashlib.sha256(json.dumps({k: ev[k] for k in ("run_id", "content", "basis")}, sort_keys=True, default=str).encode()).hexdigest()
    return ev


def policy(ev):
    field = next((b for b in ev["basis"] if b.get("field") == "disruption"), {})
    domains = sorted({domain(c["url"]) for c in (field.get("citations") or [])})
    ok = bool(ev["content"].get("disruption")) and field.get("confidence") in ("high", "medium") and len(domains) >= 2
    return ok, {"disruption": ev["content"].get("disruption"), "confidence": field.get("confidence"),
                "independent_sources": domains, "summary": ev["content"].get("summary")}


@notarise(notary, action="authorize:refund_shipping")  # [agentmint]
def authorize(pi_id, amount, ev):
    ok, why = policy(ev)
    binding = hashlib.sha256(f"{pi_id}|{amount}|{ev['sha256']}".encode()).hexdigest()
    return {"allowed": ok, "binding": binding, "pi": pi_id, "amount": amount, "evidence_sha256": ev["sha256"], "why": why}


@notarise(notary, action="stripe:partial_refund")  # [agentmint]
def refund(auth):
    if not auth["allowed"]:
        return {"executed": False, "reason": "refused by policy", "why": auth["why"]}
    try:
        r = stripe.Refund.create(payment_intent=auth["pi"], amount=auth["amount"], idempotency_key=f"agentmint-{auth['binding']}")
    except stripe.StripeError as e:
        # Return (not raise) so the blocked attempt is still signed into a receipt.
        msg = str(getattr(e, "user_message", None) or e)
        apreq = next((w.strip(".,") for w in msg.split() if w.startswith("apreq_") or "/approval_requests/apreq_" in w), None)
        return {"executed": False, "reason": "stripe_error", "error_type": type(e).__name__,
                "http_status": getattr(e, "http_status", None), "request_id": getattr(e, "request_id", None),
                "approval_request": apreq.rsplit("/", 1)[-1] if apreq else None, "message": msg[:200]}
    return {"executed": True, "refund_id": r.id, "amount": r.amount}


def rid(fn):
    return fn.last_receipt.id if getattr(fn, "last_receipt", None) else None


def main(case, claim=None):
    claim = claim or CLAIMS[case]
    print(f"\nCASE {case}: {claim['text']} ({claim['carrier']}, {claim['region']}, {claim['date']})")
    pi = seed_payment(); print(f"payment {pi.id} $100.00")
    receipts = []
    ev = research(claim); receipts.append(rid(research))
    print(f"parallel run {ev['run_id']} latency {ev['latency_s']}s evidence {ev['sha256'][:12]}")
    auth = authorize(pi.id, SHIPPING_FEE, ev); receipts.append(rid(authorize))
    print(json.dumps(auth["why"], indent=2))
    print("AUTHORIZED" if auth["allowed"] else "REFUSED", "binding", auth["binding"][:12])
    a1 = refund(auth); receipts.append(rid(refund)); print("attempt 1:", a1)
    a2 = refund(auth); receipts.append(rid(refund)); print("attempt 2 (retry):", a2)
    refunds = stripe.Refund.list(payment_intent=pi.id).data
    print(f"refunds on {pi.id}: {len(refunds)} (${sum(r.amount for r in refunds)/100:.2f})")
    print("RECEIPTS:", " ".join(r for r in receipts if r))
    return {"case": case, "payment_intent": pi.id, "parallel_run_id": ev["run_id"], "allowed": auth["allowed"],
            "binding": auth["binding"], "attempt_1": a1, "attempt_2": a2, "refunds": len(refunds),
            "refunded_cents": sum(r.amount for r in refunds), "why": auth["why"],
            "latency_s": ev["latency_s"], "receipts": receipts}


if __name__ == "__main__":
    print(json.dumps(main(sys.argv[1] if len(sys.argv) > 1 else "holds"), indent=2, default=str))
