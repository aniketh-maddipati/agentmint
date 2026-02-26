//! Application metrics for observability.
//! Used by: handlers, state.

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::Serialize;

pub struct Metrics {
    tokens_minted: AtomicU64,
    tokens_verified: AtomicU64,
    tokens_rejected: AtomicU64,
    replays_blocked: AtomicU64,
    total_verify_time_us: AtomicU64,
    started_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub tokens_minted: u64,
    pub tokens_verified: u64,
    pub tokens_rejected: u64,
    pub replays_blocked: u64,
    pub avg_verify_time_us: u64,
    pub uptime_seconds: u64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            tokens_minted: AtomicU64::new(0),
            tokens_verified: AtomicU64::new(0),
            tokens_rejected: AtomicU64::new(0),
            replays_blocked: AtomicU64::new(0),
            total_verify_time_us: AtomicU64::new(0),
            started_at: Utc::now(),
        }
    }

    pub fn record_mint(&self) {
        self.tokens_minted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_verify(&self, time_us: u64) {
        self.tokens_verified.fetch_add(1, Ordering::Relaxed);
        self.total_verify_time_us.fetch_add(time_us, Ordering::Relaxed);
    }

    pub fn record_rejection(&self) {
        self.tokens_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_replay(&self) {
        self.replays_blocked.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let count = self.tokens_verified.load(Ordering::Relaxed);
        let total_us = self.total_verify_time_us.load(Ordering::Relaxed);
        let avg = if count > 0 { total_us / count } else { 0 };
        let uptime = (Utc::now() - self.started_at).num_seconds().max(0) as u64;
        MetricsSnapshot {
            tokens_minted: self.tokens_minted.load(Ordering::Relaxed),
            tokens_verified: self.tokens_verified.load(Ordering::Relaxed),
            tokens_rejected: self.tokens_rejected.load(Ordering::Relaxed),
            replays_blocked: self.replays_blocked.load(Ordering::Relaxed),
            avg_verify_time_us: avg,
            uptime_seconds: uptime,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_start_at_zero() {
        let m = Metrics::new();
        let s = m.snapshot();
        assert_eq!(s.tokens_minted, 0);
        assert_eq!(s.tokens_verified, 0);
        assert_eq!(s.tokens_rejected, 0);
        assert_eq!(s.replays_blocked, 0);
        assert_eq!(s.avg_verify_time_us, 0);
    }

    #[test]
    fn record_mint_increments() {
        let m = Metrics::new();
        m.record_mint();
        m.record_mint();
        assert_eq!(m.snapshot().tokens_minted, 2);
    }

    #[test]
    fn record_verify_tracks_average() {
        let m = Metrics::new();
        m.record_verify(100);
        m.record_verify(200);
        let s = m.snapshot();
        assert_eq!(s.tokens_verified, 2);
        assert_eq!(s.avg_verify_time_us, 150);
    }

    #[test]
    fn record_rejection_increments() {
        let m = Metrics::new();
        m.record_rejection();
        assert_eq!(m.snapshot().tokens_rejected, 1);
    }

    #[test]
    fn record_replay_increments() {
        let m = Metrics::new();
        m.record_replay();
        assert_eq!(m.snapshot().replays_blocked, 1);
    }
}
