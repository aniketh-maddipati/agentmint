//! In-memory JTI replay protection store.
//! Used by: handlers::proxy, state.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::error::{Error, Result};

pub struct JtiStore {
    seen: Mutex<HashSet<String>>,
}

impl JtiStore {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(HashSet::new()),
        }
    }

    pub fn check_and_insert(&self, jti: &str) -> Result<()> {
        let mut seen = self.seen.lock().map_err(|e| Error::Signing(e.to_string()))?;
        if !seen.insert(jti.to_owned()) {
            return Err(Error::ReplayDetected(jti.to_owned()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_jti_succeeds() -> Result<()> {
        let store = JtiStore::new();
        store.check_and_insert("jti-1")?;
        Ok(())
    }

    #[test]
    fn duplicate_jti_rejected() -> Result<()> {
        let store = JtiStore::new();
        store.check_and_insert("jti-1")?;
        let result = store.check_and_insert("jti-1");
        assert!(matches!(result, Err(Error::ReplayDetected(_))));
        Ok(())
    }

    #[test]
    fn different_jtis_both_succeed() -> Result<()> {
        let store = JtiStore::new();
        store.check_and_insert("jti-1")?;
        store.check_and_insert("jti-2")?;
        Ok(())
    }
}
