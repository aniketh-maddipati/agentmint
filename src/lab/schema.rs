//! Canonical JSON Schema 2020-12 (and OpenAPI 3.1) for the PA/BV lab contract.
//! Used by: MCP `tools/list`, snapshot tests under `schemas/lab/`, REST `GET /lab/openapi.json`.
//! Rust types remain the source of truth. `hidden_facts` is never in these schemas.

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::agent::AgentOutput;
use crate::lab::eval::EvalReport;
use crate::lab::inspect::InspectReport;
use crate::lab::tools::{
    json_schema_for, AskPayerRequest, AskPayerResponse, AssignedContext, EvidenceBlob,
    ReadAssignedContextRequest, ReadPermittedEvidenceRequest, ReportObservationsRequest,
    ReportObservationsResponse, RequestClarificationRequest, RequestClarificationResponse,
    ToolTrace, TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT, TOOL_READ_PERMITTED_EVIDENCE,
    TOOL_REPORT_OBSERVATIONS, TOOL_REQUEST_CLARIFICATION,
};

pub const JSON_SCHEMA_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
pub const OPENAPI_VERSION: &str = "3.1.0";

/// Buyer explainer artifact. Runtime `explain_run` lands in a later PR; this type locks the schema.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainToolEvent {
    pub tool: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainUnknown {
    pub class: crate::lab::inspect::EpistemicClass,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainWorkflow {
    pub stage: crate::lab::domain::CaseStage,
    pub disposition: Option<String>,
    pub outstanding_work: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainClaim {
    pub class: crate::lab::inspect::EpistemicClass,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainReport {
    pub run_id: Uuid,
    pub called: Vec<ExplainToolEvent>,
    pub accepted: Vec<ExplainToolEvent>,
    pub rejected: Vec<ExplainToolEvent>,
    pub unknowns: Vec<ExplainUnknown>,
    pub workflow: ExplainWorkflow,
    pub claims: Vec<ExplainClaim>,
}

#[derive(Debug, Clone)]
pub struct SchemaFile {
    pub rel_path: &'static str,
    pub schema: Value,
}

pub fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/lab")
}

pub fn lab_schema_files() -> Vec<SchemaFile> {
    let mut files = vec![
        SchemaFile {
            rel_path: "tools/read_assigned_context.input.json",
            schema: json_schema_for::<ReadAssignedContextRequest>(),
        },
        SchemaFile {
            rel_path: "tools/read_assigned_context.output.json",
            schema: json_schema_for::<AssignedContext>(),
        },
        SchemaFile {
            rel_path: "tools/ask_payer.input.json",
            schema: json_schema_for::<AskPayerRequest>(),
        },
        SchemaFile {
            rel_path: "tools/ask_payer.output.json",
            schema: json_schema_for::<AskPayerResponse>(),
        },
        SchemaFile {
            rel_path: "tools/read_permitted_evidence.input.json",
            schema: json_schema_for::<ReadPermittedEvidenceRequest>(),
        },
        SchemaFile {
            rel_path: "tools/read_permitted_evidence.output.json",
            schema: json_schema_for::<EvidenceBlob>(),
        },
        SchemaFile {
            rel_path: "tools/report_observations.input.json",
            schema: json_schema_for::<ReportObservationsRequest>(),
        },
        SchemaFile {
            rel_path: "tools/report_observations.output.json",
            schema: json_schema_for::<ReportObservationsResponse>(),
        },
        SchemaFile {
            rel_path: "tools/request_clarification_or_review.input.json",
            schema: json_schema_for::<RequestClarificationRequest>(),
        },
        SchemaFile {
            rel_path: "tools/request_clarification_or_review.output.json",
            schema: json_schema_for::<RequestClarificationResponse>(),
        },
        SchemaFile {
            rel_path: "agent_output.json",
            schema: json_schema_for::<AgentOutput>(),
        },
        SchemaFile {
            rel_path: "inspect_report.json",
            schema: json_schema_for::<InspectReport>(),
        },
        SchemaFile {
            rel_path: "eval_report.json",
            schema: json_schema_for::<EvalReport>(),
        },
        SchemaFile {
            rel_path: "explain_report.json",
            schema: json_schema_for::<ExplainReport>(),
        },
        SchemaFile {
            rel_path: "tool_trace.json",
            schema: json_schema_for::<ToolTrace>(),
        },
    ];
    files.push(SchemaFile {
        rel_path: "openapi.json",
        schema: openapi_document(),
    });
    files
}

pub fn openapi_document() -> Value {
    let defs = json!({
        "ReadAssignedContextRequest": json_schema_for::<ReadAssignedContextRequest>(),
        "AssignedContext": json_schema_for::<AssignedContext>(),
        "AskPayerRequest": json_schema_for::<AskPayerRequest>(),
        "AskPayerResponse": json_schema_for::<AskPayerResponse>(),
        "ReadPermittedEvidenceRequest": json_schema_for::<ReadPermittedEvidenceRequest>(),
        "EvidenceBlob": json_schema_for::<EvidenceBlob>(),
        "ReportObservationsRequest": json_schema_for::<ReportObservationsRequest>(),
        "ReportObservationsResponse": json_schema_for::<ReportObservationsResponse>(),
        "RequestClarificationRequest": json_schema_for::<RequestClarificationRequest>(),
        "RequestClarificationResponse": json_schema_for::<RequestClarificationResponse>(),
        "AgentOutput": json_schema_for::<AgentOutput>(),
        "InspectReport": json_schema_for::<InspectReport>(),
        "EvalReport": json_schema_for::<EvalReport>(),
        "ExplainReport": json_schema_for::<ExplainReport>(),
        "ToolTrace": json_schema_for::<ToolTrace>(),
    });
    json!({
        "openapi": OPENAPI_VERSION,
        "info": {
            "title": "mint-lab-pa-bv",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Loopback PA/BV lab contract. MCP for agents, REST for buyers/UI. Served at GET /lab/openapi.json on mint lab mcp-http."
        },
        "jsonSchemaDialect": JSON_SCHEMA_2020_12,
        "servers": [{ "url": "http://127.0.0.1:8787" }],
        "paths": {
            "/lab/bv/read_assigned_context": bv_post(
                TOOL_READ_ASSIGNED_CONTEXT,
                "ReadAssignedContextRequest",
                "AssignedContext"
            ),
            "/lab/bv/ask_payer": bv_post(TOOL_ASK_PAYER, "AskPayerRequest", "AskPayerResponse"),
            "/lab/bv/read_permitted_evidence": bv_post(
                TOOL_READ_PERMITTED_EVIDENCE,
                "ReadPermittedEvidenceRequest",
                "EvidenceBlob"
            ),
            "/lab/bv/report_observations": bv_post(
                TOOL_REPORT_OBSERVATIONS,
                "ReportObservationsRequest",
                "ReportObservationsResponse"
            ),
            "/lab/bv/request_clarification_or_review": bv_post(
                TOOL_REQUEST_CLARIFICATION,
                "RequestClarificationRequest",
                "RequestClarificationResponse"
            ),
            "/lab/runs/{run_id}/inspect": {
                "get": {
                    "operationId": "inspect_run",
                    "summary": "Epistemic inspect report for a run",
                    "parameters": [run_id_param()],
                    "responses": {
                        "200": schema_response("InspectReport")
                    }
                }
            },
            "/lab/runs/{run_id}/trace": {
                "get": {
                    "operationId": "run_trace",
                    "summary": "bv-tools-v1 tool trace for a run",
                    "parameters": [run_id_param()],
                    "responses": {
                        "200": schema_response("ToolTrace")
                    }
                }
            },
            "/mcp": {
                "post": {
                    "operationId": "mcp_jsonrpc",
                    "summary": "JSON-RPC MCP (agents only; UI never uses this)",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    },
                    "responses": { "200": { "description": "JSON-RPC response" } }
                }
            },
            "/health": {
                "get": {
                    "operationId": "health",
                    "responses": { "200": { "description": "ok" } }
                }
            },
            "/lab/openapi.json": {
                "get": {
                    "operationId": "openapi",
                    "summary": "This OpenAPI 3.1 document",
                    "responses": { "200": { "description": "OpenAPI 3.1" } }
                }
            }
        },
        "components": {
            "schemas": defs,
            "securitySchemes": {
                "labBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "MINT_LAB_MCP_TOKEN"
                }
            }
        },
        "security": [{ "labBearer": [] }]
    })
}

