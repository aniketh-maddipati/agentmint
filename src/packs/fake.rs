//! Deterministic fake refund provider for local demo and tests.
//! Used by: Pack::Fake, demo-local, integration tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::json;

use crate::canonical::hashed_canonical;
use crate::credentials::ProviderCredential;
use crate::domain::ExecutionAttempt;
use crate::domain::{
    ActionIntent, CanonicalAction, ProviderExecution, ReconciliationResult, RiskClassification,
};
use crate::error::{Error, Result};
use crate::packs::{parse_refund_intent, redact_value};

#[derive(Debug, Clone, Copy)]
pub enum FakeScript {
    Success,
    Timeout,
    Reject,
}

pub struct FakePack {
    pub calls: AtomicU64,
    pub effects: AtomicU64,
    outcomes: Mutex<HashMap<String, ProviderExecution>>,
    next: Mutex<Option<FakeScript>>,
}

impl Default for FakePack {
    fn default() -> Self {
        Self {
            calls: AtomicU64::new(0),
            effects: AtomicU64::new(0),
            outcomes: Mutex::new(HashMap::new()),
            next: Mutex::new(None),
        }
    }
}

impl FakePack {
    pub fn set_next(&self, script: FakeScript) -> Result<()> {
        let mut next = self
            .next
            .lock()
            .map_err(|err| Error::internal("fake script", err))?;
        *next = Some(script);
        Ok(())
    }

    pub fn call_count(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn effect_count(&self) -> u64 {
        self.effects.load(Ordering::SeqCst)
    }

    pub fn canonicalize(&self, intent: &ActionIntent) -> Result<CanonicalAction> {
        let refund = parse_refund_intent(intent, true)?;
        hashed_canonical(intent, refund)
    }

    pub fn classify(&self, action: &CanonicalAction) -> Result<RiskClassification> {
        Ok(RiskClassification {
            amount_cents: action.refund.amount_cents,
            currency: action.refund.currency.clone(),
        })
    }

    pub async fn preflight(
        &self,
        _action: &CanonicalAction,
        _credential: &ProviderCredential,
    ) -> Result<()> {
        Ok(())
    }

    pub async fn execute(
        &self,
        action: &CanonicalAction,
        _credential: &ProviderCredential,
    ) -> Result<ProviderExecution> {
        self.perform(&action.refund.mint_action_id.to_string(), action)
    }

    pub async fn reconcile(
        &self,
        action: &CanonicalAction,
        attempt: &ExecutionAttempt,
        _credential: &ProviderCredential,
    ) -> Result<ReconciliationResult> {
        let key = attempt.provider_idempotency_key.clone();
        if let Some(existing) = self.existing(&key)? {
            return Ok(ReconciliationResult {
                established: true,
                execution: Some(existing),
                failed: false,
            });
        }
        match self.perform(&key, action) {
            Ok(execution) => Ok(ReconciliationResult {
                established: true,
                execution: Some(execution),
                failed: false,
            }),
            Err(Error::ProviderRejected) => Ok(ReconciliationResult {
                established: true,
                execution: None,
                failed: true,
            }),
            Err(Error::ProviderTimeout) => Ok(ReconciliationResult {
                established: false,
                execution: None,
                failed: false,
            }),
            Err(other) => Err(other),
        }
    }

    pub fn redact(&self, value: &serde_json::Value) -> serde_json::Value {
        redact_value(value)
    }

    fn existing(&self, key: &str) -> Result<Option<ProviderExecution>> {
        let outcomes = self
            .outcomes
            .lock()
            .map_err(|err| Error::internal("fake outcomes", err))?;
        Ok(outcomes.get(key).cloned())
    }

    fn perform(&self, key_hint: &str, action: &CanonicalAction) -> Result<ProviderExecution> {
        let key = crate::domain::provider_idempotency_key(
            action.refund.mint_action_id,
            &action.operation,
        );
        let _ = key_hint;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let script = {
            let mut next = self
                .next
                .lock()
                .map_err(|err| Error::internal("fake script", err))?;
            next.take().unwrap_or(FakeScript::Success)
        };
        match script {
            FakeScript::Timeout => Err(Error::ProviderTimeout),
            FakeScript::Reject => Err(Error::ProviderRejected),
            FakeScript::Success => {
                let mut outcomes = self
                    .outcomes
                    .lock()
                    .map_err(|err| Error::internal("fake outcomes", err))?;
                if let Some(existing) = outcomes.get(&key) {
                    return Ok(existing.clone());
                }
                let resource_id = format!("re_fake_{}", action.refund.mint_action_id.simple());
                let execution = ProviderExecution {
                    provider_request_id: Some(format!(
                        "req_{}",
                        action.refund.mint_action_id.simple()
                    )),
                    provider_resource_id: resource_id,
                    redacted_payload: json!({
                        "id": format!("re_fake_{}", action.refund.mint_action_id.simple()),
                        "amount": action.refund.amount_cents,
                        "currency": action.refund.currency,
                        "status": "succeeded"
                    }),
                };
                outcomes.insert(key, execution.clone());
                self.effects.fetch_add(1, Ordering::SeqCst);
                Ok(execution)
            }
        }
    }
}
