//! Mint Lab MCP server for the five BV tools.
//! Used by: `mint lab mcp-stdio` / `mint lab mcp-http` and McpAgentRunner.
//! Loopback or stdio only; bearer `MINT_LAB_MCP_TOKEN`; no stage-mutation tools.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::agent::{
    allowed_evidence_ids, assigned_context, build_result, AgentOutput, DraftObservation,
    PendingQuestion,
};
use crate::lab::domain::{CaseSnapshot, ObservationKind, Role, Uncertainty};
use crate::lab::error::{LabError, LabResult};
use crate::lab::inspect::InspectReport;
use crate::lab::payer::{BvInquiry, PayerAdapter};
use crate::lab::tools::{
    digest_args, is_allowed_tool, mcp_tool_list_payload, parse_tool_args, parse_tool_trace,
    tool_call, AskPayerMode, AskPayerRequest, AskPayerResponse, AskPayerStatus,
    ClarificationReason, EvidenceBlob, ReadAssignedContextRequest, ReadPermittedEvidenceRequest,
    ReportObservationsRequest, ReportObservationsResponse, RequestClarificationRequest,
    RequestClarificationResponse, ToolTrace, TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT,
    TOOL_READ_PERMITTED_EVIDENCE, TOOL_REPORT_OBSERVATIONS, TOOL_REQUEST_CLARIFICATION,
};
use crate::lab::verifiers::{detect_injection, validate_output_evidence};
use crate::lab::workflow::LabEngine;

pub mod client;
pub mod http;
pub mod stdio;

pub const SERVER_NAME: &str = "mint-lab-pa-bv";
pub const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub struct McpState {
    pub engine: Arc<LabEngine>,
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub token: String,
    pub apply: bool,
    pub auto_payer: bool,
    pub trace: Mutex<ToolTrace>,
}

#[derive(Debug, Clone)]
pub struct McpCallContext {
    pub authorized: bool,
}

pub fn require_mcp_token() -> LabResult<String> {
    let token = std::env::var("MINT_LAB_MCP_TOKEN").map_err(|_| {
        LabError::Unverified("MINT_LAB_MCP_TOKEN not set; MCP will not fabricate success".into())
    })?;
    if token.trim().is_empty() {
        return Err(LabError::Unverified(
            "MINT_LAB_MCP_TOKEN empty; MCP will not fabricate success".into(),
        ));
    }
    Ok(token)
}

pub fn auto_payer_enabled() -> bool {
    matches!(
        std::env::var("MINT_LAB_AUTO_PAYER").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

pub fn looks_like_real_identifier(text: &str) -> bool {
    if has_ssn_pattern(text) {
        return true;
    }
    let lower = text.to_lowercase();
    lower.contains("social security number")
        && text.chars().filter(|c| c.is_ascii_digit()).count() >= 9
}

fn has_ssn_pattern(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 11 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3] == b'-'
            && bytes[i + 4].is_ascii_digit()
            && bytes[i + 5].is_ascii_digit()
            && bytes[i + 6] == b'-'
            && bytes[i + 7].is_ascii_digit()
            && bytes[i + 8].is_ascii_digit()
            && bytes[i + 9].is_ascii_digit()
            && bytes[i + 10].is_ascii_digit()
        {
            return true;
        }
        i += 1;
    }
    false
}

pub fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    provided == Some(expected)
}

pub fn handle_rpc(state: &McpState, req: JsonRpcRequest, ctx: &McpCallContext) -> JsonRpcResponse {
    let id = req.id.clone().unwrap_or(Value::Null);
    if req.jsonrpc != "2.0" {
        return error_response(id, -32600, "invalid request", None);
    }
    if !ctx.authorized {
        return error_response(
            id,
            -32001,
            "unauthorized",
            Some(json!({"error": "unauthorized"})),
        );
    }
    match req.method.as_str() {
        "initialize" => success(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "notifications/initialized" | "initialized" => success(id, json!({})),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({ "tools": mcp_tool_list_payload() })),
        "tools/call" => match call_tool(state, &req.params) {
            Ok(result) => success(id, result),
            Err(err) => {
                let (code, message, data) = map_tool_error(&err);
                if code == "unauthorized" {
                    error_response(id, -32001, &message, data)
                } else {
                    success(
                        id,
                        tool_result(json!({"error": code, "message": message}), true),
                    )
                }
            }
        },
        "resources/list" => match list_resources(state) {
            Ok(result) => success(id, result),
            Err(err) => error_response(id, -32603, &err.to_string(), None),
        },
        "resources/read" => match read_resource(state, &req.params) {
            Ok(result) => success(id, result),
            Err(err) => {
                let (code, message, _data) = map_tool_error(&err);
                success(
                    id,
                    tool_result(json!({"error": code, "message": message}), true),
                )
            }
        },
        other => error_response(
            id,
            -32601,
            "method not found",
            Some(json!({"method": other})),
        ),
    }
}

