//! Gear loop orchestration for turn.
//! Used by: the turn CLI run command.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::process::Command;
use tokio::time::timeout;

use crate::checkpoint::{CheckpointService, GitCliCheckpointService};
use crate::turnrt::belief::{
    BeliefRecord, StreamBeliefParser, ToolCallRecord, BELIEF_PROMPT_CONVENTION,
};
use crate::turnrt::engine::{ChatMessage, Engine, EngineConfig, ModelSnapshot};
use crate::turnrt::policy::{
    BigDeletion, NonZeroExit, PolicyHit, ResolutionMismatch, ThrashDetector, ToolResultRecord,
    TurnPolicy,
};
use crate::turnrt::tape::{EventKind, Tape, TapeError, TapeEvent};
use crate::turnrt::tui::{format_gear_line, shared_input_state, GearView, InputState};

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error(transparent)]
    Tape(#[from] TapeError),
    #[error(transparent)]
    Engine(#[from] crate::turnrt::engine::EngineError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("tool timeout after {0:?}")]
    ToolTimeout(Duration),
    #[error("tool output decode failed")]
    ToolDecode,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub repo: PathBuf,
    pub task: String,
    pub tape_path: PathBuf,
    pub max_gears: usize,
    pub engine: EngineConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub total_gears: usize,
    pub beliefs_parsed: usize,
    pub beliefs_failed: usize,
    pub policy_hits: HashMap<String, usize>,
    pub pauses_by_trigger: HashMap<String, usize>,
}

pub async fn run(options: RunOptions) -> Result<RunSummary, LoopError> {
    let mut tape = Tape::create(&options.tape_path)?;
    let mut engine = Engine::new();
    let model_snapshot = engine
        .fetch_model_snapshot(&options.engine.base_url, Some(&options.engine.model))
        .await?;
    let input = shared_input_state();
    let mut state = LoopState::new(options.clone(), model_snapshot, input);
    state.write_run_start(&mut tape)?;

    for gear in 1..=options.max_gears {
        if state.input.lock().expect("lock").quit {
            break;
        }

        state.current_gear = gear;
        state
            .input
            .lock()
            .expect("lock")
            .set_frontier(gear.saturating_sub(1));
        state.write_gear_start(&mut tape)?;
        let checkpoint = GitCliCheckpointService::new().capture_checkpoint(&options.repo);
        if let Ok(snapshot) = checkpoint {
            state.last_checkpoint_id = Some(snapshot.checkpoint_id);
        }

        let mut parser = StreamBeliefParser::default();
        let completion = engine
            .stream_completion(&options.engine, &state.messages, |chunk| {
                parser.feed(&chunk.text_delta, chunk.token_logprob);
            })
            .await?;

        let mut belief = parser.belief().unwrap_or_else(|| {
            state.beliefs_failed += 1;
            StreamBeliefParser::parse_missing_belief(&completion.text)
        });
        let tool = parser.tool();
        state.messages.push(ChatMessage {
            role: "assistant".to_owned(),
            content: completion.text.clone(),
        });

        if belief.parse_error.is_none() {
            state.beliefs_parsed += 1;
        } else {
            state.beliefs_failed += 1;
        }

        state.write_belief(&mut tape, &belief)?;
        if let Some(hit) = state.fire_belief_policies(&belief, &mut tape)? {
            state.pause("policy", &hit.reason, &belief, "", &mut tape)?;
        }

        if completion.text.contains("```done") {
            state.write_run_end(&mut tape)?;
            return Ok(state.finish());
        }

        let tool = match tool {
            Some(tool) if tool.name == "shell" => tool,
            Some(tool) => {
                belief.parse_error = Some(format!("unsupported tool {}", tool.name));
                state.write_belief(&mut tape, &belief)?;
                state.write_run_end(&mut tape)?;
                return Ok(state.finish());
            }
            None => {
                state.write_run_end(&mut tape)?;
                return Ok(state.finish());
            }
        };

        if let Some(hit) = state.big_deletion.inspect_command(&tool.raw) {
            state.record_policy_hit(&hit, &mut tape)?;
            state.pause(
                "policy",
                &hit.reason,
                &belief,
                &extract_shell_command(&tool.args),
                &mut tape,
            )?;
        }

        let command = extract_shell_command(&tool.args);
        state.write_tool_call(&mut tape, &tool, &command)?;
        let result = run_shell(&options.repo, &command).await?;
        let result_record = ToolResultRecord {
            gear,
            command: command.clone(),
            exit_code: result.exit_code,
            deleted_lines: crate::turnrt::policy::count_deleted_lines(&tool.raw),
        };
        state.write_tool_result(&mut tape, &result)?;
        if let Some(hit) = state.fire_tool_policies(&result_record, &mut tape)? {
            state.pause("policy", &hit.reason, &belief, &command, &mut tape)?;
        }

        let gear_view = GearView {
            gear,
            frontier: gear,
            claim: belief.claim.clone(),
            said: belief.said,
            logit: belief.logit,
            command: command.clone(),
            exit_code: result.exit_code,
        };
        println!("{}", format_gear_line(&gear_view));
        state.messages.push(ChatMessage {
            role: "user".to_owned(),
            content: format!(
                "[TOOL RESULT]\ncommand: {}\nexit_code: {}\nstdout:\n{}\nstderr:\n{}",
                command,
                result
                    .exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "timeout".to_owned()),
                result.stdout,
                result.stderr
            ),
        });
        state.write_gear_end(&mut tape, result.exit_code)?;
    }

    state.write_run_end(&mut tape)?;
    Ok(state.finish())
}