fn bv_post(operation_id: &str, request: &str, response: &str) -> Value {
    json!({
        "post": {
            "operationId": operation_id,
            "summary": operation_id,
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": format!("#/components/schemas/{request}") }
                    }
                }
            },
            "responses": {
                "200": schema_response(response)
            }
        }
    })
}

fn schema_response(name: &str) -> Value {
    json!({
        "description": name,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{name}") }
            }
        }
    })
}

fn run_id_param() -> Value {
    json!({
        "name": "run_id",
        "in": "path",
        "required": true,
        "schema": { "type": "string", "format": "uuid" }
    })
}

pub fn pretty_schema(value: &Value) -> String {
    let mut out = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn schema_text_leaks_hidden_facts(text: &str) -> bool {
    text.contains("hidden_facts")
}

pub fn write_lab_schemas(dir: &Path) -> std::io::Result<()> {
    for file in lab_schema_files() {
        let path = dir.join(file.rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, pretty_schema(&file.schema))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::tools::{tool_definitions, ALLOWED_TOOLS, FORBIDDEN_TOOLS};

    #[test]
    fn generated_schemas_are_draft_2020_12() {
        for file in lab_schema_files() {
            if file.rel_path == "openapi.json" {
                assert_eq!(file.schema["openapi"], OPENAPI_VERSION);
                assert_eq!(file.schema["jsonSchemaDialect"], JSON_SCHEMA_2020_12);
                continue;
            }
            assert_eq!(
                file.schema["$schema"], JSON_SCHEMA_2020_12,
                "{}",
                file.rel_path
            );
        }
    }

    #[test]
    fn schemas_never_contain_hidden_facts() {
        for file in lab_schema_files() {
            let text = pretty_schema(&file.schema);
            assert!(
                !schema_text_leaks_hidden_facts(&text),
                "{} leaked hidden_facts",
                file.rel_path
            );
        }
        let openapi = pretty_schema(&openapi_document());
        for tool in ALLOWED_TOOLS {
            assert!(openapi.contains(tool), "openapi missing {tool}");
        }
        for forbidden in FORBIDDEN_TOOLS {
            assert!(
                !openapi.contains(&format!("/lab/bv/{forbidden}")),
                "openapi must not expose {forbidden}"
            );
        }
    }

    #[test]
    fn mcp_tool_list_uses_generated_input_schemas() {
        let defs = tool_definitions();
        let files = lab_schema_files();
        for def in defs {
            let want = files
                .iter()
                .find(|f| f.rel_path == format!("tools/{}.input.json", def.name))
                .unwrap_or_else(|| panic!("missing snapshot mapping for {}", def.name));
            assert_eq!(def.input_schema, want.schema, "{}", def.name);
        }
    }

    #[test]
    fn committed_schemas_match_generated() {
        let dir = schemas_dir();
        let mut missing = Vec::new();
        let mut drifted = Vec::new();
        for file in lab_schema_files() {
            let path = dir.join(file.rel_path);
            let expected = pretty_schema(&file.schema);
            match fs::read_to_string(&path) {
                Ok(actual) => {
                    if actual != expected {
                        drifted.push(file.rel_path);
                    }
                }
                Err(_) => missing.push(file.rel_path),
            }
        }
        assert!(
            missing.is_empty() && drifted.is_empty(),
            "lab schema snapshots out of date (missing={missing:?} drifted={drifted:?}). \
             Re-run with UPDATE_LAB_SCHEMAS=1 or `cargo test -q lab::schema::write_committed_schemas -- --ignored`"
        );
    }

    #[test]
    #[ignore = "writes schemas/lab; run explicitly to refresh snapshots"]
    fn write_committed_schemas() {
        write_lab_schemas(&schemas_dir()).expect("write schemas");
    }
}
