//! Gear loop orchestration for turn.
//! Used by: the turn CLI run command.

use std::collections::HashMap;
use std::io::{stdout, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::Utc;
use colored::Colorize;
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
use crate::turnrt::tui::{
    format_gear_line, install_panic_cleanup, render_pause_card, setup_terminal, shared_input_state,
    spawn_input_thread, CleanupHandle, CrosstermControl, GearView, InputState, PauseTrigger,
    StreamPrinter, TerminalControl,
};

const WAIT_POLL: Duration = Duration::from_millis(50);
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error(transparent)]
    Tape(#[from] TapeError),
    #[error(transparent)]
    Engine(#[from] crate::turnrt::engine::EngineError),
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Validation(String),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeAction {
    Resume,
    Quit,
}

pub async fn run(options: RunOptions) -> Result<RunSummary, LoopError> {
    validate_repo(&options.repo)?;
    validate_tape_path(&options.tape_path)?;

    let mut tape = Tape::create(&options.tape_path)?;
    let mut engine = Engine::new();
    let model_snapshot = engine
        .fetch_model_snapshot(&options.engine.base_url, Some(&options.engine.model))
        .await?;

    let input = shared_input_state();
    let tty = stdout().is_terminal();
    let _session = InteractiveSession::start(tty, input.clone());

    let mut state = LoopState::new(options.clone(), model_snapshot, input);
    state.write_run_start(&mut tape)?;

    'gears: for gear in 1..=options.max_gears {
        if state.wants_quit() {
            break 'gears;
        }
        if let Some(trigger) = state.take_pause_request() {
            let belief = state.last_belief.clone();
            if state.pause(trigger.label(), "paused by request", belief.as_ref(), "", &mut tape)?
                == ResumeAction::Quit
            {
                break 'gears;
            }
        }
        state.apply_staged_correction(&mut tape)?;

        state.current_gear = gear;
        state
            .input
            .lock()
            .expect("input lock")
            .set_frontier(gear.saturating_sub(1));
        state.write_gear_start(&mut tape)?;
        if let Ok(snapshot) = GitCliCheckpointService::new().capture_checkpoint(&options.repo) {
            state.last_checkpoint_id = Some(snapshot.checkpoint_id);
        }

        let completion = state.stream_gear(&mut engine, tty).await?;
        let mut belief = state
            .parser_belief
            .take()
            .unwrap_or_else(|| StreamBeliefParser::parse_missing_belief(&completion.text));
        let tool = state.parser_tool.take();
        state.messages.push(ChatMessage {
            role: "assistant".to_owned(),
            content: completion.text.clone(),
        });

        if belief.parse_error.is_none() {
            state.beliefs_parsed += 1;
        } else {
            state.beliefs_failed += 1;
        }
        state.last_belief = Some(belief.clone());
        state.write_belief(&mut tape, &belief)?;

        if let Some(hit) = state.fire_belief_policies(&belief, &mut tape)? {
            if state.pause("policy", &hit.reason, Some(&belief), "", &mut tape)? == ResumeAction::Quit
            {
                break 'gears;
            }
        }

        if completion.text.contains("```done") {
            break 'gears;
        }

        let tool = match tool {
            Some(tool) if tool.name == "shell" => tool,
            Some(tool) => {
                belief.parse_error = Some(format!("unsupported tool {}", tool.name));
                state.write_belief(&mut tape, &belief)?;
                break 'gears;
            }
            None => break 'gears,
        };

        let command = extract_shell_command(&tool.args);
        if let Some(hit) = state.big_deletion.inspect_command(&tool.raw) {
            state.record_policy_hit(&hit, &mut tape)?;
            if state.pause("policy", &hit.reason, Some(&belief), &command, &mut tape)?
                == ResumeAction::Quit
            {
                break 'gears;
            }
        }

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
            if state.pause("policy", &hit.reason, Some(&belief), &command, &mut tape)?
                == ResumeAction::Quit
            {
                break 'gears;
            }
        }

        let gear_view = GearView {
            gear,
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

fn validate_repo(repo: &Path) -> Result<(), LoopError> {
    if !repo.exists() {
        return Err(LoopError::Validation(format!(
            "error: --repo path does not exist: {}",
            repo.display()
        )));
    }
    if !repo.is_dir() {
        return Err(LoopError::Validation(format!(
            "error: --repo is not a directory: {}",
            repo.display()
        )));
    }
    if !is_git_repo(repo) {
        return Err(LoopError::Validation(format!(
            "error: --repo is not a git repository: {}",
            repo.display()
        )));
    }
    if has_dirty_worktree(repo) {
        eprintln!(
            "{}",
            format!("warning: --repo has uncommitted changes: {}", repo.display()).dimmed()
        );
    }
    Ok(())
}

fn is_git_repo(repo: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn has_dirty_worktree(repo: &Path) -> bool {
    std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

fn validate_tape_path(path: &Path) -> Result<(), LoopError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if !parent.exists() {
        return Err(LoopError::Validation(format!(
            "error: tape directory does not exist: {}",
            parent.display()
        )));
    }
    let readonly = std::fs::metadata(parent)
        .map(|meta| meta.permissions().readonly())
        .unwrap_or(false);
    if readonly {
        return Err(LoopError::Validation(format!(
            "error: tape directory is not writable: {}",
            parent.display()
        )));
    }
    Ok(())
}

fn wait_for_resume<F>(input: &Arc<Mutex<InputState>>, mut on_correction: F) -> ResumeAction
where
    F: FnMut(String),
{
    enum WaitStep {
        Quit,
        Resume,
        Correct(String),
    }

    loop {
        let step = {
            let mut guard = input.lock().expect("input lock");
            if guard.quit {
                Some(WaitStep::Quit)
            } else if let Some(text) = guard.staged_correction.take() {
                Some(WaitStep::Correct(text))
            } else if guard.approved {
                guard.approved = false;
                Some(WaitStep::Resume)
            } else {
                None
            }
        };
        match step {
            Some(WaitStep::Quit) => return ResumeAction::Quit,
            Some(WaitStep::Resume) => return ResumeAction::Resume,
            Some(WaitStep::Correct(text)) => on_correction(text),
            None => std::thread::sleep(WAIT_POLL),
        }
    }
}

struct InteractiveSession {
    cleanup: Arc<CleanupHandle>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    interactive: bool,
}

impl InteractiveSession {
    fn start(tty: bool, input: Arc<Mutex<InputState>>) -> Self {
        let control: Arc<dyn TerminalControl> = Arc::new(CrosstermControl);
        let cleanup = Arc::new(CleanupHandle::new(control.clone()));
        let stop = Arc::new(AtomicBool::new(false));

        if !tty {
            eprintln!("{}", "turn: stdout is not a tty; running non-interactive".dimmed());
            return Self::inactive(cleanup, stop);
        }
        if let Err(error) = setup_terminal(control.as_ref()) {
            eprintln!(
                "{}",
                format!("turn: input disabled ({error}); running non-interactive").dimmed()
            );
            return Self::inactive(cleanup, stop);
        }

        install_panic_cleanup(cleanup.clone());
        let handle = spawn_input_thread(input, stop.clone());
        Self {
            cleanup,
            stop,
            handle: Some(handle),
            interactive: true,
        }
    }

    fn inactive(cleanup: Arc<CleanupHandle>, stop: Arc<AtomicBool>) -> Self {
        Self {
            cleanup,
            stop,
            handle: None,
            interactive: false,
        }
    }
}

impl Drop for InteractiveSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        if self.interactive {
            self.cleanup.cleanup();
        }
    }
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
    last_belief: Option<BeliefRecord>,
    parser_belief: Option<BeliefRecord>,
    parser_tool: Option<ToolCallRecord>,
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
            last_belief: None,
            parser_belief: None,
            parser_tool: None,
        }
    }

    async fn stream_gear(
        &mut self,
        engine: &mut Engine,
        tty: bool,
    ) -> Result<crate::turnrt::engine::EngineCompletion, LoopError> {
        let mut parser = StreamBeliefParser::default();
        let messages = self.messages.clone();
        let mut printer = StreamPrinter::new(stdout(), tty);
        let mut belief_shown = false;
        let mut last_status = String::new();
        let input = self.input.clone();
        let completion = engine
            .stream_completion(&self.options.engine, &messages, |chunk| {
                let _ = printer.delta(&chunk.text_delta);
                parser.feed(&chunk.text_delta, chunk.token_logprob);
                if !belief_shown {
                    if let Some(belief) = parser.belief() {
                        let _ = printer.belief(&belief);
                        belief_shown = true;
                    }
                }
                let live = input.lock().expect("input lock").live_input.clone();
                if !live.is_empty() && live != last_status {
                    let _ = printer.status(&live);
                    last_status = live;
                }
            })
            .await?;
        self.parser_belief = parser.belief();
        self.parser_tool = parser.tool();
        Ok(completion)
    }

    fn wants_quit(&self) -> bool {
        self.input.lock().expect("input lock").quit
    }

    fn take_pause_request(&mut self) -> Option<PauseTrigger> {
        self.input.lock().expect("input lock").pause_requested.take()
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

    fn write_correction(&mut self, tape: &mut Tape, text: &str) -> Result<(), TapeError> {
        tape.append(&self.next_event(
            EventKind::Correction,
            json!({
                "text": text,
                "gear": self.current_gear,
            }),
        ))
    }

    fn write_run_end(&mut self, tape: &mut Tape) -> Result<(), TapeError> {
        let corrections = self
            .messages
            .iter()
            .filter(|message| message.content.starts_with("[HUMAN CORRECTION]"))
            .count();
        tape.append(&self.next_event(
            EventKind::RunEnd,
            json!({
                "gears": self.current_gear,
                "beliefs_parsed": self.beliefs_parsed,
                "beliefs_failed": self.beliefs_failed,
                "policy_hits": self.policy_hits,
                "corrections": corrections,
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

    fn apply_staged_correction(&mut self, tape: &mut Tape) -> Result<(), TapeError> {
        let staged = self
            .input
            .lock()
            .expect("input lock")
            .staged_correction
            .take();
        let Some(text) = staged else {
            return Ok(());
        };
        self.write_correction(tape, &text)?;
        self.messages.push(ChatMessage {
            role: "user".to_owned(),
            content: format!("[HUMAN CORRECTION]\n{text}"),
        });
        Ok(())
    }

    fn pause(
        &mut self,
        trigger: &str,
        reason: &str,
        belief: Option<&BeliefRecord>,
        command: &str,
        tape: &mut Tape,
    ) -> Result<ResumeAction, TapeError> {
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
        let _ = render_pause_card(&mut stdout(), reason, belief, command);

        self.input.lock().expect("input lock").set_paused(true);
        let mut corrections = Vec::new();
        let action = wait_for_resume(&self.input, |text| corrections.push(text));
        self.input.lock().expect("input lock").set_paused(false);

        for text in corrections {
            self.write_correction(tape, &text)?;
            self.messages.push(ChatMessage {
                role: "user".to_owned(),
                content: format!("[HUMAN CORRECTION]\n{text}"),
            });
        }
        Ok(action)
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

#[derive(Debug)]
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
        .spawn()
        .map_err(|source| LoopError::Io {
            context: format!("spawning shell in {}", repo.display()),
            source,
        })?;
    let output = timeout(TOOL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| LoopError::ToolTimeout(TOOL_TIMEOUT))?
        .map_err(|source| LoopError::Io {
            context: format!("waiting on shell in {}", repo.display()),
            source,
        })?;
    let stdout = String::from_utf8(output.stdout).map_err(|_| LoopError::ToolDecode)?;
    let stderr = String::from_utf8(output.stderr).map_err(|_| LoopError::ToolDecode)?;
    Ok(ShellResult {
        stdout,
        stderr,
        exit_code: output.status.code(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentmint-loop-{}-{}.jsonl",
            name,
            std::process::id()
        ))
    }

    fn test_state(tape_path: PathBuf) -> LoopState {
        let options = RunOptions {
            repo: PathBuf::from("."),
            task: "make tests pass".to_owned(),
            tape_path,
            max_gears: 1,
            engine: EngineConfig {
                model: "test-model".to_owned(),
                base_url: "http://localhost:1234".to_owned(),
                temperature: Some(0.2),
                top_p: Some(0.95),
                seed: None,
            },
        };
        let snapshot = ModelSnapshot {
            id: Some("test-model".to_owned()),
            owned_by: None,
            raw: serde_json::Value::Null,
        };
        LoopState::new(options, snapshot, shared_input_state())
    }

    fn belief(claim: &str) -> BeliefRecord {
        BeliefRecord {
            claim: claim.to_owned(),
            confidence: 0.5,
            source: "tests".to_owned(),
            said: 0.5,
            logit: Some(0.4),
            raw: String::new(),
            parse_error: None,
        }
    }

    #[test]
    fn policy_pause_resume() {
        let path = temp_path("pause-resume");
        let _ = std::fs::remove_file(&path);
        let mut state = test_state(path.clone());
        state.current_gear = 3;
        let mut tape = Tape::create(&path).expect("tape");

        let driver = state.input.clone();
        let handle = std::thread::spawn(move || {
            loop {
                if driver.lock().expect("lock").is_paused() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            driver.lock().expect("lock").staged_correction = Some("use --release".to_owned());
            loop {
                if driver.lock().expect("lock").staged_correction.is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            driver.lock().expect("lock").approved = true;
        });

        let action = state
            .pause(
                "policy",
                "two consecutive non-zero exits",
                Some(&belief("still broken")),
                "cargo test",
                &mut tape,
            )
            .expect("pause");
        handle.join().expect("join");

        assert_eq!(action, ResumeAction::Resume);
        assert!(!state.input.lock().expect("lock").is_paused());

        let events = Tape::load(&path).expect("load");
        let kinds = events.iter().map(|event| event.kind).collect::<Vec<_>>();
        assert_eq!(kinds, vec![EventKind::Pause, EventKind::Correction]);
        assert!(state.messages.iter().any(|message| {
            message.content.starts_with("[HUMAN CORRECTION]")
                && message.content.contains("use --release")
        }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn repo_validation() {
        let missing = Path::new("/nonexistent-turn-repo-xyz");
        assert!(matches!(
            validate_repo(missing),
            Err(LoopError::Validation(message)) if message.contains("does not exist")
        ));

        let non_git = std::env::temp_dir().join(format!("turn-nongit-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&non_git);
        assert!(matches!(
            validate_repo(&non_git),
            Err(LoopError::Validation(message)) if message.contains("not a git repository")
        ));
        let _ = std::fs::remove_dir_all(&non_git);

        assert!(validate_repo(Path::new(".")).is_ok());
    }

    #[tokio::test]
    async fn io_context() {
        let missing = Path::new("/definitely/not/a/real/turn-dir");
        let error = run_shell(missing, "echo hi").await.expect_err("spawn fails");
        let message = error.to_string();
        assert!(message.contains("spawning"), "message was: {message}");
        assert!(
            message.contains("/definitely/not/a/real/turn-dir"),
            "message was: {message}"
        );
    }
}
