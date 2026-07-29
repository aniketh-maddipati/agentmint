//! Fork-from-here correction execution across fixture and live Codex modes.
//! Used by: intervene routes and live correction tests.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use chrono::Utc;
use serde_json::json;

use crate::aerf::adapters::codex_app_server::{
    installed_codex_version, CodexAppServerAdapter, CodexInitializeHandshake,
    CodexInstalledVersion, CodexRpcCall, CodexTranscript, LifecyclePhase, NormalizedCodexEvent,
    StdioJsonlTransport,
};
use crate::aerf::intervention::{
    apply_recorded_demo_correction, generate_live_correction_preview,
    generate_recorded_demo_correction_preview, recorded_verification_command, Attempt,
    AttemptOutcome, CorrectionComparison, CorrectionExecution, CorrectionExecutionMode,
    CorrectionFork, CorrectionPacket, Episode, Evidence, InputEnvelope, RawEvent, RawEventOutcome,
    RawEventParseState, RawEventSource, Run, TerminalHandoff, TokenUsage, TokenUsageComparison,
    Verification, VerificationComparison, VerificationCounts, VerificationVerdict,
    RECORDED_DEMO_SCHEMA_EPISODE_ID,
};
use crate::aerf::{AerfError, AerfResult};
use crate::checkpoint::{
    CheckpointCapture, CheckpointError, CheckpointService, GitCliCheckpointService, GitRunner,
    MaterializedCheckpoint, SystemGitRunner,
};

pub trait CorrectionExecutor {
    fn preview(
        &self,
        run: &Run,
        episode_id: &str,
    ) -> Option<crate::aerf::intervention::CorrectionPreview>;
    fn confirm(&self, run: &Run, episode_id: &str) -> ForkFromHereResult<CorrectionFork>;
}

pub type ForkFromHereResult<T> = Result<T, ForkFromHereError>;

