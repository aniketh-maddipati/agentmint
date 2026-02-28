//! Unified error types with secure client messages.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("token expired")]
    TokenExpired,

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid token: {0}")]
    InvalidToken(String),

    #[error("replay: {0}")]
    ReplayDetected(String),

    #[error("policy: {0}")]
    PolicyViolation(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("rate limited: {0}")]
    RateLimited(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("db: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("json: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("signing: {0}")]
    Signing(String),
}

impl Error {
    fn status(&self) -> StatusCode {
        match self {
            Self::TokenExpired | Self::InvalidSignature | Self::InvalidToken(_) | Self::Unauthorized(_) => {
                StatusCode::UNAUTHORIZED
            }
            Self::ReplayDetected(_) => StatusCode::CONFLICT,
            Self::PolicyViolation(_) => StatusCode::FORBIDDEN,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Validation(_) | Self::Base64(_) => StatusCode::BAD_REQUEST,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Database(_) | Self::Serialization(_) | Self::Signing(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Safe message for clients - never leak internals
    fn client_msg(&self) -> &'static str {
        match self {
            Self::TokenExpired => "token expired",
            Self::InvalidSignature => "invalid signature",
            Self::InvalidToken(_) => "invalid token",
            Self::ReplayDetected(_) => "token already used",
            Self::PolicyViolation(_) => "policy violation",
            Self::Unauthorized(_) => "unauthorized",
            Self::RateLimited(_) => "rate limited",
            Self::Validation(_) => "invalid request",
            Self::ServiceUnavailable(_) => "service unavailable",
            Self::Database(_) | Self::Serialization(_) | Self::Signing(_) | Self::Base64(_) => "internal error",
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        tracing::warn!(error = %self, status = %status.as_u16(), "request failed");
        (status, self.client_msg()).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn lock_err<T>(name: &str) -> impl FnOnce(std::sync::PoisonError<T>) -> Error + '_ {
    move |_| Error::Signing(format!("{name} lock poisoned"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes() {
        assert_eq!(Error::TokenExpired.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(Error::ReplayDetected("x".into()).status(), StatusCode::CONFLICT);
        assert_eq!(Error::PolicyViolation("x".into()).status(), StatusCode::FORBIDDEN);
        assert_eq!(Error::RateLimited("x".into()).status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(Error::ServiceUnavailable("x".into()).status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn no_internal_leak() {
        assert_eq!(Error::Database(rusqlite::Error::QueryReturnedNoRows).client_msg(), "internal error");
        assert_eq!(Error::Signing("secret key".into()).client_msg(), "internal error");
    }
}
