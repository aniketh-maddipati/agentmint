# PA/BV Lab — MCP & Advanced Reasoning Mock Plan

**Status:** planning only (no implementation in this change)  
**Scope:** Mock Hippocratic AI–style and comparable prior-auth / benefits-verification **agent systems** against the Mint `medical-mri-pa-v1` lab.  
**Worktree:** `/workspace/.wt-pa-bv` on `cursor/pa-bv-case-console-0c0c`  
**Review date:** 2026-09-17  

Related docs: [`pa-bv-domain.md`](./pa-bv-domain.md), [`pa-bv-lab.md`](./pa-bv-lab.md).  
Does **not** supersede the agent-roster plan (BV-only LLM eligibility); this plan assumes that roster.

---

## 1. Grounding in current lab

| Surface | Current behavior relevant to this plan |
| ------- | -------------------------------------- |
| `AgentRunner` | BV-only; `ScriptedAgentRunner` + optional `OpenAiAgentRunner`. Outputs: `PendingQuestion` \| `Observations` \| `Clarification`. Never mutates case status. |
| Allowed tools (prompt/context) | Already declared: `read_assigned_context`, `ask_payer`, `read_permitted_evidence`, `report_observations`, `request_clarification_or_review`. Scripted path fakes a short `tool_calls_json` trace. |
| Workflow | Owns stage transitions; synthesizes `Determination` from observations (+ fixture `hidden_facts` in guided mode). |
| Human console | Roles `payer` / `customer` / `reviewer` / `operator`; payer speech, doc supply, packet review, appeal initiation. |
| Fake payer | Separate SQLite ledger; `inquire_bv` / submit / lookup; no real connectivity. |
| Eval | Emerging `eval.rs` / `lab eval-model`: score dimensions FactExtraction, Uncertainty, ToolAuthority, WorkflowDispositionHint. |

**Implication:** MCP and “advanced reasoning” should wrap or front the existing BV agent contract—not invent parallel clinical agents or give tools authority over stages.

---

## 2. What public systems claim (and what we mock)

### 2.1 Hippocratic AI (primary / public)

