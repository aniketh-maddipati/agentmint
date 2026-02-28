//! Metrics tracking.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Metrics {
    pub tokens_minted: AtomicU64,
    pub tokens_verified: AtomicU64,
    pub tokens_rejected: AtomicU64,
    pub replays_blocked: AtomicU64,
    pub policy_denials: AtomicU64,
    pub oidc_failures: AtomicU64,
    pub rate_limited: AtomicU64,
    pub webauthn_registers: AtomicU64,
    pub webauthn_successes: AtomicU64,
    pub webauthn_failures: AtomicU64,
    pub webauthn_lockouts: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            tokens_minted: AtomicU64::new(0),
            tokens_verified: AtomicU64::new(0),
            tokens_rejected: AtomicU64::new(0),
            replays_blocked: AtomicU64::new(0),
            policy_denials: AtomicU64::new(0),
            oidc_failures: AtomicU64::new(0),
            rate_limited: AtomicU64::new(0),
            webauthn_registers: AtomicU64::new(0),
            webauthn_successes: AtomicU64::new(0),
            webauthn_failures: AtomicU64::new(0),
            webauthn_lockouts: AtomicU64::new(0),
        }
    }

    pub fn record_mint(&self) {
        self.tokens_minted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_verify(&self) {
        self.tokens_verified.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_reject(&self) {
        self.tokens_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_replay(&self) {
        self.replays_blocked.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_policy_denial(&self) {
        self.policy_denials.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_oidc_failure(&self) {
        self.oidc_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rate_limited(&self) {
        self.rate_limited.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webauthn_register(&self) {
        self.webauthn_registers.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webauthn_success(&self) {
        self.webauthn_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webauthn_failure(&self) {
        self.webauthn_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webauthn_lockout(&self) {
        self.webauthn_lockouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            tokens_minted: self.tokens_minted.load(Ordering::Relaxed),
            tokens_verified: self.tokens_verified.load(Ordering::Relaxed),
            tokens_rejected: self.tokens_rejected.load(Ordering::Relaxed),
            replays_blocked: self.replays_blocked.load(Ordering::Relaxed),
            policy_denials: self.policy_denials.load(Ordering::Relaxed),
            oidc_failures: self.oidc_failures.load(Ordering::Relaxed),
            rate_limited: self.rate_limited.load(Ordering::Relaxed),
            webauthn_registers: self.webauthn_registers.load(Ordering::Relaxed),
            webauthn_successes: self.webauthn_successes.load(Ordering::Relaxed),
            webauthn_failures: self.webauthn_failures.load(Ordering::Relaxed),
            webauthn_lockouts: self.webauthn_lockouts.load(Ordering::Relaxed),
        }
    }
}

#[derive(Serialize)]
pub struct MetricsSnapshot {
    pub tokens_minted: u64,
    pub tokens_verified: u64,
    pub tokens_rejected: u64,
    pub replays_blocked: u64,
    pub policy_denials: u64,
    pub oidc_failures: u64,
    pub rate_limited: u64,
    pub webauthn_registers: u64,
    pub webauthn_successes: u64,
    pub webauthn_failures: u64,
    pub webauthn_lockouts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_start_at_zero() {
        let s = Metrics::new().snapshot();
        assert_eq!(s.tokens_minted, 0);
        assert_eq!(s.rate_limited, 0);
        assert_eq!(s.webauthn_successes, 0);
    }

    #[test]
    fn record_rate_limited_increments() {
        let m = Metrics::new();
        m.record_rate_limited();
        assert_eq!(m.snapshot().rate_limited, 1);
    }

    #[test]
    fn record_webauthn_success_increments() {
        let m = Metrics::new();
        m.record_webauthn_success();
        assert_eq!(m.snapshot().webauthn_successes, 1);
    }
}