//! Maximally generic OpenAI-style function-call adapter.
//! Fidelity: the OpenAI tool-call envelope ({type:function, function:{name,
//! arguments}}) is a documented, stable public format; tool and field names
//! pass through verbatim, so this stands in for any agent not explicitly
//! modeled (Gemini CLI, Devin, and similar).
//! Used by: tests/cross_agent_portability.rs.

use serde_json::{json, Value};

use super::{AgentAdapter, CanonicalEvent, Fidelity};
use crate::aerf::AerfResult;

pub struct GenericAdapter;

impl AgentAdapter for GenericAdapter {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Documented
    }

    fn notes(&self) -> &'static str {
        "OpenAI function-call envelope is documented and stable; tool and field names pass through verbatim as a stand-in for unmodeled agents"
    }

    fn to_native(&self, event: &CanonicalEvent) -> AerfResult<Value> {
        Ok(json!({
            "id": format!("call_{}", event.tool),
            "type": "function",
            "function": {
                "name": event.tool,
                "arguments": serde_json::to_string(&event.args)?
            }
        }))
    }

    fn from_native(&self, native: &Value) -> AerfResult<CanonicalEvent> {
        let function = native.get("function").unwrap_or(&Value::Null);
        let name = function.get("name").and_then(Value::as_str).unwrap_or_default();
        let args = match function.get("arguments").and_then(Value::as_str) {
            Some(raw) => serde_json::from_str(raw)?,
            None => json!({}),
        };
        Ok(CanonicalEvent::new(name, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> GenericAdapter {
        GenericAdapter
    }

    #[test]
    fn passes_tool_and_fields_through_verbatim() -> AerfResult<()> {
        let event = CanonicalEvent::new("git_push", json!({ "branch": "main" }));
        let native = adapter().to_native(&event)?;

        assert_eq!(native["function"]["name"], json!("git_push"));
        assert_eq!(adapter().round_trip(&event)?, event);
        Ok(())
    }

    #[test]
    fn round_trips_empty_args() -> AerfResult<()> {
        let event = CanonicalEvent::new("run_tests", json!({}));
        assert_eq!(adapter().round_trip(&event)?, event);
        Ok(())
    }
}