| Claim (public) | Source | Mock in Mint? |
| -------------- | ------ | ------------- |
| Orchestrators = teams of specialized agents for outcomes (provider / payor / life sciences) | [Orchestrator overview](https://hippocraticai.com/orchestrator-overview/) | **Partial:** one orchestrator façade + **one** BV specialist; other “roles” = human console or workflow, not LLMs. |
| AI Physician Front Door: eligibility/benefits/PA Q&A; assemble/submit PA package; track status; IVR hold; EHR write-back | [AI Physician Front Door](https://hippocraticai.com/ai-physician-front-door/) | **Partial:** eligibility/PA-requirement Q&A via BV tools; **not** package assembly LLM, IVR, EHR write-back. |
| AI Access & Reimb: PA status navigation, benefits explanation, IVR, pharmacy | [AI Access & Reimb](https://hippocraticai.com/ai-access-reimb/) | **Out of scope** (pharmacy / patient hub); keep medical MRI path only. |
| Polaris “constellation”: 30+ satellite models; narrow verifiable jobs; online + offline tool-call verifiers; escalate when unsure | [Constellation post](https://hippocraticai.com/constellation/) | **Mock pattern:** structured plan steps + tool-trace verifiers + fail-closed / human review—not 30 models. |
| MCP / tool-calling as deployment pattern | Job postings mention MCP; **no public tool schemas** | Treat as **integration style**, not a Hippocratic API to clone. |
| Safety-focused multi-model checking / patent narrative | [Safety patent post](https://hippocraticai.com/safety-focused-llm-patent/) | Mock as **deterministic validators** (evidence refs, schema, injection markers)—not clinical safety engines. |

**Speculation (mark clearly):** Exact Hippocratic tool names, MCP server layout, and PA/BV wire contracts are **not** public. Any concrete schemas below are **Mint lab inventions** inspired by public product language (eligibility, PA requirement, documentation completeness, escalation), not reverse-engineered APIs.

### 2.2 Comparable systems (secondary)

| System | Public pattern | Lab takeaway |
| ------ | -------------- | ------------ |
| AWS Bedrock AgentCore PA demo | Multi-agent: eligibility → clinical assembly → PA submit | Map eligibility agent → Mint BV; keep submit/review as workflow + humans. |
| SamaCare | Medical-benefit PA; human-in-the-loop specialists; escalate on friction | Aligns with console reviewer/operator; no autonomous clinical agent. |
| Cohere Health | Plan-side UM automation; clinicians retain final decisions | Lab already separates agent **observations** from workflow **determinations** and human review. |

---

## 3. What to mock vs what not to build

### Mock (in scope)

1. **External Hippocratic-like client** that discovers Mint tools via MCP (`tools/list` / `tools/call`) and drives a single BV task.
2. **Mint as MCP server** exposing a **minimal, read-mostly + typed-report** tool surface over an open BV task / case snapshot.
3. **Structured reasoning artifacts** suitable for a credible demo: short plan, tool trace, uncertainty flags, bounded replan (≤2 repairs)—already directionally present in `OpenAiAgentRunner`.
4. **Verifier satellites (deterministic):** schema, evidence-id allowlist, injection heuristic, “may require → unknown”, tool-authority checks—mirroring constellation *idea* without constellation *scale*.
5. **Eval parity:** scripted vs MCP-backed / OpenAI BV scored with the same `EvalReport` dimensions.

### Do not build

| Non-goal | Why |
| -------- | --- |
| Real Hippocratic / Polaris / constellation models | Not available; not needed for a lab mock. |
| Voice, telephony, IVR navigation | Out of lab; domain exclusions. |
| EHR / practice-management write-back | Domain exclusions; HIPAA surface. |
| FHIR CRD/DTR/PAS wire | Lab models concepts only. |
| Reviewer / appeal / documentation / payer **LLM** agents | Roster rule; humans + fixtures remain. |
| Agent tools that mutate `CaseStage`, approve packets, submit PA, or open appeals | Product rule: workflow owns transitions. |
| Full chain-of-thought dumps or open-ended clinical necessity reasoning | Unsafe + non-evaluable; domain forbids autonomous clinical justification. |
| Pharmacy / specialty-drug / copay / enrollment agents | Wrong benefit type for this lab. |
| Production OAuth IdP, PHI stores, live payer credentials | Lab remains synthetic. |
| Full MCP OAuth 2.1 PRM stack in v1 | Overkill; use lab-scoped bearer (see auth). |

---

## 4. MCP surface (concrete)

### 4.1 Architecture choice (recommended)

**Mint Lab MCP Server** (stdio or localhost HTTP) is the **source of truth** for case context. An **external mock agent client** (Hippocratic-like) calls tools; Mint validates and records traces into `AgentRunRecord.tool_calls_json`.

```
[Mock external agent / OpenAiAgentRunner via MCP client]
        │  MCP tools/call
        ▼
[Mint Lab MCP Server] ──read──► CaseStore snapshot (assigned context only)
        │
        │  report_* tools → AgentOutput (typed)
        ▼
[LabEngine.drive_bv] ──unchanged──► determinations / stages / humans
```

**Alternative (defer):** Mint as MCP *client* to a fake “Hippocratic tools” server. Useful for demo theater; worse for eval authority. Prefer Mint-as-server first.

### 4.2 Auth boundaries

| Layer | Rule |
| ----- | ---- |
| Transport | Loopback only in lab (`127.0.0.1` or stdio). No public bind by default. |
| Auth (v1) | Shared secret: `MINT_LAB_MCP_TOKEN` bearer on each `tools/call`. Fail closed if unset when MCP enabled. |
| Auth (later) | Optional MCP OAuth 2.1 PRM for realism demos—not required for mock credibility. |
| Scope | Token grants **one** `run_id` + **open BV `task_id`** (or short-lived session binding both). |
| Deny by default | Any tool not in the allowlist; any write that would change stage/disposition/submission. |
| Data | Synthetic fixtures only; refuse payloads that look like real MRN/SSN patterns (heuristic reject + log). |
| Audit | Every tool call appended to `tool_calls_json` with timestamp, tool name, ok/error, latency_us. |

### 4.3 Resources (optional, read-only)

| URI | Contents |
| --- | -------- |
| `mint-lab://run/{run_id}/bv-task/{task_id}/context` | Same shape as `assigned_context_value` today. |
| `mint-lab://run/{run_id}/evidence/{evidence_id}` | Permitted evidence blob only if id ∈ allowlist. |

Resources are convenience; **tools remain authoritative** for agent action.

### 4.4 Tools (minimal set)

Align names with today’s `allowed_tools` where possible. All inputs/outputs JSON Schema; all fail closed with typed errors (`unauthorized`, `unknown_evidence`, `invalid_schema`, `task_not_open`, `injection_detected`).

#### `read_assigned_context`

| | |
| -- | -- |
| **Purpose** | Hippocratic-like “policy/benefits lookup” context bootstrap without hidden_facts. |
| **Input** | `{ "run_id": uuid, "task_id": uuid }` |
| **Output** | Assigned context JSON: service, coverage (synthetic), conversation ids/text, document metadata (hashes/names, not clinical free-for-all beyond fixtures), `allowed_evidence_ids`, `allowed_tools`, `rules[]`. |
| **Side effects** | None (read). Trace entry only. |

#### `ask_payer`

| | |
| -- | -- |
| **Purpose** | Mock outbound BV inquiry / “hold line” *request*. |
| **Input** | `{ "run_id", "task_id", "question": string, "evidence_hint": "payer_bv_response" }` |
| **Output** | `{ "status": "pending", "pending_id": uuid }` matching `AgentOutput::PendingQuestion` path; or `{ "status": "answered", ... }` when auto-payer is on and a synthetic answer is available. |
| **Side effects** | Creates `PendingKind::AgentQuestion` via existing workflow APIs. |
| **Modes (toggle — build both)** | Default (`MINT_LAB_AUTO_PAYER` unset/`0`): wait for human `/role payer` speech or scenario `scripted_payer_answers`. Optional (`MINT_LAB_AUTO_PAYER=1`): resolve from `FakePayer.inquire_bv` / fixture auto-payer for noninteractive demos; always record mode in the tool trace. |

#### `read_permitted_evidence`

| | |
| -- | -- |
| **Purpose** | Narrow RAG stand-in: read one allowed evidence id. |
| **Input** | `{ "run_id", "task_id", "evidence_id": string }` |
| **Output** | `{ "evidence_id", "kind": "msg"|"doc"|"conversation"|..., "text"|"content_hash", "truncated": bool }` |
| **Side effects** | None. Reject unknown ids. |

#### `report_observations`

| | |
| -- | -- |
| **Purpose** | Typed BV conclusions (eligibility / coverage / PA requirement / docs / injection / clarification). |
| **Input** | `{ "run_id", "task_id", "observations": DraftObservation[], "needs_human_review": bool }` |
| **Output** | `{ "accepted": true, "observation_draft_count": n }` or validation error. |
| **Side effects** | Completes the agent turn; workflow persists observations and synthesizes determination—**agent still does not set stage**. |
| **Validators** | Non-empty observations; every `evidence_refs` ∈ allowlist; injection → force `needs_human_review`. |

#### `request_clarification_or_review`

| | |
| -- | -- |
| **Purpose** | Escalation satellite / human-in-the-loop (constellation “escalate”). |
| **Input** | `{ "run_id", "task_id", "message": string, "reason": "unclear"|"conflict"|"injection"|"malformed_payer"|"other" }` |
| **Output** | `{ "accepted": true }` → `AgentOutput::Clarification` or observations+review flag per reason mapping. |
| **Side effects** | Opens clarify/review tasks via existing engine paths; no stage self-mutation beyond what workflow already does. |

### 4.5 Explicitly excluded tools

Do **not** expose: `set_stage`, `approve_packet`, `submit_pa`, `initiate_appeal`, `supply_document`, `write_ehr`, `guarantee_payment`, `generate_clinical_justification`, `navigate_ivr`, `configure_hidden_facts`.

### 4.6 Mapping to MCP protocol ops

| MCP op | Lab behavior |
| ------ | ------------ |
| `initialize` | Advertise `tools` (+ optional `resources`); server name `mint-lab-pa-bv`. |
| `tools/list` | Return the five tools above with JSON Schema. |
| `tools/call` | AuthZ → validate task open & purpose=`benefits_verification` → execute → append trace. |
| `resources/read` | Optional; same allowlist as tools. |
| Prompts | Optional single prompt `bv_mri_72148_v1` pointing at prompt_version constants—defer. |

---

## 5. Advanced reasoning (credible mock only)

Goal: look like a constellation **lite**, not dump CoT or invent medical necessity.

### 5.1 Structured plan (required artifact)

Before/with first tool use, agent (or runner wrapper) emits a compact plan object (`bv-plan-v1`). **Storage toggle (build both paths):**

- **Default:** nest under `tool_calls_json` (e.g. `{ "plan": {...}, "calls": [...] }`) — no schema migration required for early PRs.
- **Opt-in columns:** when `MINT_LAB_REASONING_COLUMNS=1`, also persist `plan_json` / `reasoning_json` on `AgentRunRecord` for inspect UX.

Same plan schema either way:

```json
{
  "plan_version": "bv-plan-v1",
  "goal": "determine_pa_requirement",
  "steps": [
    {"id": "s1", "action": "read_assigned_context"},
    {"id": "s2", "action": "ask_payer_or_read_payer_msgs"},
    {"id": "s3", "action": "extract_observations"},
    {"id": "s4", "action": "verify_evidence_and_uncertainty"}
  ],
  "stop_conditions": ["pending_payer", "clarification", "observations_accepted"]
}
```

No free-text clinical essays. Steps are enum-like actions matching tools.

### 5.2 Tool trace

Every call: `{tool, args_digest, ok, error?, latency_us, step_id?}`. Already partially present; standardize for MCP and scripted.

### 5.3 Uncertainty

Reuse `Uncertainty::{Known, Unknown, NotApplicable}`. Rules:

- Ambiguous payer language (`may require`, `unable to determine`) → `Unknown` + clarification or `needs_human_review`.
- Never upgrade `Unknown` to `Known` without new evidence id.
- Payment guarantee language forbidden in statements (lint in validator).

### 5.4 Bounded replan

| Bound | Value |
| ----- | ----- |
| Max LLM/tool repair loops | 2 (matches current OpenAI runner) |
| Replan triggers | Schema fail, unknown evidence, empty observations |
| Replan action | One corrective `read_permitted_evidence` or tighten output; then `request_clarification_or_review` if still invalid |
| Forbidden | Open-ended “think harder” loops; inventing evidence |

### 5.5 Verifier satellites (deterministic, not LLM)

Mock constellation verifiers as pure functions (testable):

1. **SchemaVerifier** — `AgentOutput` deserializes.
2. **EvidenceVerifier** — `validate_output_evidence`.
3. **InjectionVerifier** — `detect_injection` (expand markers cautiously).
4. **UncertaintyVerifier** — “may require” ⇒ unknown.
5. **ToolAuthorityVerifier** — only allowlisted tools appeared in trace; no stage-mutation tools.

Online = run before accept; offline = `lab eval-model` / `lab check` post-hoc.

### 5.6 What “reasoning” must not include

- Token-level CoT or hidden scratchpads persisted to disk by default.
- Differential diagnosis or medical-necessity narrative generation.
- Appeals letter drafting.
- Claims that Mint “approved” care.

---

## 6. Role mapping (Hippocratic-like → Mint)

| Hippocratic-like public role / skill | Mint lab mapping | LLM? |
| ------------------------------------ | ---------------- | ---- |
| Orchestrator / supervisor | `LabEngine` + operator console `/status` | No |
| Eligibility & benefits / PA requirement agent | `AgentRunner::run_bv` (+ MCP tools) | **Yes (only)** |
| Policy / benefits lookup satellite | `read_assigned_context` + `read_permitted_evidence` | Tool, not agent |
| Payer IVR / hold-line agent | Human `/role payer` or scripted answers | No |
| Documentation assembly agent | Customer `/supply` + fixtures; workflow `drive_documentation` | No |
| Clinical justification collector | **Out of scope** (domain exclusion) | No |
| Packet submit / track / escalate | Workflow submission + follow-up; operator | No |
| Clinician / UM reviewer | `/role reviewer` + `/review` | No |
| Call supervisor / compliance | Deterministic verifiers + `lab check` + injection path | No LLM judge in v1 |
| Patient Access & Reimb hub | Not in this lab | — |

---

## 7. Evaluation hooks (mock vs scripted)

Reuse and extend current eval dimensions:

| Dimension | Pass criteria (MCP-aware) |
| --------- | ------------------------- |
| **FactExtraction** | Observation kinds match scenario expectation; no invented CPT/plan facts beyond context. |
| **Uncertainty** | Ambiguity scenarios require `Unknown` or clarification; no false `Known` PA. |
| **ToolAuthority** | Trace ⊆ allowlist; evidence refs valid; no stage-mutation tools; MCP auth failures counted as fail-closed success when expected. |
| **WorkflowDispositionHint** | After engine synthesis, disposition/stage hint matches scenario (`pa_required`, `unclear`, `human_review`, …). |

### Harness modes

1. **Scripted baseline** — `ScriptedAgentRunner` (CI default).  
2. **OpenAI JSON** — existing `OpenAiAgentRunner` (opt-in).  
3. **MCP mock client** — thin client that calls Mint MCP tools in a fixed or LLM-planned order; same `score_output` / end-to-end `lab run`.  

### Metrics to record (JSON)

- `plan_steps_executed`, `tool_call_count`, `repair_count`, `verifier_failures`, `latency_us` per tool, `overall_passed`.  
- Compare scripted vs MCP vs live on same scenario ids (`approval`, `no_pa`, `unclear_bv`, `conflict_bv`, `injection_attempt`).

### Non-metrics

- Empathy / voice EQ.  
- IVR success.  
- Clinical necessity accuracy (not in fixtures).

---

## 8. HIPAA / safety constraints (lab)

| Constraint | Enforcement |
| ---------- | ----------- |
| No real patient data | Fixtures + synthetic member ids only; MCP rejects high-entropy identifier patterns. |
| No real payer connectivity | `FakePayer` only; MCP cannot open network to payers. |
| Agents do not mutate case status | No stage tools; workflow sole writer of `CaseStage`. |
| Typed outputs + evidence | Validators gate `report_observations`. |
| Fail closed | Missing token / invalid schema / OpenAI errors → `Unverified` or clarification—not fabricated success. |
| Prompt injection | Detect → observation + human review / manual disposition (existing path). |
| No payment guarantee | Statement linter + docs. |
| Auditability | Event log + `AgentRunRecord` traces; MCP calls included. |
| Secrets | API keys and `MINT_LAB_MCP_TOKEN` via env; never in fixtures or traces (redact). |

---

## 9. Incremental delivery order (small PRs)

| PR | Deliverable | Acceptance |
| -- | ----------- | ---------- |
| **PR1** | Stabilize BV tool contract: document schemas for the five tools; standardize `tool_calls_json` shape for scripted + OpenAI (no MCP server yet). | Unit tests on trace shape; existing acceptance green. |
| **PR2** | Extract pure verifiers module used by agent + eval (schema/evidence/injection/uncertainty/tool-authority). | `cargo test`; eval dimensions call shared verifiers. |
| **PR3** | Add `bv-plan-v1` + bounded replan metadata; default embed in `tool_calls_json`; optional first-class columns behind `MINT_LAB_REASONING_COLUMNS=1`. | Snapshot/inspect shows plan; repair ≤2; both storage paths round-trip. |
| **PR4** | Mint MCP server with **both** transports sharing handlers: stdio (`mint lab mcp-stdio`) and localhost HTTP (loopback + `MINT_LAB_MCP_TOKEN`). Five tools only. | MCP inspector (stdio) and curl/client (HTTP) complete `unclear_bv` / `approval` with payer fixture or `MINT_LAB_AUTO_PAYER=1`. |
| **PR5** | `McpAgentRunner` implementing `AgentRunner` via stdio or HTTP client (still BV-only). | `MINT_LAB_AGENT=mcp` fail-closed without token/server. |
| **PR6** | Eval: score MCP runner vs scripted on fixture matrix; CI runs scripted only; live/MCP behind env flags. | JSON reports; no flake in default CI. |
| **PR7** (optional) | Read-only MCP resources + demo script in `scripts/` + short section in `pa-bv-lab.md`. | Demo without real PHI/keys. |

Do **not** fold reviewer LLM agents, FHIR, or OAuth IdP into these PRs.

---

## 10. Critical design decisions (locked for this plan)

1. **Only BV is agentic/LLM-eligible**; MCP does not expand the roster.  
2. **Mint owns the tool authority boundary**; external agents are untrusted clients.  
3. **Reasoning artifacts are structured and bounded**, not CoT transcripts.  
4. **Credibility comes from verifiers + HITL**, not from simulating voice/EHR/constellation scale.  
5. **`ask_payer` modes (toggle — build both):**  
   - Default: `MINT_LAB_AUTO_PAYER` unset/`0` → pending question waits for human console speech or scenario `scripted_payer_answers` (epistemic honesty).  
   - Optional: `MINT_LAB_AUTO_PAYER=1` → lab may resolve the pending question from `FakePayer.inquire_bv` / fixture-backed auto-payer for noninteractive demos. Never silent; record which mode ran in the tool trace.  
6. **Reasoning storage (toggle):**  
   - Default / CI: embed `bv-plan-v1` (+ repair metadata) inside `tool_calls_json` (or a nested `reasoning` object there) so no migration blocks PR3.  
   - Opt-in: `MINT_LAB_REASONING_COLUMNS=1` (or equivalent) persists first-class `plan_json` / `reasoning_json` on `AgentRunRecord` when inspect UX needs it. Both paths must round-trip the same plan schema.  
7. **MCP transport (build both):**  
   - **stdio** — primary for Cursor / MCP inspector / local scripts (`mint lab mcp-stdio`).  
   - **localhost HTTP** — same tool handlers behind loopback HTTP (`127.0.0.1` only) for “external Hippocratic-like client” demos. Shared auth (`MINT_LAB_MCP_TOKEN`), shared allowlist, no public bind by default.

---

## 11. Open questions

None — decisions in §10 items 5–7 supersede the prior open questions.

---

## 12. Sources

| Source | Role | Reviewed |
| ------ | ---- | -------- |
| https://hippocraticai.com/orchestrator-overview/ | Multi-agent orchestrator marketing | 2026-09-17 |
| https://hippocraticai.com/ai-physician-front-door/ | PA / eligibility / submit / track claims | 2026-09-17 |
| https://hippocraticai.com/ai-access-reimb/ | Access & reimb / PA support (mostly Rx hub) | 2026-09-17 |
| https://hippocraticai.com/constellation/ | Constellation, tool-call verifiers | 2026-09-17 |
| https://hippocraticai.com/safety-focused-llm-patent/ | Multi-model safety narrative | 2026-09-17 |
| https://modelcontextprotocol.io/ (tools, auth specs) | MCP tool/resource/auth patterns | 2026-09-17 |
| AWS Bedrock AgentCore PA blog; SamaCare; Cohere Health | Comparable HITL / multi-agent PA patterns | 2026-09-17 |
| In-repo `src/lab/{agent,workflow,payer,console,eval}.rs`, domain docs | Lab ground truth | 2026-09-17 |
