//! Metrics tracking.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Metrics {
    pub tokens_minted: AtomicU64,
    pub tokens_verified: AtomicU64,
    pub tokens_rejected: AtomicU64,
    pub replays_blocked: AtomicU64,
    pub policy_denials: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            tokens_minted: AtomicU64::new(0),
            tokens_verified: AtomicU64::new(0),
            tokens_rejected: AtomicU64::new(0),
            replays_blocked: AtomicU64::new(0),
            policy_denials: AtomicU64::new(0),
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

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            tokens_minted: self.tokens_minted.load(Ordering::Relaxed),
            tokens_verified: self.tokens_verified.load(Ordering::Relaxed),
            tokens_rejected: self.tokens_rejected.load(Ordering::Relaxed),
            replays_blocked: self.replays_blocked.load(Ordering::Relaxed),
            policy_denials: self.policy_denials.load(Ordering::Relaxed),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_start_at_zero() {
        let s = Metrics::new().snapshot();
        assert_eq!(s.tokens_minted, 0);
        assert_eq!(s.tokens_verified, 0);
        assert_eq!(s.tokens_rejected, 0);
        assert_eq!(s.replays_blocked, 0);
        assert_eq!(s.policy_denials, 0);
    }

    #[test]
    fn record_mint_increments() {
        let m = Metrics::new();
        m.record_mint();
        m.record_mint();
        assert_eq!(m.snapshot().tokens_minted, 2);
    }

    #[test]
    fn record_policy_denial_increments() {
        let m = Metrics::new();
        m.record_policy_denial();
        assert_eq!(m.snapshot().policy_denials, 1);
    }
}