#[derive(Debug, thiserror::Error)]
pub enum ForkFromHereError {
    #[error("unsupported correction branch source: {0}")]
    UnsupportedEpisode(String),
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
    #[error(transparent)]
    Aerf(#[from] AerfError),
    #[error("codex app server command is missing a stdout pipe")]
    MissingStdoutPipe,
    #[error("codex app server command is missing a stdin pipe")]
    MissingStdinPipe,
    #[error("codex app server returned no thread identifier")]
    MissingThreadId,
    #[error("codex app server did not emit token usage")]
    MissingTokenUsage,
    #[error("verification command is empty")]
    EmptyVerificationCommand,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct OfflineFixtureExecutor;

impl CorrectionExecutor for OfflineFixtureExecutor {
    fn preview(
        &self,
        run: &Run,
        episode_id: &str,
    ) -> Option<crate::aerf::intervention::CorrectionPreview> {
        generate_recorded_demo_correction_preview(run, episode_id)
    }

    fn confirm(&self, run: &Run, episode_id: &str) -> ForkFromHereResult<CorrectionFork> {
        apply_recorded_demo_correction(run, episode_id)
            .ok_or_else(|| ForkFromHereError::UnsupportedEpisode(episode_id.to_owned()))
    }
}

pub trait CodexAttemptStreamer {
    fn stream_corrected_attempt(
        &self,
        worktree_path: &Path,
        checkpoint: &CheckpointCapture,
        packet: &CorrectionPacket,
        episode_id: &str,
    ) -> ForkFromHereResult<CodexAttemptStream>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAttemptStream {
    pub codex_version: String,
    pub thread_id: String,
    pub transcript: CodexTranscript,
}

pub struct CodexAppServerStreamer {
    installed_version: CodexInstalledVersion,
    program: String,
    args: Vec<String>,
}

impl CodexAppServerStreamer {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> AerfResult<Self> {
        Ok(Self {
            installed_version: installed_codex_version()?,
            program: program.into(),
            args,
        })
    }
}

impl CodexAttemptStreamer for CodexAppServerStreamer {
    fn stream_corrected_attempt(
        &self,
        worktree_path: &Path,
        checkpoint: &CheckpointCapture,
        packet: &CorrectionPacket,
        episode_id: &str,
    ) -> ForkFromHereResult<CodexAttemptStream> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .current_dir(worktree_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ForkFromHereError::MissingStdoutPipe)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(ForkFromHereError::MissingStdinPipe)?;
        let transport = StdioJsonlTransport::new(BufReader::new(stdout), stdin);
        let mut adapter = CodexAppServerAdapter::new(transport, self.installed_version.clone());
        let handshake = adapter.initialize()?;
        let thread_call = adapter.call(
            "threads.create",
            json!({
                "cwd": worktree_path.display().to_string(),
                "title": format!("AgentMint correction from {episode_id}"),
                "metadata": {
                    "checkpointId": checkpoint.checkpoint_id,
                    "packetId": packet.packet_id,
                    "branchFromEpisodeId": episode_id,
                }
            }),
        )?;
        let thread_id = extract_thread_id(&thread_call)?.to_owned();
        let turn_call = adapter.call(
            "turns.create",
            json!({
                "threadId": thread_id,
                "cwd": worktree_path.display().to_string(),
                "input": packet.request.input,
                "cachedInput": packet.request.cached_input,
                "packet": packet,
            }),
        )?;
        let transcript = transcript_from_calls(
            &handshake,
            &self.installed_version.version,
            &[thread_call, turn_call],
        );
        let _ = child.kill();
        let _ = child.wait();

        Ok(CodexAttemptStream {
            codex_version: self.installed_version.version.clone(),
            thread_id,
            transcript,
        })
    }
}

pub trait ProcessRunner {
    fn run(&self, cwd: &Path, command: &[String]) -> ForkFromHereResult<ProcessOutput>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, cwd: &Path, command: &[String]) -> ForkFromHereResult<ProcessOutput> {
        let Some(program) = command.first() else {
            return Err(ForkFromHereError::EmptyVerificationCommand);
        };
        let output = Command::new(program)
            .args(&command[1..])
            .current_dir(cwd)
            .output()?;
        Ok(ProcessOutput {
            exit_code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub trait Clock {
    fn now_rfc3339(&self) -> String;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_rfc3339(&self) -> String {
        Utc::now().to_rfc3339()
    }
}

pub struct LiveCodexExecutor {
    repo_root: PathBuf,
    checkpoint_service: Arc<dyn CheckpointService + Send + Sync>,
    codex_streamer: Arc<dyn CodexAttemptStreamer + Send + Sync>,
    git_runner: Arc<dyn GitRunner + Send + Sync>,
    process_runner: Arc<dyn ProcessRunner + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
    branch_prefix: String,
}

impl LiveCodexExecutor {
    pub fn new(
        repo_root: PathBuf,
        checkpoint_service: Arc<dyn CheckpointService + Send + Sync>,
        codex_streamer: Arc<dyn CodexAttemptStreamer + Send + Sync>,
        git_runner: Arc<dyn GitRunner + Send + Sync>,
        process_runner: Arc<dyn ProcessRunner + Send + Sync>,
        clock: Arc<dyn Clock + Send + Sync>,
        branch_prefix: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            checkpoint_service,
            codex_streamer,
            git_runner,
            process_runner,
            clock,
            branch_prefix: branch_prefix.into(),
        }
    }

    pub fn system(repo_root: PathBuf) -> AerfResult<Self> {
        Ok(Self::new(
            repo_root,
            Arc::new(GitCliCheckpointService::new()),
            Arc::new(CodexAppServerStreamer::new(
                "codex",
                vec!["app-server".to_owned(), "--stdio-jsonl".to_owned()],
            )?),
            Arc::new(SystemGitRunner),
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            "agentmint",
        ))
    }
}

impl CorrectionExecutor for LiveCodexExecutor {
    fn preview(
        &self,
        run: &Run,
        episode_id: &str,
    ) -> Option<crate::aerf::intervention::CorrectionPreview> {
        generate_live_correction_preview(run, episode_id)
    }

    fn confirm(&self, run: &Run, episode_id: &str) -> ForkFromHereResult<CorrectionFork> {
        if episode_id != RECORDED_DEMO_SCHEMA_EPISODE_ID {
            return Err(ForkFromHereError::UnsupportedEpisode(episode_id.to_owned()));
        }

        let preview = generate_live_correction_preview(run, episode_id)
            .ok_or_else(|| ForkFromHereError::UnsupportedEpisode(episode_id.to_owned()))?;
        let checkpoint = self
            .checkpoint_service
            .capture_checkpoint(&self.repo_root)?;
        let materialized = self
            .checkpoint_service
            .materialize_checkpoint(&self.repo_root, &checkpoint)?;
        let codex_stream = self.codex_streamer.stream_corrected_attempt(
            &materialized.worktree_path,
            &checkpoint,
            &preview.packet,
            episode_id,
        )?;
        let branch_name = format!(
            "{}/{}",
            self.branch_prefix, preview.comparison.corrected_attempt.attempt_id
        );
        let switch_args = ["switch", "-c", branch_name.as_str()];
        self.git_runner
            .run_no_output(&materialized.worktree_path, &switch_args)?;
        let verification_command = recorded_verification_command(run);
        let verification_output = self
            .process_runner
            .run(&materialized.worktree_path, &verification_command)?;
        let actual_patch = String::from_utf8_lossy(&self.git_runner.run_bytes(
            &materialized.worktree_path,
            &["diff", "--binary", "HEAD", "--", "."],
        )?)
        .to_string();
        let actual_changed_paths = nul_split(self.git_runner.run_bytes(
            &materialized.worktree_path,
            &["diff", "--name-only", "-z", "HEAD", "--", "."],
        )?);
        let captured_token_usage = captured_token_usage(&codex_stream.transcript.events)
            .ok_or(ForkFromHereError::MissingTokenUsage)?;
        let now = self.clock.now_rfc3339();
        let corrected_attempt = build_actual_attempt(
            run,
            &preview.packet.request,
            &captured_token_usage,
            &codex_stream.transcript.events,
            verification_output.exit_code,
            &now,
        );
        let comparison = build_actual_comparison(
            run,
            &preview.comparison,
            corrected_attempt.clone(),
            &captured_token_usage,
            verification_output.exit_code,
            &verification_command,
        );
        let execution = CorrectionExecution {
            mode: CorrectionExecutionMode::LiveCodex,
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            worktree_path: materialized.worktree_path.display().to_string(),
            branch_name: branch_name.clone(),
            detached_head: false,
            codex_version: codex_stream.codex_version.clone(),
            thread_id: Some(codex_stream.thread_id.clone()),
            attempt_id: corrected_attempt.attempt_id.clone(),
            verification_command: verification_command.clone(),
            verification_exit_code: verification_output.exit_code,
            actual_patch,
            actual_changed_paths,
            captured_token_usage: captured_token_usage.clone(),
            handoff: build_terminal_handoff(
                &materialized.worktree_path,
                &branch_name,
                false,
                &checkpoint.checkpoint_id,
                &corrected_attempt.attempt_id,
                &verification_command,
            ),
            demo_limitations: live_demo_limitations(),
            codex_transcript: codex_stream.transcript.clone(),
        };
        let appended_episode = build_live_episode(
            &corrected_attempt,
            verification_output.exit_code,
            &branch_name,
            &materialized,
            &codex_stream,
        );
        let appended_episode_id = appended_episode.episode_id.clone();
        let appended_event = build_live_raw_event(
            &preview.packet,
            &corrected_attempt,
            &execution,
            &verification_output,
            &now,
        );
        let forked_run = build_live_forked_run(LiveForkedRunInputs {
            run,
            packet: &preview.packet,
            corrected_attempt: corrected_attempt.clone(),
            appended_episode,
            appended_event,
            captured_token_usage: captured_token_usage.clone(),
            now: &now,
            verification_exit_code: verification_output.exit_code,
        });

        Ok(CorrectionFork {
            branch_from_episode_id: episode_id.to_owned(),
            packet: preview.packet,
            comparison,
            forked_run,
            appended_episode_id,
            execution: Some(execution),
        })
    }
}

fn build_actual_attempt(
    run: &Run,
    request: &InputEnvelope,
    captured_token_usage: &TokenUsage,
    events: &[NormalizedCodexEvent],
    verification_exit_code: i32,
    now: &str,
) -> Attempt {
    let _ = captured_token_usage;
    let success = verification_exit_code == 0 && completion_succeeded(events);
    Attempt {
        attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
        endpoint: run
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == "attempt-v2")
            .map(|attempt| attempt.endpoint.clone())
            .unwrap_or_else(|| "/v2/responses".to_owned()),
        started_at: now.to_owned(),
        outcome: if success {
            AttemptOutcome::Succeeded
        } else {
            AttemptOutcome::Failed
        },
        request: request.clone(),
        status_code: Some(if success { 200 } else { 500 }),
        retry_after_ms: None,
    }
}

fn build_actual_comparison(
    run: &Run,
    preview: &CorrectionComparison,
    corrected_attempt: Attempt,
    captured_token_usage: &TokenUsage,
    verification_exit_code: i32,
    verification_command: &[String],
) -> CorrectionComparison {
    let corrected_verdict = if verification_exit_code == 0 {
        VerificationVerdict::Confirmed
    } else {
        VerificationVerdict::Rejected
    };

    CorrectionComparison {
        original_attempt_id: preview.original_attempt_id.clone(),
        corrected_attempt,
        patch: preview.patch.clone(),
        verification: VerificationComparison {
            original_verdict: run.verification.verdict.clone(),
            corrected_verdict,
            original_counts: verification_counts(&run.verification),
            corrected_counts: VerificationCounts {
                observed_facts: run.verification.observed_fact_ids.len() + 1,
                agent_statements: run.verification.agent_statement_ids.len() + 1,
                derived_inferences: run.verification.derived_inference_ids.len() + 1,
                human_contexts: run.verification.human_context_ids.len() + 1,
            },
            note: format!(
                "Recorded verification command `{}` exited with status {}.",
                verification_command.join(" "),
                verification_exit_code
            ),
        },
        token_usage: TokenUsageComparison {
            original: run.token_usage.clone(),
            corrected: captured_token_usage.clone(),
            delta_input_tokens: captured_token_usage.input_tokens as i64
                - run.token_usage.input_tokens as i64,
            delta_cached_input_tokens: captured_token_usage.cached_input_tokens as i64
                - run.token_usage.cached_input_tokens as i64,
            delta_output_tokens: captured_token_usage.output_tokens as i64
                - run.token_usage.output_tokens as i64,
            delta_total_tokens: captured_token_usage.total_tokens as i64
                - run.token_usage.total_tokens as i64,
            note: "Captured Codex token usage from the live correction branch.".to_owned(),
        },
    }
}

fn build_live_episode(
    attempt: &Attempt,
    verification_exit_code: i32,
    branch_name: &str,
    materialized: &MaterializedCheckpoint,
    codex_stream: &CodexAttemptStream,
) -> Episode {
    let agent_summary =
        last_agent_statement(&codex_stream.transcript.events).unwrap_or_else(|| {
            "Codex streamed a corrected attempt in the isolated worktree.".to_owned()
        });
    let observed_id = "episode-codex-corrected-attempt-observed".to_owned();
    let agent_id = "episode-codex-corrected-attempt-agent".to_owned();
    let inference_id = "episode-codex-corrected-attempt-inference".to_owned();
    let human_id = "episode-codex-corrected-attempt-human".to_owned();

    Episode {
        episode_id: "episode-codex-corrected-attempt".to_owned(),
        title: "codex corrected attempt".to_owned(),
        raw_event_ids: vec!["codex-corrected-attempt-1".to_owned()],
        evidence: vec![
            Evidence::ObservedFact {
                evidence_id: observed_id.clone(),
                summary: format!(
                    "Created isolated worktree {} and attached branch {} after running the recorded verification command with exit code {}.",
                    materialized.worktree_path.display(),
                    branch_name,
                    verification_exit_code
                ),
                source_event_id: "codex-corrected-attempt-1".to_owned(),
            },
            Evidence::AgentStatement {
                evidence_id: agent_id.clone(),
                summary: agent_summary,
                source_event_id: "codex-corrected-attempt-1".to_owned(),
            },
            Evidence::DerivedInference {
                evidence_id: inference_id,
                summary: format!(
                    "The live Codex branch for {} preserved the schema-aligned /v2 retry path in an AgentMint-owned worktree.",
                    attempt.attempt_id
                ),
                derived_from: vec![observed_id, agent_id],
            },
            Evidence::HumanContext {
                evidence_id: human_id,
                summary: "The fixture path remains the offline deterministic mode; this branch is the live worktree-backed execution path.".to_owned(),
                provided_by: "agentmint-intervene".to_owned(),
            },
        ],
        checkpoints: Vec::new(),
    }
}

fn build_live_raw_event(
    packet: &CorrectionPacket,
    attempt: &Attempt,
    execution: &CorrectionExecution,
    verification_output: &ProcessOutput,
    now: &str,
) -> RawEvent {
    RawEvent {
        event_id: "codex-corrected-attempt-1".to_owned(),
        recorded_at: now.to_owned(),
        source: RawEventSource::Recorder,
        endpoint: attempt.endpoint.clone(),
        request: packet.request.clone(),
        tool_name: Some("fork_correction_live".to_owned()),
        file_path: execution.actual_changed_paths.first().cloned(),
        summary: Some(
            "Live Codex corrected attempt completed in an isolated AgentMint worktree.".to_owned(),
        ),
        outcome: Some(if verification_output.exit_code == 0 {
            RawEventOutcome::Succeeded
        } else {
            RawEventOutcome::Failed
        }),
        parse_state: RawEventParseState::Complete,
        status_code: attempt.status_code,
        retry_after_ms: None,
        body: Some(json!({
            "thread_id": execution.thread_id,
            "branch_name": execution.branch_name,
            "checkpoint_id": execution.checkpoint_id,
            "verification_command": execution.verification_command,
            "verification_exit_code": verification_output.exit_code,
            "actual_changed_paths": execution.actual_changed_paths,
        })),
    }
}

struct LiveForkedRunInputs<'a> {
    run: &'a Run,
    packet: &'a CorrectionPacket,
    corrected_attempt: Attempt,
    appended_episode: Episode,
    appended_event: RawEvent,
    captured_token_usage: TokenUsage,
    now: &'a str,
    verification_exit_code: i32,
}

fn build_live_forked_run(inputs: LiveForkedRunInputs) -> Run {
    let LiveForkedRunInputs {
        run,
        packet,
        corrected_attempt,
        appended_episode,
        appended_event,
        captured_token_usage,
        now,
        verification_exit_code,
    } = inputs;
    let observed_id = "episode-codex-corrected-attempt-observed".to_owned();
    let agent_id = "episode-codex-corrected-attempt-agent".to_owned();
    let inference_id = "episode-codex-corrected-attempt-inference".to_owned();
    let human_id = "episode-codex-corrected-attempt-human".to_owned();
    let mut forked_run = run.clone();
    forked_run.run_id = format!("{}-codex-fork", run.run_id);
    forked_run.session_id = format!("{}-codex-fork", run.session_id);
    forked_run.raw_events.push(appended_event);
    forked_run.episodes.push(appended_episode);
    forked_run.correction_packets.push(packet.clone());
    forked_run.attempts.push(corrected_attempt);
    forked_run.verification = Verification {
        verified_at: now.to_owned(),
        verdict: if verification_exit_code == 0 {
            VerificationVerdict::Confirmed
        } else {
            VerificationVerdict::Rejected
        },
        observed_fact_ids: extend_ids(&run.verification.observed_fact_ids, observed_id),
        agent_statement_ids: extend_ids(&run.verification.agent_statement_ids, agent_id),
        derived_inference_ids: extend_ids(&run.verification.derived_inference_ids, inference_id),
        human_context_ids: extend_ids(&run.verification.human_context_ids, human_id),
    };
    forked_run.token_usage = captured_token_usage;
    forked_run
}

fn extend_ids(existing: &[String], next: String) -> Vec<String> {
    let mut ids = existing.to_vec();
    ids.push(next);
    ids
}

fn transcript_from_calls(
    handshake: &CodexInitializeHandshake,
    codex_version: &str,
    calls: &[CodexRpcCall],
) -> CodexTranscript {
    let mut events = Vec::new();
    for call in calls {
        events.extend(call.events.clone());
    }
    CodexTranscript {
        codex_version: codex_version.to_owned(),
        handshake: handshake.clone(),
        events,
    }
}

fn extract_thread_id(call: &CodexRpcCall) -> ForkFromHereResult<&str> {
    call.response
        .pointer("/result/threadId")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            call.response
                .pointer("/result/thread/id")
                .and_then(serde_json::Value::as_str)
        })
        .ok_or(ForkFromHereError::MissingThreadId)
}

