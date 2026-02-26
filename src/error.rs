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

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("signing error: {0}")]
    Signing(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Error::TokenExpired | Error::InvalidSignature | Error::InvalidToken(_) => {
                StatusCode::UNAUTHORIZED
            }
            Error::ReplayDetected(_) => StatusCode::CONFLICT,
            Error::Database(_) | Error::Serialization(_) | Error::Signing(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Error::Base64(_) => StatusCode::BAD_REQUEST,
        };
        (status, self.to_string()).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expired_returns_401() {
        let response = Error::TokenExpired.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn invalid_signature_returns_401() {
        let response = Error::InvalidSignature.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn invalid_token_returns_401() {
        let response = Error::InvalidToken("bad".into()).into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn replay_detected_returns_409() {
        let response = Error::ReplayDetected("abc-123".into()).into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn base64_error_returns_400() {
        let err = base64::DecodeError::InvalidLength(3);
        let response = Error::Base64(err).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn signing_error_returns_500() {
        let response = Error::Signing("key failure".into()).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert_eq!(Error::TokenExpired.to_string(), "token expired");
        assert_eq!(Error::InvalidSignature.to_string(), "invalid signature");
        assert_eq!(
            Error::ReplayDetected("jti-1".into()).to_string(),
            "token already used (jti: jti-1)"
        );
    }
}
