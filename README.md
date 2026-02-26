# 🔐 AgentMint

**What it does:** Signs short-lived tokens proving a human approved a specific AI agent action. Verifies those tokens in <1ms. Logs everything.

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

Output:
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
  -d '{"sub":"alice@company.com","action":"refund_order_123","ttl_seconds":60}'
```

Response:
```json
{
  "token": "eyJqdGkiOi...",
  "jti": "8680bbb6-c276-4507-ad34-13c9de1b5333",
  "exp": "2026-02-26T13:45:01+00:00"
}
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
  "sub": "alice@company.com",
  "action": "refund_order_123",
  "jti": "8680bbb6-c276-4507-ad34-13c9de1b5333"
}
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

### 4. View audit log

```bash
curl http://localhost:3000/audit
```

### 5. View metrics

```bash
curl http://localhost:3000/metrics
```

## One-liner Test

```bash
TOKEN=$(curl -s -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"test","action":"test"}' | grep -o '"token":"[^"]*"' | cut -d'"' -f4) && \
curl -X POST http://localhost:3000/proxy \
  -H "Content-Type: application/json" \
  -d "{\"token\":\"$TOKEN\"}"
```

## Schema Agnostic

The action field is a freeform string. You define what it means.

| Use Case | Example Action |
|----------|----------------|
| Refund | `refund_order_123_max_50` |
| Deploy | `deploy_prod_api_v2` |
| Data export | `export_users_csv` |
| Email | `email_to_ceo` |
| DB write | `db_delete_user_456` |

AgentMint signs it, verifies it, logs it. Your app interprets it.

## Example Payloads

Refund:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"support@co.com","action":"refund_ORD123_max99","ttl_seconds":120}'
```

Deploy:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"deploy_bot","action":"deploy_prod_api_v2","ttl_seconds":300}'
```

GDPR deletion:
```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{"sub":"gdpr_proc","action":"delete_user_789_all_pii","ttl_seconds":60}'
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
  "action": "what_they_approved",
  "iat": "2026-02-26T13:44:01Z",
  "exp": "2026-02-26T13:45:01Z"
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

## Performance

| Operation | Time |
|-----------|------|
| Verify signature | <100μs |
| JTI check | <10μs |
| Full request | <1ms |

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

token = approve("alice@co.com", "refund_order_123")
result = verify(token)
print(f"Authorized by: {result['sub']}")
```

## License

MIT

## Author

Aniketh Maddipati - https://github.com/aniketh-maddipati