fn captured_token_usage(events: &[NormalizedCodexEvent]) -> Option<TokenUsage> {
    events.iter().rev().find_map(|event| match event {
        NormalizedCodexEvent::TokenUsage(usage) => Some(TokenUsage {
            input_tokens: usage.input_tokens?,
            cached_input_tokens: usage.cached_input_tokens?,
            output_tokens: usage.output_tokens?,
            total_tokens: usage.total_tokens?,
        }),
        _ => None,
    })
}

fn completion_succeeded(events: &[NormalizedCodexEvent]) -> bool {
    events.iter().rev().any(|event| match event {
        NormalizedCodexEvent::Completion(completion) => {
            completion.status.as_deref() == Some("completed")
                || completion.finish_reason.as_deref() == Some("stop")
        }
        NormalizedCodexEvent::TurnLifecycle(turn) => turn.phase == LifecyclePhase::Completed,
        _ => false,
    })
}

fn last_agent_statement(events: &[NormalizedCodexEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| match event {
        NormalizedCodexEvent::AgentMessage(message) => message.text.clone(),
        _ => None,
    })
}

fn verification_counts(verification: &Verification) -> VerificationCounts {
    VerificationCounts {
        observed_facts: verification.observed_fact_ids.len(),
        agent_statements: verification.agent_statement_ids.len(),
        derived_inferences: verification.derived_inference_ids.len(),
        human_contexts: verification.human_context_ids.len(),
    }
}

