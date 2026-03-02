# 🔐 AgentMint

**Cryptographic proof that a human approved an AI agent action.**

Ed25519 signed receipts. Single-use. Time-limited. Extended for multi-agent delegation chains with checkpoint escalation.

~1200 lines of Rust. MIT licensed.

---

## The Problem

One agent is easy. You approve, it acts.

Three agents working in parallel on the same codebase? Who approved what? Can Agent 3 deploy to production while you're reviewing Agent 1? If something breaks, can you prove which human authorized which action through which chain of delegation?

Existing solutions don't solve this. Session auth says "this agent can access Stripe." Logging says "something happened." Neither proves a human authorized a specific action at a specific time through a specific delegation chain.

---

## The Solution

AgentMint issues cryptographic receipts that prove human authorization for AI agent actions.

**Basic flow:** Human approves action → AgentMint signs a receipt (Ed25519, single-use, 60s default TTL) → Agent presents receipt → Provider verifies before executing.

**Orchestration flow:** Human approves a plan with scoped authorization → Sub-agents receive delegated receipts within the approved scope → Actions outside scope trigger checkpoint escalation → Every receipt chains back to the original human approval.

---

## Quick Start

```bash
git clone https://github.com/aniketh-maddipati/agentmint
cd agentmint
cargo run
```

```
🔐 AgentMint v0.1.0
Cryptographic proof of human authorization

✓ Server ready → http://0.0.0.0:3000
```

### Run the orchestration demo

```bash
pip3 install requests
python3 demo.py
```

The demo walks through the full flow: plan approval, scoped delegation, checkpoint escalation, rogue agent denial, and audit trail.

---

## Orchestration: Delegation Chains

This is what makes AgentMint different. Not just signed tokens — verifiable delegation chains across multiple agents.

### 1. Human approves a plan

```bash
curl -X POST http://localhost:3000/mint \
  -H "Content-Type: application/json" \
  -d '{
    "sub": "aniketh@company.com",
    "action": "deploy:api-v2",
    "ttl_seconds": 300,
    "scope": ["build:*", "test:*", "deploy:staging"],
    "delegates_to": ["build-agent", "test-agent", "deploy-agent"],
    "requires_checkpoint": ["deploy:production"],
    "max_delegation_depth": 2
  }'
```

The plan receipt carries the rules: which agents, which actions, which actions need re-approval.

### 2. Agents request scoped delegation

```bash
curl -X POST http://localhost:3000/delegate \
  -H "Content-Type: application/json" \
  -d '{
    "parent_token": "<plan receipt>",
    "agent_id": "build-agent",
    "action": "build:docker"
  }'
```

```json
{
  "status": "ok",
  "jti": "9fbd8b71-...",
  "chain": ["87971956-...", "9fbd8b71-..."]
}
```

The chain links every delegated receipt back to the original plan.

### 3. Dangerous actions trigger checkpoints

```bash
curl -X POST http://localhost:3000/delegate \
  -d '{"parent_token": "<plan>", "agent_id": "deploy-agent", "action": "deploy:production"}'
```

```json
{
  "status": "checkpoint_required",
  "reason": "action 'deploy:production' requires explicit human approval"
}
```

The agent cannot proceed. A human must approve a new receipt specifically for this action.

### 4. Unauthorized agents are denied

```json
{
  "status": "denied",
  "reason": "agent_not_authorized"
}
```

---

## Threat Model

| Threat | Protection |
|--------|------------|
| **Substitution** — approve one payload, execute another | Receipt signature covers the exact action string |
| **Replay** — reuse an old approval | Single-use JTI tracking; each receipt consumed on first use |
| **Enforcement bypass** — execute without authorization | Fail-closed validation; any check failure = denial |
| **Scope creep** — agent acts beyond approved scope | Wildcard pattern matching on scope field |
| **Delegation abuse** — unbounded agent-to-agent delegation | Depth limits + named agent authorization |

---

## How It Works

```
Human approves plan
    │
    ▼
┌──────────────────────────────────────────────┐
│  AgentMint mints plan receipt                │
│  scope: [build:*, test:*, deploy:staging]    │
│  delegates_to: [build-agent, test-agent]     │
│  requires_checkpoint: [deploy:production]    │
└──────────┬───────────────┬───────────────────┘
           │               │
           ▼               ▼
     ┌───────────┐   ┌───────────┐
     │ build-agent│   │ test-agent│
     │ build:*   ✓│   │ test:*   ✓│
     └───────────┘   └───────────┘
                           │
                           ▼
                    ┌─────────────┐
                    │ deploy-agent│
                    │ staging   ✓ │
                    │ production ⚠│ ← CHECKPOINT
                    └─────────────┘
                           │
                    human re-approves
                           │
                           ▼
                    ┌─────────────┐
                    │ production ✓│
                    └─────────────┘
```

