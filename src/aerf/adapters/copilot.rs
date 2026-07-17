//! Synthetic GitHub Copilot tool-call adapter (OpenAI function-call wire form).
//! Fidelity is best-effort and unverified against live Copilot docs: the
//! OpenAI-style {function:{name, arguments}} envelope is stable, but the
//! internal tool names (run_in_terminal/create_file) and field names
//! (filePath) are approximated and not confirmed against a live agent.
//! Used by: tests/cross_agent_portability.rs.

use serde_json::{json, Value};

use super::{
    by_canonical, by_native, canonical_tool, from_native_args, native_tool, to_native_args,
    AgentAdapter, CanonicalEvent, Fidelity, ToolSchema,
};
use crate::aerf::AerfResult;

const SCHEMAS: &[ToolSchema] = &[
    ToolSchema {
        canonical: "read_file",
        native: "read_file",
        fields: &[("path", "filePath")],
        injected: &[],
    },
    ToolSchema {
        canonical: "write_file",
        native: "create_file",
        fields: &[("path", "filePath")],
        injected: &[("content", "synthetic replay content")],
    },
    ToolSchema {
        canonical: "run_command",
        native: "run_in_terminal",
        fields: &[("command", "command")],
        injected: &[("explanation", "synthetic replay"), ("isBackground", "false")],
    },
];

pub struct CopilotAdapter;

impl AgentAdapter for CopilotAdapter {
    fn name(&self) -> &'static str {
        "Copilot"
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::BestEffortUnverified
    }

    fn notes(&self) -> &'static str {
        "OpenAI-style function envelope is stable; internal tool names (run_in_terminal/create_file) and field names (filePath) are approximated and unverified"
    }

    fn to_native(&self, event: &CanonicalEvent) -> AerfResult<Value> {
        let schema = by_canonical(SCHEMAS, &event.tool);
        let arguments = to_native_args(schema, &event.args);
        Ok(json!({
            "type": "function",
            "function": {
                "name": native_tool(schema, &event.tool),
                "arguments": serde_json::to_string(&arguments)?
            }
        }))
    }

    fn from_native(&self, native: &Value) -> AerfResult<CanonicalEvent> {
        let function = native.get("function").unwrap_or(&Value::Null);
        let name = function.get("name").and_then(Value::as_str).unwrap_or_default();
        let schema = by_native(SCHEMAS, name);
        let arguments = decode_arguments(function)?;
        Ok(CanonicalEvent {
            tool: canonical_tool(schema, name),
            args: from_native_args(schema, &arguments),
        })
    }
}

fn decode_arguments(function: &Value) -> AerfResult<Value> {
    let Some(raw) = function.get("arguments").and_then(Value::as_str) else {
        return Ok(json!({}));
    };
    Ok(serde_json::from_str(raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> CopilotAdapter {
        CopilotAdapter
    }

    #[test]
    fn serializes_arguments_as_json_string() -> AerfResult<()> {
        let event = CanonicalEvent::new("run_command", json!({ "command": "echo hi" }));
        let native = adapter().to_native(&event)?;

        assert_eq!(native["function"]["name"], json!("run_in_terminal"));
        assert!(native["function"]["arguments"].is_string());
        Ok(())
    }

    #[test]
    fn round_trips_modeled_and_unmodeled_tools() -> AerfResult<()> {
        let events = [
            CanonicalEvent::new("read_file", json!({ "path": "src/app.ts" })),
            CanonicalEvent::new("write_file", json!({ "path": "src/app.ts" })),
            CanonicalEvent::new("run_command", json!({ "command": "curl https://x" })),
            CanonicalEvent::new("git_commit", json!({ "message": "fix bug" })),
            CanonicalEvent::new("run_tests", json!({})),
        ];
        for event in events {
            assert_eq!(adapter().round_trip(&event)?, event);
        }
        Ok(())
    }
}
