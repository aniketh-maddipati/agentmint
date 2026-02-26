//! SQLite-backed audit log for token usage.
//! Used by: handlers::proxy, handlers::audit, state.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;

pub struct AuditLog {
    conn: Mutex<Connection>,
}

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub jti: String,
    pub sub: String,
    pub action: String,
    pub verified_at: String,
}

impl AuditLog {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                jti TEXT PRIMARY KEY,
                sub TEXT NOT NULL,
                action TEXT NOT NULL,
                verified_at TEXT NOT NULL
            )",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::open(":memory:")
    }

    pub fn log(&self, jti: &str, sub: &str, action: &str, verified_at: DateTime<Utc>) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| crate::error::Error::Signing(e.to_string()))?;
        conn.execute(
            "INSERT INTO audit_log (jti, sub, action, verified_at) VALUES (?1, ?2, ?3, ?4)",
            (jti, sub, action, verified_at.to_rfc3339()),
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().map_err(|e| crate::error::Error::Signing(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT jti, sub, action, verified_at FROM audit_log ORDER BY rowid DESC LIMIT ?1",
        )?;
        let entries = stmt
            .query_map([limit], |row| {
                Ok(AuditEntry {
                    jti: row.get(0)?,
                    sub: row.get(1)?,
                    action: row.get(2)?,
                    verified_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_and_retrieve_entry() -> Result<()> {
        let audit = AuditLog::open_in_memory()?;
        audit.log("jti-1", "agent-1", "deploy", Utc::now())?;
        let entries = audit.recent(10)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].jti, "jti-1");
        assert_eq!(entries[0].sub, "agent-1");
        assert_eq!(entries[0].action, "deploy");
        Ok(())
    }

    #[test]
    fn recent_respects_limit() -> Result<()> {
        let audit = AuditLog::open_in_memory()?;
        audit.log("jti-1", "a", "x", Utc::now())?;
        audit.log("jti-2", "b", "y", Utc::now())?;
        audit.log("jti-3", "c", "z", Utc::now())?;
        let entries = audit.recent(2)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].jti, "jti-3");
        assert_eq!(entries[1].jti, "jti-2");
        Ok(())
    }

    #[test]
    fn empty_log_returns_empty_vec() -> Result<()> {
        let audit = AuditLog::open_in_memory()?;
        let entries = audit.recent(10)?;
        assert!(entries.is_empty());
        Ok(())
    }

    #[test]
    fn duplicate_jti_rejected_by_db() -> Result<()> {
        let audit = AuditLog::open_in_memory()?;
        audit.log("jti-1", "a", "x", Utc::now())?;
        let result = audit.log("jti-1", "a", "x", Utc::now());
        assert!(result.is_err());
        Ok(())
    }
}
