//! One-off replay of the FULL 62-call session tool-call log: parse verbatim,
//! map each call to a canonical AERF ToolCall with the same rules as
//! examples/axum_replay.rs, feed all 62 through one continuous tier1 gate
//! session, then render the receipt chain and list every block in order.

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
        .join("tests/fixtures/replay/session_toolcalls_clean.txt")
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

fn path_of(args: &Value) -> String {
    args.get("path").and_then(Value::as_str).unwrap_or("<none>").to_owned()
}

fn main() -> Result<(), Box<dyn Error>> {
    let text = fs::read_to_string(fixture_path())?;
    let parsed = parse_log(&text);

    let engine = GateEngine::tier1();
    let session = engine.open_session("full-session-replay");

    let mut receipts = Vec::new();
    let mut blocks = Vec::new();
    let mut by_number = Vec::new();
    let mut mapped = 0usize;

    for call in &parsed {
        let Some(tool_call) = to_tool_call(call) else {
            continue;
        };
        mapped += 1;
        let tool = tool_call.tool.clone();
        let args = tool_call.args.clone();
        let outcome = session.evaluate(tool_call)?;
        let decision = outcome.decision;
        by_number.push((call.number.clone(), tool.clone(), args.clone(), decision, outcome.reason.clone()));
        if matches!(decision, GateDecision::Block) {
            blocks.push((call.number.clone(), tool, args, outcome.reason.clone()));
        }
        receipts.push(outcome.receipt);
    }

    println!(
        "parsed {} tool calls, mapped {mapped}, replayed through ONE tier1 session\n",
        parsed.len()
    );

    println!("== 1. reconstruct_chain() / render_entries() ==");
    let entries = reconstruct_chain(&receipts)?;
    println!("{}", render_entries(&entries));

    println!("\n== 2. GateDecision::Block results in order ({}) ==", blocks.len());
    if blocks.is_empty() {
        println!("(no blocks)");
    } else {
        for (number, tool, args, reason) in &blocks {
            println!("  #{number} {tool} args={args} -> BLOCK: {reason}");
        }
    }

    println!("\n== 3. Specific confirmation for original calls #49 and #55 ==");
    for target in ["49", "55"] {
        match by_number.iter().find(|(number, ..)| number == target) {
            Some((number, tool, args, decision, reason)) => {
                let verdict = match decision {
                    GateDecision::Pass => "PASS",
                    GateDecision::Block => "BLOCK",
                };
                println!(
                    "  original #{number}: {tool} path={} -> {verdict} (reason: {reason})",
                    path_of(args)
                );
            }
            None => println!("  original #{target}: not found among mapped calls"),
        }
    }

    Ok(())
}
