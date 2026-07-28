//! Normalized intervention records for deterministic replay fixtures and parsing.
//! Used by: focused intervention parsing tests and future offline analysis.

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aerf::adapters::codex_app_server::CodexTranscript;
use crate::aerf::AerfResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: String,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub request: InputEnvelope,
    pub raw_events: Vec<RawEvent>,
    pub episodes: Vec<Episode>,
    pub interventions: Vec<Intervention>,
    pub correction_packets: Vec<CorrectionPacket>,
    pub attempts: Vec<Attempt>,
    pub verification: Verification,
    pub token_usage: TokenUsage,
}

impl Run {
    pub fn from_slice(bytes: &[u8]) -> AerfResult<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn build_episodes(&self) -> Vec<Episode> {
        build_semantic_episodes(&self.raw_events)
    }
}

pub const RECORDED_DEMO_SCHEMA_EPISODE_ID: &str = "episode-edit-schema";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputEnvelope {
    pub input: Vec<InputItem>,
    #[serde(default)]
    pub cached_input: Vec<InputItem>,
}

impl InputEnvelope {
    fn validate(&self) -> Result<(), String> {
        if self
            .cached_input
            .iter()
            .all(|cached| self.input.contains(cached))
        {
            return Ok(());
        }

        Err("cached_input must be a subset of input".to_owned())
    }
}

