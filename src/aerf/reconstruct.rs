//! Receipt-only reconstruction of human-readable action histories.
//! Used by: Tier 2 evidentiary fidelity tests and future offline review flows.

use serde_json::Value;

use super::receipt::Receipt;
use super::AerfResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedAction {
    pub actor: String,
    pub tool: String,
    pub input_summary: String,
    pub decision: String,
    pub receipt_hash: String,
}

pub fn reconstruct_chain(receipts: &[Receipt]) -> AerfResult<Vec<ReconstructedAction>> {
    receipts
        .iter()
        .map(|receipt| {
            Ok(ReconstructedAction {
                actor: receipt.agent.clone(),
                tool: receipt.action.clone(),
                input_summary: summarize_receipt_input(receipt),
                decision: if receipt.in_policy {
                    "pass".to_owned()
                } else {
                    "block".to_owned()
                },
                receipt_hash: receipt.chain_hash()?,
            })
        })
        .collect()
}

pub fn render_reconstruction(receipts: &[Receipt]) -> AerfResult<String> {
    let entries = reconstruct_chain(receipts)?;
    Ok(render_entries(&entries))
}

pub fn render_entries(entries: &[ReconstructedAction]) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "{}. [{}] {{tool: {}, input_summary: {}, decision: {}, receipt_hash: {}}}",
                index + 1,
                entry.actor,
                entry.tool,
                entry.input_summary,
                entry.decision,
                entry.receipt_hash
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_receipt_input(receipt: &Receipt) -> String {
    let args = receipt
        .evidence
        .get("args")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let observed = summarize_observed_output(receipt.evidence.get("result"));

    if let Some(path) = args.get("path").and_then(Value::as_str) {
        return append_observed(format!("path={path}"), observed);
    }
    if let Some(branch) = args.get("branch").and_then(Value::as_str) {
        return append_observed(format!("branch={branch}"), observed);
    }
    if let Some(message) = args.get("message").and_then(Value::as_str) {
        return append_observed(format!("message={}", summarize_string(message, 96)), observed);
    }
    if let Some(command) = args.get("command").and_then(Value::as_str) {
        return append_observed(format!("command={}", summarize_string(command, 120)), observed);
    }
    if let Some(text) = args.get("text").and_then(Value::as_str) {
        return append_observed(format!("text={}", summarize_string(text, 120)), observed);
    }

    let mut fields = args
        .iter()
        .map(|(key, value)| format!("{key}={}", summarize_value(value)))
        .collect::<Vec<_>>();
    fields.sort();
    if fields.is_empty() {
        append_observed("none".to_owned(), observed)
    } else {
        append_observed(fields.join(", "), observed)
    }
}

fn summarize_value(value: &Value) -> String {
    match value {
        Value::String(text) => summarize_string(text, 96),
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::Object(map) => format!("object(keys={})", map.len()),
        _ => value.to_string(),
    }
}

fn summarize_string(text: &str, max_len: usize) -> String {
    let sanitized = text.replace('\n', "\\n");
    let count = sanitized.chars().count();
    if count <= max_len {
        return sanitized;
    }
    let shortened = sanitized.chars().take(max_len).collect::<String>();
    format!("{shortened}...")
}

fn summarize_observed_output(result: Option<&Value>) -> Option<String> {
    let result = result?.as_object()?;
    if let Some(contents) = result.get("contents").and_then(Value::as_str) {
        return Some(format!("observed={}", summarize_string(contents, 120)));
    }
    if let Some(stdout) = result.get("stdout").and_then(Value::as_str) {
        return Some(format!("observed={}", summarize_string(stdout, 120)));
    }
    if let Some(body) = result.get("body").and_then(Value::as_str) {
        return Some(format!("observed={}", summarize_string(body, 120)));
    }
    if let Some(text) = result.get("text").and_then(Value::as_str) {
        return Some(format!("observed={}", summarize_string(text, 120)));
    }
    None
}

fn append_observed(base: String, observed: Option<String>) -> String {
    match observed {
        Some(observed) => format!("{base}; {observed}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use serde_json::Value;

    use super::{reconstruct_chain, render_entries, render_reconstruction};
    use crate::aerf::receipt::Receipt;
    use crate::aerf::sign::{key_id, sign_stripped};

    fn signed_receipt(value: Value) -> Receipt {
        let signing_key = SigningKey::from_bytes(&[31; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut receipt = json!({
            "id": "r1",
            "type": "notarised_evidence",
            "plan_id": "p1",
            "agent": "main-agent",
            "action": value["action"].clone(),
            "in_policy": value["in_policy"].clone(),
            "policy_reason": "ok",
            "evidence_hash_sha512": "abc",
            "evidence": value["evidence"].clone(),
            "observed_at": "2026-07-17T00:00:00.000Z",
            "key_id": key_id(&verifying_key),
            "signature": ""
        });
        let signature = sign_stripped(&receipt, &signing_key).expect("signature");
        receipt["signature"] = Value::String(signature);
        Receipt::from_value(receipt).expect("receipt")
    }

    #[test]
    fn reconstructs_tool_inputs_and_decisions() {
        let receipt = signed_receipt(json!({
            "action": "run_command",
            "in_policy": false,
            "evidence": {
                "args": {
                    "command": "rm -rf /"
                }
            }
        }));

        let entries = reconstruct_chain(&[receipt]).expect("entries");

        assert_eq!(entries[0].tool, "run_command");
        assert_eq!(entries[0].decision, "block");
        assert!(entries[0].input_summary.contains("command=rm -rf /"));
    }

    #[test]
    fn renders_actor_prefixed_lines() {
        let receipt = signed_receipt(json!({
            "action": "read_file",
            "in_policy": true,
            "evidence": {
                "args": {
                    "path": "src/app.ts"
                },
                "result": {
                    "contents": "IMPORTANT: keep this nearby"
                }
            }
        }));

        let rendered = render_reconstruction(&[receipt]).expect("rendered");

        assert!(rendered.contains("[main-agent]"));
        assert!(rendered.contains("tool: read_file"));
        assert!(rendered.contains("input_summary: path=src/app.ts"));
        assert!(rendered.contains("observed=IMPORTANT: keep this nearby"));
    }

    #[test]
    fn render_entries_preserves_order() {
        let first = signed_receipt(json!({
            "action": "read_file",
            "in_policy": true,
            "evidence": { "args": { "path": "a" } }
        }));
        let second = signed_receipt(json!({
            "action": "write_file",
            "in_policy": false,
            "evidence": { "args": { "path": "b" } }
        }));

        let entries = reconstruct_chain(&[first, second]).expect("entries");
        let rendered = render_entries(&entries);

        assert!(rendered.lines().next().is_some_and(|line| line.contains("read_file")));
        assert!(rendered.lines().nth(1).is_some_and(|line| line.contains("write_file")));
    }
}
