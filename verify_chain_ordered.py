"""Verify the receipt hash chain in link order (agentmint 0.2.0 `chain verify` sorts by
filename, i.e. random UUID order, so it reports a false break). Usage: python3 verify_chain_ordered.py [dir]"""
import hashlib, sys
from pathlib import Path
from agentmint.notary import verify_chain, _canonical_json
from agentmint.verify import read_receipt

rs = [read_receipt(p) for p in sorted(Path(sys.argv[1] if len(sys.argv) > 1 else "receipts").glob("*.json"))]
h = lambda r: hashlib.sha256(_canonical_json({**r.signable_dict(), "signature": r.signature})).hexdigest()
by_prev = {r.previous_receipt_hash: r for r in rs}
ordered, cur = [], None
while cur in by_prev:
    r = by_prev.pop(cur); ordered.append(r); cur = h(r)
res = verify_chain(ordered)
for i, r in enumerate(ordered):
    print(f"{i:2d} {r.id[:8]} {r.action:28s} prev={str(r.previous_receipt_hash)[:12]}")
print(f"linked {len(ordered)}/{len(rs)} receipts; unlinked: {[r.id[:8] for r in by_prev.values()]}")
print("CHAIN", "VALID" if res.valid and len(ordered) == len(rs) else "INVALID", res.reason or f"root {res.root_hash[:16]}")
sys.exit(0 if res.valid and len(ordered) == len(rs) else 1)