fn build_terminal_handoff(
    worktree_path: &Path,
    branch_name: &str,
    detached_head: bool,
    checkpoint_id: &str,
    attempt_id: &str,
    verification_command: &[String],
) -> TerminalHandoff {
    let branch_state = if detached_head {
        "detached HEAD".to_owned()
    } else {
        format!("branch {branch_name}")
    };
    let command = verification_command.join(" ");
    let copy_text = format!(
        "cd {}\n# branch state: {}\n# checkpoint: {}\n# attempt: {}\n{}",
        worktree_path.display(),
        branch_state,
        checkpoint_id,
        attempt_id,
        command
    );

    TerminalHandoff {
        directory: worktree_path.display().to_string(),
        branch_state,
        checkpoint_id: checkpoint_id.to_owned(),
        attempt_id: attempt_id.to_owned(),
        verification_command: verification_command.to_vec(),
        copy_text,
    }
}

fn live_demo_limitations() -> Vec<String> {
    vec![
        "The live Codex App Server method names are adapter assumptions and may differ from a future production protocol.".to_owned(),
        "The verification command is inferred from the recorded fixture and currently replays only that recorded test target.".to_owned(),
        "The browser UI shows the completed handoff after the run returns; it does not live-stream token usage or command output incrementally.".to_owned(),
        "The corrected run remains an in-browser forked copy of the recorded run and is not persisted back to the fixture file or the main server state.".to_owned(),
    ]
}

