"""Measure @notarise overhead on authorize(): decorated vs authorize.__wrapped__, 50 iterations
each, fake evidence, no Stripe/Parallel calls. Runs in a scratch dir so real receipts are untouched.
Usage: python3 bench_overhead.py <scratch_dir> <config|ephemeral>"""
import json, os, statistics, sys, time
from pathlib import Path

PROJECT = Path(__file__).resolve().parent
for line in open(PROJECT / ".env"):
    if "=" in line and not line.startswith("#"):
        k, v = line.strip().split("=", 1); os.environ.setdefault(k, v)
scratch, mode = Path(sys.argv[1]), sys.argv[2]
os.chdir(scratch)
sys.path.insert(0, str(PROJECT))
import demo  # no network at import time

fake_ev = {"run_id": "trun_fake", "query": "q", "content": {"disruption": True, "summary": "x" * 160},
           "basis": [{"field": "disruption", "reasoning": "r" * 400, "confidence": "high",
                      "citations": [{"url": f"https://site{i}.example/a", "title": "t", "excerpts": ["e" * 600]} for i in range(3)]},
                     {"field": "summary", "reasoning": "r" * 300, "confidence": "high",
                      "citations": [{"url": "https://site0.example/a", "title": "t", "excerpts": ["e" * 600]}]}],
           "latency_s": 0.0, "sha256": "0" * 64}

def bench(fn, n=50):
    fn("pi_fake", 1299, fake_ev)  # warm-up
    out = []
    for _ in range(n):
        t = time.perf_counter(); fn("pi_fake", 1299, fake_ev); out.append((time.perf_counter() - t) * 1e6)
    return out

plain = bench(demo.authorize.__wrapped__)
deco = bench(demo.authorize) if mode != "ephemeral-no-tsa" else [0.0]
mp, md = statistics.median(plain), statistics.median(deco)
if mode != "ephemeral-no-tsa": print(json.dumps({"mode": mode, "config_present": (scratch / ".agentmint/config.toml").exists(),
                  "iterations": 50, "plain_median_us": round(mp, 1), "decorated_median_us": round(md, 1),
                  "added_median_us": round(md - mp, 1), "decorated_p90_us": round(sorted(deco)[44], 1),
                  "receipts_written": len(list((scratch / "receipts").glob("*.json"))) if (scratch / "receipts").exists() else 0}))

# Ephemeral Notary with timestamping explicitly off (no network). Passing plan= disables
# config mode, so this measures pure in-memory signing.
if mode == "ephemeral-no-tsa":
    from agentmint import Notary, notarise
    n = Notary()
    plan = n.create_plan(user="bench", action="bench", scope=["*"])
    fn = notarise(n, action="authorize:refund_shipping", plan=plan, enable_timestamp=False)(demo.authorize.__wrapped__)
    d2 = bench(fn)
    print(json.dumps({"mode": mode, "iterations": 50, "plain_median_us": round(mp, 1),
                      "decorated_median_us": round(statistics.median(d2), 1),
                      "added_median_us": round(statistics.median(d2) - mp, 1)}))
