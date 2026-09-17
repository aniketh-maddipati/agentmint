//! Propose, authorize, execute, and reconcile an exact action.
//! Used by: API handlers. Provider I/O happens outside SQLite transactions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::config::Config;
use crate::credentials::CredentialSource;
use crate::domain::{
    provider_idempotency_key, ActionIntent, ActionRecord, ActionStatus, Approval, DecisionKind,
    ExecutionAttempt, ExecutionGrant, PolicyEffect, ProviderResult, ReceiptPayload, SignedReceipt,
    RECEIPT_VERSION, SIGNED_OBJECT_VERSION,
};
use crate::error::{Error, Result};
use crate::failpoints::Failpoint;
use crate::identity::AuthContext;
use crate::keys::KeyRing;
use crate::packs::Pack;
use crate::policy::PolicyProvider;
use crate::receipt::sign_receipt;
use crate::storage::Store;

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Engine {
    pub config: Arc<Config>,
    pub store: Store,
    pub keys: Arc<KeyRing>,
    pub policy: Arc<PolicyProvider>,
    pub credentials: Arc<CredentialSource>,
    pub pack: Pack,
}

impl Engine {
    pub async fn propose(
        &self,
        auth: &AuthContext,
        mut intent: ActionIntent,
    ) -> Result<ActionRecord> {
        if intent.tenant_id != auth.tenant_id {
            return Err(Error::Forbidden);
        }
        intent.actor = auth.actor.clone();
        crate::canonical::assert_intent_version(&intent.version)?;
        if self.pack.fake().is_none() && intent.provider != "stripe" {
            return Err(Error::UnsupportedField("provider"));
        }
        if self.pack.fake().is_some() && intent.provider != "fake" && intent.provider != "stripe" {
            return Err(Error::UnsupportedField("provider"));
        }
        let started = Instant::now();
        let canonical = self.pack.canonicalize(&intent)?;
        let _ = self.pack.classify(&canonical)?;
        let policy = self.policy.decide(&canonical).await?;
        let (status, grant) = match policy.effect {
            PolicyEffect::Automatic => (
                ActionStatus::Authorized,
                Some(ExecutionGrant {
                    intent_hash: canonical.intent_hash.clone(),
                    authorized_at: Utc::now(),
                    expires_at: intent.expires_at,
                    policy_version: policy.policy_version.clone(),
                }),
            ),
            PolicyEffect::ApprovalRequired => (ActionStatus::PendingApproval, None),
            PolicyEffect::Deny => (ActionStatus::Denied, None),
        };
        let record = ActionRecord {
            intent,
            status,
            intent_hash: canonical.intent_hash,
            canonical_version: canonical.canonicalization_version,
            canonical_json: canonical.canonical_json,
            policy: Some(policy),
            approval: None,
            grant,
            provider_result: None,
            reconciliation_required: false,
            latest_attempt: None,
            receipt: None,
        };
        self.store.insert_action(record.clone()).await?;
        crate::events::transition(
            record.intent.action_id,
            &record.intent.tenant_id,
            ActionStatus::Proposed,
            record.status,
            &record.intent.provider,
            &record.intent.operation,
            started.elapsed(),
            false,
        );
        Ok(record)
    }

    pub async fn get(&self, auth: &AuthContext, action_id: Uuid) -> Result<ActionRecord> {
        self.load(auth, action_id).await
    }

    pub async fn approve(
        &self,
        auth: &AuthContext,
        action_id: Uuid,
        intent_hash: &str,
    ) -> Result<ActionRecord> {
        let record = self.load(auth, action_id).await?;
        if record.status != ActionStatus::PendingApproval {
            return Err(Error::NotExecutable);
        }
        if auth.actor.subject == record.intent.actor.subject {
            return Err(Error::SelfApproval);
        }
        if record.intent.is_expired(Utc::now()) {
            return Err(Error::AuthorizationExpired);
        }
        if record.intent_hash != intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        let canonical = self.pack.canonicalize(&record.intent)?;
        if canonical.intent_hash != record.intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        let approval = Approval {
            subject: auth.actor.subject.clone(),
            approved_at: Utc::now(),
            intent_hash: record.intent_hash.clone(),
            decision: DecisionKind::Approved,
        };
        let grant = ExecutionGrant {
            intent_hash: record.intent_hash.clone(),
            authorized_at: Utc::now(),
            expires_at: record.intent.expires_at,
            policy_version: record
                .policy
                .as_ref()
                .map(|p| p.policy_version.clone())
                .unwrap_or_default(),
        };
        self.store
            .record_approval(
                &auth.tenant_id,
                action_id,
                ActionStatus::PendingApproval,
                ActionStatus::Authorized,
                Some(approval),
                Some(grant),
                None,
            )
            .await?;
        self.load(auth, action_id).await
    }

