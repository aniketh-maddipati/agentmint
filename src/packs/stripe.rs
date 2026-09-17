//! Stripe sandbox refund pack. Test-mode secrets only.
//! Used by: Pack::Stripe, optional sandbox tests.

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

    async fn create_refund(
        &self,
        action: &CanonicalAction,
        credential: &ProviderCredential,
        idempotency_key: &str,
    ) -> Result<ProviderExecution> {
        let mut form = vec![
            ("amount", action.refund.amount_cents.to_string()),
            ("currency", action.refund.currency.clone()),
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
            return Ok(ReconciliationResult {
                established: false,
                execution: None,
                failed: false,
            });
        }
        let payload: Value = response.json().await.map_err(|_| Error::UnknownOutcome)?;
        let Some(data) = payload.get("data").and_then(Value::as_array) else {
            return Ok(ReconciliationResult {
                established: false,
                execution: None,
                failed: false,
            });
        };
        let action_id = action.refund.mint_action_id.to_string();
        for item in data {
            let mint_id = item
                .pointer("/metadata/mint_action_id")
                .and_then(Value::as_str);
            if mint_id == Some(action_id.as_str()) {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or(Error::UnknownOutcome)?;
                return Ok(ReconciliationResult {
                    established: true,
                    execution: Some(ProviderExecution {
                        provider_request_id: None,
                        provider_resource_id: id.to_owned(),
                        redacted_payload: self.redact(item),
                    }),
                    failed: false,
                });
            }
        }
        Ok(ReconciliationResult {
            established: false,
            execution: None,
            failed: false,
        })
    }
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
