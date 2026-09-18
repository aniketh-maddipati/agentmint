//! Typed PA/BV lab records for medical-mri-pa-v1.
//! Used by: store, workflow, agent, inspect, console.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WORKFLOW_VERSION: &str = "medical-mri-pa-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaseStage {
    Intake,
    Bv,
    Documentation,
    Review,
    Submission,
    FollowUp,
    Decision,
    Appeal,
    Handoff,
    ManualDisposition,
    Cancelled,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    Eligibility,
    Coverage,
    PaRequirement,
    Network,
    DocumentationNeed,
    InjectionAttempt,
    Clarification,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminationKind {
    PaRequired,
    PaNotRequired,
    InactiveMember,
    NotCovered,
    Unclear,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionTransportState {
    Attempted,
    ReceiptConfirmed,
    IntakeRejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Approved,
    Denied,
    Pending,
    Unclear,
    MoreInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Customer,
    Payer,
    Reviewer,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPurpose {
    CollectMissingInfo,
    BenefitsVerification,
    ClarifyBv,
    CollectDocumentation,
    ReviewPacket,
    SubmitPa,
    FollowUpStatus,
    DeliverHandoff,
    AppealPreparation,
    HumanReview,
    ManualDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Uncertainty {
    Known,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DraftObservation {
    pub kind: ObservationKind,
    #[schemars(length(min = 1))]
    pub statement: String,
    pub uncertainty: Uncertainty,
    #[schemars(length(min = 1))]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Blocked,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Decline,
    Changes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    AgentQuestion,
    DocumentRequest,
    Review,
    FollowUp,
    AppealDecision,
    HumanClarification,
    ExternalClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceContext {
    pub cpt: String,
    pub diagnosis: String,
    pub site: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageContext {
    pub payer_name: String,
    pub member_id: String,
    pub plan_id: String,
    pub dos: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    pub id: Uuid,
    pub run_id: Uuid,
    pub scenario_id: String,
    pub workflow_version: String,
    pub stage: CaseStage,
    pub service: ServiceContext,
    pub coverage: CoverageContext,
    pub service_version: u32,
    pub coverage_version: u32,
    pub disposition: Option<String>,
    pub paused_from: Option<CaseStage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub case_id: Uuid,
    pub purpose: TaskPurpose,
    pub status: TaskStatus,
    pub owner: Role,
    pub context_json: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: Uuid,
    pub case_id: Uuid,
    pub task_id: Option<Uuid>,
    pub purpose: String,
    pub outcome: AttemptOutcome,
    pub detail_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub case_id: Uuid,
    pub role: Role,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub case_id: Uuid,
    pub kind: ObservationKind,
    pub statement: String,
    pub uncertainty: Uncertainty,
    pub evidence_refs: Vec<String>,
    pub stale: bool,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Determination {
    pub id: Uuid,
    pub case_id: Uuid,
    pub kind: DeterminationKind,
    pub rationale: String,
    pub required_docs: Vec<String>,
    pub observation_ids: Vec<Uuid>,
    pub coverage_version: u32,
    pub service_version: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub case_id: Uuid,
    pub request_id: String,
    pub fixture_name: String,
    pub content_hash: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    pub id: Uuid,
    pub case_id: Uuid,
    pub version: u32,
    pub document_ids: Vec<Uuid>,
    pub content_hash: String,
    pub appeal_of_decision_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: Uuid,
    pub case_id: Uuid,
    pub packet_id: Uuid,
    pub packet_hash: String,
    pub decision: ReviewDecision,
    pub reviewer: String,
    pub valid: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    pub id: Uuid,
    pub case_id: Uuid,
    pub packet_id: Uuid,
    pub idempotency_key: String,
    pub transport_state: SubmissionTransportState,
    pub ownership_generation: u64,
    pub payer_receipt_id: Option<String>,
    pub detail_json: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: Uuid,
    pub case_id: Uuid,
    pub submission_id: Uuid,
    pub outcome: DecisionOutcome,
    pub limitations: Vec<String>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub case_id: Uuid,
    pub seq: u64,
    pub kind: String,
    pub payload_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingWork {
    pub id: Uuid,
    pub case_id: Uuid,
    pub kind: PendingKind,
    pub ref_id: String,
    pub detail: String,
    pub due_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunRecord {
    pub id: Uuid,
    pub case_id: Uuid,
    pub task_id: Uuid,
    pub prompt_version: String,
    pub model_id: String,
    pub context_version: u32,
    pub tool_calls_json: String,
    pub structured_output_json: String,
    pub evidence_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseSnapshot {
    pub case: Case,
    pub tasks: Vec<Task>,
    pub attempts: Vec<Attempt>,
    pub conversation: Vec<ConversationMessage>,
    pub observations: Vec<Observation>,
    pub determinations: Vec<Determination>,
    pub documents: Vec<Document>,
    pub packets: Vec<Packet>,
    pub reviews: Vec<Review>,
    pub submissions: Vec<Submission>,
    pub decisions: Vec<Decision>,
    pub events: Vec<Event>,
    pub pending: Vec<PendingWork>,
    pub agent_runs: Vec<AgentRunRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_serde_snake_case() {
        let raw = serde_json::to_string(&CaseStage::FollowUp).unwrap();
        assert_eq!(raw, "\"follow_up\"");
    }
}