    pub async fn deny(
        &self,
        auth: &AuthContext,
        action_id: Uuid,
        intent_hash: &str,
    ) -> Result<ActionRecord> {
        let record = self.load(auth, action_id).await?;
        if record.status != ActionStatus::PendingApproval {
            return Err(Error::NotExecutable);
        }
        if auth.actor.subject == record.intent.actor.subject {
            return Err(Error::SelfApproval);
        }
        if record.intent_hash != intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        let denial = Approval {
            subject: auth.actor.subject.clone(),
            approved_at: Utc::now(),
            intent_hash: record.intent_hash.clone(),
            decision: DecisionKind::Denied,
        };
        self.store
            .record_approval(
                &auth.tenant_id,
                action_id,
                ActionStatus::PendingApproval,
                ActionStatus::Denied,
                Some(denial),
                None,
                None,
            )
            .await?;
        self.load(auth, action_id).await
    }

    pub async fn execute(
        &self,
        auth: &AuthContext,
        action_id: Uuid,
        expected_arguments: Option<Value>,
        failpoint: Option<Failpoint>,
    ) -> Result<ActionRecord> {
        let record = self.load(auth, action_id).await?;
        match record.status {
            ActionStatus::Succeeded
            | ActionStatus::Failed
            | ActionStatus::Denied
            | ActionStatus::Compensated => return Ok(record),
            ActionStatus::Unknown => return Err(Error::ReconciliationRequired),
            ActionStatus::Executing => return self.wait_for_terminal(auth, action_id).await,
            ActionStatus::Proposed | ActionStatus::PendingApproval => {
                return Err(Error::NotExecutable)
            }
            ActionStatus::Authorized => {}
        }
        if record.intent.is_expired(Utc::now()) {
            return Err(Error::AuthorizationExpired);
        }
        self.assert_hash(&record, expected_arguments.as_ref())?;
        let canonical = self.pack.canonicalize(&record.intent)?;
        if canonical.intent_hash != record.intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        let credential = self
            .credentials
            .credential(&auth.tenant_id, &record.intent.provider)
            .await?;
        self.pack.preflight(&canonical, &credential).await?;
        if failpoint == Some(Failpoint::BeforeProvider) {
            return Err(Error::Failpoint("before_provider"));
        }
        let attempt = ExecutionAttempt {
            attempt_id: Uuid::new_v4(),
            action_id,
            tenant_id: auth.tenant_id.clone(),
            attempt_no: 1,
            provider_idempotency_key: provider_idempotency_key(action_id, &record.intent.operation),
            status: "started".into(),
            started_at: Utc::now(),
            completed_at: None,
        };
        let claimed = self
            .store
            .claim_execution(&auth.tenant_id, action_id, attempt.clone())
            .await?;
        if !claimed {
            let current = self.load(auth, action_id).await?;
            return match current.status {
                ActionStatus::Executing => self.wait_for_terminal(auth, action_id).await,
                ActionStatus::Unknown => Err(Error::ReconciliationRequired),
                ActionStatus::Succeeded
                | ActionStatus::Failed
                | ActionStatus::Denied
                | ActionStatus::Compensated => Ok(current),
                _ => Err(Error::Conflict),
            };
        }
        crate::events::transition(
            action_id,
            &auth.tenant_id,
            ActionStatus::Authorized,
            ActionStatus::Executing,
            &record.intent.provider,
            &record.intent.operation,
            Duration::from_micros(0),
            false,
        );
        self.dispatch_provider(auth, record, attempt, failpoint, false)
            .await
    }

    pub async fn reconcile(&self, auth: &AuthContext, action_id: Uuid) -> Result<ActionRecord> {
        let record = self.load(auth, action_id).await?;
        match record.status {
            ActionStatus::Succeeded
            | ActionStatus::Failed
            | ActionStatus::Denied
            | ActionStatus::Compensated => return Ok(record),
            ActionStatus::Unknown | ActionStatus::Executing => {}
            _ => return Err(Error::NotExecutable),
        }
        self.assert_hash(&record, None)?;
        let Some(attempt) = record.latest_attempt.clone() else {
            return Err(Error::ReconciliationRequired);
        };
        let canonical = self.pack.canonicalize(&record.intent)?;
        let credential = self
            .credentials
            .credential(&auth.tenant_id, &record.intent.provider)
            .await?;
        let started = Instant::now();
        let outcome = self
            .pack
            .reconcile(&canonical, &attempt, &credential)
            .await?;
        if let Some(execution) = outcome.execution {
            return self
                .finish_success(
                    &auth.tenant_id,
                    record,
                    attempt,
                    execution,
                    None,
                    true,
                    started,
                )
                .await;
        }
        if outcome.failed {
            return self
                .finish_failure(
                    &auth.tenant_id,
                    record,
                    attempt,
                    ActionStatus::Failed,
                    false,
                    started,
                    true,
                )
                .await;
        }
        self.finish_failure(
            &auth.tenant_id,
            record,
            attempt,
            ActionStatus::Unknown,
            true,
            started,
            true,
        )
        .await?;
        Err(Error::UnknownOutcome)
    }

