//! Narrow development/test failpoints. Compiled out of release builds.
//! Used by: execution engine and API extractors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failpoint {
    BeforeProvider,
    AfterProviderSuccess,
    AfterCommitBeforeResponse,
}

pub fn enabled() -> bool {
    cfg!(debug_assertions)
}

pub fn parse_name(value: &str) -> Option<Failpoint> {
    if !enabled() {
        return None;
    }
    match value {
        "before_provider" => Some(Failpoint::BeforeProvider),
        "after_provider_success" => Some(Failpoint::AfterProviderSuccess),
        "after_commit_before_response" => Some(Failpoint::AfterCommitBeforeResponse),
        _ => None,
    }
}
