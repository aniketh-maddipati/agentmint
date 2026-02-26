# 🔐 AgentMint

**What it does:** Signs short-lived tokens proving a human approved a specific AI agent action. Verifies those tokens in milliseconds. Logs everything.

**Why it matters:** AI agents can call APIs, but nothing proves a human authorized a specific call. Session-level auth says "this agent can access Stripe." AgentMint says "Alice approved this $50 refund for order #123 at 10:03am."

**How it works:**
1. Human approves action → AgentMint signs a token (60s expiry, single-use)
2. Agent includes token in request → AgentMint verifies signature, checks expiry, blocks replay
3. Action proceeds → Full audit trail of who approved what, when

## Quick Start

```bash
git clone https://github.com/aniketh-maddipati/agentmint
cd agentmint
cargo run
```

You'll see:
```
╔═══════════════════════════════════════════════════════════╗
║     🔐 AgentMint v0.1.0                                   ║
║     Cryptographic proof of human authorization            ║
╚═══════════════════════════════════════════════════════════╝

✓ Server started
  → http://0.0.0.0:3000

Endpoints:
  POST /mint   Issue signed token
  POST /proxy  Verify token
  GET  /audit  View audit log
  GET  /metrics Telemetry
  GET  /health Health check
```

## Try It

Open a new terminal:

### 1. Mint a token

```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"alice@test.com","action":"refund:order:123","ttl_seconds":60}'
```

Response:
```json
{
  "token": "eyJqdGkiOi...",
  "jti": "f1268944-d428-4d55-b1a4-db3560650c03",
  "exp": "2026-02-26T14:39:14+00:00"
}
```

Console shows:
```
 MINT  sub: alice@test.com action: refund:order:123 jti:f1268944
```

### 2. Verify the token

```bash
curl -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d '{"token":"<paste token from step 1>"}'
```

Response:
```json
{
  "sub": "alice@test.com",
  "action": "refund:order:123",
  "jti": "f1268944-d428-4d55-b1a4-db3560650c03"
}
```

Console shows:
```
 OK  jti:f1268944 3305μs ✓
```

### 3. Replay the token (blocked)

```bash
curl -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d '{"token":"<same token>"}'
```

Response:
```
token rejected
```

Console shows:
```
 REPLAY  jti:f1268944 blocked
```

### 4. Check metrics

```bash
curl http://localhost:3000/metrics
```

```json
{
  "tokens_minted": 1,
  "tokens_verified": 1,
  "tokens_rejected": 0,
  "replays_blocked": 1
}
```

### 5. View audit log

```bash
curl http://localhost:3000/audit
```

```json
[
  {
    "jti": "f1268944-d428-4d55-b1a4-db3560650c03",
    "sub": "alice@test.com",
    "action": "refund:order:123",
    "verified_at": "2026-02-26T14:38:14+00:00"
  }
]
```

## One-liner Test

```bash
TOKEN=$(curl -s -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"test","action":"test","ttl_seconds":60}' | grep -o '"token":"[^"]*"' | cut -d'"' -f4) && \
curl -s -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d "{\"token\":\"$TOKEN\"}"
```

## Schema Agnostic

The action field is a freeform string. You define what it means.

| Use Case | Example Action |
|----------|----------------|
| Refund | `refund:order:123:max:50` |
| Deploy | `deploy:prod:api:v2.1.0` |
| Data export | `export:users:csv` |
| Email | `email:to:ceo@company.com` |
| DB write | `db:delete:user:456` |

AgentMint signs it, verifies it, logs it. Your app interprets it.

## Example Payloads

Refund:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"support@co.com","action":"refund:order:123:max:99","ttl_seconds":120}'
```

Deploy:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"deploy-bot","action":"deploy:prod:api:v2","ttl_seconds":300}'
```

GDPR deletion:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"gdpr-proc","action":"delete:user:789","ttl_seconds":60}'
```

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| /mint | POST | Issue signed token |
| /proxy | POST | Verify token |
| /audit | GET | Recent verified tokens |
| /metrics | GET | Counters |
| /health | GET | Health check |

## Token Format

```json
{
  "jti": "unique-id",
  "sub": "who-approved",
  "action": "what-they-approved",
  "iat": "2026-02-26T14:38:14Z",
  "exp": "2026-02-26T14:39:14Z"
}
```

Signed with Ed25519. Format: `base64url(payload).base64url(signature)`

## Security

- Ed25519 signatures (constant-time)
- Single-use tokens (JTI tracking)
- Short expiry (1-300s, default 60)
- Input validation (sub ≤256, action ≤64)
- Token size limit (2KB max)
- Errors don't leak internals
- Security headers (X-Frame-Options, X-Content-Type-Options, Cache-Control)

## Performance

Measured on Apple M1:

| Operation | Time |
|-----------|------|
| Signature verify | ~1.6ms |
| JTI check | ~15μs |
| Audit log write | ~1.6ms |
| **Total request** | **~3.3ms** |

Check `X-Verify-Time-Us` response header for actual timing.

## Docker

```bash
docker build -t agentmint .
docker run -p 3000:3000 agentmint
```

## Integration Example (Python)

```python
import requests

def approve(user: str, action: str) -> str:
    r = requests.post("http://localhost:3000/mint", json={
        "sub": user, "action": action, "ttl_seconds": 60
    })
    return r.json()["token"]

def verify(token: str) -> dict:
    r = requests.post("http://localhost:3000/proxy", json={"token": token})
    if r.ok:
        return r.json()
    raise Exception("Rejected")

token = approve("alice@co.com", "refund:order:123")
result = verify(token)
print(f"Authorized by: {result['sub']}")
```

## License

MIT

## Author

Aniketh Maddipati - https://github.com/aniketh-maddipati