    async fn dispatch_provider(
        &self,
        auth: &AuthContext,
        record: ActionRecord,
        attempt: ExecutionAttempt,
        failpoint: Option<Failpoint>,
        reconciliation: bool,
    ) -> Result<ActionRecord> {
        let canonical = self.pack.canonicalize(&record.intent)?;
        if canonical.intent_hash != record.intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        let credential = self
            .credentials
            .credential(&auth.tenant_id, &record.intent.provider)
            .await?;
        let started = Instant::now();
        match self.pack.execute(&canonical, &credential).await {
            Ok(execution) => {
                if failpoint == Some(Failpoint::AfterProviderSuccess) {
                    self.finish_failure(
                        &auth.tenant_id,
                        record,
                        attempt,
                        ActionStatus::Unknown,
                        true,
                        started,
                        reconciliation,
                    )
                    .await?;
                    return Err(Error::UnknownOutcome);
                }
                let finished = self
                    .finish_success(
                        &auth.tenant_id,
                        record,
                        attempt,
                        execution,
                        failpoint,
                        reconciliation,
                        started,
                    )
                    .await?;
                if failpoint == Some(Failpoint::AfterCommitBeforeResponse) {
                    return Err(Error::Failpoint("after_commit_before_response"));
                }
                Ok(finished)
            }
            Err(Error::ProviderTimeout) | Err(Error::UnknownOutcome) => {
                self.finish_failure(
                    &auth.tenant_id,
                    record,
                    attempt,
                    ActionStatus::Unknown,
                    true,
                    started,
                    reconciliation,
                )
                .await?;
                Err(Error::UnknownOutcome)
            }
            Err(Error::ProviderRejected) => {
                self.finish_failure(
                    &auth.tenant_id,
                    record,
                    attempt,
                    ActionStatus::Failed,
                    false,
                    started,
                    reconciliation,
                )
                .await
            }
            Err(other) => Err(other),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_success(
        &self,
        tenant_id: &str,
        record: ActionRecord,
        mut attempt: ExecutionAttempt,
        execution: crate::domain::ProviderExecution,
        failpoint: Option<Failpoint>,
        reconciliation: bool,
        started: Instant,
    ) -> Result<ActionRecord> {
        let _ = failpoint;
        attempt.completed_at = Some(Utc::now());
        let provider_result = ProviderResult {
            provider: record.intent.provider.clone(),
            operation: record.intent.operation.clone(),
            provider_request_id: execution.provider_request_id.clone(),
            provider_resource_id: Some(execution.provider_resource_id.clone()),
            outcome: "succeeded".into(),
            redacted_payload: self.pack.redact(&execution.redacted_payload),
        };
        let policy = record.policy.clone().ok_or(Error::Internal)?;
        let payload = ReceiptPayload {
            format_version: RECEIPT_VERSION.to_owned(),
            action_id: record.intent.action_id,
            tenant_id: tenant_id.to_owned(),
            actor: record.intent.actor.clone(),
            intent_hash: record.intent_hash.clone(),
            policy,
            approval: record.approval.clone(),
            attempt_id: attempt.attempt_id,
            provider: record.intent.provider.clone(),
            operation: record.intent.operation.clone(),
            provider_idempotency_key: attempt.provider_idempotency_key.clone(),
            provider_resource_id: Some(execution.provider_resource_id),
            status: ActionStatus::Succeeded,
            started_at: attempt.started_at,
            completed_at: attempt.completed_at.unwrap_or_else(Utc::now),
            reconciliation_required: reconciliation,
            kid: self.keys.kid.clone(),
        };
        let signed = sign_receipt(
            &self.keys,
            SignedReceipt {
                format_version: SIGNED_OBJECT_VERSION.to_owned(),
                kid: self.keys.kid.clone(),
                payload,
                signature: String::new(),
            },
        )?;
        let from = if record.status == ActionStatus::Unknown {
            ActionStatus::Unknown
        } else {
            ActionStatus::Executing
        };
        self.store
            .complete(
                tenant_id,
                record.intent.action_id,
                from,
                ActionStatus::Succeeded,
                &attempt,
                Some(provider_result),
                Some(signed),
                reconciliation,
            )
            .await?;
        crate::events::transition(
            record.intent.action_id,
            tenant_id,
            from,
            ActionStatus::Succeeded,
            &record.intent.provider,
            &record.intent.operation,
            started.elapsed(),
            reconciliation,
        );
        self.store
            .get_action(tenant_id, record.intent.action_id)
            .await?
            .ok_or(Error::NotFound)
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_failure(
        &self,
        tenant_id: &str,
        record: ActionRecord,
        mut attempt: ExecutionAttempt,
        to: ActionStatus,
        reconciliation_required: bool,
        started: Instant,
        reconciliation: bool,
    ) -> Result<ActionRecord> {
        attempt.completed_at = Some(Utc::now());
        let from = if record.status == ActionStatus::Unknown {
            ActionStatus::Unknown
        } else {
            ActionStatus::Executing
        };
        let provider_result = ProviderResult {
            provider: record.intent.provider.clone(),
            operation: record.intent.operation.clone(),
            provider_request_id: None,
            provider_resource_id: None,
            outcome: to.as_str().to_ascii_lowercase(),
            redacted_payload: serde_json::json!({}),
        };
        let receipt = if to == ActionStatus::Failed {
            let policy = record.policy.clone().ok_or(Error::Internal)?;
            Some(sign_receipt(
                &self.keys,
                SignedReceipt {
                    format_version: SIGNED_OBJECT_VERSION.to_owned(),
                    kid: self.keys.kid.clone(),
                    payload: ReceiptPayload {
                        format_version: RECEIPT_VERSION.to_owned(),
                        action_id: record.intent.action_id,
                        tenant_id: tenant_id.to_owned(),
                        actor: record.intent.actor.clone(),
                        intent_hash: record.intent_hash.clone(),
                        policy,
                        approval: record.approval.clone(),
                        attempt_id: attempt.attempt_id,
                        provider: record.intent.provider.clone(),
                        operation: record.intent.operation.clone(),
                        provider_idempotency_key: attempt.provider_idempotency_key.clone(),
                        provider_resource_id: None,
                        status: ActionStatus::Failed,
                        started_at: attempt.started_at,
                        completed_at: attempt.completed_at.unwrap_or_else(Utc::now),
                        reconciliation_required: false,
                        kid: self.keys.kid.clone(),
                    },
                    signature: String::new(),
                },
            )?)
        } else {
            None
        };
        if to == ActionStatus::Unknown && from == ActionStatus::Unknown {
            crate::events::transition(
                record.intent.action_id,
                tenant_id,
                from,
                to,
                &record.intent.provider,
                &record.intent.operation,
                started.elapsed(),
                reconciliation,
            );
            return self
                .store
                .get_action(tenant_id, record.intent.action_id)
                .await?
                .ok_or(Error::NotFound);
        }
        self.store
            .complete(
                tenant_id,
                record.intent.action_id,
                from,
                to,
                &attempt,
                Some(provider_result),
                receipt,
                reconciliation_required,
            )
            .await?;
        crate::events::transition(
            record.intent.action_id,
            tenant_id,
            from,
            to,
            &record.intent.provider,
            &record.intent.operation,
            started.elapsed(),
            reconciliation,
        );
        self.store
            .get_action(tenant_id, record.intent.action_id)
            .await?
            .ok_or(Error::NotFound)
    }

    fn assert_hash(&self, record: &ActionRecord, expected_arguments: Option<&Value>) -> Result<()> {
        let mut intent = record.intent.clone();
        if let Some(arguments) = expected_arguments {
            intent.arguments = arguments.clone();
        }
        let canonical = self.pack.canonicalize(&intent)?;
        if canonical.intent_hash != record.intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        if let Some(grant) = &record.grant {
            if grant.intent_hash != canonical.intent_hash {
                return Err(Error::IntentHashMismatch);
            }
        }
        let stored = self.pack.canonicalize(&record.intent)?;
        if stored.intent_hash != record.intent_hash {
            return Err(Error::IntentHashMismatch);
        }
        Ok(())
    }

    async fn load(&self, auth: &AuthContext, action_id: Uuid) -> Result<ActionRecord> {
        self.store
            .get_action(&auth.tenant_id, action_id)
            .await?
            .ok_or(Error::NotFound)
    }

    async fn wait_for_terminal(&self, auth: &AuthContext, action_id: Uuid) -> Result<ActionRecord> {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            let record = self.load(auth, action_id).await?;
            if record.status.is_terminal() {
                return Ok(record);
            }
            if record.status == ActionStatus::Unknown {
                return Err(Error::ReconciliationRequired);
            }
            if Instant::now() >= deadline {
                return Ok(record);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}
