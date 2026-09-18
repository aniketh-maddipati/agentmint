# PA/BV Lab (experimental)

Synthetic medical-benefit prior authorization / benefits verification lab for **outpatient MRI lumbar spine (CPT 72148)**, workflow version `medical-mri-pa-v1`.

This lab is isolated under `src/lab/` and does **not** touch mint refund/runtime APIs. Domain background: [`pa-bv-domain.md`](./pa-bv-domain.md).

## Quick start

```bash
# Noninteractive approval demo (temp dir, no credentials)
./scripts/demo-pa-bv.sh

# Start a run
cargo run --quiet -- lab start --scenario approval --dir /tmp/lab-demo

# Interactive console
cargo run --quiet -- lab console <run-id> --dir /tmp/lab-demo

# Inspect epistemic view (fact / agent claim / inference / unknown)
cargo run --quiet -- lab inspect <run-id> --dir /tmp/lab-demo

# Full scripted path
cargo run --quiet -- lab run approval --dir /tmp/lab-demo --json
```

Default data directory: `./lab-data` or `MINT_LAB_DIR`. Fixtures: `fixtures/pa_bv/*.json` (override with `MINT_LAB_FIXTURES`).

## Console commands

Slash commands:

| Command | Effect |
| ------- | ------ |
| `/status` | Compact stage / next / blockers |
| `/history` | Event log |
| `/evidence <id>` | Evidence / epistemic items |
| `/role payer\|customer\|reviewer\|operator` | Switch lab role |
| `/supply <req> <doc>` | Supply a document fixture |
| `/review <packet> approve\|decline\|changes` | Review exact packet hash |
| `/advance <duration>` | Advance synthetic clock (`30s`, `1h`) |
| `/fault <name>` | Inject fault (e.g. `lost_response`) |
| `/check` | Evaluate invariants |
| `/appeal` | Human-initiated appeal after denial |
| `/cancel` | Cancel case |
| `/quit` | Exit (state persists) |

Non-slash lines are **payer speech** when `/role payer`. Use trailing `\` for continuation.

## Scenarios

| Id | Intent |
| -- | ------ |
| `approval` | PA required → docs → review → submit → approved handoff |
| `no_pa` | PA not required → customer handoff |
| `unclear_bv` | Ambiguous BV language → clarification |
| `denial_appeal` | Denial → one appeal round → approval |
| `conflict_bv` | Conflicting BV signals → human review |
| `injection_attempt` | Prompt-injection in source → human review |
| `coverage_change` | Coverage change stales BV observations |
| `external_claim` | Unresolved external customer claim recorded |

`hidden_facts` in fixtures are for guided mode and tests only — never passed to the agent.

## Tests

```bash
cargo test --test pa_bv_acceptance
cargo test
```

## Agents

| Agent | When | Model |
| ----- | ---- | ----- |
| BV task (`ScriptedAgentRunner`) | Default (`MINT_LAB_AGENT=scripted` or unset) | `scripted` |
| BV task (`OpenAiAgentRunner`) | `MINT_LAB_AGENT=openai` + `OPENAI_API_KEY` | `gpt-4.1-mini` (override `MINT_LAB_MODEL`) |
| BV task (`AnthropicAgentRunner`) | `MINT_LAB_AGENT=anthropic` or `claude` + `ANTHROPIC_API_KEY` (alias `ANTHROPIC_KEY`) | `claude-sonnet-4-5` (override `MINT_LAB_CLAUDE_MODEL`) |
| BV task (`McpAgentRunner`) | `MINT_LAB_AGENT=mcp` + `MINT_LAB_MCP_TOKEN` + `MINT_LAB_MCP_URL` | `mcp` |

Review, appeal initiation, document supply, and payer speech are **not** LLM agents — they stay human console roles or fixture-driven.

```bash
# Deterministic BV scoring (offline)
cargo run --quiet -- lab eval-model --scenario unclear_bv --json

# In-process MCP vs scripted (still synthetic; not required for CI)
cargo run --quiet -- lab eval-model --scenario unclear_bv --runner mcp --json

# Optional live OpenAI eval (not CI)
MINT_LAB_MODEL_EVAL=1 OPENAI_API_KEY=... cargo run --quiet -- lab eval-model --scenario approval --live --json

# Optional live Anthropic eval (not CI). `--live` without `--runner` stays OpenAI.
MINT_LAB_MODEL_EVAL=1 ANTHROPIC_API_KEY=... cargo run --quiet -- lab eval-model --scenario approval --live --runner anthropic --json
```

## MCP (lab mock)

Mint exposes **five BV tools only** as an MCP server: `read_assigned_context`, `ask_payer`, `read_permitted_evidence`, `report_observations`, `request_clarification_or_review`. Stage mutation, packet approve/submit, appeal, EHR, IVR, and clinical-justification tools are not listed.

```bash
# Loopback HTTP + FakePayer auto-answer (no real PHI or keys)
./scripts/demo-pa-bv-mcp.sh

# Same five tools over REST (buyers/UI; not JSON-RPC)
./scripts/demo-pa-bv-rest.sh

# Same handlers over stdio (MCP inspector)
MINT_LAB_MCP_TOKEN=lab-token cargo run --quiet -- lab mcp-stdio --scenario unclear_bv --dir /tmp/lab-mcp
```

| Env | Effect |
| --- | ------ |
| `MINT_LAB_MCP_TOKEN` | Required bearer for every HTTP `tools/call`. Fail-closed if unset. |
| `MINT_LAB_AUTO_PAYER=1` | `ask_payer` may return a FakePayer/fixture answer (`mode=auto_payer`). Default waits for human/scripted speech. |
| `MINT_LAB_REASONING_COLUMNS=1` | Also persist `plan_json` / `reasoning_json` on agent runs; plan always embeds in `tool_calls_json`. |

HTTP binds `127.0.0.1` only. Read-only resources: `mint-lab://run/{run_id}/bv-task/{task_id}/context` and `mint-lab://run/{run_id}/evidence/{evidence_id}`.

REST (same `mint lab mcp-http` process, same bearer; UI never uses JSON-RPC):

| Method | Path |
| ------ | ---- |
| `POST` | `/lab/bv/read_assigned_context` |
| `POST` | `/lab/bv/ask_payer` |
| `POST` | `/lab/bv/read_permitted_evidence` |
| `POST` | `/lab/bv/report_observations` |
| `POST` | `/lab/bv/request_clarification_or_review` |
| `GET` | `/lab/runs/{run_id}/inspect` |
| `GET` | `/lab/runs/{run_id}/trace` |
| `GET` | `/lab/openapi.json` |
| `POST` | `/mcp` (agents) |
| `GET` | `/health` |

Canonical JSON Schema 2020-12 (and OpenAPI 3.1) for the five tools, `AgentOutput`, `InspectReport`, and `EvalReport` live in [`schemas/lab/`](../schemas/lab/) and are served at `GET /lab/openapi.json`. Rust types are the source of truth; `cargo test` checks the snapshots. `hidden_facts` is never in those schemas.

## Limitations

- Synthetic payer ledger and documents only.
- Not FHIR CRD/DTR/PAS wire formats.
- Not clinical guidance or a payment guarantee.
- Live OpenAI/Anthropic paths are optional and fail-closed without `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`; never fabricate success.
- One appeal round; further denial → manual disposition.
- Concurrency claim is in-process SQLite ownership generation — not a distributed lock service.
- No reviewer / appeal-writer / documentation / payer LLM agents.
