//! Rate limiting with global, per-IP, and per-user limits.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

pub struct RateLimiter {
    config: RateLimitConfig,
    state: Mutex<RateLimitState>,
}

pub struct RateLimitConfig {
    pub global_per_sec: u32,
    pub per_ip_per_min: u32,
    pub per_user_per_min: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global_per_sec: 1000,
            per_ip_per_min: 100,
            per_user_per_min: 20,
        }
    }
}

struct RateLimitState {
    ip_counts: HashMap<Box<str>, WindowCounter>,
    user_counts: HashMap<Box<str>, WindowCounter>,
    global_count: WindowCounter,
    last_cleanup: Instant,
}

struct WindowCounter {
    count: u32,
    window_start: Instant,
}

impl WindowCounter {
    fn new() -> Self {
        Self { count: 0, window_start: Instant::now() }
    }

    fn increment(&mut self, limit: u32, window: Duration) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > window {
            self.count = 0;
            self.window_start = now;
        }
        self.count += 1;
        self.count <= limit
    }
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            state: Mutex::new(RateLimitState {
                ip_counts: HashMap::new(),
                user_counts: HashMap::new(),
                global_count: WindowCounter::new(),
                last_cleanup: Instant::now(),
            }),
        }
    }

    pub fn check_ip(&self, ip: &str) -> Result<(), RateLimitError> {
        let mut state = self.state.lock().unwrap();
        self.maybe_cleanup(&mut state);

        // Global check (per second)
        if !state.global_count.increment(self.config.global_per_sec, Duration::from_secs(1)) {
            return Err(RateLimitError::Global);
        }

        // Per-IP check (per minute)
        let counter = state.ip_counts
            .entry(ip.into())
            .or_insert_with(WindowCounter::new);

        if !counter.increment(self.config.per_ip_per_min, WINDOW) {
            return Err(RateLimitError::PerIp {
                limit: self.config.per_ip_per_min,
                window_secs: WINDOW.as_secs(),
            });
        }

        Ok(())
    }

    pub fn check_user(&self, user_id: &str) -> Result<(), RateLimitError> {
        let mut state = self.state.lock().unwrap();

        let counter = state.user_counts
            .entry(user_id.into())
            .or_insert_with(WindowCounter::new);

        if !counter.increment(self.config.per_user_per_min, WINDOW) {
            return Err(RateLimitError::PerUser {
                limit: self.config.per_user_per_min,
                window_secs: WINDOW.as_secs(),
            });
        }

        Ok(())
    }

    fn maybe_cleanup(&self, state: &mut RateLimitState) {
        let now = Instant::now();
        if now.duration_since(state.last_cleanup) > CLEANUP_INTERVAL {
            let cutoff = now - WINDOW - Duration::from_secs(60);
            state.ip_counts.retain(|_, c| c.window_start > cutoff);
            state.user_counts.retain(|_, c| c.window_start > cutoff);
            state.last_cleanup = now;
        }
    }

    #[allow(dead_code)]
    pub fn stats(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.ip_counts.len(), state.user_counts.len())
    }
}

#[derive(Debug)]
pub enum RateLimitError {
    Global,
    PerIp { limit: u32, window_secs: u64 },
    PerUser { limit: u32, window_secs: u64 },
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "global rate limit exceeded"),
            Self::PerIp { limit, window_secs } => {
                write!(f, "rate limit: {} requests per {}s", limit, window_secs)
            }
            Self::PerUser { limit, window_secs } => {
                write!(f, "rate limit: {} requests per {}s per user", limit, window_secs)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_under_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            global_per_sec: 1000,
            per_ip_per_min: 5,
            per_user_per_min: 5,
        });

        for _ in 0..5 {
            assert!(limiter.check_ip("127.0.0.1").is_ok());
        }
    }

    #[test]
    fn blocks_over_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            global_per_sec: 1000,
            per_ip_per_min: 2,
            per_user_per_min: 5,
        });

        assert!(limiter.check_ip("127.0.0.1").is_ok());
        assert!(limiter.check_ip("127.0.0.1").is_ok());
        assert!(limiter.check_ip("127.0.0.1").is_err());
    }

    #[test]
    fn separate_ips_have_separate_limits() {
        let limiter = RateLimiter::new(RateLimitConfig {
            global_per_sec: 1000,
            per_ip_per_min: 1,
            per_user_per_min: 5,
        });

        assert!(limiter.check_ip("1.1.1.1").is_ok());
        assert!(limiter.check_ip("1.1.1.1").is_err());
        assert!(limiter.check_ip("2.2.2.2").is_ok());
    }

    #[test]
    fn user_rate_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            global_per_sec: 1000,
            per_ip_per_min: 100,
            per_user_per_min: 2,
        });

        assert!(limiter.check_user("alice").is_ok());
        assert!(limiter.check_user("alice").is_ok());
        assert!(limiter.check_user("alice").is_err());
        assert!(limiter.check_user("bob").is_ok());
    }
}