//! MCP client BV runner (`MINT_LAB_AGENT=mcp`).
//! Used by: LabEngine agent roster and eval. Fail-closed without token/server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::agent::{
    block_on_local, build_result, decide_bv, push_tool, AgentOutput, AgentRunResult, AgentRunner,
    BvDecision,
};
use crate::lab::domain::{CaseSnapshot, ConversationMessage, Role, Task};
use crate::lab::error::{LabError, LabResult};
use crate::lab::mcp::{handle_rpc, require_mcp_token, JsonRpcRequest, McpCallContext, McpState};
use crate::lab::tools::{
    ToolTrace, TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT, TOOL_READ_PERMITTED_EVIDENCE,
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
                structured_from_result(result)
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
                block_on_local(async {
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
                    structured_from_result(result)
                })
            }
        }
    }

    fn apply_decision(
        &self,
        task: &Task,
        snapshot: &CaseSnapshot,
        decision: &BvDecision,
        trace: &mut ToolTrace,
        started: Instant,
    ) -> LabResult<()> {
        match decision {
            BvDecision::AskPayer { .. } => Ok(()),
            BvDecision::Injection { .. } | BvDecision::Observations { .. } => {
                if let BvDecision::Observations { evidence_ids, .. } = decision {
                    for evidence_id in evidence_ids {
                        let args = json!({
                            "run_id": snapshot.case.run_id,
                            "task_id": task.id,
                            "evidence_id": evidence_id
                        });
                        let _ = self.call_tool(TOOL_READ_PERMITTED_EVIDENCE, args.clone())?;
                        push_tool(
                            trace,
                            TOOL_READ_PERMITTED_EVIDENCE,
                            &args,
                            started,
                            "s2",
                            None,
                        );
                    }
                }
                let output = decision.output();
                let AgentOutput::Observations {
                    observations,
                    needs_human_review,
                } = output
                else {
                    return Err(LabError::Invalid(
                        "expected observations output for report".into(),
                    ));
                };
                let args = json!({
                    "run_id": snapshot.case.run_id,
                    "task_id": task.id,
                    "observations": observations,
                    "needs_human_review": needs_human_review
                });
                let _ = self.call_tool(TOOL_REPORT_OBSERVATIONS, args.clone())?;
                push_tool(
                    trace,
                    TOOL_REPORT_OBSERVATIONS,
                    &args,
                    started,
                    "s3",
                    Some("accepted"),
                );
                Ok(())
            }
            BvDecision::Clarification { message, reason } => {
                let args = json!({
                    "run_id": snapshot.case.run_id,
                    "task_id": task.id,
                    "message": message,
                    "reason": reason
                });
                let _ = self.call_tool(TOOL_REQUEST_CLARIFICATION, args.clone())?;
                push_tool(
                    trace,
                    TOOL_REQUEST_CLARIFICATION,
                    &args,
                    started,
                    "s3",
                    Some("accepted"),
                );
                Ok(())
            }
        }
    }
}

fn structured_from_result(result: Value) -> LabResult<Value> {
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

impl AgentRunner for McpAgentRunner {
    fn run_bv(&self, task: &Task, snapshot: &CaseSnapshot) -> LabResult<AgentRunResult> {
        let ids = json!({
            "run_id": snapshot.case.run_id,
            "task_id": task.id
        });
        let mut trace = ToolTrace::new();
        let started = Instant::now();
        let _ctx = self.call_tool(TOOL_READ_ASSIGNED_CONTEXT, ids.clone())?;
        push_tool(
            &mut trace,
            TOOL_READ_ASSIGNED_CONTEXT,
            &ids,
            started,
            "s1",
            None,
        );

        let mut snapshot = snapshot.clone();
        let mut decision = decide_bv(task, &snapshot);
        if let BvDecision::AskPayer { question } = &decision {
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
            push_tool(
                &mut trace,
                TOOL_ASK_PAYER,
                &args,
                started,
                "s2",
                Some(status),
            );
            if status != "answered" {
                return Ok(build_result(
                    task,
                    &snapshot,
                    decision.output(),
                    trace,
                    PROMPT_VERSION_MCP,
                    "mcp",
                ));
            }
            if let Some(text) = resp.get("text").and_then(Value::as_str) {
                snapshot.conversation.push(ConversationMessage {
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
            decision = match decide_bv(task, &snapshot) {
                BvDecision::AskPayer { .. } => BvDecision::Clarification {
                    message: "Payer response malformed or unsupported; need clarification.".into(),
                    reason: "malformed_payer",
                },
                other => other,
            };
        }

        self.apply_decision(task, &snapshot, &decision, &mut trace, started)?;
        Ok(build_result(
            task,
            &snapshot,
            decision.output(),
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
}