fn extract_shell_command(args: &serde_json::Value) -> String {
    args.get("cmd")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned()
}

struct LoopState {
    options: RunOptions,
    model_snapshot: ModelSnapshot,
    seq: u64,
    current_gear: usize,
    beliefs_parsed: usize,
    beliefs_failed: usize,
    messages: Vec<ChatMessage>,
    policy_hits: HashMap<String, usize>,
    pauses_by_trigger: HashMap<String, usize>,
    input: Arc<Mutex<InputState>>,
    non_zero_exit: NonZeroExit,
    big_deletion: BigDeletion,
    thrash: ThrashDetector,
    resolution_mismatch: ResolutionMismatch,
    last_checkpoint_id: Option<String>,
}

impl LoopState {
    fn new(
        options: RunOptions,
        model_snapshot: ModelSnapshot,
        input: Arc<Mutex<InputState>>,
    ) -> Self {
        Self {
            messages: vec![
                ChatMessage {
                    role: "system".to_owned(),
                    content: BELIEF_PROMPT_CONVENTION.to_owned(),
                },
                ChatMessage {
                    role: "user".to_owned(),
                    content: options.task.clone(),
                },
            ],
            options,
            model_snapshot,
            seq: 0,
            current_gear: 0,
            beliefs_parsed: 0,
            beliefs_failed: 0,
            policy_hits: HashMap::new(),
            pauses_by_trigger: HashMap::new(),
            input,
            non_zero_exit: NonZeroExit::new(),
            big_deletion: BigDeletion::new(),
            thrash: ThrashDetector::new(),
            resolution_mismatch: ResolutionMismatch::new(),
            last_checkpoint_id: None,
        }
    }

    fn next_event(&mut self, kind: EventKind, body: serde_json::Value) -> TapeEvent {
        self.seq += 1;
        TapeEvent {
            seq: self.seq,
            branch: 0,
            parent: None,
            ts: Utc::now(),
            kind,
            body,
        }
    }

