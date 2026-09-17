//! Core action, policy, grant, and receipt types.
//! Used by: storage, execution, API, packs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const INTENT_VERSION: &str = "mint-intent-v1";
pub const CANONICALIZATION_VERSION: &str = "jcs-rfc8785-v1";
pub const RECEIPT_VERSION: &str = "mint-receipt-v1";
pub const SIGNED_OBJECT_VERSION: &str = "mint-signed-v1";
pub const REFUND_OPERATION: &str = "refund.create";
pub const REFUND_OPERATION_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorIdentity {
    pub subject: String,
    pub agent_id: String,
    #[serde(default)]
    pub delegated_by: Option<String>,
    pub issuer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceRef {
    pub resource_type: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionContext {
    #[serde(default)]
    pub support_ticket_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionIntent {
    pub version: String,
    pub action_id: Uuid,
    pub tenant_id: String,
    pub actor: ActorIdentity,
    pub provider: String,
    pub operation: String,
    pub resource: ResourceRef,
    pub arguments: Value,
    pub context: ActionContext,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ActionIntent {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionStatus {
    Proposed,
    PendingApproval,
    Authorized,
    Denied,
    Executing,
    Succeeded,
    Failed,
    Unknown,
    Compensated,
}

impl ActionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "Proposed",
            Self::PendingApproval => "PendingApproval",
            Self::Authorized => "Authorized",
            Self::Denied => "Denied",
            Self::Executing => "Executing",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
            Self::Compensated => "Compensated",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "Proposed" => Some(Self::Proposed),
            "PendingApproval" => Some(Self::PendingApproval),
            "Authorized" => Some(Self::Authorized),
            "Denied" => Some(Self::Denied),
            "Executing" => Some(Self::Executing),
            "Succeeded" => Some(Self::Succeeded),
            "Failed" => Some(Self::Failed),
            "Unknown" => Some(Self::Unknown),
            "Compensated" => Some(Self::Compensated),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Denied | Self::Compensated
        )
    }
}

/// Invariant: transitions never move backward and never authorize a second execution attempt.
pub fn can_transition(from: ActionStatus, to: ActionStatus) -> bool {
    use ActionStatus::*;
    matches!(
        (from, to),
        (Proposed, PendingApproval)
            | (Proposed, Authorized)
            | (Proposed, Denied)
            | (PendingApproval, Authorized)
            | (PendingApproval, Denied)
            | (Authorized, Executing)
            | (Executing, Succeeded)
            | (Executing, Failed)
            | (Executing, Unknown)
            | (Unknown, Succeeded)
            | (Unknown, Failed)
            | (Unknown, Compensated)
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Automatic,
    ApprovalRequired,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDecision {
    pub effect: PolicyEffect,
    pub policy_version: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Approved,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Approval {
    pub subject: String,
    pub approved_at: DateTime<Utc>,
    pub intent_hash: String,
    #[serde(default = "default_approved")]
    pub decision: DecisionKind,
}

fn default_approved() -> DecisionKind {
    DecisionKind::Approved
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionGrant {
    pub intent_hash: String,
    pub authorized_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAttempt {
    pub attempt_id: Uuid,
    pub action_id: Uuid,
    pub tenant_id: String,
    pub attempt_no: i64,
    pub provider_idempotency_key: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderResult {
    pub provider: String,
    pub operation: String,
    pub provider_request_id: Option<String>,
    pub provider_resource_id: Option<String>,
    pub outcome: String,
    pub redacted_payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReceiptPayload {
    pub format_version: String,
    pub action_id: Uuid,
    pub tenant_id: String,
    pub actor: ActorIdentity,
    pub intent_hash: String,
    pub policy: PolicyDecision,
    pub approval: Option<Approval>,
    pub attempt_id: Uuid,
    pub provider: String,
    pub operation: String,
    pub provider_idempotency_key: String,
    pub provider_resource_id: Option<String>,
    pub status: ActionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub reconciliation_required: bool,
    pub kid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedReceipt {
    pub format_version: String,
    pub kid: String,
    pub payload: ReceiptPayload,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalAction {
    pub canonicalization_version: String,
    pub canonical_json: String,
    pub intent_hash: String,
    pub provider: String,
    pub operation: String,
    pub refund: RefundAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundAction {
    pub charge_or_pi: String,
    pub resource_type: String,
    pub amount_cents: i64,
    pub currency: String,
    pub reason: String,
    pub mint_action_id: Uuid,
    pub support_ticket_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskClassification {
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderExecution {
    pub provider_request_id: Option<String>,
    pub provider_resource_id: String,
    pub redacted_payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconciliationResult {
    pub established: bool,
    pub execution: Option<ProviderExecution>,
    pub failed: bool,
}

#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub intent: ActionIntent,
    pub status: ActionStatus,
    pub intent_hash: String,
    pub canonical_version: String,
    pub canonical_json: String,
    pub policy: Option<PolicyDecision>,
    pub approval: Option<Approval>,
    pub grant: Option<ExecutionGrant>,
    pub provider_result: Option<ProviderResult>,
    pub reconciliation_required: bool,
    pub latest_attempt: Option<ExecutionAttempt>,
    pub receipt: Option<SignedReceipt>,
}

pub fn provider_idempotency_key(action_id: Uuid, operation: &str) -> String {
    format!("mint:{action_id}:{operation}:{REFUND_OPERATION_VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_never_moves_backward() {
        let terminals = [
            ActionStatus::Succeeded,
            ActionStatus::Failed,
            ActionStatus::Denied,
            ActionStatus::Compensated,
        ];
        for terminal in terminals {
            for to in [
                ActionStatus::Proposed,
                ActionStatus::PendingApproval,
                ActionStatus::Authorized,
                ActionStatus::Executing,
            ] {
                assert!(!can_transition(terminal, to));
            }
        }
        assert!(!can_transition(
            ActionStatus::Authorized,
            ActionStatus::Authorized
        ));
        assert!(!can_transition(
            ActionStatus::Executing,
            ActionStatus::Authorized
        ));
        assert!(can_transition(
            ActionStatus::Authorized,
            ActionStatus::Executing
        ));
    }
}