fn nul_split(bytes: Vec<u8>) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect()
}

pub fn default_executor_for_run_path(
    run_path: &Path,
    repo_root: &Path,
) -> AerfResult<Arc<dyn CorrectionExecutor + Send + Sync>> {
    if is_fixture_path(run_path) {
        return Ok(Arc::new(OfflineFixtureExecutor));
    }
    Ok(Arc::new(LiveCodexExecutor::system(
        repo_root.to_path_buf(),
    )?))
}

fn is_fixture_path(path: &Path) -> bool {
    path.ends_with(Path::new(
        "tests/fixtures/interventions/retry-after-v1-v2.json",
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::{
        captured_token_usage, default_executor_for_run_path, last_agent_statement, Clock,
        CodexAttemptStream, CodexAttemptStreamer, CorrectionExecutor, ForkFromHereError,
        LiveCodexExecutor, ProcessOutput, ProcessRunner,
    };
    use crate::aerf::adapters::codex_app_server::{
        CodexInitializeHandshake, CodexTranscript, LifecyclePhase, NormalizedAgentMessage,
        NormalizedCodexEvent, NormalizedCompletion, NormalizedLifecycle, NormalizedTokenUsage,
    };
    use crate::aerf::intervention::Run;
    use crate::checkpoint::{
        BranchState, CheckpointCapture, CheckpointService, GitRunner, MaterializedCheckpoint,
    };

    struct MockCheckpointService {
        capture: CheckpointCapture,
        materialized: MaterializedCheckpoint,
    }

    impl CheckpointService for MockCheckpointService {
        fn capture_checkpoint(
            &self,
            _repo_root: &Path,
        ) -> Result<CheckpointCapture, crate::checkpoint::CheckpointError> {
            Ok(self.capture.clone())
        }

        fn materialize_checkpoint(
            &self,
            _repo_root: &Path,
            _checkpoint: &CheckpointCapture,
        ) -> Result<MaterializedCheckpoint, crate::checkpoint::CheckpointError> {
            Ok(self.materialized.clone())
        }
    }

    struct MockStreamer {
        stream: CodexAttemptStream,
    }

    impl CodexAttemptStreamer for MockStreamer {
        fn stream_corrected_attempt(
            &self,
            _worktree_path: &Path,
            _checkpoint: &CheckpointCapture,
            _packet: &crate::aerf::intervention::CorrectionPacket,
            _episode_id: &str,
        ) -> Result<CodexAttemptStream, ForkFromHereError> {
            Ok(self.stream.clone())
        }
    }

    struct MockGitRunner {
        outputs: HashMap<String, Vec<u8>>,
        commands: Mutex<Vec<String>>,
    }

    impl GitRunner for MockGitRunner {
        fn run(
            &self,
            _repo_root: &Path,
            args: &[&str],
        ) -> Result<String, crate::checkpoint::CheckpointError> {
            self.commands.lock().expect("commands").push(args.join(" "));
            let key = args.join(" ");
            let bytes = self.outputs.get(&key).cloned().unwrap_or_default();
            Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
        }

        fn run_bytes(
            &self,
            _repo_root: &Path,
            args: &[&str],
        ) -> Result<Vec<u8>, crate::checkpoint::CheckpointError> {
            self.commands.lock().expect("commands").push(args.join(" "));
            Ok(self
                .outputs
                .get(&args.join(" "))
                .cloned()
                .unwrap_or_default())
        }

        fn run_optional(
            &self,
            _repo_root: &Path,
            _args: &[&str],
        ) -> Result<Option<String>, crate::checkpoint::CheckpointError> {
            Ok(None)
        }

        fn run_no_output(
            &self,
            _repo_root: &Path,
            args: &[&str],
        ) -> Result<(), crate::checkpoint::CheckpointError> {
            self.commands.lock().expect("commands").push(args.join(" "));
            Ok(())
        }

        fn run_with_stdin(
            &self,
            _repo_root: &Path,
            _args: &[&str],
            _stdin: &[u8],
        ) -> Result<(), crate::checkpoint::CheckpointError> {
            Ok(())
        }
    }

    struct MockProcessRunner;

    impl ProcessRunner for MockProcessRunner {
        fn run(&self, _cwd: &Path, command: &[String]) -> Result<ProcessOutput, ForkFromHereError> {
            Ok(ProcessOutput {
                exit_code: 0,
                stdout: command.join(" "),
                stderr: String::new(),
            })
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn now_rfc3339(&self) -> String {
            "2026-07-28T12:34:56Z".to_owned()
        }
    }

    #[test]
    fn live_executor_builds_branch_worktree_and_usage_comparison() {
        let run = recorded_demo_run();
        let checkpoint = CheckpointCapture {
            checkpoint_id: "checkpoint-abc123".to_owned(),
            head_commit: "abc123".to_owned(),
            branch_state: BranchState {
                branch_name: Some("main".to_owned()),
                detached_head: false,
                upstream_branch: Some("origin/main".to_owned()),
                ahead_count: 0,
                behind_count: 0,
            },
            staged_binary_patch: String::new(),
            unstaged_binary_patch: String::new(),
            untracked_files: Vec::new(),
            changed_paths: vec!["src/schema.rs".to_owned()],
            commands: Vec::new(),
        };
        let materialized = MaterializedCheckpoint {
            worktree_path: Path::new("/tmp/agentmint-worktree").to_path_buf(),
            head_commit: "abc123".to_owned(),
            applied_commands: vec!["git worktree add".to_owned()],
        };
        let transcript = CodexTranscript {
            codex_version: "0.130.0".to_owned(),
            handshake: CodexInitializeHandshake {
                codex_version: "0.130.0".to_owned(),
                initialize_request: json!({"method":"initialize"}),
                initialize_response: json!({"result":{"serverInfo":{"version":"0.130.0"}}}),
                initialized_notification: json!({"method":"initialized"}),
            },
            events: vec![
                NormalizedCodexEvent::TurnLifecycle(NormalizedLifecycle {
                    raw_event_name: "turn.started".to_owned(),
                    phase: LifecyclePhase::Started,
                    thread_id: Some("thread-1".to_owned()),
                    turn_id: Some("turn-1".to_owned()),
                }),
                NormalizedCodexEvent::AgentMessage(NormalizedAgentMessage {
                    raw_event_name: "message.delta".to_owned(),
                    thread_id: Some("thread-1".to_owned()),
                    turn_id: Some("turn-1".to_owned()),
                    item_id: Some("item-1".to_owned()),
                    role: Some("assistant".to_owned()),
                    text: Some("Updated the schema and client to keep /v2 aligned.".to_owned()),
                }),
                NormalizedCodexEvent::Completion(NormalizedCompletion {
                    raw_event_name: "turn.completed".to_owned(),
                    thread_id: Some("thread-1".to_owned()),
                    turn_id: Some("turn-1".to_owned()),
                    item_id: None,
                    status: Some("completed".to_owned()),
                    finish_reason: Some("stop".to_owned()),
                }),
                NormalizedCodexEvent::TokenUsage(NormalizedTokenUsage {
                    raw_event_name: "turn.completed".to_owned(),
                    thread_id: Some("thread-1".to_owned()),
                    turn_id: Some("turn-1".to_owned()),
                    item_id: None,
                    input_tokens: Some(40),
                    cached_input_tokens: Some(20),
                    output_tokens: Some(10),
                    total_tokens: Some(50),
                }),
            ],
        };
        let git_runner = Arc::new(MockGitRunner {
            outputs: HashMap::from([
                (
                    "diff --binary HEAD -- .".to_owned(),
                    b"diff --git a/src/schema.rs b/src/schema.rs\n".to_vec(),
                ),
                (
                    "diff --name-only -z HEAD -- .".to_owned(),
                    b"src/schema.rs\0src/client.rs\0".to_vec(),
                ),
            ]),
            commands: Mutex::new(Vec::new()),
        });
        let executor = LiveCodexExecutor::new(
            Path::new("/repo").to_path_buf(),
            Arc::new(MockCheckpointService {
                capture: checkpoint,
                materialized,
            }),
            Arc::new(MockStreamer {
                stream: CodexAttemptStream {
                    codex_version: "0.130.0".to_owned(),
                    thread_id: "thread-1".to_owned(),
                    transcript,
                },
            }),
            git_runner.clone(),
            Arc::new(MockProcessRunner),
            Arc::new(FixedClock),
            "agentmint",
        );

        let fork = executor
            .confirm(
                &run,
                crate::aerf::intervention::RECORDED_DEMO_SCHEMA_EPISODE_ID,
            )
            .expect("live fork");

        let execution = fork.execution.expect("execution");
        assert_eq!(
            execution.mode,
            crate::aerf::intervention::CorrectionExecutionMode::LiveCodex
        );
        assert_eq!(
            execution.branch_name,
            "agentmint/attempt-codex-corrected-from-schema"
        );
        assert_eq!(execution.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(
            execution.verification_command,
            vec!["cargo", "test", "--test", "intervention_parsing"]
        );
        assert_eq!(execution.captured_token_usage.total_tokens, 50);
        assert_eq!(fork.comparison.token_usage.corrected.total_tokens, 50);
        assert_eq!(fork.appended_episode_id, "episode-codex-corrected-attempt");
        assert_eq!(fork.forked_run.run_id, "run-retry-after-v1-v2-codex-fork");
        assert!(git_runner
            .commands
            .lock()
            .expect("commands")
            .iter()
            .any(|command| command == "switch -c agentmint/attempt-codex-corrected-from-schema"));
    }

    #[test]
    fn default_executor_keeps_fixture_in_offline_mode() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/interventions/retry-after-v1-v2.json");
        let executor =
            default_executor_for_run_path(&fixture, Path::new("/repo")).expect("executor");
        let preview = executor.preview(
            &recorded_demo_run(),
            crate::aerf::intervention::RECORDED_DEMO_SCHEMA_EPISODE_ID,
        );
        assert_eq!(
            preview.expect("preview").packet.packet_id,
            "packet-fixture-correction-from-schema"
        );
    }

    #[test]
    fn relative_fixture_path_stays_in_offline_mode() {
        let fixture = Path::new("tests/fixtures/interventions/retry-after-v1-v2.json");
        let executor =
            default_executor_for_run_path(fixture, Path::new("/repo")).expect("executor");
        let preview = executor.preview(
            &recorded_demo_run(),
            crate::aerf::intervention::RECORDED_DEMO_SCHEMA_EPISODE_ID,
        );

        assert_eq!(
            preview.expect("preview").packet.packet_id,
            "packet-fixture-correction-from-schema"
        );
    }

    #[test]
    fn token_usage_and_agent_message_helpers_use_latest_stream_values() {
        let events = vec![
            NormalizedCodexEvent::AgentMessage(NormalizedAgentMessage {
                raw_event_name: "message.delta".to_owned(),
                thread_id: None,
                turn_id: None,
                item_id: None,
                role: Some("assistant".to_owned()),
                text: Some("first".to_owned()),
            }),
            NormalizedCodexEvent::AgentMessage(NormalizedAgentMessage {
                raw_event_name: "message.delta".to_owned(),
                thread_id: None,
                turn_id: None,
                item_id: None,
                role: Some("assistant".to_owned()),
                text: Some("second".to_owned()),
            }),
            NormalizedCodexEvent::TokenUsage(NormalizedTokenUsage {
                raw_event_name: "turn.completed".to_owned(),
                thread_id: None,
                turn_id: None,
                item_id: None,
                input_tokens: Some(3),
                cached_input_tokens: Some(1),
                output_tokens: Some(2),
                total_tokens: Some(5),
            }),
        ];

        assert_eq!(last_agent_statement(&events).as_deref(), Some("second"));
        assert_eq!(
            captured_token_usage(&events).expect("usage").total_tokens,
            5
        );
    }

    fn recorded_demo_run() -> Run {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/interventions/retry-after-v1-v2.json");
        Run::from_slice(&std::fs::read(path).expect("fixture bytes")).expect("fixture run")
    }
}
