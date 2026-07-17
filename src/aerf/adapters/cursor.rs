//! Synthetic Cursor tool-call adapter (run_terminal_cmd-style schema).
//! Fidelity is best-effort and unverified against live Cursor docs: the
//! `run_terminal_cmd`/`command` and `read_file`/`target_file` shapes are
//! modeled from observed behavior, and unmodeled tools pass through untouched.
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
        fields: &[("path", "target_file")],
        injected: &[],
    },
    ToolSchema {
        canonical: "write_file",
        native: "edit_file",
        fields: &[("path", "target_file")],
        injected: &[("instructions", "synthetic replay edit")],
    },
    ToolSchema {
        canonical: "run_command",
        native: "run_terminal_cmd",
        fields: &[("command", "command")],
        injected: &[("is_background", "false")],
    },
];

pub struct CursorAdapter;

impl AgentAdapter for CursorAdapter {
    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::BestEffortUnverified
    }

    fn notes(&self) -> &'static str {
        "run_terminal_cmd/read_file/edit_file field names modeled from observed behavior; git and test tools are not explicitly modeled and pass through"
    }

    fn to_native(&self, event: &CanonicalEvent) -> AerfResult<Value> {
        let schema = by_canonical(SCHEMAS, &event.tool);
        Ok(json!({
            "type": "tool_call",
            "name": native_tool(schema, &event.tool),
            "arguments": to_native_args(schema, &event.args)
        }))
    }

    fn from_native(&self, native: &Value) -> AerfResult<CanonicalEvent> {
        let name = native.get("name").and_then(Value::as_str).unwrap_or_default();
        let schema = by_native(SCHEMAS, name);
        let arguments = native.get("arguments").cloned().unwrap_or_else(|| json!({}));
        Ok(CanonicalEvent {
            tool: canonical_tool(schema, name),
            args: from_native_args(schema, &arguments),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> CursorAdapter {
        CursorAdapter
    }

    #[test]
    fn renames_read_path_to_target_file() -> AerfResult<()> {
        let event = CanonicalEvent::new("read_file", json!({ "path": "src/app.ts" }));
        let native = adapter().to_native(&event)?;

        assert_eq!(native["name"], json!("read_file"));
        assert_eq!(native["arguments"]["target_file"], json!("src/app.ts"));
        Ok(())
    }

    #[test]
    fn round_trips_modeled_tools() -> AerfResult<()> {
        let events = [
            CanonicalEvent::new("read_file", json!({ "path": ".env" })),
            CanonicalEvent::new("write_file", json!({ "path": "src/app.ts" })),
            CanonicalEvent::new("run_command", json!({ "command": "rm -rf /" })),
        ];
        for event in events {
            assert_eq!(adapter().round_trip(&event)?, event);
        }
        Ok(())
    }

    #[test]
    fn round_trips_unmodeled_tools() -> AerfResult<()> {
        let event = CanonicalEvent::new("git_push", json!({ "branch": "main" }));
        assert_eq!(adapter().round_trip(&event)?, event);
        Ok(())
    }
}