---

## AIUC-1 Control Mapping

AgentMint satisfies several mandatory controls from the AIUC-1 AI certification standard:

**D003 — Restrict unsafe tool calls (mandatory)**
The `scope` field on plan receipts defines exactly which actions agents can perform. Wildcard patterns (`build:*`) allow flexibility within boundaries. Actions outside scope are denied. The `/delegate` endpoint enforces this on every request.

**E004 — Document approvals with evidence (mandatory)**
Every approval is an Ed25519 signed receipt with: who approved it (`sub`), what was approved (`action`), when (`iat`, `exp`), and a unique identifier (`jti`). The SQLite audit log provides a tamper-evident record. This is not a log entry — it's a cryptographic artifact.

**B006 — Limit agent scope (mandatory)**
The `scope` field directly implements agent scope limiting. `["build:*", "test:*", "deploy:staging"]` means the agent can build and test anything but can only deploy to staging. Production requires a checkpoint.

**B007 — Enforce user access (mandatory)**
The `delegates_to` field names which agents can receive delegation. Combined with `original_approver` tracking through the chain, every action traces back to a verified human identity.

**C007 — Flag high-risk actions for review (optional)**
The `requires_checkpoint` field flags specific action patterns for mandatory human re-approval. When triggered, the system returns `checkpoint_required` and the agent cannot proceed.

---

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/mint` | POST | Issue signed receipt (basic or plan) |
| `/delegate` | POST | Request scoped delegation from a parent receipt |
| `/proxy` | POST | Verify and consume a receipt |
| `/audit` | GET | View audit trail |
| `/metrics` | GET | Telemetry counters |
| `/health` | GET | Health check |

### Mint request (with orchestration)

```json
{
  "sub": "alice@company.com",
  "action": "deploy:api-v2",
  "ttl_seconds": 300,
  "scope": ["build:*", "test:*"],
  "delegates_to": ["build-agent", "test-agent"],
  "requires_checkpoint": ["deploy:production"],
  "max_delegation_depth": 2
}
```

All orchestration fields are optional. Without them, mint behaves as a basic single-action receipt.

### Delegate request

```json
{
  "parent_token": "<signed plan receipt>",
  "agent_id": "build-agent",
  "action": "build:docker"
}
```

### Delegate response

```json
{
  "status": "ok",
  "token": "<signed delegated receipt>",
  "jti": "9fbd8b71-...",
  "chain": ["87971956-...", "9fbd8b71-..."]
}
```

Status is one of: `ok`, `denied`, `checkpoint_required`.

---

## Security

| Property | Implementation |
|----------|----------------|
| Signatures | Ed25519 (constant-time, via ed25519-dalek) |
| Replay protection | Single-use JTI tracking |
| Expiry | 1–300 seconds (default 60) |
| Delegation depth | Configurable max, default 2 |
| Enforcement | Fail-closed on any validation error |
| Input validation | sub ≤256 chars, action ≤64 chars, 2KB token limit |
| Audit | SQLite with JTI primary key (duplicates rejected) |

---

## WebAuthn (Optional)

Hardware key authentication for high-security environments:

```bash
WEBAUTHN_RP_ID=localhost WEBAUTHN_RP_ORIGIN=http://localhost:3000 cargo run
```

---

## Integration (Python)

```python
import requests

BASE = "http://localhost:3000"

# Mint a plan receipt
plan = requests.post(f"{BASE}/mint", json={
    "sub": "alice@company.com",
    "action": "deploy:api-v2",
    "scope": ["build:*", "test:*"],
    "delegates_to": ["build-agent"],
    "requires_checkpoint": ["deploy:production"],
}).json()

# Delegate to an agent
result = requests.post(f"{BASE}/delegate", json={
    "parent_token": plan["token"],
    "agent_id": "build-agent",
    "action": "build:docker",
}).json()

print(f"Status: {result['status']}")
print(f"Chain: {result['chain']}")
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

## Design Partners

Building agents that take real actions? I'd love your feedback.

[Open an issue](https://github.com/aniketh-maddipati/agentmint/issues) · [LinkedIn](https://linkedin.com/in/aniketh-maddipati) · [X](https://x.com/aniketh745)