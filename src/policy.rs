//! Deliberately small Stripe-refund policy and optional fail-closed HTTP PDP.
//! Used by: propose path.

use serde::Deserialize;
use serde_json::json;

use crate::config::{Config, PolicyKind};
use crate::domain::{CanonicalAction, PolicyDecision, PolicyEffect};
use crate::error::{Error, Result};

pub enum PolicyProvider {
    StripeThreshold(StripeThresholdPolicy),
    Http(HttpPolicy),
}

impl PolicyProvider {
    pub fn from_config(config: &Config, http: reqwest::Client) -> Result<Self> {
        match config.policy {
            PolicyKind::Threshold => Ok(Self::StripeThreshold(StripeThresholdPolicy::from_config(
                config,
            ))),
            PolicyKind::Http => {
                let url = config
                    .pdp_url
                    .clone()
                    .ok_or(Error::Misconfigured("MINT_PDP_URL required"))?;
                Ok(Self::Http(HttpPolicy {
                    url,
                    http,
                    fallback: StripeThresholdPolicy::from_config(config),
                }))
            }
        }
    }

    pub async fn decide(&self, action: &CanonicalAction) -> Result<PolicyDecision> {
        match self {
            Self::StripeThreshold(policy) => policy.decide(action),
            Self::Http(policy) => policy.decide(action).await,
        }
    }
}

#[derive(Clone)]
pub struct StripeThresholdPolicy {
    pub auto_cents: i64,
    pub approval_cents: i64,
    pub policy_version: String,
}

impl StripeThresholdPolicy {
    pub fn from_config(config: &Config) -> Self {
        Self {
            auto_cents: config.auto_cents,
            approval_cents: config.approval_cents,
            policy_version: config.policy_version.clone(),
        }
    }

    pub fn decide(&self, action: &CanonicalAction) -> Result<PolicyDecision> {
        let amount = action.refund.amount_cents;
        if amount <= 0 {
            return Err(Error::MalformedRefund("amount must be positive"));
        }
        if amount <= self.auto_cents {
            return Ok(PolicyDecision {
                effect: PolicyEffect::Automatic,
                policy_version: self.policy_version.clone(),
                reason: format!("refund <= {} cents automatic", self.auto_cents),
            });
        }
        if amount <= self.approval_cents {
            return Ok(PolicyDecision {
                effect: PolicyEffect::ApprovalRequired,
                policy_version: self.policy_version.clone(),
                reason: format!(
                    "refund {}-{} cents requires approval",
                    self.auto_cents + 1,
                    self.approval_cents
                ),
            });
        }
        Ok(PolicyDecision {
            effect: PolicyEffect::Deny,
            policy_version: self.policy_version.clone(),
            reason: format!("refund > {} cents denied", self.approval_cents),
        })
    }
}

pub struct HttpPolicy {
    url: String,
    http: reqwest::Client,
    fallback: StripeThresholdPolicy,
}

#[derive(Deserialize)]
struct PdpResponse {
    decision: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    policy_version: Option<String>,
}

impl HttpPolicy {
    async fn decide(&self, action: &CanonicalAction) -> Result<PolicyDecision> {
        let body = json!({
            "canonical_json": action.canonical_json,
            "intent_hash": action.intent_hash,
            "provider": action.provider,
            "operation": action.operation,
            "amount_cents": action.refund.amount_cents,
            "currency": action.refund.currency,
        });
        let response = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|_| Error::PolicyDenied)?;
        if !response.status().is_success() {
            return Err(Error::PolicyDenied);
        }
        let parsed: PdpResponse = response.json().await.map_err(|_| Error::PolicyDenied)?;
        let effect = match parsed.decision.as_str() {
            "automatic" => PolicyEffect::Automatic,
            "approval_required" => PolicyEffect::ApprovalRequired,
            "deny" => PolicyEffect::Deny,
            _ => return Err(Error::PolicyDenied),
        };
        Ok(PolicyDecision {
            effect,
            policy_version: parsed
                .policy_version
                .unwrap_or_else(|| self.fallback.policy_version.clone()),
            reason: parsed.reason.unwrap_or_else(|| "external pdp".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RefundAction, CANONICALIZATION_VERSION};
    use uuid::Uuid;

    fn action(amount: i64) -> CanonicalAction {
        CanonicalAction {
            canonicalization_version: CANONICALIZATION_VERSION.to_owned(),
            canonical_json: "{}".into(),
            intent_hash: "sha256:x".into(),
            provider: "stripe".into(),
            operation: "refund.create".into(),
            refund: RefundAction {
                charge_or_pi: "ch_123".into(),
                resource_type: "charge".into(),
                amount_cents: amount,
                currency: "usd".into(),
                reason: "duplicate".into(),
                mint_action_id: Uuid::nil(),
                support_ticket_id: "ticket_982".into(),
            },
        }
    }

    #[test]
    fn threshold_policy_splits_automatic_approval_and_deny() {
        let policy = StripeThresholdPolicy {
            auto_cents: 5000,
            approval_cents: 50_000,
            policy_version: "stripe-refund-thresholds-v1".into(),
        };
        assert_eq!(
            policy.decide(&action(4200)).expect("auto").effect,
            PolicyEffect::Automatic
        );
        assert_eq!(
            policy.decide(&action(5000)).expect("auto").effect,
            PolicyEffect::Automatic
        );
        assert_eq!(
            policy.decide(&action(5001)).expect("appr").effect,
            PolicyEffect::ApprovalRequired
        );
        assert_eq!(
            policy.decide(&action(50_000)).expect("appr").effect,
            PolicyEffect::ApprovalRequired
        );
        assert_eq!(
            policy.decide(&action(50_001)).expect("deny").effect,
            PolicyEffect::Deny
        );
    }
}
