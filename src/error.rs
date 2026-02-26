//! Unified error types for AgentMint.
//! Used by: token, jti, audit, handlers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("token expired")]
    TokenExpired,

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid token format: {0}")]
    InvalidToken(String),

    #[error("token already used (jti: {0})")]
    ReplayDetected(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("signing error: {0}")]
    Signing(String),
}

fn client_message(err: &Error) -> String {
    match err {
        Error::TokenExpired => "token expired".into(),
        Error::InvalidSignature => "invalid signature".into(),
        Error::InvalidToken(_) => "invalid token".into(),
        Error::ReplayDetected(_) => "token rejected".into(),
        Error::Validation(msg) => msg.clone(),
        Error::ServiceUnavailable(_) => "service temporarily unavailable".into(),
        Error::Database(_) => "internal error".into(),
        Error::Serialization(_) => "invalid request body".into(),
        Error::Base64(_) => "invalid encoding".into(),
        Error::Signing(_) => "internal error".into(),
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Error::TokenExpired | Error::InvalidSignature | Error::InvalidToken(_) => {
                StatusCode::UNAUTHORIZED
            }
            Error::ReplayDetected(_) => StatusCode::CONFLICT,
            Error::Validation(_) | Error::Base64(_) => StatusCode::BAD_REQUEST,
            Error::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Error::Database(_) | Error::Serialization(_) | Error::Signing(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        tracing::warn!(error = %self, status = %status.as_u16(), "request failed");
        (status, client_message(&self)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn lock_err<T>(msg: &str) -> impl FnOnce(std::sync::PoisonError<T>) -> Error + '_ {
    move |_| Error::Signing(format!("{msg} lock poisoned"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_status(err: Error, expected: StatusCode) {
        assert_eq!(err.into_response().status(), expected);
    }

    #[test]
    fn status_code_mapping() {
        assert_status(Error::TokenExpired, StatusCode::UNAUTHORIZED);
        assert_status(Error::InvalidSignature, StatusCode::UNAUTHORIZED);
        assert_status(Error::InvalidToken("x".into()), StatusCode::UNAUTHORIZED);
        assert_status(Error::ReplayDetected("x".into()), StatusCode::CONFLICT);
        assert_status(Error::Validation("x".into()), StatusCode::BAD_REQUEST);
        assert_status(Error::Base64(base64::DecodeError::InvalidLength(3)), StatusCode::BAD_REQUEST);
        assert_status(Error::ServiceUnavailable("x".into()), StatusCode::SERVICE_UNAVAILABLE);
        assert_status(Error::Signing("x".into()), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        assert_eq!(client_message(&Error::Database(rusqlite::Error::QueryReturnedNoRows)), "internal error");
        assert_eq!(client_message(&Error::Signing("secret".into())), "internal error");
    }
}