fn success(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn error_response(id: Value, code: i32, message: &str, data: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_owned(),
            data,
        }),
    }
}

fn tool_result(body: Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": body.to_string() }],
        "isError": is_error,
        "structuredContent": body
    })
}

pub(crate) fn map_tool_error(err: &LabError) -> (&'static str, String, Option<Value>) {
    let message = err.to_string();
    let code = if message.contains("unauthorized") {
        "unauthorized"
    } else if message.contains("task_not_open") || message.contains("no open BV") {
        "task_not_open"
    } else if message.contains("unknown evidence") {
        "unknown_evidence"
    } else if message.contains("injection") {
        "injection_detected"
    } else if matches!(err, LabError::NotFound(_)) {
        "not_found"
    } else {
        "invalid_schema"
    };
    (code, message, Some(json!({"error": code})))
}

fn call_tool(state: &McpState, params: &Value) -> LabResult<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid("missing tool name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let body = dispatch_bv_tool(state, name, &args)?;
    let ok = body.get("error").is_none();
    Ok(tool_result(body, !ok))
}

/// Shared BV tool dispatch for MCP JSON-RPC and REST `POST /lab/bv/{tool}`.
pub fn dispatch_bv_tool(state: &McpState, name: &str, args: &Value) -> LabResult<Value> {
    if !is_allowed_tool(name) {
        return Err(LabError::Invalid(format!(
            "tool {name} is not on the BV allowlist"
        )));
    }
    scan_for_real_identifiers(args)?;
    let started = Instant::now();
    let body = match name {
        TOOL_READ_ASSIGNED_CONTEXT => tool_read_assigned_context(state, args)?,
        TOOL_ASK_PAYER => tool_ask_payer(state, args)?,
        TOOL_READ_PERMITTED_EVIDENCE => tool_read_permitted_evidence(state, args)?,
        TOOL_REPORT_OBSERVATIONS => tool_report_observations(state, args)?,
        TOOL_REQUEST_CLARIFICATION => tool_request_clarification(state, args)?,
        other => return Err(LabError::Invalid(format!("unknown tool {other}"))),
    };
    record_dispatch_trace(state, name, args, &body, started)?;
    Ok(body)
}

fn record_dispatch_trace(
    state: &McpState,
    name: &str,
    args: &Value,
    body: &Value,
    started: Instant,
) -> LabResult<()> {
    let latency_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    let ok = body.get("error").is_none();
    let status = body.get("status").and_then(Value::as_str).or_else(|| {
        if body.get("accepted").and_then(Value::as_bool) == Some(true) {
            Some("accepted")
        } else {
            None
        }
    });
    let mut trace = state
        .trace
        .lock()
        .map_err(|_| LabError::Storage("mcp trace lock poisoned".into()))?;
    let mut entry = tool_call(name, args, ok, None, latency_us, None, status);
    if name == TOOL_ASK_PAYER {
        entry.result_status = Some(
            if state.auto_payer {
                "answered"
            } else {
                "pending"
            }
            .into(),
        );
        if let Some(mode) = body.get("mode").and_then(Value::as_str) {
            trace.push_diagnostic(json!({
                "tool": TOOL_ASK_PAYER,
                "auto_payer": state.auto_payer,
                "mode": mode,
                "args_digest": digest_args(args)
            }));
        }
    }
    if !ok {
        entry.error = body
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned);
        entry.ok = false;
    }
    trace.push(entry);
    Ok(())
}

fn require_bound_run(state: &McpState, run_id: Uuid) -> LabResult<()> {
    if run_id != state.run_id {
        return Err(LabError::Invalid(
            "run_id/task_id do not match MCP session binding".into(),
        ));
    }
    Ok(())
}

pub fn inspect_bound_run(state: &McpState, run_id: Uuid) -> LabResult<InspectReport> {
    require_bound_run(state, run_id)?;
    crate::lab::inspect::inspect_run(&state.engine, run_id)
}

