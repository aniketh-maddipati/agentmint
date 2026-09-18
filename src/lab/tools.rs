//! BV tool contract and standard `tool_calls_json` trace shape.
//! Used by: scripted/OpenAI/Anthropic/MCP runners, verifiers, and eval.
//! Five tools only — no stage mutation, IVR, EHR, or clinical-justification tools.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::lab::domain::{DraftObservation, Role, TaskPurpose};
use crate::lab::error::{LabError, LabResult};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceHint {
    PayerBvResponse,
}

impl EvidenceHint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PayerBvResponse => "payer_bv_response",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClarificationReason {
    Unclear,
    Conflict,
    Injection,
    MalformedPayer,
    Other,
}

impl ClarificationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unclear => "unclear",
            Self::Conflict => "conflict",
            Self::Injection => "injection",
            Self::MalformedPayer => "malformed_payer",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AskPayerStatus {
    Pending,
    Answered,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AskPayerMode {
    HumanOrScripted,
    AutoPayer,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAssignedContextRequest {
    pub run_id: Uuid,
    pub task_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedService {
    pub cpt: String,
    pub diagnosis: String,
    pub site: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedCoverage {
    pub payer_name: String,
    pub member_id: String,
    pub plan_id: String,
    pub dos: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedMessage {
    pub id: String,
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedDocument {
    pub id: String,
    pub name: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AssignedContext {
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub task_purpose: TaskPurpose,
    #[schemars(schema_with = "any_json_schema")]
    pub task_context: Value,
    pub service: AssignedService,
    pub coverage: AssignedCoverage,
    pub conversation: Vec<AssignedMessage>,
    pub documents: Vec<AssignedDocument>,
    pub allowed_evidence_ids: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskPayerRequest {
    pub run_id: Uuid,
    pub task_id: Uuid,
    #[schemars(length(min = 1))]
    pub question: String,
    pub evidence_hint: EvidenceHint,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskPayerResponse {
    pub status: AskPayerStatus,
    pub mode: AskPayerMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadPermittedEvidenceRequest {
    pub run_id: Uuid,
    pub task_id: Uuid,
    #[schemars(length(min = 1))]
    pub evidence_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceBlob {
    pub evidence_id: String,
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportObservationsRequest {
    pub run_id: Uuid,
    pub task_id: Uuid,
    #[schemars(length(min = 1))]
    pub observations: Vec<DraftObservation>,
    pub needs_human_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportObservationsResponse {
    pub accepted: bool,
    pub observation_draft_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestClarificationRequest {
    pub run_id: Uuid,
    pub task_id: Uuid,
    #[schemars(length(min = 1))]
    pub message: String,
    pub reason: ClarificationReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequestClarificationResponse {
    pub accepted: bool,
}

fn any_json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "description": "Arbitrary JSON value"
    })
}

pub fn json_schema_for<T: JsonSchema>() -> Value {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<T>();
    Value::from(schema)
}

pub fn parse_tool_args<T: serde::de::DeserializeOwned>(args: &Value) -> LabResult<T> {
    serde_json::from_value(args.clone())
        .map_err(|err| LabError::Invalid(format!("invalid_schema: {err}")))
}

/// Compact per-call audit entry stored in `tool_calls_json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ToolTrace {
    pub trace_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<crate::lab::plan::BvPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<crate::lab::plan::RepairMeta>,
    pub calls: Vec<ToolCallEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(schema_with = "any_json_array_schema")]
    pub diagnostics: Vec<Value>,
}

fn any_json_array_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "items": { "description": "Arbitrary JSON value" }
    })
}

impl ToolTrace {
    pub fn new() -> Self {
        Self {
            trace_version: TRACE_VERSION.to_owned(),
            plan: Some(crate::lab::plan::default_bv_plan()),
            repair: Some(crate::lab::plan::RepairMeta::none()),
            calls: Vec::new(),
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
            input_schema: json_schema_for::<ReadAssignedContextRequest>(),
            output_schema: json_schema_for::<AssignedContext>(),
        },
        ToolDefinition {
            name: TOOL_ASK_PAYER.into(),
            description:
                "Request a synthetic payer BV answer. Default waits for human/scripted speech; \
                 MINT_LAB_AUTO_PAYER=1 may return a FakePayer fixture answer."
                    .into(),
            input_schema: json_schema_for::<AskPayerRequest>(),
            output_schema: json_schema_for::<AskPayerResponse>(),
        },
        ToolDefinition {
            name: TOOL_READ_PERMITTED_EVIDENCE.into(),
            description: "Read one allowlisted evidence id (message, document, or conversation)."
                .into(),
            input_schema: json_schema_for::<ReadPermittedEvidenceRequest>(),
            output_schema: json_schema_for::<EvidenceBlob>(),
        },
        ToolDefinition {
            name: TOOL_REPORT_OBSERVATIONS.into(),
            description: "Submit typed BV observations. Does not set case stage.".into(),
            input_schema: json_schema_for::<ReportObservationsRequest>(),
            output_schema: json_schema_for::<ReportObservationsResponse>(),
        },
        ToolDefinition {
            name: TOOL_REQUEST_CLARIFICATION.into(),
            description:
                "Escalate to human clarification or review; does not mutate stage directly.".into(),
            input_schema: json_schema_for::<RequestClarificationRequest>(),
            output_schema: json_schema_for::<RequestClarificationResponse>(),
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
            assert_eq!(
                def.input_schema["$schema"],
                "https://json-schema.org/draft/2020-12/schema"
            );
            assert_eq!(def.input_schema["type"], "object");
            assert!(def.input_schema["required"].is_array());
            assert_eq!(def.output_schema["type"], "object");
            assert!(!is_forbidden_tool(&def.name));
            let dumped = def.input_schema.to_string() + &def.output_schema.to_string();
            assert!(
                !dumped.contains("hidden_facts"),
                "{} schema must not mention hidden_facts",
                def.name
            );
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

    #[test]
    fn parse_tool_args_rejects_unknown_fields() {
        let err = parse_tool_args::<ReadAssignedContextRequest>(&json!({
            "run_id": "00000000-0000-0000-0000-000000000001",
            "task_id": "00000000-0000-0000-0000-000000000002",
            "hidden_facts": { "bv_outcome": "pa_required" }
        }))
        .expect_err("unknown fields");
        assert!(err.to_string().contains("invalid_schema"), "{err}");
    }
}
