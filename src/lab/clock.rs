//! Controllable clocks for synthetic deadlines.
//! Used by: workflow engine, CLI, console, and tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::lab::error::{LabError, LabResult};

#[derive(Debug, Clone)]
pub struct MutableClock {
    inner: Arc<Mutex<DateTime<Utc>>>,
}

impl MutableClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(start)),
        }
    }

    pub fn advance(&self, duration: Duration) -> LabResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| LabError::Storage("clock lock poisoned".into()))?;
        *guard += chrono::Duration::from_std(duration)
            .map_err(|err| LabError::Invalid(format!("duration: {err}")))?;
        Ok(())
    }

    pub fn now(&self) -> DateTime<Utc> {
        match self.inner.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

pub fn parse_duration(raw: &str) -> LabResult<Duration> {
    let raw = raw.trim();
    let (num, multiplier) = if let Some(num) = raw.strip_suffix('s') {
        (num, 1)
    } else if let Some(num) = raw.strip_suffix('m') {
        (num, 60)
    } else if let Some(num) = raw.strip_suffix('h') {
        (num, 3600)
    } else {
        (raw, 1)
    };
    let n: u64 = num
        .parse()
        .map_err(|_| LabError::Invalid(format!("duration {raw}")))?;
    Ok(Duration::from_secs(n.saturating_mul(multiplier)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn mutable_clock_advances() {
        let start = Utc.with_ymd_and_hms(2026, 9, 17, 12, 0, 0).unwrap();
        let clock = MutableClock::new(start);
        clock.advance(Duration::from_secs(60)).unwrap();
        assert_eq!(clock.now(), start + chrono::Duration::seconds(60));
    }

    #[test]
    fn parse_duration_accepts_hms_suffixes() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("15").unwrap(), Duration::from_secs(15));
    }
}
