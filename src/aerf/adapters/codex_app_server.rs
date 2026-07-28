//! Codex App Server stdio JSONL adapter with deterministic event normalization.
//! Used by: protocol tests and future Codex ingestion.

use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::aerf::{AerfError, AerfResult};

const INITIALIZE_REQUEST_ID: &str = "agentmint-codex-initialize";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexInstalledVersion {
    pub version: String,
    pub package_json_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexProtocolSchemas {
    pub codex_version: String,
    pub transport: String,
    pub initialize_request: Value,
    pub initialize_notification: Value,
    pub initialize_result_shape: Value,
    pub normalized_event_shape: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexInitializeHandshake {
    pub codex_version: String,
    pub initialize_request: Value,
    pub initialize_response: Value,
    pub initialized_notification: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexTranscript {
    pub codex_version: String,
    pub handshake: CodexInitializeHandshake,
    pub events: Vec<NormalizedCodexEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NormalizedCodexEvent {
    ThreadLifecycle(NormalizedLifecycle),
    TurnLifecycle(NormalizedLifecycle),
    ItemLifecycle(NormalizedItemLifecycle),
    CommandExecution(NormalizedCommandExecution),
    FileChange(NormalizedFileChange),
    AgentMessage(NormalizedAgentMessage),
    Completion(NormalizedCompletion),
    TokenUsage(NormalizedTokenUsage),
    Unknown { raw: Value },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedLifecycle {
    pub raw_event_name: String,
    pub phase: LifecyclePhase,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedItemLifecycle {
    pub raw_event_name: String,
    pub phase: LifecyclePhase,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub item_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedCommandExecution {
    pub raw_event_name: String,
    pub phase: LifecyclePhase,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedFileChange {
    pub raw_event_name: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub path: Option<String>,
    pub change_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedAgentMessage {
    pub raw_event_name: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub role: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedCompletion {
    pub raw_event_name: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub status: Option<String>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTokenUsage {
    pub raw_event_name: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    Started,
    Updated,
    Completed,
    Unknown,
}

pub trait JsonlTransport {
    fn write_json(&mut self, value: &Value) -> AerfResult<()>;
    fn read_json(&mut self) -> AerfResult<Option<Value>>;
}

pub struct StdioJsonlTransport<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> StdioJsonlTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
}

impl<R, W> JsonlTransport for StdioJsonlTransport<R, W>
where
    R: BufRead,
    W: Write,
{
    fn write_json(&mut self, value: &Value) -> AerfResult<()> {
        serde_json::to_writer(&mut self.writer, value)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    fn read_json(&mut self) -> AerfResult<Option<Value>> {
        let mut line = String::new();
        let bytes = self.reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(trimmed)?))
    }
}

pub struct CodexAppServerAdapter<T> {
    transport: T,
    installed_version: CodexInstalledVersion,
    next_request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexRpcCall {
    pub codex_version: String,
    pub request: Value,
    pub response: Value,
    pub events: Vec<NormalizedCodexEvent>,
}

impl<T> CodexAppServerAdapter<T>
where
    T: JsonlTransport,
{
    pub fn new(transport: T, installed_version: CodexInstalledVersion) -> Self {
        Self {
            transport,
            installed_version,
            next_request_id: 1,
        }
    }

    pub fn initialize(&mut self) -> AerfResult<CodexInitializeHandshake> {
        let request = initialize_request();
        self.transport.write_json(&request)?;
        let response = self
            .transport
            .read_json()?
            .ok_or_else(|| AerfError::UnsupportedValue {
                path: "$".to_owned(),
                kind: "missing initialize response".to_owned(),
            })?;
        let initialized = initialize_notification();
        self.transport.write_json(&initialized)?;

        Ok(CodexInitializeHandshake {
            codex_version: self.installed_version.version.clone(),
            initialize_request: request,
            initialize_response: response,
            initialized_notification: initialized,
        })
    }

    pub fn collect_transcript(&mut self) -> AerfResult<CodexTranscript> {
        let handshake = self.initialize()?;
        let mut events = Vec::new();
        while let Some(value) = self.transport.read_json()? {
            events.extend(normalize_events(&value)?);
        }

        Ok(CodexTranscript {
            codex_version: self.installed_version.version.clone(),
            handshake,
            events,
        })
    }

    pub fn call(&mut self, method: &str, params: Value) -> AerfResult<CodexRpcCall> {
        let request_id = format!("agentmint-codex-call-{}", self.next_request_id);
        self.next_request_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        self.transport.write_json(&request)?;

        let mut events = Vec::new();
        loop {
            let value = self
                .transport
                .read_json()?
                .ok_or_else(|| AerfError::UnsupportedValue {
                    path: "$".to_owned(),
                    kind: format!("missing response for {method}"),
                })?;
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                events.extend(normalize_events(&value)?);
                continue;
            };
            if id != request_id {
                events.extend(normalize_events(&value)?);
                continue;
            }
            if value.get("error").is_some() {
                return Err(AerfError::UnsupportedValue {
                    path: "$.error".to_owned(),
                    kind: serde_json::to_string(&value["error"])?,
                });
            }
            return Ok(CodexRpcCall {
                codex_version: self.installed_version.version.clone(),
                request,
                response: value,
                events,
            });
        }
    }
}

pub fn installed_codex_version() -> AerfResult<CodexInstalledVersion> {
    let package_json_path = installed_codex_package_json_path().ok_or_else(|| {
        AerfError::UnsupportedValue {
            path: "$".to_owned(),
            kind: "unable to locate installed Codex package.json".to_owned(),
        }
    })?;
    let value: Value = serde_json::from_slice(&fs::read(&package_json_path)?)?;
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| AerfError::UnsupportedValue {
            path: "$.version".to_owned(),
            kind: "missing Codex package version".to_owned(),
        })?
        .to_owned();

    Ok(CodexInstalledVersion {
        version,
        package_json_path,
    })
}

pub fn generate_protocol_schemas(installed_version: &CodexInstalledVersion) -> CodexProtocolSchemas {
    CodexProtocolSchemas {
        codex_version: installed_version.version.clone(),
        transport: "stdio-jsonl".to_owned(),
        initialize_request: initialize_request(),
        initialize_notification: initialize_notification(),
        initialize_result_shape: json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_REQUEST_ID,
            "result": {
                "serverInfo": {
                    "name": "codex-app-server",
                    "version": installed_version.version
                }
            }
        }),
        normalized_event_shape: json!({
            "codex_version": installed_version.version,
            "event_kinds": [
                "thread_lifecycle",
                "turn_lifecycle",
                "item_lifecycle",
                "command_execution",
                "file_change",
                "agent_message",
                "completion",
                "token_usage",
                "unknown"
            ],
            "notes": [
                "unknown events are preserved in order",
                "items are normalized as lifecycle records only",
                "item lifecycle does not imply a forkable model-state boundary"
            ]
        }),
    }
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": INITIALIZE_REQUEST_ID,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "clientInfo": {
                "name": "agentmint",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "preserveUnknownEvents": true,
                "normalizeLifecycle": true
            }
        }
    })
}

fn initialize_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
}

fn normalize_events(value: &Value) -> AerfResult<Vec<NormalizedCodexEvent>> {
    let event_name = raw_event_name(value);
    let params = event_params(value);
    let mut out = Vec::new();
    if is_thread_event(&event_name) {
        out.push(NormalizedCodexEvent::ThreadLifecycle(NormalizedLifecycle {
            raw_event_name: event_name,
            phase: lifecycle_phase(value),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
        }));
        return Ok(out);
    }
    if is_turn_event(&event_name) {
        out.push(NormalizedCodexEvent::TurnLifecycle(NormalizedLifecycle {
            raw_event_name: event_name.clone(),
            phase: lifecycle_phase(value),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id", "id"]),
        }));
        if is_completion_event(&event_name, params) {
            out.push(NormalizedCodexEvent::Completion(NormalizedCompletion {
                raw_event_name: raw_event_name(value),
                thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
                turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
                item_id: string_value(params, &["item_id", "itemId", "item.id"]),
                status: string_value(params, &["status", "state"]),
                finish_reason: string_value(params, &["finish_reason", "finishReason", "reason"]),
            }));
        }
        if let Some(usage) = usage_payload(params) {
            validate_token_usage(usage, "$.params.usage")?;
            out.push(NormalizedCodexEvent::TokenUsage(NormalizedTokenUsage {
                raw_event_name: raw_event_name(value),
                thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
                turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
                item_id: string_value(params, &["item_id", "itemId", "item.id"]),
                input_tokens: number_field(usage, &["input_tokens", "inputTokens"]),
                cached_input_tokens: number_field(usage, &["cached_input_tokens", "cachedInputTokens"]),
                output_tokens: number_field(usage, &["output_tokens", "outputTokens"]),
                total_tokens: number_field(usage, &["total_tokens", "totalTokens"]),
            }));
        }
        return Ok(out);
    }
    if is_command_event(&event_name, params) {
        out.push(NormalizedCodexEvent::CommandExecution(NormalizedCommandExecution {
            raw_event_name: event_name,
            phase: lifecycle_phase(value),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id"]),
            command: string_value(params, &["command", "cmd", "execution.command"]),
            exit_code: integer_value(params, &["exit_code", "exitCode", "execution.exitCode"]),
            cwd: string_value(params, &["cwd", "execution.cwd"]),
        }));
        return Ok(out);
    }
    if is_file_change_event(&event_name, params) {
        out.push(NormalizedCodexEvent::FileChange(NormalizedFileChange {
            raw_event_name: event_name,
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id"]),
            path: string_value(params, &["path", "file.path", "change.path"]),
            change_kind: string_value(params, &["change_kind", "changeKind", "kind"]),
        }));
        return Ok(out);
    }
    if is_message_event(&event_name, params) {
        out.push(NormalizedCodexEvent::AgentMessage(NormalizedAgentMessage {
            raw_event_name: event_name,
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id", "id"]),
            role: string_value(params, &["role", "message.role"]),
            text: string_value(params, &["text", "message.text", "delta", "content"]),
        }));
        return Ok(out);
    }
    if is_completion_event(&event_name, params) {
        out.push(NormalizedCodexEvent::Completion(NormalizedCompletion {
            raw_event_name: event_name.clone(),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id"]),
            status: string_value(params, &["status", "state"]),
            finish_reason: string_value(params, &["finish_reason", "finishReason", "reason"]),
        }));
    }
    if let Some(usage) = usage_payload(params) {
        validate_token_usage(usage, "$.params.usage")?;
        out.push(NormalizedCodexEvent::TokenUsage(NormalizedTokenUsage {
            raw_event_name: event_name.clone(),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id"]),
            input_tokens: number_field(usage, &["input_tokens", "inputTokens"]),
            cached_input_tokens: number_field(usage, &["cached_input_tokens", "cachedInputTokens"]),
            output_tokens: number_field(usage, &["output_tokens", "outputTokens"]),
            total_tokens: number_field(usage, &["total_tokens", "totalTokens"]),
        }));
        if !out.is_empty() {
            return Ok(out);
        }
    }
    if is_item_event(&event_name, params) {
        out.push(NormalizedCodexEvent::ItemLifecycle(NormalizedItemLifecycle {
            raw_event_name: event_name,
            phase: lifecycle_phase(value),
            thread_id: string_value(params, &["thread_id", "threadId", "thread.id"]),
            turn_id: string_value(params, &["turn_id", "turnId", "turn.id"]),
            item_id: string_value(params, &["item_id", "itemId", "item.id", "id"]),
            item_type: string_value(params, &["item_type", "itemType", "type"]),
        }));
        return Ok(out);
    }
    if out.is_empty() {
        out.push(NormalizedCodexEvent::Unknown { raw: value.clone() });
    }
    Ok(out)
}

fn raw_event_name(value: &Value) -> String {
    value
        .get("method")
        .and_then(Value::as_str)
        .or_else(|| value.get("event").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .or_else(|| value.pointer("/event/type").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_owned()
}

fn event_params(value: &Value) -> &Value {
    value
        .get("params")
        .or_else(|| value.get("result"))
        .or_else(|| value.get("event"))
        .unwrap_or(value)
}

fn lifecycle_phase(value: &Value) -> LifecyclePhase {
    let name = raw_event_name(value);
    if name.contains("start") || name.contains("begin") || name.contains("create") {
        return LifecyclePhase::Started;
    }
    if name.contains("complete") || name.contains("finish") || name.contains("done") {
        return LifecyclePhase::Completed;
    }
    if name.contains("update") || name.contains("delta") || name.contains("change") {
        return LifecyclePhase::Updated;
    }
    LifecyclePhase::Unknown
}

fn is_thread_event(name: &str) -> bool {
    name.contains("thread")
}

fn is_turn_event(name: &str) -> bool {
    name.contains("turn")
}

fn is_item_event(name: &str, params: &Value) -> bool {
    name.contains("item")
        || string_value(params, &["item_id", "itemId", "item.id"]).is_some()
            && !is_command_event(name, params)
            && !is_message_event(name, params)
            && !is_file_change_event(name, params)
}

fn is_command_event(name: &str, params: &Value) -> bool {
    name.contains("command")
        || string_value(params, &["command", "cmd", "execution.command"]).is_some()
}

fn is_file_change_event(name: &str, params: &Value) -> bool {
    (name.contains("file") && name.contains("change"))
        || string_value(params, &["path", "file.path", "change.path"]).is_some()
            && string_value(params, &["change_kind", "changeKind", "kind"]).is_some()
}

fn is_message_event(name: &str, params: &Value) -> bool {
    name.contains("message")
        || string_value(params, &["message.text", "text", "delta", "content"]).is_some()
            && string_value(params, &["role", "message.role"]).is_some()
}

fn is_completion_event(name: &str, params: &Value) -> bool {
    name.contains("completion")
        || string_value(params, &["finish_reason", "finishReason", "reason"]).is_some()
            && string_value(params, &["status", "state"]).is_some()
}

fn usage_payload(value: &Value) -> Option<&Value> {
    value
        .get("usage")
        .or_else(|| value.pointer("/result/usage"))
        .or_else(|| value.pointer("/message/usage"))
}

fn string_value(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| find_path(value, path).and_then(Value::as_str).map(str::to_owned))
}

fn integer_value(value: &Value, paths: &[&str]) -> Option<i64> {
    paths.iter().find_map(|path| find_path(value, path).and_then(Value::as_i64))
}

fn number_field(value: &Value, paths: &[&str]) -> Option<u64> {
    paths.iter().find_map(|path| find_path(value, path).and_then(Value::as_u64))
}

fn validate_token_usage(usage: &Value, path: &str) -> AerfResult<()> {
    let input_tokens = number_field(usage, &["input_tokens", "inputTokens"]);
    let cached_input_tokens = number_field(usage, &["cached_input_tokens", "cachedInputTokens"]);
    if let (Some(input), Some(cached)) = (input_tokens, cached_input_tokens) {
        if cached > input {
            return Err(AerfError::UnsupportedValue {
                path: path.to_owned(),
                kind: "cached input tokens exceed input tokens".to_owned(),
            });
        }
    }
    Ok(())
}

fn find_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn installed_codex_package_json_path() -> Option<PathBuf> {
    let env_path = std::env::var_os("AGENTMINT_CODEX_PACKAGE_JSON").map(PathBuf::from);
    let candidates = [
        env_path,
        Some(PathBuf::from("/usr/local/lib/node_modules/@openai/codex/package.json")),
        Some(PathBuf::from("/opt/homebrew/lib/node_modules/@openai/codex/package.json")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use serde_json::{json, Value};

    use super::{
        generate_protocol_schemas, installed_codex_version, CodexAppServerAdapter,
        JsonlTransport, NormalizedCodexEvent,
    };
    use crate::aerf::{AerfError, AerfResult};

    #[derive(Default)]
    struct MockTransport {
        writes: Vec<Value>,
        reads: VecDeque<Value>,
    }

    impl MockTransport {
        fn with_reads(reads: Vec<Value>) -> Self {
            Self {
                writes: Vec::new(),
                reads: reads.into(),
            }
        }
    }

    impl JsonlTransport for MockTransport {
        fn write_json(&mut self, value: &Value) -> AerfResult<()> {
            self.writes.push(value.clone());
            Ok(())
        }

        fn read_json(&mut self) -> AerfResult<Option<Value>> {
            Ok(self.reads.pop_front())
        }
    }

    #[test]
    fn installed_version_generates_protocol_schemas() {
        let installed = installed_codex_version().expect("installed version");
        let schemas = generate_protocol_schemas(&installed);
        let snapshot = serde_json::to_string_pretty(&schemas).expect("schema json");

        assert_eq!(installed.version, "0.130.0");
        assert_eq!(
            snapshot,
            r#"{
  "codex_version": "0.130.0",
  "transport": "stdio-jsonl",
  "initialize_request": {
    "id": "agentmint-codex-initialize",
    "jsonrpc": "2.0",
    "method": "initialize",
    "params": {
      "capabilities": {
        "normalizeLifecycle": true,
        "preserveUnknownEvents": true
      },
      "clientInfo": {
        "name": "agentmint",
        "version": "0.1.0"
      },
      "protocolVersion": "2026-07-28"
    }
  },
  "initialize_notification": {
    "jsonrpc": "2.0",
    "method": "initialized",
    "params": {}
  },
  "initialize_result_shape": {
    "id": "agentmint-codex-initialize",
    "jsonrpc": "2.0",
    "result": {
      "serverInfo": {
        "name": "codex-app-server",
        "version": "0.130.0"
      }
    }
  },
  "normalized_event_shape": {
    "codex_version": "0.130.0",
    "event_kinds": [
      "thread_lifecycle",
      "turn_lifecycle",
      "item_lifecycle",
      "command_execution",
      "file_change",
      "agent_message",
      "completion",
      "token_usage",
      "unknown"
    ],
    "notes": [
      "unknown events are preserved in order",
      "items are normalized as lifecycle records only",
      "item lifecycle does not imply a forkable model-state boundary"
    ]
  }
}"#
        );
    }

    #[test]
    fn performs_initialize_handshake_and_normalizes_transcript() {
        let installed = installed_codex_version().expect("installed version");
        let transport = MockTransport::with_reads(vec![
            json!({
                "jsonrpc": "2.0",
                "id": "agentmint-codex-initialize",
                "result": {
                    "serverInfo": {
                        "name": "codex-app-server",
                        "version": "0.130.0"
                    }
                }
            }),
            json!({
                "method": "thread.started",
                "params": { "threadId": "thread-1" }
            }),
            json!({
                "method": "turn.started",
                "params": { "threadId": "thread-1", "turnId": "turn-1" }
            }),
            json!({
                "method": "item.started",
                "params": { "threadId": "thread-1", "turnId": "turn-1", "itemId": "item-1", "type": "message" }
            }),
            json!({
                "method": "command.completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-2",
                    "command": "cargo test",
                    "cwd": "/repo",
                    "exitCode": 0
                }
            }),
            json!({
                "method": "file.change",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-3",
                    "path": "src/lib.rs",
                    "changeKind": "modified"
                }
            }),
            json!({
                "method": "message.delta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-1",
                    "role": "assistant",
                    "delta": "patched file"
                }
            }),
            json!({
                "method": "turn.completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "status": "completed",
                    "finishReason": "stop",
                    "usage": {
                        "inputTokens": 12,
                        "cachedInputTokens": 3,
                        "outputTokens": 7,
                        "totalTokens": 19
                    }
                }
            }),
            json!({
                "method": "mystery.event",
                "params": {
                    "surprise": true
                }
            }),
        ]);

        let mut adapter = CodexAppServerAdapter::new(transport, installed);
        let transcript = adapter.collect_transcript().expect("transcript");

        assert_eq!(transcript.codex_version, "0.130.0");
        assert_eq!(transcript.handshake.initialize_request["method"], json!("initialize"));
        assert_eq!(transcript.handshake.initialized_notification["method"], json!("initialized"));
        assert_eq!(transcript.events.len(), 10);
        assert!(matches!(
            transcript.events[0],
            NormalizedCodexEvent::ThreadLifecycle(_)
        ));
        assert!(matches!(
            transcript.events[1],
            NormalizedCodexEvent::TurnLifecycle(_)
        ));
        assert!(matches!(
            transcript.events[2],
            NormalizedCodexEvent::ItemLifecycle(_)
        ));
        assert!(matches!(
            transcript.events[3],
            NormalizedCodexEvent::CommandExecution(_)
        ));
        assert!(matches!(
            transcript.events[4],
            NormalizedCodexEvent::FileChange(_)
        ));
        assert!(matches!(
            transcript.events[5],
            NormalizedCodexEvent::AgentMessage(_)
        ));
        assert!(matches!(
            transcript.events[6],
            NormalizedCodexEvent::TurnLifecycle(_)
        ));
        assert!(matches!(
            transcript.events[7],
            NormalizedCodexEvent::Completion(_)
        ));
        assert!(matches!(
            transcript.events[8],
            NormalizedCodexEvent::TokenUsage(_)
        ));
        assert!(matches!(
            transcript.events[9],
            NormalizedCodexEvent::Unknown { .. }
        ));
    }

    #[test]
    fn rejects_cached_input_tokens_above_input_tokens() {
        let installed = installed_codex_version().expect("installed version");
        let transport = MockTransport::with_reads(vec![
            json!({
                "jsonrpc": "2.0",
                "id": "agentmint-codex-initialize",
                "result": {
                    "serverInfo": {
                        "name": "codex-app-server",
                        "version": "0.130.0"
                    }
                }
            }),
            json!({
                "method": "turn.completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "status": "completed",
                    "finishReason": "stop",
                    "usage": {
                        "inputTokens": 2,
                        "cachedInputTokens": 3,
                        "outputTokens": 1,
                        "totalTokens": 3
                    }
                }
            }),
        ]);

        let mut adapter = CodexAppServerAdapter::new(transport, installed);
        let err = adapter.collect_transcript().expect_err("invalid usage should fail");

        assert!(matches!(
            err,
            AerfError::UnsupportedValue { ref path, ref kind }
            if path == "$.params.usage" && kind == "cached input tokens exceed input tokens"
        ));
    }

    #[test]
    fn call_collects_events_until_matching_response() {
        let installed = installed_codex_version().expect("installed version");
        let transport = MockTransport::with_reads(vec![
            json!({
                "jsonrpc": "2.0",
                "id": "agentmint-codex-call-1",
                "result": {
                    "threadId": "thread-1"
                }
            }),
            json!({
                "method": "turn.started",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
            json!({
                "method": "message.delta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "item-1",
                    "role": "assistant",
                    "delta": "patched schema"
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": "agentmint-codex-call-2",
                "result": {
                    "status": "completed"
                }
            }),
        ]);

        let mut adapter = CodexAppServerAdapter::new(transport, installed);
        let thread = adapter
            .call("threads.create", json!({ "cwd": "/tmp/worktree" }))
            .expect("thread call");
        let turn = adapter
            .call("turns.create", json!({ "threadId": "thread-1" }))
            .expect("turn call");

        assert_eq!(thread.response["result"]["threadId"], json!("thread-1"));
        assert_eq!(turn.events.len(), 2);
        assert!(matches!(
            turn.events[0],
            NormalizedCodexEvent::TurnLifecycle(_)
        ));
        assert!(matches!(
            turn.events[1],
            NormalizedCodexEvent::AgentMessage(_)
        ));
        assert_eq!(turn.response["result"]["status"], json!("completed"));
    }
}
