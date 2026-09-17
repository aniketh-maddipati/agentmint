//! Independent fake payer ledger (separate SQLite file).
//! Used by: workflow via PayerAdapter trait only.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::lab::error::{LabError, LabResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BvInquiry {
    pub member_id: String,
    pub plan_id: String,
    pub cpt: String,
    pub dos: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BvResponse {
    pub text: String,
    pub configured_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketSubmissionRequest {
    pub case_id: Uuid,
    pub packet_id: Uuid,
    pub packet_hash: String,
    pub idempotency_key: String,
    pub member_id: String,
    pub cpt: String,
    pub is_appeal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionReceipt {
    pub receipt_id: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLookup {
    pub receipt_id: String,
    pub outcome: String,
    pub limitations: Vec<String>,
    pub reason: Option<String>,
}

pub trait PayerAdapter: Send + Sync {
    fn inquire_bv(&self, inquiry: &BvInquiry) -> LabResult<BvResponse>;
    fn submit_packet(&self, request: &PacketSubmissionRequest) -> LabResult<SubmissionReceipt>;
    fn lookup_submission(&self, idempotency_key: &str) -> LabResult<Option<SubmissionReceipt>>;
    fn retrieve_decision(&self, receipt_id: &str) -> LabResult<Option<DecisionLookup>>;
}

#[derive(Debug, Clone)]
struct ConfiguredOutcome {
    decision: String,
    limitations: Vec<String>,
    reason: Option<String>,
}

#[derive(Clone)]
pub struct FakePayer {
    conn: Arc<Mutex<Connection>>,
    bv_text: Arc<Mutex<String>>,
    bv_kind: Arc<Mutex<Option<String>>>,
    outcome: Arc<Mutex<ConfiguredOutcome>>,
    lost_keys: Arc<Mutex<Vec<String>>>,
    submission_count: Arc<Mutex<u64>>,
}

impl FakePayer {
    pub fn open(path: &Path) -> LabResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS submissions (
                idempotency_key TEXT PRIMARY KEY,
                receipt_id TEXT NOT NULL,
                case_id TEXT NOT NULL,
                packet_id TEXT NOT NULL,
                packet_hash TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS decisions (
                receipt_id TEXT PRIMARY KEY,
                outcome TEXT NOT NULL,
                limitations_json TEXT NOT NULL,
                reason TEXT,
                created_at TEXT NOT NULL
            );
            ",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            bv_text: Arc::new(Mutex::new(
                "Member is active. Prior authorization is required for CPT 72148.".into(),
            )),
            bv_kind: Arc::new(Mutex::new(Some("pa_required".into()))),
            outcome: Arc::new(Mutex::new(ConfiguredOutcome {
                decision: "approved".into(),
                limitations: vec![
                    "outpatient site of service only".into(),
                    "authorization valid 30 days".into(),
                ],
                reason: None,
            })),
            lost_keys: Arc::new(Mutex::new(Vec::new())),
            submission_count: Arc::new(Mutex::new(0)),
        })
    }

    pub fn count_submissions(&self) -> LabResult<u64> {
        let guard = self
            .submission_count
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
        Ok(*guard)
    }

    pub fn configure_outcome(
        &self,
        decision: &str,
        limitations: Vec<String>,
        reason: Option<String>,
    ) -> LabResult<()> {
        let mut guard = self
            .outcome
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
        guard.decision = decision.to_string();
        guard.limitations = limitations;
        guard.reason = reason;
        Ok(())
    }

    pub fn configure_bv(&self, text: &str, kind: Option<&str>) -> LabResult<()> {
        {
            let mut guard = self
                .bv_text
                .lock()
                .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
            *guard = text.to_string();
        }
        {
            let mut guard = self
                .bv_kind
                .lock()
                .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
            *guard = kind.map(|s| s.to_string());
        }
        Ok(())
    }

    pub fn inject_lost_response(&self, idempotency_key: &str) -> LabResult<()> {
        let mut guard = self
            .lost_keys
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
        guard.push(idempotency_key.to_string());
        Ok(())
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> LabResult<T>) -> LabResult<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
        f(&guard)
    }
}

