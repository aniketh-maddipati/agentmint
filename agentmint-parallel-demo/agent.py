"""Minimal refund support agent: Claude + the refund-desk MCP server (2 tools).

The agent reads one customer message, calls the tools it needs, and replies to the customer.
It can only pass an order id. Whether a refund happens is decided by the refund system:
Parallel evidence -> policy -> signed AgentMint authorization -> gate -> mint.run -> Stripe.

Usage: python3 agent.py ["customer message"]   (needs ANTHROPIC_API_KEY in .env or the environment)
"""
import asyncio, json, os, sys
from pathlib import Path

import anthropic
from mcp import Client, StdioServerParameters

HERE = Path(__file__).resolve().parent
for line in open(HERE / ".env") if (HERE / ".env").exists() else []:
    if "=" in line and not line.startswith("#"):
        k, v = line.strip().split("=", 1); os.environ.setdefault(k.strip(), v.strip())

MODEL = "claude-opus-5-5"
SYSTEM = ("You are a support agent for an online store. Use the tools to check the customer's order and, if they ask "
          "for one, request the shipping refund. The refund system makes the decision; report its outcome accurately "
          "and never promise a refund it did not grant. Reply to the customer in two or three plain sentences.")
CUSTOMER = ("Hi, my order #1042 was supposed to arrive in Memphis on Monday, January 26th, and it showed up days late "
            "because of the snowstorm. Can I get the $12.99 shipping back?")


def show(label, text):
    print(f"{label} {text}", flush=True)


def summarize(name, d):
    """One readable line per tool result for the terminal (the agent gets the full JSON)."""
    if name == "check_delivery_disruption":
        return (f"Parallel run {d['parallel_run_id']}: disruption={str(d['disruption']).lower()} "
                f"confidence={d['confidence']} sites={', '.join(d['cited_sites']) or 'none'}")
    if name == "refund_shipping":
        return f"{d['decision'].upper()} ${d.get('amount_cents', 1299)/100:.2f} → {d.get('refund') or d.get('reason')}"
    return json.dumps(d)


async def main(message):
    server = StdioServerParameters(command=sys.executable, args=[str(HERE / "refund_desk_mcp.py")], env=dict(os.environ))
    async with Client(server) as mcp:
        tools = [{"name": t.name, "description": t.description or "", "input_schema": t.input_schema}
                 for t in (await mcp.list_tools()).tools]
        claude = anthropic.AsyncAnthropic()
        messages = [{"role": "user", "content": message}]
        show("customer:", message)
        for _ in range(6):  # small cap: this task needs 2 tool calls
            resp = await claude.beta.messages.create(
                model=MODEL, max_tokens=4000, system=SYSTEM, tools=tools, messages=messages,
                output_config={"effort": "low"},
                betas=["server-side-fallback-2026-07-01"], fallbacks="default")
            if resp.stop_reason == "refusal":
                show("agent:", "(request declined by the model)"); return
            messages.append({"role": "assistant", "content": resp.content})
            calls = [b for b in resp.content if b.type == "tool_use"]
            if not calls:
                show("agent:", " ".join(b.text for b in resp.content if b.type == "text").strip()); return
            results = []
            for call in calls:
                show("agent → tool:", f"{call.name}({json.dumps(call.input)})")
                out = await mcp.call_tool(call.name, call.input)
                text = "".join(getattr(c, "text", "") for c in out.content)
                d = json.loads(text) if text.startswith("{") else {}
                show("tool → agent:", summarize(call.name, d) if d else text)
                if d.get("authorization_receipt"):
                    print("RECEIPTS: _ " + d["authorization_receipt"], flush=True)  # for loom_demo.sh / retry_refund.py
                results.append({"type": "tool_result", "tool_use_id": call.id, "content": text,
                                "is_error": bool(getattr(out, "is_error", False))})
            messages.append({"role": "user", "content": results})


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1] if len(sys.argv) > 1 else CUSTOMER))