pub fn trace_bound_run(state: &McpState, run_id: Uuid) -> LabResult<ToolTrace> {
    require_bound_run(state, run_id)?;
    let session = state
        .trace
        .lock()
        .map_err(|_| LabError::Storage("mcp trace lock poisoned".into()))?
        .clone();
    if !session.calls.is_empty() {
        return Ok(session);
    }
    let snap = state.engine.snapshot(run_id)?;
    let Some(run) = snap.agent_runs.last() else {
        return Ok(session);
    };
    match parse_tool_trace(&run.tool_calls_json) {
        Ok(trace) => Ok(trace),
        Err(_) => Ok(session),
    }
}

fn scan_for_real_identifiers(value: &Value) -> LabResult<()> {
    match value {
        Value::String(s) if looks_like_real_identifier(s) => Err(LabError::Invalid(
            "payload resembles a real identifier; lab accepts synthetic fixtures only".into(),
        )),
        Value::Array(items) => {
            for item in items {
                scan_for_real_identifiers(item)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for v in map.values() {
                scan_for_real_identifiers(v)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn list_resources(state: &McpState) -> LabResult<Value> {
    let snap = state.engine.snapshot(state.run_id)?;
    let mut resources = vec![json!({
        "uri": format!("mint-lab://run/{}/bv-task/{}/context", state.run_id, state.task_id),
        "name": "assigned BV context",
        "mimeType": "application/json"
    })];
    let mut ids: Vec<String> = allowed_evidence_ids(&snap).into_iter().collect();
    ids.sort();
    for evidence_id in ids {
        resources.push(json!({
            "uri": format!("mint-lab://run/{}/evidence/{}", state.run_id, evidence_id),
            "name": format!("evidence {evidence_id}"),
            "mimeType": "application/json"
        }));
    }
    Ok(json!({ "resources": resources }))
}

fn read_resource(state: &McpState, params: &Value) -> LabResult<Value> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid("missing resource uri".into()))?;
    let snap = state.engine.snapshot(state.run_id)?;
    let task = snap
        .tasks
        .iter()
        .find(|t| t.id == state.task_id)
        .ok_or_else(|| LabError::NotFound(format!("task {}", state.task_id)))?;
    let context_prefix = format!(
        "mint-lab://run/{}/bv-task/{}/context",
        state.run_id, state.task_id
    );
    if uri == context_prefix {
        let body = assigned_context(task, &snap);
        let text = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
        return Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": text
            }]
        }));
    }
    let evidence_prefix = format!("mint-lab://run/{}/evidence/", state.run_id);
    if let Some(evidence_id) = uri.strip_prefix(&evidence_prefix) {
        let allowed = allowed_evidence_ids(&snap);
        if !allowed.contains(evidence_id) {
            return Err(LabError::Invalid(format!(
                "unknown evidence id {evidence_id}"
            )));
        }
        let body = read_evidence_blob(&snap, evidence_id);
        return Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": body.to_string()
            }]
        }));
    }
    Err(LabError::NotFound(format!("resource {uri}")))
}

fn bound_ids(state: &McpState, run_id: Uuid, task_id: Uuid) -> LabResult<(Uuid, Uuid)> {
    if run_id != state.run_id || task_id != state.task_id {
        return Err(LabError::Invalid(
            "run_id/task_id do not match MCP session binding".into(),
        ));
    }
    Ok((run_id, task_id))
}

fn to_json<T: serde::Serialize>(value: T) -> LabResult<Value> {
    serde_json::to_value(value).map_err(|err| LabError::Serialization(err.to_string()))
}

