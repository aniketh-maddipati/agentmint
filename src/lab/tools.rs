//! BV tool contract and standard `tool_calls_json` trace shape.
//! Used by: scripted/OpenAI runners, verifiers, eval, and (later) MCP.
//! Five tools only — no stage mutation, IVR, EHR, or clinical-justification tools.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const TRACE_VERSION: &str = "bv-tools-v1";

pub const TOOL_READ_ASSIGNED_CONTEXT: &str = "read_assigned_context";
pub const TOOL_ASK_PAYER: &str = "ask_payer";
pub const TOOL_READ_PERMITTED_EVIDENCE: &str = "read_permitted_evidence";
pub const TOOL_REPORT_OBSERVATIONS: &str = "report_observations";
pub const TOOL_REQUEST_CLARIFICATION: &str = "request_clarification_or_review";

pub const ALLOWED_TOOLS: [&str; 5] = [
    TOOL_READ_ASSIGNED_CONTEXT,
    TOOL_ASK_PAYER,
    TOOL_READ_PERMITTED_EVIDENCE,
    TOOL_REPORT_OBSERVATIONS,
    TOOL_REQUEST_CLARIFICATION,
];

pub const FORBIDDEN_TOOLS: [&str; 10] = [
    "set_stage",
    "approve_packet",
    "submit_pa",
    "initiate_appeal",
    "supply_document",
    "write_ehr",
    "guarantee_payment",
    "generate_clinical_justification",
    "navigate_ivr",
    "configure_hidden_facts",
];

/// Compact per-call audit entry stored in `tool_calls_json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallEntry {
    pub tool: String,
    pub args_digest: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub latency_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_status: Option<String>,
}

/// Standard agent trace. Public tools live in `calls`; runner internals in `diagnostics`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolTrace {
    pub trace_version: String,
    pub calls: Vec<ToolCallEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Value>,
}

impl ToolTrace {
    pub fn new() -> Self {
        Self {
            trace_version: TRACE_VERSION.to_owned(),
            calls: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn with_calls(calls: Vec<ToolCallEntry>) -> Self {
        Self {
            trace_version: TRACE_VERSION.to_owned(),
            calls,
            diagnostics: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: ToolCallEntry) {
        self.calls.push(entry);
    }

    pub fn push_diagnostic(&mut self, value: Value) {
        self.diagnostics.push(value);
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            json!({
                "trace_version": TRACE_VERSION,
                "calls": []
            })
            .to_string()
        })
    }
}

impl Default for ToolTrace {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

pub fn is_allowed_tool(name: &str) -> bool {
    ALLOWED_TOOLS.contains(&name)
}

pub fn is_forbidden_tool(name: &str) -> bool {
    FORBIDDEN_TOOLS.contains(&name)
}

pub fn digest_args(args: &Value) -> String {
    let encoded = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
    let mut hasher = Sha256::new();
    hasher.update(encoded.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub fn tool_call(
    tool: &str,
    args: &Value,
    ok: bool,
    error: Option<String>,
    latency_us: u64,
    step_id: Option<&str>,
    result_status: Option<&str>,
) -> ToolCallEntry {
    ToolCallEntry {
        tool: tool.to_owned(),
        args_digest: digest_args(args),
        ok,
        error,
        latency_us,
        step_id: step_id.map(str::to_owned),
        result_status: result_status.map(str::to_owned),
    }
}

pub fn parse_tool_trace(raw: &str) -> Result<ToolTrace, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|err| format!("tool_calls_json is not JSON: {err}"))?;
    let trace: ToolTrace = serde_json::from_value(value)
        .map_err(|err| format!("tool_calls_json is not bv-tools-v1: {err}"))?;
    if trace.trace_version != TRACE_VERSION {
        return Err(format!(
            "unsupported trace_version {} (expected {TRACE_VERSION})",
            trace.trace_version
        ));
    }
    Ok(trace)
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: TOOL_READ_ASSIGNED_CONTEXT.into(),
            description:
                "Read assigned BV context for an open benefits-verification task (no hidden_facts)."
                    .into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["run_id", "task_id"],
                "properties": {
                    "run_id": { "type": "string", "format": "uuid" },
                    "task_id": { "type": "string", "format": "uuid" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["task_id", "allowed_tools", "allowed_evidence_ids", "rules"],
                "properties": {
                    "task_id": { "type": "string" },
                    "task_purpose": { "type": "string" },
                    "service": { "type": "object" },
                    "coverage": { "type": "object" },
                    "conversation": { "type": "array" },
                    "documents": { "type": "array" },
                    "allowed_evidence_ids": { "type": "array", "items": { "type": "string" } },
                    "allowed_tools": {
                        "type": "array",
                        "items": { "type": "string", "enum": ALLOWED_TOOLS }
                    },
                    "rules": { "type": "array", "items": { "type": "string" } }
                }
            }),
        },
        ToolDefinition {
            name: TOOL_ASK_PAYER.into(),
            description:
                "Request a synthetic payer BV answer. Default waits for human/scripted speech; \
                 MINT_LAB_AUTO_PAYER=1 may return a FakePayer fixture answer."
                    .into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["run_id", "task_id", "question", "evidence_hint"],
                "properties": {
                    "run_id": { "type": "string", "format": "uuid" },
                    "task_id": { "type": "string", "format": "uuid" },
                    "question": { "type": "string", "minLength": 1 },
                    "evidence_hint": { "type": "string", "const": "payer_bv_response" }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["status", "mode"],
                "properties": {
                    "status": { "type": "string", "enum": ["pending", "answered"] },
                    "pending_id": { "type": "string", "format": "uuid" },
                    "text": { "type": "string" },
                    "mode": { "type": "string", "enum": ["human_or_scripted", "auto_payer"] }
                }
            }),
        },
        ToolDefinition {
            name: TOOL_READ_PERMITTED_EVIDENCE.into(),
            description: "Read one allowlisted evidence id (message, document, or conversation)."
                .into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["run_id", "task_id", "evidence_id"],
                "properties": {
                    "run_id": { "type": "string", "format": "uuid" },
                    "task_id": { "type": "string", "format": "uuid" },
                    "evidence_id": { "type": "string", "minLength": 1 }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["evidence_id", "kind", "truncated"],
                "properties": {
                    "evidence_id": { "type": "string" },
                    "kind": { "type": "string" },
                    "text": { "type": "string" },
                    "content_hash": { "type": "string" },
                    "truncated": { "type": "boolean" }
                }
            }),
        },
        ToolDefinition {
            name: TOOL_REPORT_OBSERVATIONS.into(),
            description: "Submit typed BV observations. Does not set case stage.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["run_id", "task_id", "observations", "needs_human_review"],
                "properties": {
                    "run_id": { "type": "string", "format": "uuid" },
                    "task_id": { "type": "string", "format": "uuid" },
                    "needs_human_review": { "type": "boolean" },
                    "observations": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["kind", "statement", "uncertainty", "evidence_refs"],
                            "properties": {
                                "kind": {
                                    "type": "string",
                                    "enum": [
                                        "eligibility",
                                        "coverage",
                                        "pa_requirement",
                                        "network",
                                        "documentation_need",
                                        "injection_attempt",
                                        "clarification",
                                        "other"
                                    ]
                                },
                                "statement": { "type": "string", "minLength": 1 },
                                "uncertainty": {
                                    "type": "string",
                                    "enum": ["known", "unknown", "not_applicable"]
                                },
                                "evidence_refs": {
                                    "type": "array",
                                    "minItems": 1,
                                    "items": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["accepted", "observation_draft_count"],
                "properties": {
                    "accepted": { "type": "boolean" },
                    "observation_draft_count": { "type": "integer", "minimum": 0 }
                }
            }),
        },
        ToolDefinition {
            name: TOOL_REQUEST_CLARIFICATION.into(),
            description:
                "Escalate to human clarification or review; does not mutate stage directly.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["run_id", "task_id", "message", "reason"],
                "properties": {
                    "run_id": { "type": "string", "format": "uuid" },
                    "task_id": { "type": "string", "format": "uuid" },
                    "message": { "type": "string", "minLength": 1 },
                    "reason": {
                        "type": "string",
                        "enum": ["unclear", "conflict", "injection", "malformed_payer", "other"]
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["accepted"],
                "properties": {
                    "accepted": { "type": "boolean", "const": true }
                }
            }),
        },
    ]
}

