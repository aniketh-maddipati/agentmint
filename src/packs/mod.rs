//! Provider packs. Only Stripe refunds and a deterministic fake are implemented.
//! Used by: execution engine.

mod fake;
mod stripe;

pub use fake::{FakePack, FakeScript};
pub use stripe::{create_test_charge, retrieve_refund, StripePack};

use serde_json::Value;

use crate::credentials::ProviderCredential;
use crate::domain::ExecutionAttempt;
use crate::domain::{
    ActionIntent, CanonicalAction, ProviderExecution, ReconciliationResult, RiskClassification,
};
use crate::error::Result;

#[derive(Clone)]
pub enum Pack {
    Fake(std::sync::Arc<FakePack>),
    Stripe(StripePack),
}

impl Pack {
    pub fn canonicalize(&self, intent: &ActionIntent) -> Result<CanonicalAction> {
        match self {
            Self::Fake(pack) => pack.canonicalize(intent),
            Self::Stripe(pack) => pack.canonicalize(intent),
        }
    }

    pub fn classify(&self, action: &CanonicalAction) -> Result<RiskClassification> {
        match self {
            Self::Fake(pack) => pack.classify(action),
            Self::Stripe(pack) => pack.classify(action),
        }
    }

    pub async fn execute(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<ProviderExecution> {
        match self {
            Self::Fake(pack) => pack.execute(action, credential).await,
            Self::Stripe(pack) => pack.execute(action, credential).await,
        }
    }

    pub async fn preflight(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<()> {
        match self {
            Self::Fake(pack) => pack.preflight(action, credential).await,
            Self::Stripe(pack) => pack.preflight(action, credential).await,
        }
    }

    pub async fn reconcile(
        &self,
        action: &CanonicalAction,
        attempt: &ExecutionAttempt,
        credential: &ProviderCredential,
    ) -> Result<ReconciliationResult> {
        match self {
            Self::Fake(pack) => pack.reconcile(action, attempt, credential).await,
            Self::Stripe(pack) => pack.reconcile(action, attempt, credential).await,
        }
    }

    pub fn redact(&self, value: &Value) -> Value {
        match self {
            Self::Fake(pack) => pack.redact(value),
            Self::Stripe(pack) => pack.redact(value),
        }
    }

    pub fn fake(&self) -> Option<std::sync::Arc<FakePack>> {
        match self {
            Self::Fake(pack) => Some(pack.clone()),
            Self::Stripe(_) => None,
        }
    }
}

pub(crate) fn allowed_reason(reason: &str) -> bool {
    matches!(reason, "duplicate" | "fraudulent" | "requested_by_customer")
}

pub(crate) fn parse_refund_intent(
    intent: &ActionIntent,
    allow_fake_provider: bool,
) -> Result<crate::domain::RefundAction> {
    if intent.operation != crate::domain::REFUND_OPERATION {
        return Err(crate::error::Error::UnsupportedField("operation"));
    }
    if intent.provider == "stripe" || (allow_fake_provider && intent.provider == "fake") {
        // ok
    } else {
        return Err(crate::error::Error::UnsupportedField("provider"));
    }
    let resource_type = intent.resource.resource_type.as_str();
    if resource_type != "charge" && resource_type != "payment_intent" {
        return Err(crate::error::Error::UnsupportedField("resource.type"));
    }
    let object = intent
        .arguments
        .as_object()
        .ok_or(crate::error::Error::MalformedRefund(
            "arguments must be an object",
        ))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "amount" | "currency" | "reason") {
            return Err(crate::error::Error::UnsupportedField("arguments"));
        }
    }
    let amount = parse_positive_cents(object.get("amount"))?;
    let currency = object
        .get("currency")
        .and_then(Value::as_str)
        .ok_or(crate::error::Error::MalformedRefund("currency required"))?;
    if currency != "usd" {
        return Err(crate::error::Error::UnsupportedField("currency"));
    }
    let reason = object
        .get("reason")
        .and_then(Value::as_str)
        .ok_or(crate::error::Error::MalformedRefund("reason required"))?;
    if !allowed_reason(reason) {
        return Err(crate::error::Error::UnsupportedField("reason"));
    }
    let support_ticket_id =
        intent
            .context
            .support_ticket_id
            .clone()
            .ok_or(crate::error::Error::InvalidRequest(
                "supportTicketId required",
            ))?;
    Ok(crate::domain::RefundAction {
        charge_or_pi: intent.resource.resource_id.clone(),
        resource_type: intent.resource.resource_type.clone(),
        amount_cents: amount,
        currency: currency.to_owned(),
        reason: reason.to_owned(),
        mint_action_id: intent.action_id,
        support_ticket_id,
    })
}

pub(crate) fn parse_positive_cents(value: Option<&Value>) -> Result<i64> {
    let Some(value) = value else {
        return Err(crate::error::Error::MalformedRefund("amount required"));
    };
    if let Some(amount) = value.as_i64() {
        if amount <= 0 {
            return Err(crate::error::Error::MalformedRefund(
                "amount must be positive",
            ));
        }
        return Ok(amount);
    }
    if let Some(amount) = value.as_u64() {
        if amount == 0 || amount > i64::MAX as u64 {
            return Err(crate::error::Error::MalformedRefund("amount overflow"));
        }
        return Ok(amount as i64);
    }
    Err(crate::error::Error::MalformedRefund(
        "amount must be a positive integer",
    ))
}

pub(crate) fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let lowered = key.to_ascii_lowercase();
                if lowered.contains("secret")
                    || lowered.contains("authorization")
                    || lowered.contains("api_key")
                    || lowered == "key"
                {
                    out.insert(key.clone(), Value::String("[REDACTED]".into()));
                } else {
                    out.insert(key.clone(), redact_value(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ActionContext, ActionIntent, ActorIdentity, ResourceRef, INTENT_VERSION};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn intent(arguments: Value) -> ActionIntent {
        ActionIntent {
            version: INTENT_VERSION.to_owned(),
            action_id: Uuid::nil(),
            tenant_id: "acme".into(),
            actor: ActorIdentity {
                subject: "user_123".into(),
                agent_id: "support-agent-7".into(),
                delegated_by: None,
                issuer: "https://identity.example.com".into(),
            },
            provider: "stripe".into(),
            operation: "refund.create".into(),
            resource: ResourceRef {
                resource_type: "charge".into(),
                resource_id: "ch_123".into(),
            },
            arguments,
            context: ActionContext {
                support_ticket_id: Some("ticket_982".into()),
                reason: Some("duplicate".into()),
            },
            idempotency_key: "k".into(),
            created_at: Utc::now(),
            expires_at: Utc::now(),
        }
    }

    #[test]
    fn malformed_and_overflowing_amounts_fail_closed() {
        let cases = [
            json!({"currency":"usd","reason":"duplicate"}),
            json!({"amount":0,"currency":"usd","reason":"duplicate"}),
            json!({"amount":-1,"currency":"usd","reason":"duplicate"}),
            json!({"amount":1.5,"currency":"usd","reason":"duplicate"}),
            json!({"amount":"4200","currency":"usd","reason":"duplicate"}),
            json!({"amount":18446744073709551615u64,"currency":"usd","reason":"duplicate"}),
        ];
        for arguments in cases {
            assert!(parse_refund_intent(&intent(arguments), false).is_err());
        }
        assert!(parse_refund_intent(
            &intent(json!({"amount":4200,"currency":"usd","reason":"duplicate"})),
            false
        )
        .is_ok());
    }
}
