import base64, hashlib, json, os, sys, time, urllib.error, urllib.request
from pathlib import Path
from urllib.parse import urlparse
import stripe
from parallel import Parallel
from agentmint import Notary, notarise  # [agentmint]
from agentmint import verify as am_verify  # [agentmint]

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
PUBLIC_KEY = Path(__file__).resolve().parent / ".agentmint" / "keys" / "public_key.pem"

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


def watch(run_id, t0):
    """Print Parallel's live progress events for a run (display only; the result is fetched after)."""
    print(f"parallel task {run_id} started (processor=base)", flush=True)
    seen, last = set(), None
    try:
        for e in client.task_run.events(run_id, timeout=1800):
            el = f"{time.time() - t0:5.0f}s"
            if getattr(e, "message", None):
                print(f"  parallel {el} | {e.type.rsplit('.', 1)[-1]:<11} {e.message[:110]}", flush=True)
            elif getattr(e, "source_stats", None):
                st = e.source_stats
                for u in (st.sources_read_sample or []):
                    d = domain(u)
                    if d not in seen:
                        seen.add(d); print(f"  parallel {el} | read {d}", flush=True)
                now = (st.num_sources_considered, st.num_sources_read)
                if now != last:
                    last = now; print(f"  parallel {el} | {now[0] or 0} sources considered, {now[1] or 0} read", flush=True)
            elif getattr(e, "run", None) and not e.run.is_active:
                print(f"  parallel {el} | run {e.run.status}", flush=True)
                break
    except Exception as ex:  # streaming is cosmetic; never block the demo on it
        print(f"  parallel | event stream ended ({type(ex).__name__}); waiting for result", flush=True)


def replay_research(claim):
    """DEMO_REPLAY=1: reuse the newest signed research receipt for this exact claim
    (signature checked first) instead of a new Parallel Task run."""
    import glob
    from agentmint import verify as am_verify
    found = []
    for p in glob.glob("receipts/*.json"):
        d = json.load(open(p))
        if (d["action"] == "research:carrier_disruption" and d["evidence"].get("args") == [claim]
                and "replayed_from_receipt" not in (d["evidence"].get("result") or {})):
            found.append((d["observed_at"], p, d))
    if not found:
        sys.exit("no saved research receipt for this claim; run once without DEMO_REPLAY")
    _, p, d = max(found)
    assert am_verify.verify(p)[0].ok, f"saved receipt {p} does not verify"
    print(f"REPLAY: Parallel result {d['evidence']['result']['run_id']} from signed receipt {d['id'][:8]} (no new Task run)")
    return {**d["evidence"]["result"], "replayed_from_receipt": d["id"]}


@notarise(notary, action="research:carrier_disruption")  # [agentmint]
def research(claim):
    if os.environ.get("DEMO_REPLAY") == "1":
        return replay_research(claim)
    # Asks for corroboration explicitly; the policy (2+ distinct sites) is unchanged.
    q = (f"Was there a {claim['carrier']} delivery disruption in {claim['region']} on or around {claim['date']}? "
         "Cite independent sources (different publishers) that document it.")
    take_task_budget()
    t0 = time.time()
    run = client.task_run.create(input=q, task_spec={"output_schema": SCHEMA}, processor="base", enable_events=True)
    if os.environ.get("LOOM") == "1":
        watch(run.run_id, t0)
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
def refund(auth_receipt_path):
    """AgentMint is the gate: Stripe is called only for a valid, signed, allowed authorization receipt."""
    # [agentmint] verify against the project's own key, not a public_key.pem sitting next to the receipt
    v = am_verify.verify_receipt_path(Path(auth_receipt_path), PUBLIC_KEY)
    if not v.ok:
        return {"executed": False, "reason": f"authorization receipt invalid: {v.reason}"}
    d = json.load(open(auth_receipt_path))
    auth = d["evidence"]["result"]
    ref = {"authorization_receipt": d["id"]}
    if d["action"] != "authorize:refund_shipping" or not auth["allowed"]:
        return {"executed": False, "reason": "refused by policy", "why": auth.get("why"), **ref}
    expected = hashlib.sha256(f"{auth['pi']}|{auth['amount']}|{auth['evidence_sha256']}".encode()).hexdigest()
    if expected != auth["binding"]:
        return {"executed": False, "reason": "binding mismatch", **ref}
    # Gate passed. mint.run owns the Stripe call: one execution winner, durable attempt, reconcile.
    return {**mint_refund(auth, d["id"]), **ref}


# ---- mint.run client (stdlib only) ------------------------------------------------------
MINT_URL = os.environ.get("MINT_URL", "http://127.0.0.1:8787")
MINT_ACTOR = {"subject": "user_123", "agentId": "support-agent-7", "issuer": "https://identity.example.com"}
MINT_TOKEN = "dev." + base64.urlsafe_b64encode(json.dumps(
    {"tenant_id": "acme", "subject": MINT_ACTOR["subject"], "agent_id": MINT_ACTOR["agentId"],
     "issuer": MINT_ACTOR["issuer"], "delegated_by": None}, separators=(",", ":")).encode()).decode().rstrip("=")
