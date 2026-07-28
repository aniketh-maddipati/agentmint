//! AERF crypto-core primitives and verifiers.
//! Used by: conformance tests and future verifier integration.

pub mod adapters;
pub mod canonical;
pub mod chain;
pub mod gate;
pub mod intervention;
pub mod merkle;
pub mod receipt;
pub mod reconstruct;
pub mod sign;

use thiserror::Error;

pub type AerfResult<T> = Result<T, AerfError>;

#[derive(Debug, Error)]
pub enum AerfError {
    #[error("{message} at JSON path {path}")]
    CanonicalNumber { path: String, message: String },
    #[error("duplicate object key after NFC normalization at JSON path {path}: {key}")]
    DuplicateNormalizedKey { path: String, key: String },
    #[error("unsupported JSON value at path {path}: {kind}")]
    UnsupportedValue { path: String, kind: String },
    #[error("invalid hexadecimal value for {field}")]
    InvalidHex { field: &'static str },
    #[error("invalid signature for {field}")]
    InvalidSignature { field: &'static str },
    #[error("invalid public key PEM: {0}")]
    InvalidPublicKey(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
