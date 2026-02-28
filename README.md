# 🔐 AgentMint

**Prove a human approved a specific agent action.**

Session auth says "this agent can access Stripe."
AgentMint says "Alice approved this $50 refund for order #123 at 10:03am."

---

## The Problem

AI agents can call APIs. Nothing proves a human authorized a specific call.

- No audit trail of who approved what
- No replay protection
- No per-action scoping

## The Solution

1. Human approves action → AgentMint signs a token (60s expiry, single-use)
2. Agent passes token to resource provider → Provider verifies before executing
3. Full audit trail of who approved what, when

---

## Quick Start
```bash
git clone https://github.com/aniketh-maddipati/agentmint
cd agentmint
cargo run
```
```
╔═══════════════════════════════════════════════════════════╗
║     🔐 AgentMint v0.1.0                                   ║
║     Cryptographic proof of human authorization            ║
╚═══════════════════════════════════════════════════════════╝

✓ Server ready → http://0.0.0.0:3000
```

---

## Demo

### 1. Mint a token
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"alice@company.com","action":"refund:order:123:amount:50","ttl_seconds":60}'
```

### 2. Verify the token
```bash
curl -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d '{"token":"<token from step 1>"}'
```

### 3. Replay blocked
```bash
# Same token again
curl -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d '{"token":"<same token>"}'

# → "token already used"
```

### 4. Audit trail
```bash
curl http://localhost:3000/audit
```
```json
[
  {
    "jti": "f1268944-...",
    "sub": "alice@company.com",
    "action": "refund:order:123:amount:50",
    "verified_at": "2026-02-26T14:38:14Z"
  }
]
```

---

## How It Works
```
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│   Human     │ ──── │  AgentMint  │ ──── │  Resource   │
│  approves   │  1   │  mints      │  2   │  Provider   │
│  action     │      │  token      │      │  verifies   │
└─────────────┘      └─────────────┘      └─────────────┘
                            │
                            ▼
                     ┌─────────────┐
                     │   Agent     │
                     │  (transport)│
                     └─────────────┘
```

1. Human approves action in your UI
2. Your backend calls AgentMint `/mint` → gets signed token
3. Agent carries token to resource provider (Stripe, your API)
4. Resource provider calls `/proxy` to verify → executes if valid
5. Token can't be reused, forged, or used after expiry

---

## Why Not OAuth?

| OAuth | AgentMint |
|-------|-----------|
| Tokens last hours/days | 60 seconds default |
| Reusable | Single-use (JTI tracking) |
| Scope = "can access Stripe" | Scope = "refund $50 for order #123" |
| Requires IdP coordination | Drop-in sidecar |

---

## Security

| Feature | Implementation |
|---------|----------------|
| Signatures | Ed25519 (constant-time) |
| Replay protection | JTI tracking, single-use |
| Expiry | 1-300 seconds (default 60) |
| Input validation | sub ≤256 chars, action ≤64 chars |
| Token size | 2KB max |
| Error handling | No internal details leaked |

**Performance:** ~3.3ms total verify time (Apple M1)

---

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/mint` | POST | Issue signed token |
| `/proxy` | POST | Verify token |
| `/audit` | GET | Recent verified tokens |
| `/metrics` | GET | Telemetry counters |
| `/health` | GET | Health check |

---

## Use Cases

| Scenario | Action string |
|----------|---------------|
| Refund | `refund:order:123:amount:50` |
| Deploy | `deploy:prod:api:v2.1.0` |
| Data export | `export:users:csv` |
| GDPR deletion | `delete:user:789` |
| Compute purchase | `compute:aws:instances:10` |

Action is freeform. You define the schema. AgentMint signs, verifies, logs.

---

## WebAuthn (Optional)

Hardware key authentication for high-security environments:
```bash
WEBAUTHN_RP_ID=localhost WEBAUTHN_RP_ORIGIN=http://localhost:3000 cargo run
```

| Endpoint | Description |
|----------|-------------|
| `/webauthn/register/start` | Begin passkey registration |
| `/webauthn/register/finish` | Complete registration |
| `/webauthn/auth/start` | Begin authentication |
| `/webauthn/auth/finish` | Complete authentication |

- Lockout after 5 failed attempts (15 min)
- Challenge TTL: 5 minutes
- Rate limiting per user

---

## FAQ

**How are users verified?**

AgentMint trusts the caller (your backend). It sits behind your existing auth. The protection is the Ed25519 signature—you can't forge a token without the private key.

**Who verifies the token?**

The resource provider. Agent is just transport.

**What's missing?**

- No built-in IdP verification (trusts caller)
- No resource provider integrations yet
- Open question: should this become an OAuth extension?

See: [IETF AAuth Draft](https://www.ietf.org/archive/id/draft-patwhite-aauth-00.html)

---

## Integration (Python)
```python
import requests

def mint(user: str, action: str) -> str:
    r = requests.post("http://localhost:3000/mint", json={
        "sub": user, "action": action, "ttl_seconds": 60
    })
    return r.json()["token"]

def verify(token: str) -> dict:
    r = requests.post("http://localhost:3000/proxy", json={"token": token})
    return r.json() if r.ok else None

token = mint("alice@company.com", "refund:order:123:amount:50")
result = verify(token)
print(f"Approved by: {result['sub']}")
```

---

## Docker
```bash
docker build -t agentmint .
docker run -p 3000:3000 agentmint
```

---

## License

MIT

## Author

[Aniketh Maddipati](https://github.com/aniketh-maddipati)

---

## Looking for design partners

Building agents that take real actions? I'd love your feedback on whether this solves a real problem.

[Open an issue](https://github.com/aniketh-maddipati/agentmint/issues) or DM me on [LinkedIn](https://linkedin.com/in/aniketh-maddipati) / [X](https://x.com/aniketh_m).