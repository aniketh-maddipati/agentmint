//! Case SQLite store for the PA/BV lab.
//! Used by: workflow, inspect, console. Transactions stay short.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::lab::domain::*;
use crate::lab::error::{LabError, LabResult};

const MIGRATION: &str = "
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS cases (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE,
    scenario_id TEXT NOT NULL,
    workflow_version TEXT NOT NULL,
    stage TEXT NOT NULL,
    service_json TEXT NOT NULL,
    coverage_json TEXT NOT NULL,
    service_version INTEGER NOT NULL,
    coverage_version INTEGER NOT NULL,
    disposition TEXT,
    paused_from TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    status TEXT NOT NULL,
    owner TEXT NOT NULL,
    context_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    completed_at TEXT
);
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    task_id TEXT,
    purpose TEXT NOT NULL,
    outcome TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conversation (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    role TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS observations (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    statement TEXT NOT NULL,
    uncertainty TEXT NOT NULL,
    evidence_refs_json TEXT NOT NULL,
    stale INTEGER NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS determinations (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    rationale TEXT NOT NULL,
    required_docs_json TEXT NOT NULL,
    observation_ids_json TEXT NOT NULL,
    coverage_version INTEGER NOT NULL,
    service_version INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    fixture_name TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS packets (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    document_ids_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    appeal_of_decision_id TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS reviews (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    packet_id TEXT NOT NULL,
    packet_hash TEXT NOT NULL,
    decision TEXT NOT NULL,
    reviewer TEXT NOT NULL,
    valid INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS submissions (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    packet_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    transport_state TEXT NOT NULL,
    ownership_generation INTEGER NOT NULL,
    payer_receipt_id TEXT,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    limitations_json TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(case_id, seq)
);
CREATE TABLE IF NOT EXISTS pending_work (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    detail TEXT NOT NULL,
    due_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    model_id TEXT NOT NULL,
    context_version INTEGER NOT NULL,
    tool_calls_json TEXT NOT NULL,
    structured_output_json TEXT NOT NULL,
    evidence_refs_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbound_dedup (
    dedup_key TEXT PRIMARY KEY,
    case_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[derive(Clone)]
pub struct CaseStore {
    conn: Arc<Mutex<Connection>>,
}

impl CaseStore {
    pub fn open(path: &Path) -> LabResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(MIGRATION)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> LabResult<T>) -> LabResult<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| LabError::Storage("case store lock poisoned".into()))?;
        f(&guard)
    }

    pub fn insert_case(&self, case: &Case) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO cases
                 (id, run_id, scenario_id, workflow_version, stage, service_json, coverage_json,
                  service_version, coverage_version, disposition, paused_from, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    case.id.to_string(),
                    case.run_id.to_string(),
                    case.scenario_id,
                    case.workflow_version,
                    serde_json::to_string(&case.stage)?,
                    serde_json::to_string(&case.service)?,
                    serde_json::to_string(&case.coverage)?,
                    case.service_version,
                    case.coverage_version,
                    case.disposition,
                    case.paused_from
                        .map(|s| serde_json::to_string(&s))
                        .transpose()?,
                    case.created_at.to_rfc3339(),
                    case.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_case(&self, case: &Case) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE cases SET stage=?2, service_json=?3, coverage_json=?4,
                 service_version=?5, coverage_version=?6, disposition=?7, paused_from=?8, updated_at=?9
                 WHERE id=?1",
                params![
                    case.id.to_string(),
                    serde_json::to_string(&case.stage)?,
                    serde_json::to_string(&case.service)?,
                    serde_json::to_string(&case.coverage)?,
                    case.service_version,
                    case.coverage_version,
                    case.disposition,
                    case.paused_from
                        .map(|s| serde_json::to_string(&s))
                        .transpose()?,
                    case.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_case_by_run(&self, run_id: Uuid) -> LabResult<Option<Case>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, run_id, scenario_id, workflow_version, stage, service_json, coverage_json,
                            service_version, coverage_version, disposition, paused_from, created_at, updated_at
                     FROM cases WHERE run_id = ?1",
                    params![run_id.to_string()],
                    map_case,
                )
                .optional()?;
            Ok(row)
        })
    }

    pub fn get_case(&self, case_id: Uuid) -> LabResult<Option<Case>> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, run_id, scenario_id, workflow_version, stage, service_json, coverage_json,
                            service_version, coverage_version, disposition, paused_from, created_at, updated_at
                     FROM cases WHERE id = ?1",
                    params![case_id.to_string()],
                    map_case,
                )
                .optional()?;
            Ok(row)
        })
    }

    pub fn list_runs(&self) -> LabResult<Vec<(Uuid, String, CaseStage)>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT run_id, scenario_id, stage FROM cases ORDER BY created_at")?;
            let rows = stmt.query_map([], |row| {
                let run_id: String = row.get(0)?;
                let scenario: String = row.get(1)?;
                let stage_raw: String = row.get(2)?;
                Ok((run_id, scenario, stage_raw))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (run_id, scenario, stage_raw) = row?;
                let stage: CaseStage = serde_json::from_str(&stage_raw)?;
                out.push((
                    Uuid::parse_str(&run_id).map_err(|e| LabError::Invalid(e.to_string()))?,
                    scenario,
                    stage,
                ));
            }
            Ok(out)
        })
    }

    pub fn insert_task(&self, task: &Task) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, case_id, purpose, status, owner, context_json, created_at, completed_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    task.id.to_string(),
                    task.case_id.to_string(),
                    serde_json::to_string(&task.purpose)?,
                    serde_json::to_string(&task.status)?,
                    serde_json::to_string(&task.owner)?,
                    task.context_json,
                    task.created_at.to_rfc3339(),
                    task.completed_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_task(&self, task: &Task) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE tasks SET status=?2, context_json=?3, completed_at=?4 WHERE id=?1",
                params![
                    task.id.to_string(),
                    serde_json::to_string(&task.status)?,
                    task.context_json,
                    task.completed_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_attempt(&self, attempt: &Attempt) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO attempts (id, case_id, task_id, purpose, outcome, detail_json, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    attempt.id.to_string(),
                    attempt.case_id.to_string(),
                    attempt.task_id.map(|id| id.to_string()),
                    attempt.purpose,
                    serde_json::to_string(&attempt.outcome)?,
                    attempt.detail_json,
                    attempt.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_observation(&self, obs: &Observation) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO observations
                 (id, case_id, kind, statement, uncertainty, evidence_refs_json, stale, source, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    obs.id.to_string(),
                    obs.case_id.to_string(),
                    serde_json::to_string(&obs.kind)?,
                    obs.statement,
                    serde_json::to_string(&obs.uncertainty)?,
                    serde_json::to_string(&obs.evidence_refs)?,
                    obs.stale as i32,
                    obs.source,
                    obs.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn mark_observations_stale(&self, case_id: Uuid) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE observations SET stale = 1 WHERE case_id = ?1",
                params![case_id.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn insert_determination(&self, det: &Determination) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO determinations
                 (id, case_id, kind, rationale, required_docs_json, observation_ids_json,
                  coverage_version, service_version, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    det.id.to_string(),
                    det.case_id.to_string(),
                    serde_json::to_string(&det.kind)?,
                    det.rationale,
                    serde_json::to_string(&det.required_docs)?,
                    serde_json::to_string(&det.observation_ids)?,
                    det.coverage_version,
                    det.service_version,
                    det.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_document(&self, doc: &Document) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO documents
                 (id, case_id, request_id, fixture_name, content_hash, content, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    doc.id.to_string(),
                    doc.case_id.to_string(),
                    doc.request_id,
                    doc.fixture_name,
                    doc.content_hash,
                    doc.content,
                    doc.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_packet(&self, packet: &Packet) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO packets
                 (id, case_id, version, document_ids_json, content_hash, appeal_of_decision_id, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    packet.id.to_string(),
                    packet.case_id.to_string(),
                    packet.version,
                    serde_json::to_string(&packet.document_ids)?,
                    packet.content_hash,
                    packet.appeal_of_decision_id.map(|id| id.to_string()),
                    packet.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_review(&self, review: &Review) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO reviews
                 (id, case_id, packet_id, packet_hash, decision, reviewer, valid, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    review.id.to_string(),
                    review.case_id.to_string(),
                    review.packet_id.to_string(),
                    review.packet_hash,
                    serde_json::to_string(&review.decision)?,
                    review.reviewer,
                    review.valid as i32,
                    review.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn invalidate_reviews_for_packet(&self, packet_id: Uuid) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE reviews SET valid = 0 WHERE packet_id = ?1",
                params![packet_id.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn invalidate_all_reviews(&self, case_id: Uuid) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE reviews SET valid = 0 WHERE case_id = ?1",
                params![case_id.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn insert_submission(&self, sub: &Submission) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO submissions
                 (id, case_id, packet_id, idempotency_key, transport_state, ownership_generation,
                  payer_receipt_id, detail_json, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    sub.id.to_string(),
                    sub.case_id.to_string(),
                    sub.packet_id.to_string(),
                    sub.idempotency_key,
                    serde_json::to_string(&sub.transport_state)?,
                    sub.ownership_generation as i64,
                    sub.payer_receipt_id,
                    sub.detail_json,
                    sub.created_at.to_rfc3339(),
                    sub.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_submission(&self, sub: &Submission) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE submissions SET transport_state=?2, ownership_generation=?3,
                 payer_receipt_id=?4, detail_json=?5, updated_at=?6 WHERE id=?1",
                params![
                    sub.id.to_string(),
                    serde_json::to_string(&sub.transport_state)?,
                    sub.ownership_generation as i64,
                    sub.payer_receipt_id,
                    sub.detail_json,
                    sub.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn claim_submission(
        &self,
        submission_id: Uuid,
        expected_generation: u64,
    ) -> LabResult<bool> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let current: u64 = tx.query_row(
                "SELECT ownership_generation FROM submissions WHERE id = ?1",
                params![submission_id.to_string()],
                |row| row.get::<_, i64>(0).map(|v| v as u64),
            )?;
            if current != expected_generation {
                return Ok(false);
            }
            let changed = tx.execute(
                "UPDATE submissions SET ownership_generation = ?2 WHERE id = ?1 AND ownership_generation = ?3",
                params![
                    submission_id.to_string(),
                    (expected_generation + 1) as i64,
                    expected_generation as i64
                ],
            )?;
            tx.commit()?;
            Ok(changed == 1)
        })
    }

    pub fn insert_decision(&self, decision: &Decision) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO decisions
                 (id, case_id, submission_id, outcome, limitations_json, reason, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    decision.id.to_string(),
                    decision.case_id.to_string(),
                    decision.submission_id.to_string(),
                    serde_json::to_string(&decision.outcome)?,
                    serde_json::to_string(&decision.limitations)?,
                    decision.reason,
                    decision.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn append_event(
        &self,
        case_id: Uuid,
        kind: &str,
        payload: &serde_json::Value,
        at: DateTime<Utc>,
    ) -> LabResult<Event> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let next: u64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE case_id = ?1",
                    params![case_id.to_string()],
                    |row| row.get::<_, i64>(0).map(|v| v as u64),
                )
                .unwrap_or(1);
            let event = Event {
                id: Uuid::new_v4(),
                case_id,
                seq: next,
                kind: kind.to_string(),
                payload_json: payload.to_string(),
                created_at: at,
            };
            tx.execute(
                "INSERT INTO events (id, case_id, seq, kind, payload_json, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    event.id.to_string(),
                    event.case_id.to_string(),
                    event.seq as i64,
                    event.kind,
                    event.payload_json,
                    event.created_at.to_rfc3339(),
                ],
            )?;
            tx.commit()?;
            Ok(event)
        })
    }

    pub fn insert_conversation(&self, msg: &ConversationMessage) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO conversation (id, case_id, role, text, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    msg.id.to_string(),
                    msg.case_id.to_string(),
                    serde_json::to_string(&msg.role)?,
                    msg.text,
                    msg.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_pending(&self, pending: &PendingWork) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO pending_work (id, case_id, kind, ref_id, detail, due_at, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    pending.id.to_string(),
                    pending.case_id.to_string(),
                    serde_json::to_string(&pending.kind)?,
                    pending.ref_id,
                    pending.detail,
                    pending.due_at.map(|t| t.to_rfc3339()),
                    pending.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_pending(&self, pending_id: Uuid) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM pending_work WHERE id = ?1",
                params![pending_id.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn clear_pending_kind(&self, case_id: Uuid, kind: PendingKind) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM pending_work WHERE case_id = ?1 AND kind = ?2",
                params![case_id.to_string(), serde_json::to_string(&kind)?],
            )?;
            Ok(())
        })
    }

    pub fn insert_agent_run(&self, run: &AgentRunRecord) -> LabResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO agent_runs
                 (id, case_id, task_id, prompt_version, model_id, context_version,
                  tool_calls_json, structured_output_json, evidence_refs_json, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    run.id.to_string(),
                    run.case_id.to_string(),
                    run.task_id.to_string(),
                    run.prompt_version,
                    run.model_id,
                    run.context_version,
                    run.tool_calls_json,
                    run.structured_output_json,
                    serde_json::to_string(&run.evidence_refs)?,
                    run.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn try_outbound_dedup(
        &self,
        case_id: Uuid,
        key: &str,
        at: DateTime<Utc>,
    ) -> LabResult<bool> {
        self.with(|conn| {
            let existing = conn
                .query_row(
                    "SELECT 1 FROM outbound_dedup WHERE dedup_key = ?1",
                    params![key],
                    |_| Ok(()),
                )
                .optional()?;
            if existing.is_some() {
                return Ok(false);
            }
            conn.execute(
                "INSERT INTO outbound_dedup (dedup_key, case_id, created_at) VALUES (?1,?2,?3)",
                params![key, case_id.to_string(), at.to_rfc3339()],
            )?;
            Ok(true)
        })
    }

    pub fn load_snapshot(&self, case_id: Uuid) -> LabResult<CaseSnapshot> {
        let case = self
            .get_case(case_id)?
            .ok_or_else(|| LabError::NotFound(format!("case {case_id}")))?;
        self.with(|conn| {
            Ok(CaseSnapshot {
                case,
                tasks: load_tasks(conn, case_id)?,
                attempts: load_attempts(conn, case_id)?,
                conversation: load_conversation(conn, case_id)?,
                observations: load_observations(conn, case_id)?,
                determinations: load_determinations(conn, case_id)?,
                documents: load_documents(conn, case_id)?,
                packets: load_packets(conn, case_id)?,
                reviews: load_reviews(conn, case_id)?,
                submissions: load_submissions(conn, case_id)?,
                decisions: load_decisions(conn, case_id)?,
                events: load_events(conn, case_id)?,
                pending: load_pending(conn, case_id)?,
                agent_runs: load_agent_runs(conn, case_id)?,
            })
        })
    }
}

fn map_case(row: &rusqlite::Row<'_>) -> rusqlite::Result<Case> {
    let paused_from: Option<String> = row.get(10)?;
    Ok(Case {
        id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
        run_id: Uuid::parse_str(&row.get::<_, String>(1)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?,
        scenario_id: row.get(2)?,
        workflow_version: row.get(3)?,
        stage: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        service: serde_json::from_str(&row.get::<_, String>(5)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        coverage: serde_json::from_str(&row.get::<_, String>(6)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
        })?,
        service_version: row.get::<_, i64>(7)? as u32,
        coverage_version: row.get::<_, i64>(8)? as u32,
        disposition: row.get(9)?,
        paused_from: paused_from
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        created_at: parse_dt(&row.get::<_, String>(11)?)?,
        updated_at: parse_dt(&row.get::<_, String>(12)?)?,
    })
}

fn parse_dt(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

fn load_tasks(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, purpose, status, owner, context_json, created_at, completed_at
         FROM tasks WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        let completed: Option<String> = row.get(7)?;
        Ok(Task {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            purpose: serde_json::from_str(&row.get::<_, String>(2)?)
                .unwrap_or(TaskPurpose::HumanReview),
            status: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or(TaskStatus::Open),
            owner: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(Role::Operator),
            context_json: row.get(5)?,
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
            completed_at: completed.and_then(|s| parse_dt(&s).ok()),
        })
    })?;
    collect_rows(rows)
}

fn load_attempts(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Attempt>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, task_id, purpose, outcome, detail_json, created_at
         FROM attempts WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        let task_id: Option<String> = row.get(2)?;
        Ok(Attempt {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            task_id: task_id.and_then(|s| Uuid::parse_str(&s).ok()),
            purpose: row.get(3)?,
            outcome: serde_json::from_str(&row.get::<_, String>(4)?)
                .unwrap_or(AttemptOutcome::Unknown),
            detail_json: row.get(5)?,
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_conversation(conn: &Connection, case_id: Uuid) -> LabResult<Vec<ConversationMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, role, text, created_at FROM conversation WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(ConversationMessage {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            role: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(Role::Operator),
            text: row.get(3)?,
            created_at: parse_dt(&row.get::<_, String>(4)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_observations(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Observation>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, kind, statement, uncertainty, evidence_refs_json, stale, source, created_at
         FROM observations WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Observation {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            kind: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(ObservationKind::Other),
            statement: row.get(3)?,
            uncertainty: serde_json::from_str(&row.get::<_, String>(4)?)
                .unwrap_or(Uncertainty::Unknown),
            evidence_refs: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            stale: row.get::<_, i32>(6)? != 0,
            source: row.get(7)?,
            created_at: parse_dt(&row.get::<_, String>(8)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_determinations(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Determination>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, kind, rationale, required_docs_json, observation_ids_json,
                coverage_version, service_version, created_at
         FROM determinations WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Determination {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            kind: serde_json::from_str(&row.get::<_, String>(2)?)
                .unwrap_or(DeterminationKind::Unclear),
            rationale: row.get(3)?,
            required_docs: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
            observation_ids: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            coverage_version: row.get::<_, i64>(6)? as u32,
            service_version: row.get::<_, i64>(7)? as u32,
            created_at: parse_dt(&row.get::<_, String>(8)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_documents(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Document>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, request_id, fixture_name, content_hash, content, created_at
         FROM documents WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Document {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            request_id: row.get(2)?,
            fixture_name: row.get(3)?,
            content_hash: row.get(4)?,
            content: row.get(5)?,
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_packets(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Packet>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, version, document_ids_json, content_hash, appeal_of_decision_id, created_at
         FROM packets WHERE case_id = ?1 ORDER BY version",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        let appeal: Option<String> = row.get(5)?;
        Ok(Packet {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            version: row.get::<_, i64>(2)? as u32,
            document_ids: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
            content_hash: row.get(4)?,
            appeal_of_decision_id: appeal.and_then(|s| Uuid::parse_str(&s).ok()),
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_reviews(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Review>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, packet_id, packet_hash, decision, reviewer, valid, created_at
         FROM reviews WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Review {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            packet_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default(),
            packet_hash: row.get(3)?,
            decision: serde_json::from_str(&row.get::<_, String>(4)?)
                .unwrap_or(ReviewDecision::Decline),
            reviewer: row.get(5)?,
            valid: row.get::<_, i32>(6)? != 0,
            created_at: parse_dt(&row.get::<_, String>(7)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_submissions(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Submission>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, packet_id, idempotency_key, transport_state, ownership_generation,
                payer_receipt_id, detail_json, created_at, updated_at
         FROM submissions WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Submission {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            packet_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default(),
            idempotency_key: row.get(3)?,
            transport_state: serde_json::from_str(&row.get::<_, String>(4)?)
                .unwrap_or(SubmissionTransportState::Unknown),
            ownership_generation: row.get::<_, i64>(5)? as u64,
            payer_receipt_id: row.get(6)?,
            detail_json: row.get(7)?,
            created_at: parse_dt(&row.get::<_, String>(8)?).unwrap_or_else(|_| Utc::now()),
            updated_at: parse_dt(&row.get::<_, String>(9)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_decisions(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Decision>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, submission_id, outcome, limitations_json, reason, created_at
         FROM decisions WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Decision {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            submission_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default(),
            outcome: serde_json::from_str(&row.get::<_, String>(3)?)
                .unwrap_or(DecisionOutcome::Unclear),
            limitations: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
            reason: row.get(5)?,
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_events(conn: &Connection, case_id: Uuid) -> LabResult<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, seq, kind, payload_json, created_at
         FROM events WHERE case_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(Event {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            seq: row.get::<_, i64>(2)? as u64,
            kind: row.get(3)?,
            payload_json: row.get(4)?,
            created_at: parse_dt(&row.get::<_, String>(5)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_pending(conn: &Connection, case_id: Uuid) -> LabResult<Vec<PendingWork>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, kind, ref_id, detail, due_at, created_at
         FROM pending_work WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        let due: Option<String> = row.get(5)?;
        Ok(PendingWork {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            kind: serde_json::from_str(&row.get::<_, String>(2)?)
                .unwrap_or(PendingKind::HumanClarification),
            ref_id: row.get(3)?,
            detail: row.get(4)?,
            due_at: due.and_then(|s| parse_dt(&s).ok()),
            created_at: parse_dt(&row.get::<_, String>(6)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn load_agent_runs(conn: &Connection, case_id: Uuid) -> LabResult<Vec<AgentRunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, case_id, task_id, prompt_version, model_id, context_version,
                tool_calls_json, structured_output_json, evidence_refs_json, created_at
         FROM agent_runs WHERE case_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![case_id.to_string()], |row| {
        Ok(AgentRunRecord {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            case_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap_or_default(),
            task_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap_or_default(),
            prompt_version: row.get(3)?,
            model_id: row.get(4)?,
            context_version: row.get::<_, i64>(5)? as u32,
            tool_calls_json: row.get(6)?,
            structured_output_json: row.get(7)?,
            evidence_refs: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
            created_at: parse_dt(&row.get::<_, String>(9)?).unwrap_or_else(|_| Utc::now()),
        })
    })?;
    collect_rows(rows)
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> LabResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn event_sequence_monotonic() {
        let dir = tempdir().unwrap();
        let store = CaseStore::open(&dir.path().join("lab.db")).unwrap();
        let now = Utc::now();
        let case = Case {
            id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            scenario_id: "t".into(),
            workflow_version: WORKFLOW_VERSION.into(),
            stage: CaseStage::Intake,
            service: ServiceContext {
                cpt: "72148".into(),
                diagnosis: "M54.5".into(),
                site: "outpatient".into(),
            },
            coverage: CoverageContext {
                payer_name: "p".into(),
                member_id: "m".into(),
                plan_id: "pl".into(),
                dos: "2026-10-01".into(),
            },
            service_version: 1,
            coverage_version: 1,
            disposition: None,
            paused_from: None,
            created_at: now,
            updated_at: now,
        };
        store.insert_case(&case).unwrap();
        let e1 = store
            .append_event(case.id, "a", &serde_json::json!({}), now)
            .unwrap();
        let e2 = store
            .append_event(case.id, "b", &serde_json::json!({}), now)
            .unwrap();
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
    }
}