impl PayerAdapter for FakePayer {
    fn inquire_bv(&self, _inquiry: &BvInquiry) -> LabResult<BvResponse> {
        let text = self
            .bv_text
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?
            .clone();
        let configured_kind = self
            .bv_kind
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?
            .clone();
        Ok(BvResponse {
            text,
            configured_kind,
        })
    }

    fn submit_packet(&self, request: &PacketSubmissionRequest) -> LabResult<SubmissionReceipt> {
        let existing = self.lookup_submission(&request.idempotency_key)?;
        if let Some(receipt) = existing {
            return Ok(receipt);
        }

        let lost = {
            let guard = self
                .lost_keys
                .lock()
                .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
            guard.iter().any(|k| k == &request.idempotency_key)
        };

        let outcome = self
            .outcome
            .lock()
            .map_err(|_| LabError::Storage("payer lock poisoned".into()))?
            .clone();

        let (status, detail) = match outcome.decision.as_str() {
            "intake_rejected" => (
                "intake_rejected".to_string(),
                "synthetic intake rejection".to_string(),
            ),
            other => ("accepted".to_string(), format!("queued:{other}")),
        };

        let receipt_id = format!("rcpt_{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO submissions
                 (idempotency_key, receipt_id, case_id, packet_id, packet_hash, status, detail, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    request.idempotency_key,
                    receipt_id,
                    request.case_id.to_string(),
                    request.packet_id.to_string(),
                    request.packet_hash,
                    status,
                    detail,
                    now,
                ],
            )?;
            Ok(())
        })?;

        {
            let mut guard = self
                .submission_count
                .lock()
                .map_err(|_| LabError::Storage("payer lock poisoned".into()))?;
            *guard += 1;
        }

        if status == "accepted" {
            let limitations_json = serde_json::to_string(&outcome.limitations)?;
            let decision_outcome = if request.is_appeal && outcome.decision == "denied" {
                "denied".to_string()
            } else {
                outcome.decision.clone()
            };
            self.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO decisions (receipt_id, outcome, limitations_json, reason, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        receipt_id,
                        decision_outcome,
                        limitations_json,
                        outcome.reason,
                        now,
                    ],
                )?;
                Ok(())
            })?;
        }

        if lost {
            return Err(LabError::Unverified(
                "payer response lost after accept (synthetic fault)".into(),
            ));
        }

        Ok(SubmissionReceipt {
            receipt_id,
            status,
            detail,
        })
    }

    fn lookup_submission(&self, idempotency_key: &str) -> LabResult<Option<SubmissionReceipt>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT receipt_id, status, detail FROM submissions WHERE idempotency_key = ?1",
                    params![idempotency_key],
                    |row| {
                        Ok(SubmissionReceipt {
                            receipt_id: row.get(0)?,
                            status: row.get(1)?,
                            detail: row.get(2)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    fn retrieve_decision(&self, receipt_id: &str) -> LabResult<Option<DecisionLookup>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT outcome, limitations_json, reason FROM decisions WHERE receipt_id = ?1",
                    params![receipt_id],
                    |row| {
                        let limitations_json: String = row.get(1)?;
                        let limitations: Vec<String> =
                            serde_json::from_str(&limitations_json).unwrap_or_default();
                        Ok(DecisionLookup {
                            receipt_id: receipt_id.to_string(),
                            outcome: row.get(0)?,
                            limitations,
                            reason: row.get(2)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn duplicate_idempotency_returns_same_receipt() {
        let dir = tempdir().unwrap();
        let payer = FakePayer::open(&dir.path().join("payer.db")).unwrap();
        let req = PacketSubmissionRequest {
            case_id: Uuid::new_v4(),
            packet_id: Uuid::new_v4(),
            packet_hash: "abc".into(),
            idempotency_key: "key-1".into(),
            member_id: "m1".into(),
            cpt: "72148".into(),
            is_appeal: false,
        };
        let a = payer.submit_packet(&req).unwrap();
        let b = payer.submit_packet(&req).unwrap();
        assert_eq!(a.receipt_id, b.receipt_id);
        assert_eq!(payer.count_submissions().unwrap(), 1);
    }
}
