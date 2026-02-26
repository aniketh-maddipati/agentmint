//! Unified error types for agentmint.
//! Used by: token, jti, audit, handlers.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("signing failed: {0}")]
    Sign(String),

    #[error("verification failed: {0}")]
    Verify(String),

    #[error("token expired")]
    Expired,

    #[error("token not yet valid")]
    NotYetValid,

    #[error("replay detected: jti {0}")]
    Replay(String),

    #[error("audit error: {0}")]
    Audit(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        let cases = vec![
            (Error::Sign("bad key".into()), "signing failed: bad key"),
            (
                Error::Verify("invalid sig".into()),
                "verification failed: invalid sig",
            ),
            (Error::Expired, "token expired"),
            (Error::NotYetValid, "token not yet valid"),
            (
                Error::Replay("abc-123".into()),
                "replay detected: jti abc-123",
            ),
            (Error::Audit("write failed".into()), "audit error: write failed"),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("{bad")
            .expect_err("should fail to parse");
        let err = Error::from(json_err);
        assert!(matches!(err, Error::Serialization(_)));
    }

    #[test]
    fn result_alias() {
        let ok: Result<i32> = Ok(42);
        assert!(ok.is_ok());

        let err: Result<i32> = Err(Error::Expired);
        assert!(err.is_err());
    }
}
