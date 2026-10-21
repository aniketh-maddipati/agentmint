"""refund-desk: a minimal MCP server (stdio) with the two tools a refund agent needs.

  check_delivery_disruption(order_id)  -> Parallel Task API, signed into an AgentMint receipt
  refund_shipping(order_id)            -> policy -> signed AgentMint authorization -> gate -> mint.run -> Stripe

The agent passes only an order id. Carrier, city, delivery date and the $12.99 amount come from the
order record, so the model can't change what gets researched or how much is refunded.
Progress (Parallel events, budget counter) goes to stderr; stdout is the MCP protocol.
"""
import contextlib, json, logging, os, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
os.chdir(HERE)
sys.path.insert(0, str(HERE))
with contextlib.redirect_stdout(sys.stderr):
    import demo  # noqa: E402  (loads .env, Stripe test key, Parallel client, AgentMint notary)

from mcp.server.mcpserver import MCPServer  # noqa: E402

ORDER_ID = "1042"
ORDER = {"carrier": "FedEx", "region": "Memphis, Tennessee", "date": "2026-01-26", "shipping_cents": demo.SHIPPING_FEE}
EVIDENCE = {}  # order_id -> research result for this session

server = MCPServer("refund-desk", instructions="Check delivery disruptions and request shipping refunds for store orders.",
                   log_level="WARNING")
logging.getLogger("stripe").setLevel(logging.WARNING)


def _order(order_id):
    if order_id.strip().lstrip("#") != ORDER_ID:
        raise ValueError(f"unknown order {order_id}")
    if "pi" not in ORDER:  # one fresh $100 test payment per server session
        with contextlib.redirect_stdout(sys.stderr):
            ORDER["pi"] = demo.seed_payment().id
    return ORDER


@server.tool()
def check_delivery_disruption(order_id: str) -> str:
    """Research whether the order's carrier had a documented delivery disruption in the delivery city
    on or around the delivery date. Uses the Parallel Task API and returns its verdict, confidence and cited sources."""
    o = _order(order_id)
    claim = {"carrier": o["carrier"], "region": o["region"], "date": o["date"]}
    with contextlib.redirect_stdout(sys.stderr):
        ev = demo.research(claim)
    EVIDENCE[ORDER_ID] = ev
    _, why = demo.policy(ev)
    return json.dumps({"order_id": ORDER_ID, "carrier": o["carrier"], "city": o["region"], "date": o["date"],
                       "parallel_run_id": ev["run_id"], "disruption": why["disruption"], "confidence": why["confidence"],
                       "cited_sites": why["independent_sources"], "summary": why["summary"],
                       "evidence_receipt": demo.research.last_receipt.id})


@server.tool()
def refund_shipping(order_id: str) -> str:
    """Request a refund of the order's shipping fee. Call check_delivery_disruption first. The refund system decides:
    it runs only if the evidence passes policy (disruption, medium/high confidence, 2+ cited sites). The amount is fixed."""
    o = _order(order_id)
    ev = EVIDENCE.get(ORDER_ID)
    if ev is None:
        return json.dumps({"order_id": ORDER_ID, "decision": "refused", "reason": "no evidence: call check_delivery_disruption first"})
    with contextlib.redirect_stdout(sys.stderr):
        auth = demo.authorize(o["pi"], o["shipping_cents"], ev)
        auth_path = f"receipts/{demo.authorize.last_receipt.id}.json"
        res = demo.refund(auth_path)
    return json.dumps({"order_id": ORDER_ID, "decision": "authorized" if auth["allowed"] else "refused",
                       "policy": auth["why"], "amount_cents": o["shipping_cents"], "authorization_receipt": demo.authorize.last_receipt.id,
                       "refund": demo.fmt(res), "mint_status": res.get("mint_status"), "refund_id": res.get("refund_id")})


if __name__ == "__main__":
    server.run("stdio")
