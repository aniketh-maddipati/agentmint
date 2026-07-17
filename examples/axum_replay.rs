//! One-off replay: parse the real axum-task tool-call log verbatim, map each
//! call to a canonical AERF ToolCall, feed all of them through one tier1 gate
//! session, then reconstruct and render the receipt chain and list any blocks.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use agentmint::aerf::gate::{GateDecision, GateEngine, ToolCall};
use agentmint::aerf::reconstruct::{reconstruct_chain, render_entries};
use serde_json::{json, Value};

struct ParsedCall {
    number: String,
    kind: String,
    input: Value,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/replay/axum_task_toolcalls_44-62.txt")
}

fn parse_log(text: &str) -> Vec<ParsedCall> {
    let lines: Vec<&str> = text.lines().collect();
    let mut calls = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(header) = line.strip_prefix("TOOL CALL #") else {
            continue;
        };
        let Some((number, rest)) = header.split_once(": ") else {
            continue;
        };
        let Some(kind) = rest.split_whitespace().next() else {
            continue;
        };
        let Some(json_line) = input_payload(&lines, index) else {
            continue;
        };
        let Ok(input) = serde_json::from_str::<Value>(json_line) else {
            continue;
        };
        calls.push(ParsedCall {
            number: number.to_owned(),
            kind: kind.to_owned(),
            input,
        });
    }
    calls
}

fn input_payload<'a>(lines: &[&'a str], header_index: usize) -> Option<&'a str> {
    let mut cursor = header_index + 1;
    while cursor < lines.len() {
        if lines[cursor].trim_start().starts_with("TOOL CALL #") {
            return None;
        }
        if lines[cursor].contains("INPUT") && lines[cursor].starts_with('-') {
            return lines.get(cursor + 1).copied();
        }
        cursor += 1;
    }
    None
}

fn to_tool_call(call: &ParsedCall) -> Option<ToolCall> {
    match call.kind.as_str() {
        "Read" => field(&call.input, "file_path")
            .map(|path| ToolCall::new("read_file", json!({ "path": path }))),
        "Write" | "Edit" => field(&call.input, "file_path")
            .map(|path| ToolCall::new("write_file", json!({ "path": path }))),
        "Bash" => field(&call.input, "command")
            .map(|command| ToolCall::new("run_command", json!({ "command": command }))),
        _ => None,
    }
}

fn field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn canonical_summary(call: &ToolCall) -> String {
    if let Some(path) = call.args.get("path").and_then(Value::as_str) {
        return format!("read_or_write path={path}");
    }
    if let Some(command) = call.args.get("command").and_then(Value::as_str) {
        let first_line = command.lines().next().unwrap_or("");
        return format!("run_command command[0]={first_line}");
    }
    "<unmapped>".to_owned()
}

fn main() -> Result<(), Box<dyn Error>> {
    let text = fs::read_to_string(fixture_path())?;
    let parsed = parse_log(&text);

    println!("== STEP 1: parsed {} tool calls -> canonical ToolCalls ==", parsed.len());
    let engine = GateEngine::tier1();
    let session = engine.open_session("axum-replay");

    let mut receipts = Vec::new();
    let mut blocks = Vec::new();
    let mut mapped = 0usize;

    for call in &parsed {
        let Some(tool_call) = to_tool_call(call) else {
            println!("  #{} {} -> <no mapping>", call.number, call.kind);
            continue;
        };
        mapped += 1;
        println!(
            "  #{} {} -> {}({})",
            call.number,
            call.kind,
            tool_call.tool,
            canonical_summary(&tool_call)
        );
        let tool = tool_call.tool.clone();
        let args = tool_call.args.clone();
        let outcome = session.evaluate(tool_call)?;
        if matches!(outcome.decision, GateDecision::Block) {
            blocks.push((call.number.clone(), tool, args, outcome.reason.clone()));
        }
        receipts.push(outcome.receipt);
    }

    println!("\n== STEP 2: fed {mapped} mapped calls through one GateEngine::tier1() session ==");
    println!("collected {} receipts", receipts.len());

    println!("\n== STEP 3: reconstruct_chain() + render_entries() ==");
    let entries = reconstruct_chain(&receipts)?;
    println!("{}", render_entries(&entries));

    println!("\n== STEP 4: GateDecision::Block results ({}) ==", blocks.len());
    if blocks.is_empty() {
        println!("(no blocks)");
    } else {
        for (number, tool, args, reason) in &blocks {
            println!("  #{number} {tool} args={args} -> BLOCK: {reason}");
        }
    }

    Ok(())
}
