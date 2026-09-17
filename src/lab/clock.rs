//! Controllable clocks for synthetic deadlines.
//! Used by: workflow engine and tests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::lab::error::{LabError, LabResult};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone)]
pub struct InstantClock;

impl Clock for InstantClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

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

    pub fn set(&self, when: DateTime<Utc>) -> LabResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| LabError::Storage("clock lock poisoned".into()))?;
        *guard = when;
        Ok(())
    }
}

impl Clock for MutableClock {
    fn now(&self) -> DateTime<Utc> {
        match self.inner.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
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
}
