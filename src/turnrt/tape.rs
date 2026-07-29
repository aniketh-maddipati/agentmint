//! Append-only tape logging and export helpers for turn runs.
//! Used by: turn loop execution, show, and export-run.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::aerf::intervention::{
    Attempt, AttemptOutcome, CorrectionPacket, InputEnvelope, InputItem, Intervention, Run,
    TokenUsage, Verification, VerificationVerdict,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapeEvent {
    pub seq: u64,
    pub branch: u32,
    pub parent: Option<u64>,
    pub ts: DateTime<Utc>,
    pub kind: EventKind,
    pub body: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunStart,
    GearStart,
    Belief,
    ToolCall,
    ToolResult,
    Pause,
    Correction,
    PolicyHit,
    GearAborted,
    GearEnd,
    RunEnd,
}

#[derive(Debug, thiserror::Error)]
pub enum TapeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub struct Tape {
    path: PathBuf,
    file: File,
}

impl Tape {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&mut self, event: &TapeEvent) -> Result<(), TapeError> {
        serde_json::to_writer(&mut self.file, event)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Vec<TapeEvent>, TapeError> {
        let file = File::open(path)?;
        let mut events = Vec::new();

        for line in BufReader::new(file).split(b'\n') {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            match serde_json::from_slice::<TapeEvent>(&line) {
                Ok(event) => events.push(event),
                Err(error) if error.is_eof() => {
                    eprintln!("warning: tape has truncated final line; dropping unreadable suffix");
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }

        Ok(events)
    }

    pub fn export_run(events: &[TapeEvent], task: &str) -> Run {
        let started_at = events
            .first()
            .map(|event| event.ts.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let run_id = events
            .first()
            .and_then(|event| event.body.get("run_id"))
            .and_then(Value::as_str)
            .unwrap_or("turn-run")
            .to_owned();
        let model = events
            .iter()
            .find(|event| matches!(event.kind, EventKind::RunStart))
            .and_then(|event| event.body.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let provider = events
            .iter()
            .find(|event| matches!(event.kind, EventKind::RunStart))
            .and_then(|event| event.body.get("base_url"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let correction_packets = events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::Correction))
            .enumerate()
            .map(|(index, event)| CorrectionPacket {
                packet_id: format!("correction-{}", index + 1),
                issued_at: event.ts.to_rfc3339(),
                summary: event
                    .body
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("correction")
                    .to_owned(),
                request: InputEnvelope {
                    input: vec![InputItem::Message {
                        role: "user".to_owned(),
                        content: event
                            .body
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    }],
                    cached_input: Vec::new(),
                },
                supporting_evidence_ids: Vec::new(),
            })
            .collect::<Vec<_>>();
        let attempts = events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ToolResult))
            .enumerate()
            .map(|(index, event)| Attempt {
                attempt_id: format!("attempt-{}", index + 1),
                endpoint: "shell".to_owned(),
                started_at: event.ts.to_rfc3339(),
                outcome: match event.body.get("exit_code").and_then(Value::as_i64) {
                    Some(0) => AttemptOutcome::Succeeded,
                    _ => AttemptOutcome::Failed,
                },
                request: InputEnvelope {
                    input: vec![InputItem::Message {
                        role: "user".to_owned(),
                        content: task.to_owned(),
                    }],
                    cached_input: Vec::new(),
                },
                status_code: None,
                retry_after_ms: None,
            })
            .collect::<Vec<_>>();

        Run {
            run_id: run_id.clone(),
            session_id: run_id,
            provider,
            model,
            request: InputEnvelope {
                input: vec![InputItem::Message {
                    role: "user".to_owned(),
                    content: task.to_owned(),
                }],
                cached_input: Vec::new(),
            },
            raw_events: Vec::new(),
            episodes: Vec::new(),
            interventions: Vec::<Intervention>::new(),
            correction_packets,
            attempts,
            verification: Verification {
                verified_at: started_at,
                verdict: VerificationVerdict::NeedsHumanReview,
                observed_fact_ids: Vec::new(),
                agent_statement_ids: Vec::new(),
                derived_inference_ids: Vec::new(),
                human_context_ids: Vec::new(),
            },
            token_usage: TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
            },
        }
    }
}

pub fn new_event(seq: u64, kind: EventKind, body: Value) -> TapeEvent {
    TapeEvent {
        seq,
        branch: 0,
        parent: None,
        ts: Utc::now(),
        kind,
        body,
    }
}

pub fn event_json(kind: EventKind, seq: u64, key: &str, value: Value) -> TapeEvent {
    new_event(seq, kind, json!({ key: value }))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentmint-turn-{}-{}.jsonl",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn tape_roundtrip() {
        let path = temp_path("roundtrip");
        let _ = fs::remove_file(&path);
        let mut tape = Tape::create(&path).expect("create tape");
        let now = Utc::now();
        let events = vec![
            TapeEvent {
                seq: 1,
                branch: 0,
                parent: None,
                ts: now,
                kind: EventKind::RunStart,
                body: json!({"a": 1}),
            },
            TapeEvent {
                seq: 2,
                branch: 0,
                parent: None,
                ts: now,
                kind: EventKind::GearStart,
                body: json!({"gear": 1}),
            },
            TapeEvent {
                seq: 3,
                branch: 0,
                parent: None,
                ts: now,
                kind: EventKind::Belief,
                body: json!({"claim": "x"}),
            },
            TapeEvent {
                seq: 4,
                branch: 1,
                parent: Some(2),
                ts: now,
                kind: EventKind::PolicyHit,
                body: json!({"policy": "p"}),
            },
            TapeEvent {
                seq: 5,
                branch: 0,
                parent: None,
                ts: now,
                kind: EventKind::GearEnd,
                body: json!({"gear": 1}),
            },
            TapeEvent {
                seq: 6,
                branch: 0,
                parent: None,
                ts: now,
                kind: EventKind::RunEnd,
                body: json!({"done": true}),
            },
        ];

        for event in &events {
            tape.append(event).expect("append");
        }

        let loaded = Tape::load(&path).expect("load");
        assert_eq!(loaded, events);

        let full = fs::read(&path).expect("read");
        let truncated = full[..full.len() - 8].to_vec();
        fs::write(&path, truncated).expect("truncate");
        let loaded = Tape::load(&path).expect("load truncated");
        assert_eq!(loaded.len(), 5);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn export_run_smoke() {
        let events = vec![
            TapeEvent {
                seq: 1,
                branch: 0,
                parent: None,
                ts: Utc::now(),
                kind: EventKind::RunStart,
                body: json!({"run_id": "run-1", "model": "qwen", "base_url": "http://localhost:1234"}),
            },
            TapeEvent {
                seq: 2,
                branch: 0,
                parent: None,
                ts: Utc::now(),
                kind: EventKind::Correction,
                body: json!({"text": "fix config"}),
            },
        ];

        let run = Tape::export_run(&events, "make tests pass");
        let bytes = serde_json::to_vec(&run).expect("serialize");
        let round_trip = Run::from_slice(&bytes).expect("round trip");
        assert_eq!(round_trip.run_id, "run-1");
        assert_eq!(round_trip.model, "qwen");
    }
}