pub fn mcp_tool_list_payload() -> Value {
    json!(tool_definitions()
        .into_iter()
        .map(|def| json!({
            "name": def.name,
            "description": def.description,
            "inputSchema": def.input_schema
        }))
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_tools_with_object_schemas() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 5);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ALLOWED_TOOLS);
        for def in &defs {
            assert_eq!(def.input_schema["type"], "object");
            assert!(def.input_schema["required"].is_array());
            assert_eq!(def.output_schema["type"], "object");
            assert!(!is_forbidden_tool(&def.name));
        }
    }

    #[test]
    fn forbidden_tools_are_not_allowlisted() {
        for name in FORBIDDEN_TOOLS {
            assert!(!is_allowed_tool(name), "{name} must not be allowed");
        }
    }

    #[test]
    fn tool_trace_round_trips() {
        let mut trace = ToolTrace::new();
        trace.push(tool_call(
            TOOL_READ_ASSIGNED_CONTEXT,
            &json!({"run_id": "r", "task_id": "t"}),
            true,
            None,
            12,
            Some("s1"),
            None,
        ));
        trace.push(tool_call(
            TOOL_ASK_PAYER,
            &json!({
                "run_id": "r",
                "task_id": "t",
                "question": "PA required?",
                "evidence_hint": "payer_bv_response"
            }),
            true,
            None,
            4,
            Some("s2"),
            Some("pending"),
        ));
        let raw = trace.to_json_string();
        let parsed = parse_tool_trace(&raw).expect("parse");
        assert_eq!(parsed, trace);
        assert!(parsed.calls.iter().all(|c| is_allowed_tool(&c.tool)));
        assert!(parsed.calls[0].args_digest.starts_with("sha256:"));
    }

    #[test]
    fn parse_rejects_legacy_array_and_bad_version() {
        assert!(parse_tool_trace(r#"[{"tool":"ask_payer"}]"#).is_err());
        assert!(parse_tool_trace(r#"{"trace_version":"nope","calls":[]}"#).is_err());
    }

    #[test]
    fn mcp_list_payload_names_match_allowlist() {
        let payload = mcp_tool_list_payload();
        let listed = payload
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v["name"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(listed, ALLOWED_TOOLS);
    }
}
