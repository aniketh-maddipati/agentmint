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

Review, appeal initiation, document supply, and payer speech are **not** LLM agents — they stay human console roles or fixture-driven.

```bash
# Deterministic BV scoring (offline)
cargo run --quiet -- lab eval-model --scenario unclear_bv --json

# Optional live OpenAI eval (not CI)
MINT_LAB_MODEL_EVAL=1 OPENAI_API_KEY=... cargo run --quiet -- lab eval-model --scenario approval --live --json
```

## Limitations

- Synthetic payer ledger and documents only.
- Not FHIR CRD/DTR/PAS wire formats.
- Not clinical guidance or a payment guarantee.
- Live OpenAI path is optional and fail-closed without `OPENAI_API_KEY`; never fabricates success.
- One appeal round; further denial → manual disposition.
- Concurrency claim is in-process SQLite ownership generation — not a distributed lock service.
- No reviewer / appeal-writer / documentation / payer LLM agents.
