//! Stripe sandbox refund pack. Test-mode secrets only.
//! Create-refund never sends currency; charge/PI preflight validates livemode,
//! currency match, and remaining refundable balance before any effect.

use reqwest::Client;
use serde_json::Value;

use crate::canonical::hashed_canonical;
use crate::credentials::{assert_test_secret, ProviderCredential};
use crate::domain::ExecutionAttempt;
use crate::domain::{
    ActionIntent, CanonicalAction, ProviderExecution, ReconciliationResult, RiskClassification,
};
use crate::error::{Error, Result};
use crate::packs::{parse_refund_intent, redact_value};

#[derive(Clone)]
pub struct StripePack {
    http: Client,
}

impl StripePack {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub fn canonicalize(&self, intent: &ActionIntent) -> Result<CanonicalAction> {
        let refund = parse_refund_intent(intent, false)?;
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
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<()> {
        assert_test_secret(&credential.secret)?;
        let charge = self.resolve_refundable_charge(action, credential).await?;
        validate_charge_for_refund(&charge, action)?;
        Ok(())
    }

    pub async fn execute(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<ProviderExecution> {
        assert_test_secret(&credential.secret)?;
        let key = crate::domain::provider_idempotency_key(
            action.refund.mint_action_id,
            &action.operation,
        );
        self.create_refund(action, credential, &key).await
    }

    pub async fn reconcile(
        &self,
        action: &CanonicalAction,
        attempt: &ExecutionAttempt,
        credential: &ProviderCredential,
    ) -> Result<ReconciliationResult> {
        assert_test_secret(&credential.secret)?;
        match self
            .create_refund(action, credential, &attempt.provider_idempotency_key)
            .await
        {
            Ok(execution) => {
                return Ok(ReconciliationResult {
                    established: true,
                    execution: Some(execution),
                    failed: false,
                })
            }
            Err(Error::ProviderRejected) => {
                return Ok(ReconciliationResult {
                    established: true,
                    execution: None,
                    failed: true,
                })
            }
            Err(Error::ProviderTimeout) | Err(Error::UnknownOutcome) => {}
            Err(other) => return Err(other),
        }
        self.lookup_refund(action, credential).await
    }

    pub fn redact(&self, value: &Value) -> Value {
        redact_value(value)
    }

    async fn resolve_refundable_charge(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<Value> {
        if action.refund.resource_type == "payment_intent" {
            let pi = self
                .get_json(
                    &format!(
                        "https://api.stripe.com/v1/payment_intents/{}",
                        action.refund.charge_or_pi
                    ),
                    credential,
                )
                .await?;
            if pi.get("livemode").and_then(Value::as_bool) != Some(false) {
                return Err(Error::LiveStripeRefused);
            }
            let charge_id =
                pi.get("latest_charge")
                    .and_then(Value::as_str)
                    .ok_or(Error::MalformedRefund(
                        "payment_intent has no unambiguous refundable charge",
                    ))?;
            return self
                .get_json(
                    &format!("https://api.stripe.com/v1/charges/{charge_id}"),
                    credential,
                )
                .await;
        }
        self.get_json(
            &format!(
                "https://api.stripe.com/v1/charges/{}",
                action.refund.charge_or_pi
            ),
            credential,
        )
        .await
    }

    async fn get_json(&self, url: &str, credential: &ProviderCredential) -> Result<Value> {
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", credential.secret))
            .send()
            .await
            .map_err(|_| Error::ProviderTimeout)?;
        let status = response.status();
        let payload: Value = response.json().await.map_err(|_| Error::UnknownOutcome)?;
        if status.is_success() {
            return Ok(payload);
        }
        if status.is_client_error() {
            return Err(Error::ProviderRejected);
        }
        Err(Error::UnknownOutcome)
    }

    async fn create_refund(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
        idempotency_key: &str,
    ) -> Result<ProviderExecution> {
        let mut form = vec![
            ("amount", action.refund.amount_cents.to_string()),
            ("reason", action.refund.reason.clone()),
            (
                "metadata[mint_action_id]",
                action.refund.mint_action_id.to_string(),
            ),
            (
                "metadata[support_ticket_id]",
                action.refund.support_ticket_id.clone(),
            ),
        ];
        if action.refund.resource_type == "payment_intent" {
            form.push(("payment_intent", action.refund.charge_or_pi.clone()));
        } else {
            form.push(("charge", action.refund.charge_or_pi.clone()));
        }
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", encode_form(k), encode_form(v)))
            .collect::<Vec<_>>()
            .join("&");
        let response = self
            .http
            .post("https://api.stripe.com/v1/refunds")
            .header("Authorization", format!("Bearer {}", credential.secret))
            .header("Idempotency-Key", idempotency_key)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| Error::ProviderTimeout)?;
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let status = response.status();
        let payload: Value = response.json().await.map_err(|_| Error::UnknownOutcome)?;
        if status.is_success() {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .ok_or(Error::UnknownOutcome)?;
            return Ok(ProviderExecution {
                provider_request_id: request_id,
                provider_resource_id: id.to_owned(),
                redacted_payload: self.redact(&payload),
            });
        }
        if status.is_client_error() {
            tracing::warn!(status = %status, payload = %self.redact(&payload), "stripe rejected refund");
            return Err(Error::ProviderRejected);
        }
        Err(Error::UnknownOutcome)
    }

    async fn lookup_refund(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<ReconciliationResult> {
        let matches = self.list_refunds_for_action(action, credential).await?;
        if matches.len() > 1 {
            tracing::error!(
                action_id = %action.refund.mint_action_id,
                count = matches.len(),
                "multiple stripe refunds share mint_action_id"
            );
            return Err(Error::UnknownOutcome);
        }
        if let Some(item) = matches.into_iter().next() {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or(Error::UnknownOutcome)?;
            return Ok(ReconciliationResult {
                established: true,
                execution: Some(ProviderExecution {
                    provider_request_id: None,
                    provider_resource_id: id.to_owned(),
                    redacted_payload: self.redact(&item),
                }),
                failed: false,
            });
        }
        Ok(ReconciliationResult {
            established: false,
            execution: None,
            failed: false,
        })
    }

    pub async fn list_refunds_for_action(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
    ) -> Result<Vec<Value>> {
        let param = if action.refund.resource_type == "payment_intent" {
            ("payment_intent", action.refund.charge_or_pi.as_str())
        } else {
            ("charge", action.refund.charge_or_pi.as_str())
        };
        let response = self
            .http
            .get("https://api.stripe.com/v1/refunds")
            .header("Authorization", format!("Bearer {}", credential.secret))
            .query(&[param, ("limit", "100")])
            .send()
            .await
            .map_err(|_| Error::UnknownOutcome)?;
        if !response.status().is_success() {
            return Err(Error::UnknownOutcome);
        }
        let payload: Value = response.json().await.map_err(|_| Error::UnknownOutcome)?;
        let Some(data) = payload.get("data").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        let action_id = action.refund.mint_action_id.to_string();
        Ok(data
            .iter()
            .filter(|item| {
                item.pointer("/metadata/mint_action_id")
                    .and_then(Value::as_str)
                    == Some(action_id.as_str())
            })
            .cloned()
            .collect())
    }
}

pub fn validate_charge_for_refund(charge: &Value, action: &CanonicalAction) -> Result<()> {
    if charge.get("livemode").and_then(Value::as_bool) != Some(false) {
        return Err(Error::LiveStripeRefused);
    }
    let currency = charge
        .get("currency")
        .and_then(Value::as_str)
        .ok_or(Error::MalformedRefund("charge currency missing"))?;
    if currency != action.refund.currency {
        return Err(Error::MalformedRefund(
            "authorized currency does not match charge currency",
        ));
    }
    if charge.get("paid").and_then(Value::as_bool) != Some(true) {
        return Err(Error::MalformedRefund("charge is not paid"));
    }
    if charge.get("captured").and_then(Value::as_bool) == Some(false) {
        return Err(Error::MalformedRefund("charge is not captured"));
    }
    let amount = charge
        .get("amount")
        .and_then(Value::as_i64)
        .ok_or(Error::MalformedRefund("charge amount missing"))?;
    let amount_refunded = charge
        .get("amount_refunded")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let remaining = amount.saturating_sub(amount_refunded);
    if action.refund.amount_cents > remaining {
        return Err(Error::MalformedRefund(
            "refund exceeds remaining refundable amount",
        ));
    }
    Ok(())
}

fn encode_form(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub async fn create_test_charge(http: &Client, secret: &str, amount_cents: i64) -> Result<String> {
    assert_test_secret(secret)?;
    let body = format!(
        "amount={}&currency=usd&source=tok_visa",
        encode_form(&amount_cents.to_string())
    );
    let response = http
        .post("https://api.stripe.com/v1/charges")
        .header("Authorization", format!("Bearer {secret}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| Error::ProviderTimeout)?;
    let payload: Value = response.json().await.map_err(|_| Error::UnknownOutcome)?;
    if payload.get("livemode").and_then(Value::as_bool) == Some(true) {
        return Err(Error::LiveStripeRefused);
    }
    payload
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or(Error::ProviderRejected)
}

pub async fn retrieve_refund(http: &Client, secret: &str, refund_id: &str) -> Result<Value> {
    assert_test_secret(secret)?;
    let response = http
        .get(format!("https://api.stripe.com/v1/refunds/{refund_id}"))
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .map_err(|_| Error::ProviderTimeout)?;
    if !response.status().is_success() {
        return Err(Error::ProviderRejected);
    }
    response.json().await.map_err(|_| Error::UnknownOutcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RefundAction, CANONICALIZATION_VERSION};
    use serde_json::json;
    use uuid::Uuid;

    fn action(amount: i64, currency: &str) -> CanonicalAction {
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
                currency: currency.into(),
                reason: "duplicate".into(),
                mint_action_id: Uuid::nil(),
                support_ticket_id: "ticket_982".into(),
            },
        }
    }

    #[test]
    fn charge_preflight_rejects_live_currency_mismatch_and_over_refund() {
        let live = json!({"livemode": true, "currency": "usd", "paid": true, "captured": true, "amount": 5000, "amount_refunded": 0});
        assert!(validate_charge_for_refund(&live, &action(100, "usd")).is_err());

        let mismatch = json!({"livemode": false, "currency": "eur", "paid": true, "captured": true, "amount": 5000, "amount_refunded": 0});
        assert!(validate_charge_for_refund(&mismatch, &action(100, "usd")).is_err());

        let over = json!({"livemode": false, "currency": "usd", "paid": true, "captured": true, "amount": 5000, "amount_refunded": 2000});
        assert!(validate_charge_for_refund(&over, &action(4000, "usd")).is_err());
        assert!(validate_charge_for_refund(&over, &action(3000, "usd")).is_ok());
    }
}