impl<'de> Deserialize<'de> for InputEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawInputEnvelope {
            input: Vec<InputItem>,
            #[serde(default)]
            cached_input: Vec<InputItem>,
        }

        let raw = RawInputEnvelope::deserialize(deserializer)?;
        let normalized = Self {
            input: raw.input,
            cached_input: raw.cached_input,
        };
        normalized.validate().map_err(de::Error::custom)?;
        Ok(normalized)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputItem {
    Message {
        role: String,
        content: String,
    },
    ToolCall {
        tool_name: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEvent {
    pub event_id: String,
    pub recorded_at: String,
    pub source: RawEventSource,
    pub endpoint: String,
    pub request: InputEnvelope,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub outcome: Option<RawEventOutcome>,
    #[serde(default)]
    pub parse_state: RawEventParseState,
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default)]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawEventSource {
    Client,
    Server,
    Recorder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawEventOutcome {
    Succeeded,
    Failed,
    RateLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RawEventParseState {
    #[default]
    Complete,
    Malformed,
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Episode {
    pub episode_id: String,
    pub title: String,
    #[serde(default)]
    pub raw_event_ids: Vec<String>,
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Evidence {
    ObservedFact {
        evidence_id: String,
        summary: String,
        source_event_id: String,
    },
    AgentStatement {
        evidence_id: String,
        summary: String,
        source_event_id: String,
    },
    DerivedInference {
        evidence_id: String,
        summary: String,
        derived_from: Vec<String>,
    },
    HumanContext {
        evidence_id: String,
        summary: String,
        provided_by: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub status: CheckpointStatus,
    pub requested_at: String,
    pub reason: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStatus {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intervention {
    pub intervention_id: String,
    pub kind: InterventionKind,
    pub summary: String,
    #[serde(default)]
    pub checkpoint_ids: Vec<String>,
    #[serde(default)]
    pub correction_packet_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    Retry,
    Escalate,
    Clarify,
    SwitchEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionPacket {
    pub packet_id: String,
    pub issued_at: String,
    pub summary: String,
    pub request: InputEnvelope,
    #[serde(default)]
    pub supporting_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub attempt_id: String,
    pub endpoint: String,
    pub started_at: String,
    pub outcome: AttemptOutcome,
    pub request: InputEnvelope,
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Succeeded,
    RateLimited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub verified_at: String,
    pub verdict: VerificationVerdict,
    #[serde(default)]
    pub observed_fact_ids: Vec<String>,
    #[serde(default)]
    pub agent_statement_ids: Vec<String>,
    #[serde(default)]
    pub derived_inference_ids: Vec<String>,
    #[serde(default)]
    pub human_context_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Confirmed,
    Rejected,
    NeedsHumanReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionPreview {
    pub branch_from_episode_id: String,
    pub packet: CorrectionPacket,
    pub comparison: CorrectionComparison,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionFork {
    pub branch_from_episode_id: String,
    pub packet: CorrectionPacket,
    pub comparison: CorrectionComparison,
    pub forked_run: Run,
    pub appended_episode_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<CorrectionExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionExecution {
    pub mode: CorrectionExecutionMode,
    pub checkpoint_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub detached_head: bool,
    pub codex_version: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub attempt_id: String,
    pub verification_command: Vec<String>,
    pub verification_exit_code: i32,
    pub actual_patch: String,
    #[serde(default)]
    pub actual_changed_paths: Vec<String>,
    pub captured_token_usage: TokenUsage,
    pub handoff: TerminalHandoff,
    #[serde(default)]
    pub demo_limitations: Vec<String>,
    pub codex_transcript: CodexTranscript,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionExecutionMode {
    LiveCodex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalHandoff {
    pub directory: String,
    pub branch_state: String,
    pub checkpoint_id: String,
    pub attempt_id: String,
    pub verification_command: Vec<String>,
    pub copy_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionComparison {
    pub original_attempt_id: String,
    pub corrected_attempt: Attempt,
    pub patch: Vec<PatchChange>,
    pub verification: VerificationComparison,
    pub token_usage: TokenUsageComparison,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchChange {
    pub field: String,
    pub original: String,
    pub corrected: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationComparison {
    pub original_verdict: VerificationVerdict,
    pub corrected_verdict: VerificationVerdict,
    pub original_counts: VerificationCounts,
    pub corrected_counts: VerificationCounts,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCounts {
    pub observed_facts: usize,
    pub agent_statements: usize,
    pub derived_inferences: usize,
    pub human_contexts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsageComparison {
    pub original: TokenUsage,
    pub corrected: TokenUsage,
    pub delta_input_tokens: i64,
    pub delta_cached_input_tokens: i64,
    pub delta_output_tokens: i64,
    pub delta_total_tokens: i64,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticEpisodeKind {
    InspectApi,
    EditSchema,
    UpdateClient,
    FailedTest,
    ReviseFixture,
    PassingTest,
}

impl SemanticEpisodeKind {
    fn title(self) -> &'static str {
        match self {
            Self::InspectApi => "inspect API",
            Self::EditSchema => "edit schema",
            Self::UpdateClient => "update client",
            Self::FailedTest => "failed test",
            Self::ReviseFixture => "revise fixture",
            Self::PassingTest => "passing test",
        }
    }

    fn episode_id(self) -> &'static str {
        match self {
            Self::InspectApi => "episode-inspect-api",
            Self::EditSchema => "episode-edit-schema",
            Self::UpdateClient => "episode-update-client",
            Self::FailedTest => "episode-failed-test",
            Self::ReviseFixture => "episode-revise-fixture",
            Self::PassingTest => "episode-passing-test",
        }
    }
}

pub fn build_semantic_episodes(raw_events: &[RawEvent]) -> Vec<Episode> {
    let mut groups: Vec<(SemanticEpisodeKind, Vec<&RawEvent>)> = Vec::new();

    for event in raw_events {
        let Some(kind) = classify_event(event) else {
            continue;
        };

        if let Some((last_kind, events)) = groups.last_mut() {
            if *last_kind == kind && matches!(kind, SemanticEpisodeKind::InspectApi) {
                events.push(event);
                continue;
            }
        }

        groups.push((kind, vec![event]));
    }

    groups
        .into_iter()
        .map(|(kind, events)| Episode {
            episode_id: kind.episode_id().to_owned(),
            title: kind.title().to_owned(),
            raw_event_ids: events.iter().map(|event| event.event_id.clone()).collect(),
            evidence: build_episode_evidence(kind, &events),
            checkpoints: Vec::new(),
        })
        .collect()
}

fn build_episode_evidence(kind: SemanticEpisodeKind, events: &[&RawEvent]) -> Vec<Evidence> {
    let Some(first) = events.first() else {
        return Vec::new();
    };

    let summary = match kind {
        SemanticEpisodeKind::InspectApi => format!(
            "Collapsed {} API inspection events while preserving their raw-event IDs.",
            events.len()
        ),
        SemanticEpisodeKind::EditSchema => "Updated the schema shape for deterministic episodes."
            .to_owned(),
        SemanticEpisodeKind::UpdateClient => {
            "Adjusted the client flow to match the revised schema.".to_owned()
        }
        SemanticEpisodeKind::FailedTest => {
            "Recorded a failing test result before revising the fixture.".to_owned()
        }
        SemanticEpisodeKind::ReviseFixture => {
            "Revised the recorded fixture after the failing test.".to_owned()
        }
        SemanticEpisodeKind::PassingTest => {
            "Recorded the passing test result after the fixture revision.".to_owned()
        }
    };

    let mut evidence = vec![Evidence::ObservedFact {
        evidence_id: format!("{}-observed", kind.episode_id()),
        summary,
        source_event_id: first.event_id.clone(),
    }];

    if events
        .iter()
        .any(|event| !matches!(event.parse_state, RawEventParseState::Complete))
    {
        evidence.push(Evidence::DerivedInference {
            evidence_id: format!("{}-recovered", kind.episode_id()),
            summary: "Recovered the semantic episode from malformed or truncated event fields."
                .to_owned(),
            derived_from: events.iter().map(|event| event.event_id.clone()).collect(),
        });
    }

    evidence
}

fn classify_event(event: &RawEvent) -> Option<SemanticEpisodeKind> {
    if is_fixture_revision(event) {
        return Some(SemanticEpisodeKind::ReviseFixture);
    }
    if is_schema_edit(event) {
        return Some(SemanticEpisodeKind::EditSchema);
    }
    if is_client_update(event) {
        return Some(SemanticEpisodeKind::UpdateClient);
    }
    if is_failed_test(event) {
        return Some(SemanticEpisodeKind::FailedTest);
    }
    if is_passing_test(event) {
        return Some(SemanticEpisodeKind::PassingTest);
    }
    if is_api_inspection(event) {
        return Some(SemanticEpisodeKind::InspectApi);
    }
    None
}

fn is_api_inspection(event: &RawEvent) -> bool {
    let hints = event_hints(event);
    hints.iter().any(|hint| {
        hint.contains("docs/api/")
            || hint.contains("/v1/responses")
            || hint.contains("/v2/responses")
            || hint.contains("openai responses api")
    }) && event_kind_matches(event, &["read_file", "inspect_api"])
}

fn is_schema_edit(event: &RawEvent) -> bool {
    let hints = event_hints(event);
    hints.iter().any(|hint| hint.contains("schema"))
        && event_kind_matches(event, &["write_file", "edit_schema"])
}

fn is_client_update(event: &RawEvent) -> bool {
    let hints = event_hints(event);
    hints.iter().any(|hint| hint.contains("client"))
        && event_kind_matches(event, &["write_file", "update_client"])
}

fn is_fixture_revision(event: &RawEvent) -> bool {
    let hints = event_hints(event);
    hints
        .iter()
        .any(|hint| hint.contains("tests/fixtures/interventions/retry-after-v1-v2.json"))
        && event_kind_matches(event, &["write_file", "revise_fixture"])
}

fn is_failed_test(event: &RawEvent) -> bool {
    event_kind_matches(event, &["run_tests", "test"])
        && matches!(event.outcome, Some(RawEventOutcome::Failed))
}

fn is_passing_test(event: &RawEvent) -> bool {
    event_kind_matches(event, &["run_tests", "test"])
        && matches!(event.outcome, Some(RawEventOutcome::Succeeded))
}

fn event_kind_matches(event: &RawEvent, accepted: &[&str]) -> bool {
    let Some(tool_name) = event.tool_name.as_deref() else {
        return true;
    };
    accepted.iter().any(|candidate| tool_name == *candidate)
}

fn event_hints(event: &RawEvent) -> Vec<String> {
    let mut hints = vec![event.endpoint.to_ascii_lowercase()];
    if let Some(tool_name) = &event.tool_name {
        hints.push(tool_name.to_ascii_lowercase());
    }
    if let Some(file_path) = &event.file_path {
        hints.push(file_path.to_ascii_lowercase());
    }
    if let Some(summary) = &event.summary {
        hints.push(summary.to_ascii_lowercase());
    }
    hints
}

pub fn generate_recorded_demo_correction_preview(
    run: &Run,
    episode_id: &str,
) -> Option<CorrectionPreview> {
    if episode_id != RECORDED_DEMO_SCHEMA_EPISODE_ID {
        return None;
    }

    let original_attempt = original_correctable_attempt(run)?;
    let corrected_attempt = corrected_attempt(run);
    let packet = corrected_packet(run);
    let comparison = CorrectionComparison {
        original_attempt_id: original_attempt.attempt_id.clone(),
        corrected_attempt,
        patch: correction_patch(run),
        verification: correction_verification_comparison(run),
        token_usage: correction_token_usage_comparison(run),
    };

    Some(CorrectionPreview {
        branch_from_episode_id: episode_id.to_owned(),
        packet,
        comparison,
    })
}

pub fn generate_live_correction_preview(
    run: &Run,
    episode_id: &str,
) -> Option<CorrectionPreview> {
    let packet = generate_compact_correction_packet(run, episode_id)?;
    let original_attempt = original_correctable_attempt(run)?;
    let corrected_attempt = live_corrected_attempt(run, packet.request.clone());
    let comparison = CorrectionComparison {
        original_attempt_id: original_attempt.attempt_id.clone(),
        corrected_attempt,
        patch: live_correction_patch(run),
        verification: live_correction_verification_comparison(run),
        token_usage: live_correction_token_usage_projection(run),
    };

    Some(CorrectionPreview {
        branch_from_episode_id: episode_id.to_owned(),
        packet,
        comparison,
    })
}

pub fn apply_recorded_demo_correction(
    run: &Run,
    episode_id: &str,
) -> Option<CorrectionFork> {
    let preview = generate_recorded_demo_correction_preview(run, episode_id)?;
    let appended_episode = corrected_episode();
    let appended_episode_id = appended_episode.episode_id.clone();
    let appended_event = corrected_raw_event(&preview.packet, &preview.comparison.corrected_attempt);
    let mut forked_run = run.clone();
    forked_run.run_id = format!("{}-fixture-fork", run.run_id);
    forked_run.session_id = format!("{}-fixture-fork", run.session_id);
    forked_run.raw_events.push(appended_event);
    forked_run.episodes.push(appended_episode);
    forked_run.correction_packets.push(preview.packet.clone());
    forked_run.attempts.push(preview.comparison.corrected_attempt.clone());
    forked_run.verification = corrected_verification(&run.verification);
    forked_run.token_usage = corrected_token_usage();

    Some(CorrectionFork {
        branch_from_episode_id: episode_id.to_owned(),
        packet: preview.packet,
        comparison: preview.comparison,
        forked_run,
        appended_episode_id,
        execution: None,
    })
}

pub fn generate_compact_correction_packet(
    run: &Run,
    episode_id: &str,
) -> Option<CorrectionPacket> {
    if episode_id != RECORDED_DEMO_SCHEMA_EPISODE_ID {
        return None;
    }

    let mut request = run.request.clone();
    request.input.push(InputItem::Message {
        role: "system".to_owned(),
        content: "Compact correction: branch from episode-edit-schema, keep the schema change, retry the /v2 path, verify with the recorded intervention parsing test, and report patch plus token usage.".to_owned(),
    });

    Some(CorrectionPacket {
        packet_id: "packet-compact-correction-from-schema".to_owned(),
        issued_at: "2026-07-28T12:00:03Z".to_owned(),
        summary: "Compact correction packet branching from the schema episode for a live Codex retry in an isolated worktree.".to_owned(),
        request,
        supporting_evidence_ids: vec![
            "episode-inspect-api-observed".to_owned(),
            "episode-edit-schema-observed".to_owned(),
        ],
    })
}

pub fn recorded_verification_command(run: &Run) -> Vec<String> {
    let test_name = run
        .raw_events
        .iter()
        .rev()
        .find_map(|event| {
            if event.tool_name.as_deref() != Some("run_tests") {
                return None;
            }
            event.body
                .as_ref()
                .and_then(|body| body.get("test"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "intervention_parsing".to_owned());

    vec![
        "cargo".to_owned(),
        "test".to_owned(),
        "--test".to_owned(),
        test_name,
    ]
}

fn original_correctable_attempt(run: &Run) -> Option<&Attempt> {
    run.attempts.iter().find(|attempt| attempt.attempt_id == "attempt-v2")
}

fn corrected_packet(run: &Run) -> CorrectionPacket {
    let mut request = run.request.clone();
    request.input.push(InputItem::Message {
        role: "system".to_owned(),
        content: "FIXTURE-BACKED CORRECTION: branch from episode-edit-schema and issue a schema-aligned /v2 retry before the failed-test path.".to_owned(),
    });

    CorrectionPacket {
        packet_id: "packet-fixture-correction-from-schema".to_owned(),
        issued_at: "2026-07-28T12:00:03Z".to_owned(),
        summary: "Fixture-backed correction packet branching from the schema episode before the client update and later failing-test path.".to_owned(),
        request,
        supporting_evidence_ids: vec![
            "episode-inspect-api-observed".to_owned(),
            "episode-edit-schema-observed".to_owned(),
        ],
    }
}

fn live_corrected_attempt(run: &Run, request: InputEnvelope) -> Attempt {
    Attempt {
        attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
        endpoint: original_correctable_attempt(run)
            .map(|attempt| attempt.endpoint.clone())
            .unwrap_or_else(|| "/v2/responses".to_owned()),
        started_at: "2026-07-28T12:00:03Z".to_owned(),
        outcome: AttemptOutcome::Succeeded,
        request,
        status_code: Some(200),
        retry_after_ms: None,
    }
}

fn corrected_attempt(run: &Run) -> Attempt {
    Attempt {
        attempt_id: "attempt-fixture-corrected-from-schema".to_owned(),
        endpoint: "/v2/responses?fixture_branch=episode-edit-schema".to_owned(),
        started_at: "2026-07-28T12:00:03Z".to_owned(),
        outcome: AttemptOutcome::Succeeded,
        request: corrected_packet(run).request,
        status_code: Some(200),
        retry_after_ms: None,
    }
}

fn correction_patch(run: &Run) -> Vec<PatchChange> {
    let original_attempt = original_correctable_attempt(run);
    let corrected_attempt = corrected_attempt(run);

    vec![
        PatchChange {
            field: "attempt.endpoint".to_owned(),
            original: original_attempt
                .map(|attempt| attempt.endpoint.clone())
                .unwrap_or_else(|| "<missing>".to_owned()),
            corrected: corrected_attempt.endpoint,
        },
        PatchChange {
            field: "request.input[2]".to_owned(),
            original: "<missing>".to_owned(),
            corrected: "system: FIXTURE-BACKED CORRECTION: branch from episode-edit-schema and issue a schema-aligned /v2 retry before the failed-test path.".to_owned(),
        },
        PatchChange {
            field: "branch.note".to_owned(),
            original: "Continue through failed test, fixture revision, and final passing test.".to_owned(),
            corrected: "Fork directly from the schema episode into a fixture-backed corrected attempt.".to_owned(),
        },
    ]
}

fn live_correction_patch(run: &Run) -> Vec<PatchChange> {
    let original_attempt = original_correctable_attempt(run);

    vec![
        PatchChange {
            field: "attempt.endpoint".to_owned(),
            original: original_attempt
                .map(|attempt| attempt.endpoint.clone())
                .unwrap_or_else(|| "<missing>".to_owned()),
            corrected: "/v2/responses".to_owned(),
        },
        PatchChange {
            field: "request.input[2]".to_owned(),
            original: "<missing>".to_owned(),
            corrected: "system: Compact correction: branch from episode-edit-schema, keep the schema change, retry the /v2 path, verify with the recorded intervention parsing test, and report patch plus token usage.".to_owned(),
        },
        PatchChange {
            field: "verification.command".to_owned(),
            original: "Continue through the recorded failed-test and fixture-revision sequence.".to_owned(),
            corrected: recorded_verification_command(run).join(" "),
        },
    ]
}

fn correction_verification_comparison(run: &Run) -> VerificationComparison {
    VerificationComparison {
        original_verdict: run.verification.verdict.clone(),
        corrected_verdict: VerificationVerdict::Confirmed,
        original_counts: verification_counts(&run.verification),
        corrected_counts: VerificationCounts {
            observed_facts: run.verification.observed_fact_ids.len() + 1,
            agent_statements: run.verification.agent_statement_ids.len() + 1,
            derived_inferences: run.verification.derived_inference_ids.len() + 1,
            human_contexts: run.verification.human_context_ids.len() + 1,
        },
        note: "The corrected branch stays confirmed while adding explicit fixture-backed fact, agent statement, inference, and human context records.".to_owned(),
    }
}

fn live_correction_verification_comparison(run: &Run) -> VerificationComparison {
    VerificationComparison {
        original_verdict: run.verification.verdict.clone(),
        corrected_verdict: VerificationVerdict::Confirmed,
        original_counts: verification_counts(&run.verification),
        corrected_counts: VerificationCounts {
            observed_facts: run.verification.observed_fact_ids.len() + 1,
            agent_statements: run.verification.agent_statement_ids.len() + 1,
            derived_inferences: run.verification.derived_inference_ids.len() + 1,
            human_contexts: run.verification.human_context_ids.len() + 1,
        },
        note: "The live correction branch should add one observed fact, one agent statement, one derived inference, and one human context record tied to the worktree-backed Codex attempt.".to_owned(),
    }
}

fn correction_token_usage_comparison(run: &Run) -> TokenUsageComparison {
    let corrected = corrected_token_usage();
    TokenUsageComparison {
        original: run.token_usage.clone(),
        delta_input_tokens: corrected.input_tokens as i64 - run.token_usage.input_tokens as i64,
        delta_cached_input_tokens: corrected.cached_input_tokens as i64
            - run.token_usage.cached_input_tokens as i64,
        delta_output_tokens: corrected.output_tokens as i64 - run.token_usage.output_tokens as i64,
        delta_total_tokens: corrected.total_tokens as i64 - run.token_usage.total_tokens as i64,
        corrected,
        note: "Forking from the schema episode skips the later failed-test and fixture-revision path, reducing total token use in the recorded demo.".to_owned(),
    }
}

fn live_correction_token_usage_projection(run: &Run) -> TokenUsageComparison {
    let corrected = TokenUsage {
        input_tokens: run.token_usage.input_tokens,
        cached_input_tokens: run.token_usage.cached_input_tokens,
        output_tokens: run.token_usage.output_tokens,
        total_tokens: run.token_usage.total_tokens,
    };

    TokenUsageComparison {
        original: run.token_usage.clone(),
        corrected,
        delta_input_tokens: 0,
        delta_cached_input_tokens: 0,
        delta_output_tokens: 0,
        delta_total_tokens: 0,
        note: "Projected values are replaced with captured Codex token usage once the live correction branch completes.".to_owned(),
    }
}

fn corrected_token_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 96,
        cached_input_tokens: 48,
        output_tokens: 22,
        total_tokens: 166,
    }
}

fn corrected_verification(original: &Verification) -> Verification {
    let mut verification = original.clone();
    verification.verified_at = "2026-07-28T12:00:04Z".to_owned();
    verification.observed_fact_ids.push("episode-fixture-corrected-attempt-observed".to_owned());
    verification
        .agent_statement_ids
        .push("episode-fixture-corrected-attempt-agent".to_owned());
    verification
        .derived_inference_ids
        .push("episode-fixture-corrected-attempt-inference".to_owned());
    verification
        .human_context_ids
        .push("episode-fixture-corrected-attempt-human".to_owned());
    verification
}

fn corrected_episode() -> Episode {
    Episode {
        episode_id: "episode-fixture-corrected-attempt".to_owned(),
        title: "fixture-backed corrected attempt".to_owned(),
        raw_event_ids: vec![
            "edit-schema-1".to_owned(),
            "fixture-corrected-attempt-1".to_owned(),
        ],
        evidence: vec![
            Evidence::ObservedFact {
                evidence_id: "episode-fixture-corrected-attempt-observed".to_owned(),
                summary: "Recorded a fixture-backed corrected attempt branching from the schema episode.".to_owned(),
                source_event_id: "fixture-corrected-attempt-1".to_owned(),
            },
            Evidence::AgentStatement {
                evidence_id: "episode-fixture-corrected-attempt-agent".to_owned(),
                summary: "The demo agent states that the corrected branch should bypass the later failed-test path.".to_owned(),
                source_event_id: "fixture-corrected-attempt-1".to_owned(),
            },
            Evidence::DerivedInference {
                evidence_id: "episode-fixture-corrected-attempt-inference".to_owned(),
                summary: "Branching from the schema episode preserves verification while reducing token use in the recorded demo.".to_owned(),
                derived_from: vec![
                    "episode-fixture-corrected-attempt-observed".to_owned(),
                    "episode-fixture-corrected-attempt-agent".to_owned(),
                ],
            },
            Evidence::HumanContext {
                evidence_id: "episode-fixture-corrected-attempt-human".to_owned(),
                summary: "This corrected branch is fixture-backed and deterministic for the local demo only.".to_owned(),
                provided_by: "agentmint-intervene".to_owned(),
            },
        ],
        checkpoints: Vec::new(),
    }
}

fn corrected_raw_event(packet: &CorrectionPacket, attempt: &Attempt) -> RawEvent {
    RawEvent {
        event_id: "fixture-corrected-attempt-1".to_owned(),
        recorded_at: "2026-07-28T12:00:04Z".to_owned(),
        source: RawEventSource::Recorder,
        endpoint: attempt.endpoint.clone(),
        request: packet.request.clone(),
        tool_name: Some("fork_correction".to_owned()),
        file_path: Some("src/aerf/intervention.rs".to_owned()),
        summary: Some("Fixture-backed corrected attempt appended from the schema episode.".to_owned()),
        outcome: Some(RawEventOutcome::Succeeded),
        parse_state: RawEventParseState::Complete,
        status_code: attempt.status_code,
        retry_after_ms: attempt.retry_after_ms,
        body: Some(Value::String("fixture-backed corrected attempt".to_owned())),
    }
}

fn verification_counts(verification: &Verification) -> VerificationCounts {
    VerificationCounts {
        observed_facts: verification.observed_fact_ids.len(),
        agent_statements: verification.agent_statement_ids.len(),
        derived_inferences: verification.derived_inference_ids.len(),
        human_contexts: verification.human_context_ids.len(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        apply_recorded_demo_correction, build_semantic_episodes,
        generate_recorded_demo_correction_preview, Evidence, InputEnvelope, RawEvent,
        RawEventOutcome, RawEventParseState, RawEventSource, Run,
    };

    #[test]
    fn rejects_cached_input_outside_input() {
        let value = json!({
            "input": [
                { "type": "message", "role": "user", "content": "plan for v2" }
            ],
            "cached_input": [
                { "type": "message", "role": "system", "content": "carry over v1 retries" }
            ]
        });

        let err = serde_json::from_value::<InputEnvelope>(value).expect_err("subset validation");

        assert!(err.to_string().contains("cached_input must be a subset of input"));
    }

    #[test]
    fn parses_evidence_kinds_distinctly() {
        let value = json!({
            "run_id": "run-1",
            "session_id": "session-1",
            "provider": "openai",
            "model": "gpt-test",
            "request": {
                "input": [
                    { "type": "message", "role": "system", "content": "watch retry headers" }
                ],
                "cached_input": [
                    { "type": "message", "role": "system", "content": "watch retry headers" }
                ]
            },
            "raw_events": [],
            "episodes": [
                {
                    "episode_id": "episode-1",
                    "title": "retry analysis",
                    "evidence": [
                        {
                            "kind": "observed_fact",
                            "evidence_id": "fact-1",
                            "summary": "retry-after header observed",
                            "source_event_id": "event-1"
                        },
                        {
                            "kind": "agent_statement",
                            "evidence_id": "stmt-1",
                            "summary": "agent says retry v1",
                            "source_event_id": "event-1"
                        },
                        {
                            "kind": "derived_inference",
                            "evidence_id": "infer-1",
                            "summary": "v2 fallback needed",
                            "derived_from": ["fact-1", "stmt-1"]
                        },
                        {
                            "kind": "human_context",
                            "evidence_id": "ctx-1",
                            "summary": "operator requires parity",
                            "provided_by": "reviewer"
                        }
                    ],
                    "checkpoints": []
                }
            ],
            "interventions": [],
            "correction_packets": [],
            "attempts": [],
            "verification": {
                "verified_at": "2026-07-28T12:05:00Z",
                "verdict": "confirmed",
                "observed_fact_ids": ["fact-1"],
                "agent_statement_ids": ["stmt-1"],
                "derived_inference_ids": ["infer-1"],
                "human_context_ids": ["ctx-1"]
            },
            "token_usage": {
                "input_tokens": 10,
                "cached_input_tokens": 4,
                "output_tokens": 6,
                "total_tokens": 20
            }
        });

        let run = serde_json::from_value::<Run>(value).expect("run");
        let evidence = &run.episodes[0].evidence;

        assert!(matches!(evidence[0], Evidence::ObservedFact { .. }));
        assert!(matches!(evidence[1], Evidence::AgentStatement { .. }));
        assert!(matches!(evidence[2], Evidence::DerivedInference { .. }));
        assert!(matches!(evidence[3], Evidence::HumanContext { .. }));
        assert_eq!(run.verification.observed_fact_ids, vec!["fact-1"]);
        assert_eq!(run.verification.agent_statement_ids, vec!["stmt-1"]);
        assert_eq!(run.verification.derived_inference_ids, vec!["infer-1"]);
        assert_eq!(run.verification.human_context_ids, vec!["ctx-1"]);
    }

    #[test]
    fn builds_exact_six_semantic_episodes() {
        let raw_events = vec![
            raw_event(
                "inspect-1",
                Some("read_file"),
                Some("docs/api/v1/responses.md"),
                None,
                RawEventParseState::Complete,
            ),
            raw_event(
                "inspect-2",
                Some("read_file"),
                Some("docs/api/v2/responses.md"),
                None,
                RawEventParseState::Complete,
            ),
            raw_event(
                "schema-1",
                Some("write_file"),
                Some("src/schema/intervention.rs"),
                None,
                RawEventParseState::Complete,
            ),
            raw_event(
                "client-1",
                Some("write_file"),
                Some("src/client/responses.rs"),
                None,
                RawEventParseState::Complete,
            ),
            raw_event(
                "test-fail-1",
                Some("run_tests"),
                None,
                Some(RawEventOutcome::Failed),
                RawEventParseState::Complete,
            ),
            raw_event(
                "fixture-1",
                Some("write_file"),
                Some("tests/fixtures/interventions/retry-after-v1-v2.json"),
                None,
                RawEventParseState::Complete,
            ),
            raw_event(
                "test-pass-1",
                Some("run_tests"),
                None,
                Some(RawEventOutcome::Succeeded),
                RawEventParseState::Complete,
            ),
        ];

        let episodes = build_semantic_episodes(&raw_events);

        assert_eq!(episodes.len(), 6);
        assert_eq!(episodes[0].raw_event_ids, vec!["inspect-1", "inspect-2"]);
        assert_eq!(episodes[0].title, "inspect API");
        assert_eq!(episodes[5].title, "passing test");
    }

    #[test]
    fn recovers_malformed_or_truncated_event_groups() {
        let raw_events = vec![
            raw_event(
                "inspect-a",
                None,
                Some("docs/api/v1/responses.md"),
                None,
                RawEventParseState::Malformed,
            ),
            raw_event(
                "inspect-b",
                Some("read_file"),
                Some("docs/api/v2/responses.md"),
                None,
                RawEventParseState::Truncated,
            ),
            raw_event(
                "test-pass",
                Some("run_tests"),
                None,
                Some(RawEventOutcome::Succeeded),
                RawEventParseState::Complete,
            ),
        ];

        let episodes = build_semantic_episodes(&raw_events);

        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].raw_event_ids, vec!["inspect-a", "inspect-b"]);
        assert!(matches!(episodes[0].evidence[1], Evidence::DerivedInference { .. }));
        assert_eq!(episodes[1].title, "passing test");
    }

    #[test]
    fn snapshots_recorded_demo_correction_preview() {
        let run = recorded_demo_run();
        let preview = generate_recorded_demo_correction_preview(&run, super::RECORDED_DEMO_SCHEMA_EPISODE_ID)
            .expect("preview");

        let packet = serde_json::to_string_pretty(&preview.packet).expect("packet json");
        let comparison =
            serde_json::to_string_pretty(&preview.comparison).expect("comparison json");

        assert_eq!(
            packet,
            r#"{
  "packet_id": "packet-fixture-correction-from-schema",
  "issued_at": "2026-07-28T12:00:03Z",
  "summary": "Fixture-backed correction packet branching from the schema episode before the client update and later failing-test path.",
  "request": {
    "input": [
      {
        "type": "message",
        "role": "system",
        "content": "Preserve deterministic retries across API versions."
      },
      {
        "type": "message",
        "role": "user",
        "content": "Explain why /v1 rate limits should not corrupt /v2 recovery."
      },
      {
        "type": "message",
        "role": "system",
        "content": "FIXTURE-BACKED CORRECTION: branch from episode-edit-schema and issue a schema-aligned /v2 retry before the failed-test path."
      }
    ],
    "cached_input": [
      {
        "type": "message",
        "role": "system",
        "content": "Preserve deterministic retries across API versions."
      }
    ]
  },
  "supporting_evidence_ids": [
    "episode-inspect-api-observed",
    "episode-edit-schema-observed"
  ]
}"#
        );
        assert_eq!(
            comparison,
            r#"{
  "original_attempt_id": "attempt-v2",
  "corrected_attempt": {
    "attempt_id": "attempt-fixture-corrected-from-schema",
    "endpoint": "/v2/responses?fixture_branch=episode-edit-schema",
    "started_at": "2026-07-28T12:00:03Z",
    "outcome": "succeeded",
    "request": {
      "input": [
        {
          "type": "message",
          "role": "system",
          "content": "Preserve deterministic retries across API versions."
        },
        {
          "type": "message",
          "role": "user",
          "content": "Explain why /v1 rate limits should not corrupt /v2 recovery."
        },
        {
          "type": "message",
          "role": "system",
          "content": "FIXTURE-BACKED CORRECTION: branch from episode-edit-schema and issue a schema-aligned /v2 retry before the failed-test path."
        }
      ],
      "cached_input": [
        {
          "type": "message",
          "role": "system",
          "content": "Preserve deterministic retries across API versions."
        }
      ]
    },
    "status_code": 200,
    "retry_after_ms": null
  },
  "patch": [
    {
      "field": "attempt.endpoint",
      "original": "/v2/responses",
      "corrected": "/v2/responses?fixture_branch=episode-edit-schema"
    },
    {
      "field": "request.input[2]",
      "original": "<missing>",
      "corrected": "system: FIXTURE-BACKED CORRECTION: branch from episode-edit-schema and issue a schema-aligned /v2 retry before the failed-test path."
    },
    {
      "field": "branch.note",
      "original": "Continue through failed test, fixture revision, and final passing test.",
      "corrected": "Fork directly from the schema episode into a fixture-backed corrected attempt."
    }
  ],
  "verification": {
    "original_verdict": "confirmed",
    "corrected_verdict": "confirmed",
    "original_counts": {
      "observed_facts": 6,
      "agent_statements": 0,
      "derived_inferences": 0,
      "human_contexts": 0
    },
    "corrected_counts": {
      "observed_facts": 7,
      "agent_statements": 1,
      "derived_inferences": 1,
      "human_contexts": 1
    },
    "note": "The corrected branch stays confirmed while adding explicit fixture-backed fact, agent statement, inference, and human context records."
  },
  "token_usage": {
    "original": {
      "input_tokens": 120,
      "cached_input_tokens": 48,
      "output_tokens": 36,
      "total_tokens": 204
    },
    "corrected": {
      "input_tokens": 96,
      "cached_input_tokens": 48,
      "output_tokens": 22,
      "total_tokens": 166
    },
    "delta_input_tokens": -24,
    "delta_cached_input_tokens": 0,
    "delta_output_tokens": -14,
    "delta_total_tokens": -38,
    "note": "Forking from the schema episode skips the later failed-test and fixture-revision path, reducing total token use in the recorded demo."
  }
}"#
        );
    }

    #[test]
    fn applies_recorded_demo_correction_as_forked_branch() {
        let run = recorded_demo_run();
        let fork =
            apply_recorded_demo_correction(&run, super::RECORDED_DEMO_SCHEMA_EPISODE_ID)
                .expect("fork");

        assert_eq!(fork.appended_episode_id, "episode-fixture-corrected-attempt");
        assert_eq!(fork.forked_run.episodes.len(), run.episodes.len() + 1);
        assert_eq!(fork.forked_run.attempts.len(), run.attempts.len() + 1);
        assert_eq!(
            fork.forked_run.episodes.last().map(|episode| episode.title.as_str()),
            Some("fixture-backed corrected attempt")
        );
    }

    fn recorded_demo_run() -> Run {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/interventions/retry-after-v1-v2.json");
        Run::from_slice(&std::fs::read(path).expect("fixture bytes")).expect("fixture run")
    }

    fn raw_event(
        event_id: &str,
        tool_name: Option<&str>,
        file_path: Option<&str>,
        outcome: Option<RawEventOutcome>,
        parse_state: RawEventParseState,
    ) -> RawEvent {
        RawEvent {
            event_id: event_id.to_owned(),
            recorded_at: "2026-07-28T12:00:00Z".to_owned(),
            source: RawEventSource::Recorder,
            endpoint: "/local/workspace".to_owned(),
            request: InputEnvelope {
                input: vec![super::InputItem::Message {
                    role: "user".to_owned(),
                    content: "repair fixture".to_owned(),
                }],
                cached_input: Vec::new(),
            },
            tool_name: tool_name.map(str::to_owned),
            file_path: file_path.map(str::to_owned),
            summary: file_path.map(str::to_owned),
            outcome,
            parse_state,
            status_code: None,
            retry_after_ms: None,
            body: None,
        }
    }
}
