# AgentMint

Cryptographic proof that a human approved an AI agent action.

Signed receipts. Single-use. Time-limited. Works for one agent or a delegation chain.

**[Live Site](https://agent-mint.dev)** | **[GitHub](https://github.com/aniketh-maddipati/agentmint)**

## What it does

AgentMint gives you a signed receipt for a specific action.

That means:

- a human approved it
- the action string is fixed
- the receipt expires
- the receipt can only be used once
- delegated agents stay inside scope

Good fit for:

- tool use with approval
- multi-agent delegation
- checkpoint / re-approval flows
- audit trails that are more than plain logs

## Quick start

```bash
git clone https://github.com/aniketh-maddipati/agentmint
cd agentmint
cargo run
```

Server:

```text
http://0.0.0.0:3000
```

Health check:

```bash
curl http://localhost:3000/health
```

## Simple flow

### 1. Mint a receipt

```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{
    "sub": "aniketh@company.com",
    "action": "deploy:staging",
    "ttl_seconds": 60
  }'
```

Example response:

```json
{
  "token": "<signed receipt>",
  "jti": "87971956-..."
}
```

### 2. Use it

```bash
curl -X POST http://localhost:3000/proxy \
  -H "Authorization: Bearer <signed receipt>"
```

### 3. Reuse fails

Same receipt again should be rejected because it is single-use.

## Delegation flow

Mint a plan receipt with scope:

```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{
    "sub": "aniketh@company.com",
    "action": "release:api",
    "ttl_seconds": 300,
    "scope": ["build:*", "test:*", "deploy:staging"],
    "delegates_to": ["build-agent", "test-agent", "deploy-agent"],
    "requires_checkpoint": ["deploy:production"],
    "max_delegation_depth": 2
  }'
```

Delegate from that receipt:

```bash
curl -X POST http://localhost:3000/delegate \
  -H "Content-Type: application/json" \
  -d '{
    "parent_token": "<plan receipt>",
    "agent_id": "build-agent",
    "action": "build:docker"
  }'
```

Example success:

```json
{
  "status": "ok",
  "token": "<delegated receipt>",
  "jti": "9fbd8b71-...",
  "chain": ["87971956-...", "9fbd8b71-..."]
}
```

Checkpoint example:

```bash
curl -X POST http://localhost:3000/delegate \
  -H "Content-Type: application/json" \
  -d '{
    "parent_token": "<plan receipt>",
    "agent_id": "deploy-agent",
    "action": "deploy:production"
  }'
```

```json
{
  "status": "checkpoint_required",
  "reason": "action '\''deploy:production'\'' requires explicit human approval"
}
```

## Endpoints

| Endpoint | Method | Use |
|---|---|---|
| `/mint` | `POST` | Mint a signed receipt |
| `/delegate` | `POST` | Create a scoped delegated receipt |
| `/proxy` | `POST` | Verify and consume a receipt |
| `/audit` | `GET` | Read audit history |
| `/metrics` | `GET` | Read counters |
| `/health` | `GET` | Health check |

## Request shapes

Basic mint:

```json
{
  "sub": "alice@company.com",
  "action": "deploy:staging",
  "ttl_seconds": 60
}
```

Plan mint:

```json
{
  "sub": "alice@company.com",
  "action": "release:api",
  "ttl_seconds": 300,
  "scope": ["build:*", "test:*"],
  "delegates_to": ["build-agent", "test-agent"],
  "requires_checkpoint": ["deploy:production"],
  "max_delegation_depth": 2
}
```

Delegate:

```json
{
  "parent_token": "<signed plan receipt>",
  "agent_id": "build-agent",
  "action": "build:docker"
}
```

## Local demos

Run the Python demo:

```bash
pip3 install requests
python3 demo.py
```

Run the intervention viewer:

```bash
cargo run --bin agentmint-intervene
```

Open the printed local URL.

## Notes

- receipts are Ed25519 signed
- receipts are single-use through JTI tracking
- SQLite backs the audit log
- actions outside scope are denied
- checkpoint actions require fresh approval

## Status values

Delegate responses return one of:

- `ok`
- `denied`
- `checkpoint_required`

## License

MIT