MINT_ACTIONS = Path(__file__).resolve().parent / "runs" / "mint_actions.json"   # binding -> Mint action id
MINT_RECEIPTS = Path(__file__).resolve().parent / "mint_receipts"


def mint(method, path, body=None):
    req = urllib.request.Request(MINT_URL + path, method=method,
                                 data=None if body is None else json.dumps(body).encode(),
                                 headers={"Authorization": f"Bearer {MINT_TOKEN}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")
    except urllib.error.URLError:
        sys.exit(f"mint.run is not reachable at {MINT_URL}. Start it with ./mint_up.sh")


def mint_refund(auth, auth_receipt_id):
    actions = json.loads(MINT_ACTIONS.read_text()) if MINT_ACTIONS.exists() else {}
    action_id = actions.get(auth["binding"])
    if action_id is None:
        # The AgentMint receipt id and binding ride in supportTicketId, which is part of Mint's intent hash.
        code, view = mint("POST", "/v1/actions", {
            "tenantId": "acme", "actor": MINT_ACTOR, "provider": "stripe", "operation": "refund.create",
            "resource": {"type": "payment_intent", "id": auth["pi"]},
            "arguments": {"amount": auth["amount"], "currency": "usd", "reason": "requested_by_customer"},
            "context": {"supportTicketId": f"agentmint:{auth_receipt_id}:{auth['binding']}", "reason": "carrier_disruption"},
            "ttlSeconds": 3600})
        if code != 200:
            return {"executed": False, "reason": f"mint.run propose failed: {view.get('error', view)}"}
        action_id = view["id"]
        actions[auth["binding"]] = action_id
        MINT_ACTIONS.write_text(json.dumps(actions, indent=2))
    _, view = mint("GET", f"/v1/actions/{action_id}")
    status = view.get("status")
    if status == "Authorized":
        mint("POST", f"/v1/actions/{action_id}/execute", {})
    elif status in ("Unknown", "Executing"):
        mint("POST", f"/v1/actions/{action_id}/reconcile")
    _, view = mint("GET", f"/v1/actions/{action_id}")
    out = {"executed": view.get("status") == "Succeeded", "mint_action_id": action_id, "mint_status": view.get("status"),
           "intent_hash": view.get("intentHash"), "refund_id": view.get("providerResourceId"), "amount": auth["amount"]}
    if view.get("status") in ("Succeeded", "Failed"):
        code, receipt = mint("GET", f"/v1/actions/{action_id}/receipt")
        if code == 200:
            MINT_RECEIPTS.mkdir(exist_ok=True)
            (MINT_RECEIPTS / f"{action_id}.json").write_text(json.dumps(receipt, indent=2))
            out["mint_receipt"] = f"mint_receipts/{action_id}.json"
    return out


def fmt(a):
    st = a.get("mint_status")
    if a.get("executed"):
        return f"REFUND EXECUTED via mint.run {a['refund_id']} ${a['amount']/100:.2f} (Mint action {a['mint_action_id'][:8]}, Succeeded)"
    if st == "Unknown":
        return f"mint.run: Unknown, Stripe is holding it (human approval required). Mint action {a['mint_action_id'][:8]} will reconcile"
    if st:
        return f"mint.run: {st} (Mint action {a['mint_action_id'][:8]})"
    if a.get("reason") == "refused by policy":
        return "not sent to Stripe: refused by policy"
    return f"BLOCKED by AgentMint, Stripe not called: {a.get('reason')}"


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
    auth_path = f"receipts/{authorize.last_receipt.id}.json"
    a1 = refund(auth_path); receipts.append(rid(refund)); print("attempt 1:", fmt(a1))
    a2 = refund(auth_path); receipts.append(rid(refund)); print("attempt 2 (retry):", fmt(a2))
    refunds = stripe.Refund.list(payment_intent=pi.id).data
    print(f"refunds on {pi.id}: {len(refunds)} (${sum(r.amount for r in refunds)/100:.2f})")
    print("RECEIPTS:", " ".join(r for r in receipts if r))
    return {"case": case, "payment_intent": pi.id, "parallel_run_id": ev["run_id"], "allowed": auth["allowed"],
            "binding": auth["binding"], "attempt_1": a1, "attempt_2": a2, "refunds": len(refunds),
            "refunded_cents": sum(r.amount for r in refunds), "why": auth["why"],
            "latency_s": ev["latency_s"], "receipts": receipts}


if __name__ == "__main__":
    summary = main(sys.argv[1] if len(sys.argv) > 1 else "holds")
    if os.environ.get("LOOM") != "1":
        print(json.dumps(summary, indent=2, default=str))
