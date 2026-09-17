//! Lab errors with client-safe Display messages.
//! Used by: lab modules and CLI.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum LabError {
    NotFound(String),
    Invalid(String),
    Conflict(String),
    Cancelled,
    Paused,
    Unverified(String),
    Storage(String),
    Serialization(String),
    Io(String),
    Invariant(String),
}

pub type LabResult<T> = Result<T, LabError>;

impl fmt::Display for LabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::Invalid(msg) => write!(f, "invalid: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::Cancelled => write!(f, "case cancelled"),
            Self::Paused => write!(f, "case paused"),
            Self::Unverified(msg) => write!(f, "unverified: {msg}"),
            Self::Storage(msg) => write!(f, "storage error: {msg}"),
            Self::Serialization(msg) => write!(f, "serialization error: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
            Self::Invariant(msg) => write!(f, "invariant failed: {msg}"),
        }
    }
}

impl std::error::Error for LabError {}

impl From<rusqlite::Error> for LabError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<serde_json::Error> for LabError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

impl From<io::Error> for LabError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_client_safe() {
        let err = LabError::NotFound("run".into());
        assert!(err.to_string().contains("not found"));
    }
}
