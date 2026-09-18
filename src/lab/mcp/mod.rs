//! Mint Lab MCP server for the five BV tools.
//! Used by: `mint lab mcp-stdio` / `mint lab mcp-http` and McpAgentRunner.
//! Loopback or stdio only; bearer `MINT_LAB_MCP_TOKEN`; no stage-mutation tools.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::agent::{
    allowed_evidence_ids, assigned_context_value, AgentOutput, DraftObservation, PendingQuestion,
};
use crate::lab::domain::{AgentRunRecord, CaseSnapshot, ObservationKind, Role, Uncertainty};
use crate::lab::error::{LabError, LabResult};
use crate::lab::payer::{BvInquiry, PayerAdapter};
use crate::lab::plan::persist_reasoning_columns;
use crate::lab::tools::{
    digest_args, is_allowed_tool, is_forbidden_tool, mcp_tool_list_payload, tool_call, ToolTrace,
    TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT, TOOL_READ_PERMITTED_EVIDENCE,
    TOOL_REPORT_OBSERVATIONS, TOOL_REQUEST_CLARIFICATION,
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
                if req.id.is_none() {
                    return success(
                        id,
                        tool_result(json!({"error": code, "message": message}), true),
                    );
                }
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

fn map_tool_error(err: &LabError) -> (&'static str, String, Option<Value>) {
    let message = err.to_string();
    let code = if message.contains("unauthorized") {
        "unauthorized"
    } else if message.contains("task_not_open") || message.contains("no open BV") {
        "task_not_open"
    } else if message.contains("unknown evidence") {
        "unknown_evidence"
    } else if message.contains("injection") {
        "injection_detected"
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
    if is_forbidden_tool(name) || !is_allowed_tool(name) {
        return Err(LabError::Invalid(format!(
            "tool {name} is not on the BV allowlist"
        )));
    }
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    if let Some(raw) = args.as_str() {
        if looks_like_real_identifier(raw) {
            return Err(LabError::Invalid(
                "payload resembles a real identifier; lab accepts synthetic fixtures only".into(),
            ));
        }
    }
    scan_for_real_identifiers(&args)?;
    let started = Instant::now();
    let body = match name {
        TOOL_READ_ASSIGNED_CONTEXT => tool_read_assigned_context(state, &args)?,
        TOOL_ASK_PAYER => tool_ask_payer(state, &args)?,
        TOOL_READ_PERMITTED_EVIDENCE => tool_read_permitted_evidence(state, &args)?,
        TOOL_REPORT_OBSERVATIONS => tool_report_observations(state, &args)?,
        TOOL_REQUEST_CLARIFICATION => tool_request_clarification(state, &args)?,
        other => return Err(LabError::Invalid(format!("unknown tool {other}"))),
    };
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
    let mut entry = tool_call(name, &args, ok, None, latency_us, None, status);
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
                "args_digest": digest_args(&args)
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
    drop(trace);
    Ok(tool_result(body, !ok))
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
        let body = assigned_context_value(task, &snap);
        return Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": body.to_string()
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

fn bound_ids(state: &McpState, args: &Value) -> LabResult<(Uuid, Uuid)> {
    let run_id = parse_uuid(args, "run_id")?;
    let task_id = parse_uuid(args, "task_id")?;
    if run_id != state.run_id || task_id != state.task_id {
        return Err(LabError::Invalid(
            "run_id/task_id do not match MCP session binding".into(),
        ));
    }
    Ok((run_id, task_id))
}

fn parse_uuid(args: &Value, field: &str) -> LabResult<Uuid> {
    let raw = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid(format!("missing {field}")))?;
    Uuid::parse_str(raw).map_err(|_| LabError::Invalid(format!("invalid {field}")))
}

fn tool_read_assigned_context(state: &McpState, args: &Value) -> LabResult<Value> {
    let (run_id, task_id) = bound_ids(state, args)?;
    let (_case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let mut ctx = assigned_context_value(&task, &snap);
    if let Value::Object(map) = &mut ctx {
        let mut ids: Vec<String> = allowed_evidence_ids(&snap).into_iter().collect();
        ids.sort();
        map.insert("allowed_evidence_ids".into(), json!(ids));
        map.insert("run_id".into(), json!(run_id));
    }
    Ok(ctx)
}

fn tool_ask_payer(state: &McpState, args: &Value) -> LabResult<Value> {
    let (run_id, task_id) = bound_ids(state, args)?;
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid("missing question".into()))?;
    if question.trim().is_empty() {
        return Err(LabError::Invalid("question empty".into()));
    }
    let evidence_hint = args
        .get("evidence_hint")
        .and_then(Value::as_str)
        .unwrap_or("payer_bv_response");
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
        return Ok(json!({
            "status": "answered",
            "mode": "auto_payer",
            "text": response.text,
            "msg_id": msg_id,
            "configured_kind": response.configured_kind
        }));
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
    Ok(json!({
        "status": "pending",
        "mode": "human_or_scripted",
        "pending_id": pending_id
    }))
}

fn tool_read_permitted_evidence(state: &McpState, args: &Value) -> LabResult<Value> {
    let (run_id, task_id) = bound_ids(state, args)?;
    let evidence_id = args
        .get("evidence_id")
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid("missing evidence_id".into()))?;
    let (_case, _task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let allowed = allowed_evidence_ids(&snap);
    if !allowed.contains(evidence_id) {
        return Err(LabError::Invalid(format!(
            "unknown evidence id {evidence_id}"
        )));
    }
    Ok(read_evidence_blob(&snap, evidence_id))
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
    json!({
        "evidence_id": evidence_id,
        "kind": kind,
        "text": text,
        "content_hash": content_hash,
        "truncated": truncated
    })
}

fn tool_report_observations(state: &McpState, args: &Value) -> LabResult<Value> {
    let (run_id, task_id) = bound_ids(state, args)?;
    let (mut case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let mut observations: Vec<DraftObservation> = serde_json::from_value(
        args.get("observations")
            .cloned()
            .ok_or_else(|| LabError::Invalid("missing observations".into()))?,
    )
    .map_err(|err| LabError::Invalid(format!("invalid_schema: {err}")))?;
    let mut needs_human_review = args
        .get("needs_human_review")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
    Ok(json!({
        "accepted": true,
        "observation_draft_count": observations.len()
    }))
}

fn tool_request_clarification(state: &McpState, args: &Value) -> LabResult<Value> {
    let (run_id, task_id) = bound_ids(state, args)?;
    let (mut case, task, snap) = state.engine.require_open_bv_task(run_id, task_id)?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| LabError::Invalid("missing message".into()))?;
    if message.trim().is_empty() {
        return Err(LabError::Invalid("clarification message empty".into()));
    }
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("other");
    let output = match reason {
        "injection" => AgentOutput::Observations {
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
    Ok(json!({ "accepted": true }))
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
    let evidence_refs = match output {
        AgentOutput::Observations { observations, .. } => observations
            .iter()
            .flat_map(|o| o.evidence_refs.clone())
            .collect(),
        AgentOutput::PendingQuestion(q) => vec![q.evidence_hint.clone()],
        AgentOutput::Clarification { .. } => Vec::new(),
    };
    let mut record = AgentRunRecord {
        id: Uuid::new_v4(),
        case_id: snap.case.id,
        task_id: task.id,
        prompt_version: "bv-mcp-v1".into(),
        model_id: "mcp".into(),
        context_version: snap.case.coverage_version + snap.case.service_version,
        tool_calls_json: trace.to_json_string(),
        structured_output_json: serde_json::to_string(output)?,
        evidence_refs,
        created_at: state.engine.now(),
        plan_json: None,
        reasoning_json: None,
    };
    persist_reasoning_columns(&mut record, &trace);
    state.engine.store.insert_agent_run(&record)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::agent::interpret_payer_text;
    use crate::lab::domain::CaseStage;
    use crate::lab::scenarios::fixtures_dir;
    use tempfile::tempdir;

    fn state_for(scenario: &str, apply: bool, auto_payer: bool) -> (tempfile::TempDir, McpState) {
        let dir = tempdir().expect("tempdir");
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
}
