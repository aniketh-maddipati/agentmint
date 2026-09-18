//! MCP client BV runner (`MINT_LAB_AGENT=mcp`).
//! Used by: LabEngine agent roster and eval. Fail-closed without token/server.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::agent::{
    build_result, detect_injection, interpret_payer_text, AgentOutput, AgentRunResult, AgentRunner,
    DraftObservation, PendingQuestion,
};
use crate::lab::domain::{CaseSnapshot, ObservationKind, Role, Task, Uncertainty};
use crate::lab::error::{LabError, LabResult};
use crate::lab::mcp::{handle_rpc, require_mcp_token, JsonRpcRequest, McpCallContext, McpState};
use crate::lab::tools::{
    tool_call, ToolTrace, TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT, TOOL_READ_PERMITTED_EVIDENCE,
    TOOL_REPORT_OBSERVATIONS, TOOL_REQUEST_CLARIFICATION,
};

pub const PROMPT_VERSION_MCP: &str = "bv-mcp-v1";

pub enum McpTransport {
    Http {
        base_url: String,
        token: String,
        http: reqwest::Client,
    },
    InProcess(Arc<McpState>),
}

pub struct McpAgentRunner {
    transport: McpTransport,
}

impl McpAgentRunner {
    pub fn from_env() -> LabResult<Self> {
        let token = require_mcp_token()?;
        let base_url = std::env::var("MINT_LAB_MCP_URL").map_err(|_| {
            LabError::Unverified(
                "MINT_LAB_MCP_URL not set; McpAgentRunner will not fabricate success".into(),
            )
        })?;
        if base_url.trim().is_empty() {
            return Err(LabError::Unverified(
                "MINT_LAB_MCP_URL empty; McpAgentRunner will not fabricate success".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|err| LabError::Io(format!("http client: {err}")))?;
        Ok(Self {
            transport: McpTransport::Http {
                base_url: base_url.trim_end_matches('/').to_owned(),
                token,
                http,
            },
        })
    }

    pub fn in_process(state: Arc<McpState>) -> Self {
        Self {
            transport: McpTransport::InProcess(state),
        }
    }

    fn call_tool(&self, name: &str, arguments: Value) -> LabResult<Value> {
        match &self.transport {
            McpTransport::InProcess(state) => {
                let resp = handle_rpc(
                    state,
                    JsonRpcRequest {
                        jsonrpc: "2.0".into(),
                        id: Some(json!(1)),
                        method: "tools/call".into(),
                        params: json!({ "name": name, "arguments": arguments }),
                    },
                    &McpCallContext { authorized: true },
                );
                if let Some(err) = resp.error {
                    return Err(LabError::Unverified(format!(
                        "mcp {}: {}",
                        err.code, err.message
                    )));
                }
                let result = resp
                    .result
                    .ok_or_else(|| LabError::Unverified("mcp tools/call missing result".into()))?;
                if result.get("isError").and_then(Value::as_bool) == Some(true) {
                    let message = result
                        .pointer("/structuredContent/message")
                        .and_then(Value::as_str)
                        .unwrap_or("mcp tool error");
                    return Err(LabError::Unverified(message.to_owned()));
                }
                Ok(result
                    .get("structuredContent")
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            McpTransport::Http {
                base_url,
                token,
                http,
            } => {
                let url = if base_url.ends_with("/mcp") {
                    base_url.clone()
                } else {
                    format!("{base_url}/mcp")
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|err| LabError::Io(format!("runtime: {err}")))?;
                runtime.block_on(async {
                    let response = http
                        .post(&url)
                        .bearer_auth(token)
                        .json(&json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "tools/call",
                            "params": { "name": name, "arguments": arguments }
                        }))
                        .send()
                        .await
                        .map_err(|err| {
                            LabError::Unverified(format!(
                                "mcp server unreachable: {err}; will not fabricate success"
                            ))
                        })?;
                    let status = response.status();
                    let payload: Value = response
                        .json()
                        .await
                        .map_err(|err| LabError::Unverified(format!("mcp response json: {err}")))?;
                    if !status.is_success() {
                        return Err(LabError::Unverified(format!(
                            "mcp HTTP {status}: {payload}"
                        )));
                    }
                    if payload.get("error").is_some() {
                        return Err(LabError::Unverified(format!(
                            "mcp json-rpc error: {payload}"
                        )));
                    }
                    let result = payload.get("result").cloned().ok_or_else(|| {
                        LabError::Unverified("mcp tools/call missing result".into())
                    })?;
                    if result.get("isError").and_then(Value::as_bool) == Some(true) {
                        return Err(LabError::Unverified(format!("mcp tool error: {result}")));
                    }
                    Ok(result
                        .get("structuredContent")
                        .cloned()
                        .unwrap_or(Value::Null))
                })
            }
        }
    }
}

impl AgentRunner for McpAgentRunner {
    fn run_bv(&self, task: &Task, snapshot: &CaseSnapshot) -> LabResult<AgentRunResult> {
        let ids = json!({
            "run_id": snapshot.case.run_id,
            "task_id": task.id
        });
        let mut trace = ToolTrace::new();
        let started = std::time::Instant::now();
        let _ctx = self.call_tool(TOOL_READ_ASSIGNED_CONTEXT, ids.clone())?;
        trace.push(tool_call(
            TOOL_READ_ASSIGNED_CONTEXT,
            &ids,
            true,
            None,
            started.elapsed().as_micros() as u64,
            Some("s1"),
            None,
        ));

        let source_blobs: Vec<String> = snapshot
            .conversation
            .iter()
            .map(|m| m.text.clone())
            .chain(std::iter::once(task.context_json.clone()))
            .collect();
        if let Some(injection) = detect_injection(&source_blobs) {
            let obs = DraftObservation {
                kind: ObservationKind::InjectionAttempt,
                statement: format!(
                    "Possible prompt-injection content detected in source material: {injection}"
                ),
                uncertainty: Uncertainty::Known,
                evidence_refs: vec!["conversation".into()],
            };
            let output = AgentOutput::Observations {
                observations: vec![obs.clone()],
                needs_human_review: true,
            };
            let args = json!({
                "run_id": snapshot.case.run_id,
                "task_id": task.id,
                "observations": [obs],
                "needs_human_review": true
            });
            let _ = self.call_tool(TOOL_REPORT_OBSERVATIONS, args.clone())?;
            trace.push(tool_call(
                TOOL_REPORT_OBSERVATIONS,
                &args,
                true,
                None,
                started.elapsed().as_micros() as u64,
                Some("s3"),
                Some("accepted"),
            ));
            return Ok(build_result(
                task,
                snapshot,
                output,
                trace,
                PROMPT_VERSION_MCP,
                "mcp",
            ));
        }

        let mut payer_msgs: Vec<_> = snapshot
            .conversation
            .iter()
            .filter(|m| m.role == Role::Payer)
            .cloned()
            .collect();

        if payer_msgs.is_empty() {
            let question = format!(
                "Is prior authorization required for CPT {} on plan {} for DOS {}?",
                snapshot.case.service.cpt,
                snapshot.case.coverage.plan_id,
                snapshot.case.coverage.dos
            );
            let args = json!({
                "run_id": snapshot.case.run_id,
                "task_id": task.id,
                "question": question,
                "evidence_hint": "payer_bv_response"
            });
            let resp = self.call_tool(TOOL_ASK_PAYER, args.clone())?;
            let status = resp
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            trace.push(tool_call(
                TOOL_ASK_PAYER,
                &args,
                true,
                None,
                started.elapsed().as_micros() as u64,
                Some("s2"),
                Some(status),
            ));
            if status != "answered" {
                return Ok(build_result(
                    task,
                    snapshot,
                    AgentOutput::PendingQuestion(PendingQuestion {
                        question,
                        evidence_hint: "payer_bv_response".into(),
                    }),
                    trace,
                    PROMPT_VERSION_MCP,
                    "mcp",
                ));
            }
            if let Some(text) = resp.get("text").and_then(Value::as_str) {
                payer_msgs.push(crate::lab::domain::ConversationMessage {
                    id: resp
                        .get("msg_id")
                        .and_then(Value::as_str)
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .unwrap_or_else(Uuid::new_v4),
                    case_id: snapshot.case.id,
                    role: Role::Payer,
                    text: text.to_owned(),
                    created_at: snapshot.case.updated_at,
                });
            }
        }

        for msg in &payer_msgs {
            let evidence_id = format!("msg:{}", msg.id);
            let args = json!({
                "run_id": snapshot.case.run_id,
                "task_id": task.id,
                "evidence_id": evidence_id
            });
            let _ = self.call_tool(TOOL_READ_PERMITTED_EVIDENCE, args.clone())?;
            trace.push(tool_call(
                TOOL_READ_PERMITTED_EVIDENCE,
                &args,
                true,
                None,
                started.elapsed().as_micros() as u64,
                Some("s2"),
                None,
            ));
        }

        let joined = payer_msgs
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if joined.trim().is_empty() || joined.to_lowercase().contains("garbled") {
            let message =
                "Payer response malformed or unsupported; need clarification.".to_string();
            let args = json!({
                "run_id": snapshot.case.run_id,
                "task_id": task.id,
                "message": message,
                "reason": "malformed_payer"
            });
            let _ = self.call_tool(TOOL_REQUEST_CLARIFICATION, args.clone())?;
            trace.push(tool_call(
                TOOL_REQUEST_CLARIFICATION,
                &args,
                true,
                None,
                started.elapsed().as_micros() as u64,
                Some("s3"),
                Some("accepted"),
            ));
            return Ok(build_result(
                task,
                snapshot,
                AgentOutput::Clarification { message },
                trace,
                PROMPT_VERSION_MCP,
                "mcp",
            ));
        }

        let evidence_refs: Vec<String> =
            payer_msgs.iter().map(|m| format!("msg:{}", m.id)).collect();
        let observations = interpret_payer_text(&joined, &evidence_refs);
        let needs_human_review = observations.iter().any(|o| {
            matches!(o.kind, ObservationKind::InjectionAttempt)
                || (o.uncertainty == Uncertainty::Unknown
                    && joined.to_lowercase().contains("may require"))
        });
        let output = AgentOutput::Observations {
            observations: observations.clone(),
            needs_human_review,
        };
        let args = json!({
            "run_id": snapshot.case.run_id,
            "task_id": task.id,
            "observations": observations,
            "needs_human_review": needs_human_review
        });
        let _ = self.call_tool(TOOL_REPORT_OBSERVATIONS, args.clone())?;
        trace.push(tool_call(
            TOOL_REPORT_OBSERVATIONS,
            &args,
            true,
            None,
            started.elapsed().as_micros() as u64,
            Some("s3"),
            Some("accepted"),
        ));
        Ok(build_result(
            task,
            snapshot,
            output,
            trace,
            PROMPT_VERSION_MCP,
            "mcp",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_fails_closed_without_token_or_url() {
        let token = std::env::var("MINT_LAB_MCP_TOKEN").ok();
        let url = std::env::var("MINT_LAB_MCP_URL").ok();
        std::env::remove_var("MINT_LAB_MCP_TOKEN");
        std::env::remove_var("MINT_LAB_MCP_URL");
        assert!(matches!(
            McpAgentRunner::from_env(),
            Err(LabError::Unverified(_))
        ));
        std::env::set_var("MINT_LAB_MCP_TOKEN", "tok");
        assert!(matches!(
            McpAgentRunner::from_env(),
            Err(LabError::Unverified(_))
        ));
        match token {
            Some(v) => std::env::set_var("MINT_LAB_MCP_TOKEN", v),
            None => std::env::remove_var("MINT_LAB_MCP_TOKEN"),
        }
        match url {
            Some(v) => std::env::set_var("MINT_LAB_MCP_URL", v),
            None => std::env::remove_var("MINT_LAB_MCP_URL"),
        }
    }

    #[test]
    fn in_process_mcp_runner_matches_scripted_on_approval() {
        use crate::lab::agent::ScriptedAgentRunner;
        use crate::lab::domain::TaskPurpose;
        use crate::lab::scenarios::fixtures_dir;
        use crate::lab::workflow::LabEngine;
        use std::sync::Mutex;
        use tempfile::tempdir;

        let dir = tempdir().expect("tempdir");
        let engine = LabEngine::open(dir.path(), &fixtures_dir()).expect("engine");
        let run_id = engine.start_run("approval").expect("start");
        let task_id = engine.prepare_bv_task(run_id).expect("bv");
        engine
            .record_payer_speech(
                run_id,
                "Member is active. Prior authorization is required for CPT 72148 outpatient MRI lumbar spine. Required documentation: clinical notes and signed order.",
            )
            .expect("payer");
        let snap = engine.snapshot(run_id).expect("snap");
        let task = snap
            .tasks
            .iter()
            .find(|t| t.id == task_id && t.purpose == TaskPurpose::BenefitsVerification)
            .cloned()
            .expect("task");
        let state = Arc::new(crate::lab::mcp::McpState {
            engine: Arc::new(engine),
            run_id,
            task_id,
            token: "tok".into(),
            apply: false,
            auto_payer: false,
            trace: Mutex::new(crate::lab::tools::ToolTrace::new()),
        });
        let mcp = McpAgentRunner::in_process(state);
        let mcp_result = mcp.run_bv(&task, &snap).expect("mcp");
        let scripted = ScriptedAgentRunner.run_bv(&task, &snap).expect("scripted");
        match (mcp_result.output, scripted.output) {
            (
                AgentOutput::Observations {
                    observations: a, ..
                },
                AgentOutput::Observations {
                    observations: b, ..
                },
            ) => {
                let kinds_a: Vec<_> = a.iter().map(|o| o.kind).collect();
                let kinds_b: Vec<_> = b.iter().map(|o| o.kind).collect();
                assert_eq!(kinds_a, kinds_b);
            }
            other => panic!("expected observations, got {other:?}"),
        }
        assert_eq!(mcp_result.record.model_id, "mcp");
    }
}