fn tool_read_assigned_context(state: &McpState, args: &Value) -> LabResult<Value> {
    let req: ReadAssignedContextRequest = parse_tool_args(args)?;
    let (run_id, task_id) = bound_ids(state, req.run_id, req.task_id)?;
    let (_case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    to_json(assigned_context(&task, &snap))
}

fn tool_ask_payer(state: &McpState, args: &Value) -> LabResult<Value> {
    let req: AskPayerRequest = parse_tool_args(args)?;
    let (run_id, task_id) = bound_ids(state, req.run_id, req.task_id)?;
    let question = req.question.trim();
    if question.is_empty() {
        return Err(LabError::Invalid("question empty".into()));
    }
    let evidence_hint = req.evidence_hint.as_str();
    let (case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;

    if state.auto_payer {
        let inquiry = BvInquiry {
            member_id: snap.case.coverage.member_id.clone(),
            plan_id: snap.case.coverage.plan_id.clone(),
            cpt: snap.case.service.cpt.clone(),
            dos: snap.case.coverage.dos.clone(),
        };
        let response = state.engine.payer.inquire_bv(&inquiry)?;
        let msg_id = state.engine.record_payer_speech(run_id, &response.text)?;
        return to_json(AskPayerResponse {
            status: AskPayerStatus::Answered,
            mode: AskPayerMode::AutoPayer,
            pending_id: None,
            text: Some(response.text),
            msg_id: Some(msg_id),
            configured_kind: response.configured_kind,
        });
    }

    let mut case = case;
    let fixture = state.engine.scenario_for(run_id, &snap.case.scenario_id)?;

    let output = AgentOutput::PendingQuestion(PendingQuestion {
        question: question.to_owned(),
        evidence_hint: evidence_hint.to_owned(),
    });
    let pending_id = if state.apply {
        persist_mcp_run(state, &snap, &task, &output)?;
        let mut happened = Vec::new();
        state
            .engine
            .apply_bv_output(&mut case, &fixture, &mut happened, &task, output)?
    } else {
        None
    };
    to_json(AskPayerResponse {
        status: AskPayerStatus::Pending,
        mode: AskPayerMode::HumanOrScripted,
        pending_id,
        text: None,
        msg_id: None,
        configured_kind: None,
    })
}

fn tool_read_permitted_evidence(state: &McpState, args: &Value) -> LabResult<Value> {
    let req: ReadPermittedEvidenceRequest = parse_tool_args(args)?;
    let (run_id, task_id) = bound_ids(state, req.run_id, req.task_id)?;
    let evidence_id = req.evidence_id;
    let (_case, _task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let allowed = allowed_evidence_ids(&snap);
    if !allowed.contains(&evidence_id) {
        return Err(LabError::Invalid(format!(
            "unknown evidence id {evidence_id}"
        )));
    }
    Ok(read_evidence_blob(&snap, &evidence_id))
}

fn read_evidence_blob(snap: &CaseSnapshot, evidence_id: &str) -> Value {
    const MAX: usize = 8000;
    if evidence_id == "conversation" {
        let text = snap
            .conversation
            .iter()
            .map(|m| format!("{:?}: {}", m.role, m.text))
            .collect::<Vec<_>>()
            .join("\n");
        return truncate_blob(evidence_id, "conversation", &text, None, MAX);
    }
    if evidence_id == "assigned_context" {
        let text = format!(
            "CPT {} plan {} DOS {}",
            snap.case.service.cpt, snap.case.coverage.plan_id, snap.case.coverage.dos
        );
        return truncate_blob(evidence_id, "conversation", &text, None, MAX);
    }
    if evidence_id == "payer_bv_response" {
        let text = snap
            .conversation
            .iter()
            .filter(|m| m.role == Role::Payer)
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return truncate_blob(evidence_id, "msg", &text, None, MAX);
    }
    if let Some(rest) = evidence_id.strip_prefix("msg:") {
        if let Ok(id) = Uuid::parse_str(rest) {
            if let Some(msg) = snap.conversation.iter().find(|m| m.id == id) {
                return truncate_blob(evidence_id, "msg", &msg.text, None, MAX);
            }
        }
    }
    if let Some(rest) = evidence_id.strip_prefix("doc:") {
        if let Ok(id) = Uuid::parse_str(rest) {
            if let Some(doc) = snap.documents.iter().find(|d| d.id == id) {
                return truncate_blob(
                    evidence_id,
                    "doc",
                    &doc.content,
                    Some(doc.content_hash.as_str()),
                    MAX,
                );
            }
        }
    }
    if let Some(doc) = snap
        .documents
        .iter()
        .find(|d| d.fixture_name == evidence_id)
    {
        return truncate_blob(
            evidence_id,
            "doc",
            &doc.content,
            Some(doc.content_hash.as_str()),
            MAX,
        );
    }
    truncate_blob(evidence_id, "other", "", None, MAX)
}

fn truncate_blob(
    evidence_id: &str,
    kind: &str,
    text: &str,
    content_hash: Option<&str>,
    max: usize,
) -> Value {
    let truncated = text.len() > max;
    let text = if truncated {
        text.chars().take(max).collect::<String>()
    } else {
        text.to_owned()
    };
    let blob = EvidenceBlob {
        evidence_id: evidence_id.to_owned(),
        kind: kind.to_owned(),
        text,
        content_hash: content_hash.map(str::to_owned),
        truncated,
    };
    serde_json::to_value(blob).unwrap_or(Value::Null)
}

fn tool_report_observations(state: &McpState, args: &Value) -> LabResult<Value> {
    let req: ReportObservationsRequest = parse_tool_args(args)?;
    let (run_id, task_id) = bound_ids(state, req.run_id, req.task_id)?;
    let (mut case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let mut observations = req.observations;
    let mut needs_human_review = req.needs_human_review;
    let blobs: Vec<String> = snap.conversation.iter().map(|m| m.text.clone()).collect();
    if detect_injection(&blobs).is_some()
        || observations
            .iter()
            .any(|o| o.kind == ObservationKind::InjectionAttempt)
    {
        needs_human_review = true;
        if !observations
            .iter()
            .any(|o| o.kind == ObservationKind::InjectionAttempt)
        {
            observations.push(DraftObservation {
                kind: ObservationKind::InjectionAttempt,
                statement: "Possible prompt-injection content detected in source material".into(),
                uncertainty: Uncertainty::Known,
                evidence_refs: vec!["conversation".into()],
            });
        }
    }
    let output = AgentOutput::Observations {
        observations: observations.clone(),
        needs_human_review,
    };
    let allowed = allowed_evidence_ids(&snap);
    validate_output_evidence(&output, &allowed).map_err(LabError::Invalid)?;
    if state.apply {
        persist_mcp_run(state, &snap, &task, &output)?;
        let fixture = state.engine.scenario_for(run_id, &snap.case.scenario_id)?;
        let mut happened = Vec::new();
        state
            .engine
            .apply_bv_output(&mut case, &fixture, &mut happened, &task, output)?;
    }
    to_json(ReportObservationsResponse {
        accepted: true,
        observation_draft_count: u32::try_from(observations.len()).unwrap_or(u32::MAX),
    })
}

fn tool_request_clarification(state: &McpState, args: &Value) -> LabResult<Value> {
    let req: RequestClarificationRequest = parse_tool_args(args)?;
    let (run_id, task_id) = bound_ids(state, req.run_id, req.task_id)?;
    let (mut case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let message = req.message.trim();
    if message.is_empty() {
        return Err(LabError::Invalid("clarification message empty".into()));
    }
    let output = match req.reason {
        ClarificationReason::Injection => AgentOutput::Observations {
            observations: vec![DraftObservation {
                kind: ObservationKind::InjectionAttempt,
                statement: message.to_owned(),
                uncertainty: Uncertainty::Known,
                evidence_refs: vec!["conversation".into()],
            }],
            needs_human_review: true,
        },
        _ => AgentOutput::Clarification {
            message: message.to_owned(),
        },
    };
    if state.apply {
        persist_mcp_run(state, &snap, &task, &output)?;
        let fixture = state.engine.scenario_for(run_id, &snap.case.scenario_id)?;
        let mut happened = Vec::new();
        state
            .engine
            .apply_bv_output(&mut case, &fixture, &mut happened, &task, output)?;
    }
    to_json(RequestClarificationResponse { accepted: true })
}

fn persist_mcp_run(
    state: &McpState,
    snap: &CaseSnapshot,
    task: &crate::lab::domain::Task,
    output: &AgentOutput,
) -> LabResult<()> {
    let trace = state
        .trace
        .lock()
        .map_err(|_| LabError::Storage("mcp trace lock poisoned".into()))?
        .clone();
    let mut result = build_result(
        task,
        snap,
        output.clone(),
        trace,
        crate::lab::mcp::client::PROMPT_VERSION_MCP,
        "mcp",
    );
    result.record.created_at = state.engine.now();
    state.engine.store.insert_agent_run(&result.record)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_mcp_state(
    scenario: &str,
    apply: bool,
    auto_payer: bool,
) -> (tempfile::TempDir, McpState) {
    use crate::lab::scenarios::fixtures_dir;
    use crate::lab::workflow::LabEngine;

    let dir = tempfile::tempdir().expect("tempdir");
    let engine = LabEngine::open(dir.path(), &fixtures_dir()).expect("engine");
    let run_id = engine.start_run(scenario).expect("start");
    let task_id = engine.prepare_bv_task(run_id).expect("bv task");
    let state = McpState {
        engine: Arc::new(engine),
        run_id,
        task_id,
        token: "lab-token".into(),
        apply,
        auto_payer,
        trace: Mutex::new(ToolTrace::new()),
    };
    (dir, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::agent::interpret_payer_text;
    use crate::lab::domain::CaseStage;

    fn state_for(scenario: &str, apply: bool, auto_payer: bool) -> (tempfile::TempDir, McpState) {
        test_mcp_state(scenario, apply, auto_payer)
    }

    fn call(state: &McpState, name: &str, arguments: Value) -> JsonRpcResponse {
        handle_rpc(
            state,
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/call".into(),
                params: json!({ "name": name, "arguments": arguments }),
            },
            &McpCallContext { authorized: true },
        )
    }

    fn body(resp: &JsonRpcResponse) -> Value {
        resp.result
            .as_ref()
            .and_then(|v| v.get("structuredContent"))
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[test]
    fn require_token_fails_closed() {
        let previous = std::env::var("MINT_LAB_MCP_TOKEN").ok();
        std::env::remove_var("MINT_LAB_MCP_TOKEN");
        let err = require_mcp_token().expect_err("token required");
        assert!(matches!(err, LabError::Unverified(_)));
        match previous {
            Some(v) => std::env::set_var("MINT_LAB_MCP_TOKEN", v),
            None => std::env::remove_var("MINT_LAB_MCP_TOKEN"),
        }
    }

    #[test]
    fn tools_list_is_five_allowed_tools() {
        let (_dir, state) = state_for("unclear_bv", false, false);
        let resp = handle_rpc(
            &state,
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/list".into(),
                params: json!({}),
            },
            &McpCallContext { authorized: true },
        );
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        assert_eq!(tools.len(), 5);
        assert!(
            call(&state, "set_stage", json!({})).result.unwrap()["isError"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn unauthorized_rpc_is_rejected() {
        let (_dir, state) = state_for("approval", false, false);
        let resp = handle_rpc(
            &state,
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "tools/list".into(),
                params: json!({}),
            },
            &McpCallContext { authorized: false },
        );
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32001);
    }

    #[test]
    fn ssn_like_payloads_are_rejected() {
        assert!(looks_like_real_identifier("123-45-6789"));
        assert!(!looks_like_real_identifier("MEM-APPROVAL-001"));
        let (_dir, state) = state_for("approval", false, false);
        let resp = call(
            &state,
            TOOL_ASK_PAYER,
            json!({
                "run_id": state.run_id,
                "task_id": state.task_id,
                "question": "SSN 123-45-6789",
                "evidence_hint": "payer_bv_response"
            }),
        );
        assert_eq!(resp.result.unwrap()["isError"], true);
    }

    #[test]
    fn auto_payer_unclear_reports_unknown() {
        let (_dir, state) = state_for("unclear_bv", true, true);
        let ctx = call(
            &state,
            TOOL_READ_ASSIGNED_CONTEXT,
            json!({ "run_id": state.run_id, "task_id": state.task_id }),
        );
        assert_eq!(body(&ctx)["task_id"], json!(state.task_id));
        let asked = call(
            &state,
            TOOL_ASK_PAYER,
            json!({
                "run_id": state.run_id,
                "task_id": state.task_id,
                "question": "Is PA required for CPT 72148?",
                "evidence_hint": "payer_bv_response"
            }),
        );
        let asked_body = body(&asked);
        assert_eq!(asked_body["status"], "answered");
        assert_eq!(asked_body["mode"], "auto_payer");
        let text = asked_body["text"].as_str().unwrap().to_owned();
        let snap = state.engine.snapshot(state.run_id).unwrap();
        let evidence: Vec<String> = snap
            .conversation
            .iter()
            .filter(|m| m.role == Role::Payer)
            .map(|m| format!("msg:{}", m.id))
            .collect();
        let observations = interpret_payer_text(&text, &evidence);
        let reported = call(
            &state,
            TOOL_REPORT_OBSERVATIONS,
            json!({
                "run_id": state.run_id,
                "task_id": state.task_id,
                "observations": observations,
                "needs_human_review": true
            }),
        );
        assert_eq!(body(&reported)["accepted"], true);
        let snap = state.engine.snapshot(state.run_id).unwrap();
        assert!(snap
            .tasks
            .iter()
            .any(|t| t.purpose == crate::lab::domain::TaskPurpose::ClarifyBv
                && t.status == crate::lab::domain::TaskStatus::Open));
        assert_ne!(snap.case.stage, CaseStage::Handoff);
    }

    #[test]
    fn default_ask_payer_is_pending() {
        let (_dir, state) = state_for("approval", true, false);
        let asked = call(
            &state,
            TOOL_ASK_PAYER,
            json!({
                "run_id": state.run_id,
                "task_id": state.task_id,
                "question": "Is PA required?",
                "evidence_hint": "payer_bv_response"
            }),
        );
        let asked_body = body(&asked);
        assert_eq!(asked_body["status"], "pending");
        assert_eq!(asked_body["mode"], "human_or_scripted");
    }

    #[test]
    fn resources_list_and_read_assigned_context() {
        let (_dir, state) = state_for("approval", false, false);
        let listed = handle_rpc(
            &state,
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: "resources/list".into(),
                params: json!({}),
            },
            &McpCallContext { authorized: true },
        );
        let resources = listed.result.unwrap()["resources"]
            .as_array()
            .unwrap()
            .clone();
        assert!(resources
            .iter()
            .any(|r| r["uri"].as_str().unwrap().ends_with("/context")));
        let uri = format!(
            "mint-lab://run/{}/bv-task/{}/context",
            state.run_id, state.task_id
        );
        let read = handle_rpc(
            &state,
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(2)),
                method: "resources/read".into(),
                params: json!({ "uri": uri }),
            },
            &McpCallContext { authorized: true },
        );
        let text = read.result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(text.contains("72148"));
    }

    #[test]
    fn rest_dispatch_matches_mcp_read_assigned_context() {
        let (_dir, state) = state_for("unclear_bv", false, false);
        let args = json!({ "run_id": state.run_id, "task_id": state.task_id });
        let mcp_body = body(&call(&state, TOOL_READ_ASSIGNED_CONTEXT, args.clone()));
        let rest_body = dispatch_bv_tool(&state, TOOL_READ_ASSIGNED_CONTEXT, &args).expect("rest");
        assert_eq!(mcp_body, rest_body);
        assert_eq!(rest_body["service"]["cpt"], "72148");
        assert_eq!(rest_body["run_id"], json!(state.run_id));
    }

    #[test]
    fn rest_dispatch_matches_mcp_ask_payer_auto() {
        let (_dir_mcp, mcp_state) = state_for("unclear_bv", true, true);
        let (_dir_rest, rest_state) = state_for("unclear_bv", true, true);
        let mcp_args = json!({
            "run_id": mcp_state.run_id,
            "task_id": mcp_state.task_id,
            "question": "Is prior authorization required for CPT 72148?",
            "evidence_hint": "payer_bv_response"
        });
        let rest_args = json!({
            "run_id": rest_state.run_id,
            "task_id": rest_state.task_id,
            "question": "Is prior authorization required for CPT 72148?",
            "evidence_hint": "payer_bv_response"
        });
        let mcp_body = body(&call(&mcp_state, TOOL_ASK_PAYER, mcp_args));
        let rest_body = dispatch_bv_tool(&rest_state, TOOL_ASK_PAYER, &rest_args).expect("rest");
        assert_eq!(mcp_body["status"], rest_body["status"]);
        assert_eq!(mcp_body["mode"], rest_body["mode"]);
        assert_eq!(mcp_body["configured_kind"], rest_body["configured_kind"]);
        assert_eq!(mcp_body["text"], rest_body["text"]);
        assert_eq!(rest_body["status"], "answered");
        assert_eq!(rest_body["mode"], "auto_payer");
    }

    #[test]
    fn inspect_and_trace_require_bound_run() {
        let (_dir, state) = state_for("approval", false, false);
        let other = Uuid::new_v4();
        let inspect_err = inspect_bound_run(&state, other).expect_err("cross-run");
        let trace_err = trace_bound_run(&state, other).expect_err("cross-run");
        assert!(matches!(inspect_err, LabError::Invalid(_)));
        assert!(matches!(trace_err, LabError::Invalid(_)));
        let report = inspect_bound_run(&state, state.run_id).expect("inspect");
        assert_eq!(report.run_id, state.run_id);
        let trace = trace_bound_run(&state, state.run_id).expect("trace");
        assert_eq!(trace.trace_version, crate::lab::tools::TRACE_VERSION);
    }
}
