//! Versioned JSON errors with client-safe messages.
//! Used by: API handlers and the execution engine.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("cross-tenant access denied")]
    CrossTenant,
    #[error("invalid request")]
    InvalidRequest(&'static str),
    #[error("malformed refund")]
    MalformedRefund(&'static str),
    #[error("unsupported field")]
    UnsupportedField(&'static str),
    #[error("intent hash mismatch")]
    IntentHashMismatch,
    #[error("authorization expired")]
    AuthorizationExpired,
    #[error("not authorized to execute")]
    NotExecutable,
    #[error("denied by policy")]
    PolicyDenied,
    #[error("approval required")]
    ApprovalRequired,
    #[error("unknown outcome")]
    UnknownOutcome,
    #[error("reconciliation required")]
    ReconciliationRequired,
    #[error("live Stripe keys are refused")]
    LiveStripeRefused,
    #[error("self-approval refused")]
    SelfApproval,
    #[error("provider rejected the request")]
    ProviderRejected,
    #[error("provider timeout")]
    ProviderTimeout,
    #[error("identity verification failed")]
    IdentityFailed,
    #[error("unknown receipt format")]
    UnknownReceiptFormat,
    #[error("unknown key id")]
    UnknownKeyId,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("failpoint")]
    Failpoint(&'static str),
    #[error("conflict")]
    Conflict,
    #[error("internal error")]
    Internal,
    #[error("{0}")]
    Misconfigured(&'static str),
}

impl Error {
    pub fn internal(context: &str, err: impl std::fmt::Display) -> Self {
        tracing::error!(context, error = %err, "internal error");
        Self::Internal
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden | Self::CrossTenant => "forbidden",
            Self::NotFound => "not_found",
            Self::InvalidRequest(_) | Self::MalformedRefund(_) | Self::UnsupportedField(_) => {
                "invalid_request"
            }
            Self::IntentHashMismatch => "intent_hash_mismatch",
            Self::AuthorizationExpired => "authorization_expired",
            Self::NotExecutable => "not_executable",
            Self::PolicyDenied => "policy_denied",
            Self::ApprovalRequired => "approval_required",
            Self::UnknownOutcome => "unknown_outcome",
            Self::ReconciliationRequired => "reconciliation_required",
            Self::LiveStripeRefused => "live_stripe_refused",
            Self::SelfApproval => "self_approval",
            Self::ProviderRejected => "provider_rejected",
            Self::ProviderTimeout => "provider_timeout",
            Self::IdentityFailed => "identity_failed",
            Self::UnknownReceiptFormat => "unknown_receipt_format",
            Self::UnknownKeyId => "unknown_key_id",
            Self::InvalidSignature => "invalid_signature",
            Self::Failpoint(_) => "failpoint",
            Self::Conflict => "conflict",
            Self::Internal => "internal",
            Self::Misconfigured(_) => "misconfigured",
        }
    }

    fn client_message(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "authentication required",
            Self::Forbidden | Self::CrossTenant => "access denied",
            Self::NotFound => "action not found",
            Self::InvalidRequest(_) => "invalid request",
            Self::MalformedRefund(_) => "malformed or unsupported refund amount",
            Self::UnsupportedField(_) => "unsupported field",
            Self::IntentHashMismatch => "authorization is bound to a different action",
            Self::AuthorizationExpired => "authorization has expired",
            Self::NotExecutable => "action is not authorized for execution",
            Self::PolicyDenied => "policy denied this action",
            Self::ApprovalRequired => "authenticated approval is required",
            Self::UnknownOutcome => "provider outcome is unknown and requires reconciliation",
            Self::ReconciliationRequired => "reconcile this action before retrying execution",
            Self::LiveStripeRefused => "live Stripe credentials are refused",
            Self::SelfApproval => "approver must differ from the originating actor",
            Self::ProviderRejected => "provider rejected the request",
            Self::ProviderTimeout => "provider timed out",
            Self::IdentityFailed => "identity verification failed",
            Self::UnknownReceiptFormat => "unknown receipt format",
            Self::UnknownKeyId => "unknown signing key id",
            Self::InvalidSignature => "invalid signature",
            Self::Failpoint(_) => "injected failure",
            Self::Conflict => "conflicting action state",
            Self::Internal | Self::Misconfigured(_) => "internal error",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::UnknownOutcome | Self::ReconciliationRequired | Self::ProviderTimeout
        )
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated | Self::IdentityFailed => StatusCode::UNAUTHORIZED,
            Self::Forbidden
            | Self::CrossTenant
            | Self::PolicyDenied
            | Self::AuthorizationExpired
            | Self::SelfApproval => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::InvalidRequest(_)
            | Self::MalformedRefund(_)
            | Self::UnsupportedField(_)
            | Self::UnknownReceiptFormat
            | Self::UnknownKeyId
            | Self::InvalidSignature
            | Self::LiveStripeRefused => StatusCode::BAD_REQUEST,
            Self::IntentHashMismatch
            | Self::NotExecutable
            | Self::ApprovalRequired
            | Self::Conflict
            | Self::UnknownOutcome
            | Self::ReconciliationRequired
            | Self::ProviderRejected
            | Self::ProviderTimeout
            | Self::Failpoint(_) => StatusCode::CONFLICT,
            Self::Internal | Self::Misconfigured(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        tracing::warn!(code = self.code(), status = %status.as_u16(), "request failed");
        let body = json!({
            "error": {
                "code": self.code(),
                "message": self.client_message(),
                "retryable": self.retryable()
            }
        });
        (status, Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_do_not_leak_details() {
        let error = Error::internal("db", "secret connection string");
        assert_eq!(error.client_message(), "internal error");
        assert_eq!(error.code(), "internal");
    }
}
