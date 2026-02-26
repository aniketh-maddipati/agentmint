//! In-memory JTI replay protection with expiry and capacity limits.
//! Used by: handlers::proxy, state.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{Error, Result, lock_err};

const DEFAULT_MAX_CAPACITY: usize = 100_000;

pub struct JtiStore {
    entries: Mutex<HashMap<String, i64>>,
    max_capacity: usize,
}

impl JtiStore {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_CAPACITY)
    }

    pub fn with_capacity(max_capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_capacity,
        }
    }

    pub fn check_and_insert(&self, jti: &str, exp: i64) -> Result<()> {
        let mut entries = self.entries.lock().map_err(lock_err("jti"))?;
        Self::cleanup_expired_inner(&mut entries);
        if entries.len() >= self.max_capacity {
            return Err(Error::ServiceUnavailable("JTI store at capacity".into()));
        }
        if entries.contains_key(jti) {
            return Err(Error::ReplayDetected(jti.to_owned()));
        }
        entries.insert(jti.to_owned(), exp);
        Ok(())
    }

    fn cleanup_expired_inner(entries: &mut HashMap<String, i64>) {
        let now = chrono::Utc::now().timestamp();
        entries.retain(|_, exp| *exp > now);
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|e| e.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn future_exp() -> i64 {
        chrono::Utc::now().timestamp() + 300
    }

    #[test]
    fn first_jti_succeeds() -> Result<()> {
        let store = JtiStore::new();
        store.check_and_insert("jti-1", future_exp())?;
        Ok(())
    }

    #[test]
    fn duplicate_jti_rejected() -> Result<()> {
        let store = JtiStore::new();
        let exp = future_exp();
        store.check_and_insert("jti-1", exp)?;
        let result = store.check_and_insert("jti-1", exp);
        assert!(matches!(result, Err(Error::ReplayDetected(_))));
        Ok(())
    }

    #[test]
    fn different_jtis_both_succeed() -> Result<()> {
        let store = JtiStore::new();
        let exp = future_exp();
        store.check_and_insert("jti-1", exp)?;
        store.check_and_insert("jti-2", exp)?;
        Ok(())
    }

    #[test]
    fn capacity_limit_returns_503() -> Result<()> {
        let store = JtiStore::with_capacity(2);
        let exp = future_exp();
        store.check_and_insert("jti-1", exp)?;
        store.check_and_insert("jti-2", exp)?;
        let result = store.check_and_insert("jti-3", exp);
        assert!(matches!(result, Err(Error::ServiceUnavailable(_))));
        Ok(())
    }

    #[test]
    fn expired_entries_cleaned_up() -> Result<()> {
        let store = JtiStore::with_capacity(1);
        let past = chrono::Utc::now().timestamp() - 1;
        store.check_and_insert("jti-old", past)?;
        store.check_and_insert("jti-new", future_exp())?;
        assert_eq!(store.len(), 1);
        Ok(())
    }
}