    fn write_run_start(&mut self, tape: &mut Tape) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::RunStart,
            json!({
                "run_id": format!("turn-{}", Utc::now().timestamp_millis()),
                "model": self.model_snapshot.id,
                "base_url": self.options.engine.base_url,
                "temperature": self.options.engine.temperature,
                "top_p": self.options.engine.top_p,
                "seed": self.options.engine.seed,
                "model_snapshot": self.model_snapshot.raw,
            }),
        ))
    }

    fn write_gear_start(&mut self, tape: &mut Tape) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::GearStart,
            json!({
                "gear": self.current_gear,
                "checkpoint_id": self.last_checkpoint_id,
            }),
        ))
    }

    fn write_belief(&mut self, tape: &mut Tape, belief: &BeliefRecord) -> Result<(), TapeError> {
        tape.append(&self.next_event(EventKind::Belief, serde_json::to_value(belief)?))
    }

    fn write_tool_call(
        &mut self,
        tape: &mut Tape,
        tool: &ToolCallRecord,
        command: &str,
    ) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::ToolCall,
            json!({
                "name": tool.name,
                "args": tool.args,
                "command": command,
            }),
        ))
    }

    fn write_tool_result(
        &mut self,
        tape: &mut Tape,
        result: &ShellResult,
    ) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::ToolResult,
            json!({
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
            }),
        ))
    }

    fn write_gear_end(&mut self, tape: &mut Tape, exit_code: Option<i32>) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::GearEnd,
            json!({
                "gear": self.current_gear,
                "exit_code": exit_code,
            }),
        ))
    }

    fn write_run_end(&mut self, tape: &mut Tape) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::RunEnd,
            json!({
                "gears": self.current_gear,
                "beliefs_parsed": self.beliefs_parsed,
                "beliefs_failed": self.beliefs_failed,
                "policy_hits": self.policy_hits,
                "corrections": self.messages.iter().filter(|message| message.content.starts_with("[HUMAN CORRECTION]")).count(),
            }),
        ))
    }

    fn fire_belief_policies(
        &mut self,
        belief: &BeliefRecord,
        tape: &mut Tape,
    ) -> Result<Option<PolicyHit>, TapeError> {
        if let Some(hit) = [
            self.thrash.on_belief(belief),
            self.resolution_mismatch.on_belief(belief),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            self.record_policy_hit(&hit, tape)?;
            return Ok(Some(hit));
        }
        Ok(None)
    }

    fn fire_tool_policies(
        &mut self,
        result: &ToolResultRecord,
        tape: &mut Tape,
    ) -> Result<Option<PolicyHit>, TapeError> {
        if let Some(hit) = [
            self.non_zero_exit.on_tool_result(result),
            self.big_deletion.on_tool_result(result),
            self.thrash.on_tool_result(result),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            self.record_policy_hit(&hit, tape)?;
            return Ok(Some(hit));
        }
        Ok(None)
    }

    fn record_policy_hit(&mut self, hit: &PolicyHit, tape: &mut Tape) -> Result<(), TapeError> {
        *self.policy_hits.entry(hit.policy.to_owned()).or_insert(0) += 1;
        tape.append(&self.next_event(
            EventKind::PolicyHit,
            json!({
                "policy": hit.policy,
                "reason": hit.reason,
                "decision": hit.decision.as_str(),
            }),
        ))
    }

    fn pause(
        &mut self,
        trigger: &str,
        reason: &str,
        belief: &BeliefRecord,
        command: &str,
        tape: &mut Tape,
    ) -> Result<(), TapeError> {
        *self
            .pauses_by_trigger
            .entry(trigger.to_owned())
            .or_insert(0) += 1;
        tape.append(&self.next_event(
            EventKind::Pause,
            json!({
                "trigger": trigger,
                "reason": reason,
                "gear": self.current_gear,
            }),
        ))?;
        println!("⏸ {}", reason);
        println!(
            "last belief: \"{}\" {:.2} said · {}",
            belief.claim,
            belief.said,
            belief
                .logit
                .map(|value| format!("{value:.2} logit"))
                .unwrap_or_else(|| "n/a logit".to_owned())
        );
        println!("pending tool: {}", command);
        println!("y approve · type to correct · drag/←→ scrub · q quit");
        Ok(())
    }

    fn finish(self) -> RunSummary {
        RunSummary {
            total_gears: self.current_gear,
            beliefs_parsed: self.beliefs_parsed,
            beliefs_failed: self.beliefs_failed,
            policy_hits: self.policy_hits,
            pauses_by_trigger: self.pauses_by_trigger,
        }
    }
}

struct ShellResult {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

async fn run_shell(repo: &Path, command: &str) -> Result<ShellResult, LoopError> {
    let child = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let output = timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| LoopError::ToolTimeout(Duration::from_secs(120)))??;
    let stdout = String::from_utf8(output.stdout).map_err(|_| LoopError::ToolDecode)?;
    let stderr = String::from_utf8(output.stderr).map_err(|_| LoopError::ToolDecode)?;
    Ok(ShellResult {
        stdout,
        stderr,
        exit_code: output.status.code(),
    })
}
