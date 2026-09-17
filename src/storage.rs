//! SQLite persistence with short transactions and WAL.
//! Used by: execution engine. SQLite work runs off the Tokio executor.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::domain::{
    can_transition, ActionRecord, ActionStatus, Approval, ExecutionAttempt, ExecutionGrant,
    PolicyDecision, ProviderResult, SignedReceipt,
};
use crate::error::{Error, Result};

const MIGRATION_V1: &str = include_str!("../migrations/001_init.sql");

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(|err| Error::internal("open sqlite", err))?;
        configure(&conn)?;
        migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn with<T, F>(&self, func: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|err| Error::internal("sqlite lock", err))?;
            func(&guard)
        })
        .await
        .map_err(|err| Error::internal("spawn_blocking", err))?
    }

    pub async fn insert_action(&self, record: ActionRecord) -> Result<()> {
        self.with(move |conn| insert_action(conn, &record)).await
    }

    pub async fn get_action(
        &self,
        tenant_id: &str,
        action_id: Uuid,
    ) -> Result<Option<ActionRecord>> {
        let tenant_id = tenant_id.to_owned();
        self.with(move |conn| get_action(conn, &tenant_id, action_id))
            .await
    }

    pub async fn claim_execution(
        &self,
        tenant_id: &str,
        action_id: Uuid,
        attempt: ExecutionAttempt,
    ) -> Result<bool> {
        let tenant_id = tenant_id.to_owned();
        self.with(move |conn| claim_execution(conn, &tenant_id, action_id, &attempt))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn complete(
        &self,
        tenant_id: &str,
        action_id: Uuid,
        from: ActionStatus,
        to: ActionStatus,
        attempt: &ExecutionAttempt,
        provider_result: Option<ProviderResult>,
        receipt: Option<SignedReceipt>,
        reconciliation_required: bool,
    ) -> Result<()> {
        let tenant_id = tenant_id.to_owned();
        let attempt = attempt.clone();
        let provider_result = provider_result.clone();
        let receipt = receipt.clone();
        self.with(move |conn| {
            complete(
                conn,
                &tenant_id,
                action_id,
                from,
                to,
                &attempt,
                provider_result.as_ref(),
                receipt.as_ref(),
                reconciliation_required,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_approval(
        &self,
        tenant_id: &str,
        action_id: Uuid,
        from: ActionStatus,
        to: ActionStatus,
        approval: Option<Approval>,
        grant: Option<ExecutionGrant>,
        policy: Option<PolicyDecision>,
    ) -> Result<()> {
        let tenant_id = tenant_id.to_owned();
        self.with(move |conn| {
            record_decision(
                conn,
                &tenant_id,
                action_id,
                from,
                to,
                approval.as_ref(),
                grant.as_ref(),
                policy.as_ref(),
            )
        })
        .await
    }

    pub async fn tamper_arguments(
        &self,
        tenant_id: &str,
        action_id: Uuid,
        arguments: serde_json::Value,
    ) -> Result<()> {
        let tenant_id = tenant_id.to_owned();
        self.with(move |conn| tamper_arguments(conn, &tenant_id, action_id, &arguments))
            .await
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;",
    )
    .map_err(|err| Error::internal("pragma", err))?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_V1)
        .map_err(|err| Error::internal("migration", err))?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (1, ?1)",
        params![Utc::now().to_rfc3339()],
    )
    .map_err(|err| Error::internal("migration version", err))?;
    Ok(())
}

fn insert_action(conn: &Connection, record: &ActionRecord) -> Result<()> {
    let intent_json =
        serde_json::to_string(&record.intent).map_err(|err| Error::internal("intent json", err))?;
    conn.execute(
        "INSERT INTO actions (
            id, tenant_id, status, intent_json, intent_hash, canonical_version, canonical_json,
            provider, operation, arguments_json, idempotency_key, created_at, expires_at, updated_at,
            policy_json, approval_json, grant_json, provider_result_json, reconciliation_required
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        params![
            record.intent.action_id.to_string(),
            record.intent.tenant_id,
            record.status.as_str(),
            intent_json,
            record.intent_hash,
            record.canonical_version,
            record.canonical_json,
            record.intent.provider,
            record.intent.operation,
            record.intent.arguments.to_string(),
            record.intent.idempotency_key,
            record.intent.created_at.to_rfc3339(),
            record.intent.expires_at.to_rfc3339(),
            Utc::now().to_rfc3339(),
            json_opt(&record.policy)?,
            json_opt(&record.approval)?,
            json_opt(&record.grant)?,
            json_opt(&record.provider_result)?,
            record.reconciliation_required as i64,
        ],
    )
    .map_err(|err| Error::internal("insert action", err))?;
    insert_transition(
        conn,
        &record.intent.tenant_id,
        record.intent.action_id,
        ActionStatus::Proposed,
        record.status,
        "propose",
    )?;
    Ok(())
}

fn get_action(conn: &Connection, tenant_id: &str, action_id: Uuid) -> Result<Option<ActionRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT intent_json, status, intent_hash, canonical_version, canonical_json,
                    policy_json, approval_json, grant_json, provider_result_json, reconciliation_required
             FROM actions WHERE id = ?1 AND tenant_id = ?2",
        )
        .map_err(|err| Error::internal("prepare get", err))?;
    let row = stmt
        .query_row(params![action_id.to_string(), tenant_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .optional()
        .map_err(|err| Error::internal("get action", err))?;
    let Some((
        intent_json,
        status,
        intent_hash,
        canonical_version,
        canonical_json,
        policy_json,
        approval_json,
        grant_json,
        provider_result_json,
        reconciliation_required,
    )) = row
    else {
        return Ok(None);
    };
    let status = ActionStatus::parse(&status).ok_or(Error::Internal)?;
    let latest_attempt = latest_attempt(conn, action_id)?;
    let receipt = get_receipt(conn, tenant_id, action_id)?;
    Ok(Some(ActionRecord {
        intent: serde_json::from_str(&intent_json)
            .map_err(|err| Error::internal("intent parse", err))?,
        status,
        intent_hash,
        canonical_version,
        canonical_json,
        policy: parse_opt(&policy_json)?,
        approval: parse_opt(&approval_json)?,
        grant: parse_opt(&grant_json)?,
        provider_result: parse_opt(&provider_result_json)?,
        reconciliation_required: reconciliation_required != 0,
        latest_attempt,
        receipt,
    }))
}

fn latest_attempt(conn: &Connection, action_id: Uuid) -> Result<Option<ExecutionAttempt>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, action_id, tenant_id, attempt_no, provider_idempotency_key, status, started_at, completed_at
             FROM attempts WHERE action_id = ?1 ORDER BY attempt_no DESC LIMIT 1",
        )
        .map_err(|err| Error::internal("prepare attempt", err))?;
    let row = stmt
        .query_row(params![action_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .optional()
        .map_err(|err| Error::internal("get attempt", err))?;
    let Some((id, action, tenant, attempt_no, key, status, started, completed)) = row else {
        return Ok(None);
    };
    Ok(Some(ExecutionAttempt {
        attempt_id: Uuid::parse_str(&id).map_err(|err| Error::internal("attempt id", err))?,
        action_id: Uuid::parse_str(&action).map_err(|err| Error::internal("action id", err))?,
        tenant_id: tenant,
        attempt_no,
        provider_idempotency_key: key,
        status,
        started_at: parse_time(&started)?,
        completed_at: completed.as_deref().map(parse_time).transpose()?,
    }))
}

fn get_receipt(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
) -> Result<Option<SignedReceipt>> {
    let mut stmt = conn
        .prepare("SELECT signed_json FROM receipts WHERE action_id = ?1 AND tenant_id = ?2")
        .map_err(|err| Error::internal("prepare receipt", err))?;
    let row = stmt
        .query_row(params![action_id.to_string(), tenant_id], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|err| Error::internal("get receipt", err))?;
    row.map(|json| serde_json::from_str(&json).map_err(|err| Error::internal("receipt parse", err)))
        .transpose()
}

fn claim_execution(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
    attempt: &ExecutionAttempt,
) -> Result<bool> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| Error::internal("begin claim", err))?;
    let changed = tx
        .execute(
            "UPDATE actions SET status = ?1, updated_at = ?2, reconciliation_required = 0
             WHERE id = ?3 AND tenant_id = ?4 AND status = ?5",
            params![
                ActionStatus::Executing.as_str(),
                Utc::now().to_rfc3339(),
                action_id.to_string(),
                tenant_id,
                ActionStatus::Authorized.as_str()
            ],
        )
        .map_err(|err| Error::internal("cas executing", err))?;
    if changed != 1 {
        tx.rollback()
            .map_err(|err| Error::internal("rollback claim", err))?;
        return Ok(false);
    }
    if !can_transition(ActionStatus::Authorized, ActionStatus::Executing) {
        return Err(Error::Internal);
    }
    tx.execute(
        "INSERT INTO attempts (
            id, action_id, tenant_id, attempt_no, provider_idempotency_key, status, started_at, completed_at, provider_result_json
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,NULL)",
        params![
            attempt.attempt_id.to_string(),
            action_id.to_string(),
            tenant_id,
            attempt.attempt_no,
            attempt.provider_idempotency_key,
            "started",
            attempt.started_at.to_rfc3339()
        ],
    )
    .map_err(|err| Error::internal("insert attempt", err))?;
    insert_transition_on(
        &tx,
        tenant_id,
        action_id,
        ActionStatus::Authorized,
        ActionStatus::Executing,
        "claim",
    )?;
    tx.commit()
        .map_err(|err| Error::internal("commit claim", err))?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn complete(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
    from: ActionStatus,
    to: ActionStatus,
    attempt: &ExecutionAttempt,
    provider_result: Option<&ProviderResult>,
    receipt: Option<&SignedReceipt>,
    reconciliation_required: bool,
) -> Result<()> {
    if !can_transition(from, to) {
        return Err(Error::Conflict);
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| Error::internal("begin complete", err))?;
    let changed = tx
        .execute(
            "UPDATE actions SET status = ?1, updated_at = ?2, provider_result_json = ?3, reconciliation_required = ?4
             WHERE id = ?5 AND tenant_id = ?6 AND status = ?7",
            params![
                to.as_str(),
                Utc::now().to_rfc3339(),
                json_opt(&provider_result.cloned())?,
                reconciliation_required as i64,
                action_id.to_string(),
                tenant_id,
                from.as_str()
            ],
        )
        .map_err(|err| Error::internal("complete update", err))?;
    if changed != 1 {
        tx.rollback()
            .map_err(|err| Error::internal("rollback complete", err))?;
        return Err(Error::Conflict);
    }
    tx.execute(
        "UPDATE attempts SET status = ?1, completed_at = ?2, provider_result_json = ?3 WHERE id = ?4 AND tenant_id = ?5",
        params![
            to.as_str(),
            attempt.completed_at.map(|t| t.to_rfc3339()),
            json_opt(&provider_result.cloned())?,
            attempt.attempt_id.to_string(),
            tenant_id
        ],
    )
    .map_err(|err| Error::internal("complete attempt", err))?;
    if let Some(receipt) = receipt {
        let signed =
            serde_json::to_string(receipt).map_err(|err| Error::internal("receipt json", err))?;
        tx.execute(
            "INSERT OR REPLACE INTO receipts (action_id, tenant_id, signed_json, kid, signed_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                action_id.to_string(),
                tenant_id,
                signed,
                receipt.kid,
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(|err| Error::internal("insert receipt", err))?;
    }
    insert_transition_on(&tx, tenant_id, action_id, from, to, "complete")?;
    tx.commit()
        .map_err(|err| Error::internal("commit complete", err))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_decision(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
    from: ActionStatus,
    to: ActionStatus,
    approval: Option<&Approval>,
    grant: Option<&ExecutionGrant>,
    policy: Option<&PolicyDecision>,
) -> Result<()> {
    if !can_transition(from, to) {
        return Err(Error::Conflict);
    }
    let changed = conn
        .execute(
            "UPDATE actions SET status = ?1, updated_at = ?2, approval_json = ?3, grant_json = ?4, policy_json = COALESCE(?5, policy_json)
             WHERE id = ?6 AND tenant_id = ?7 AND status = ?8",
            params![
                to.as_str(),
                Utc::now().to_rfc3339(),
                json_opt(&approval.cloned())?,
                json_opt(&grant.cloned())?,
                json_opt(&policy.cloned())?,
                action_id.to_string(),
                tenant_id,
                from.as_str()
            ],
        )
        .map_err(|err| Error::internal("record decision", err))?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    insert_transition(conn, tenant_id, action_id, from, to, "decision")?;
    Ok(())
}

fn tamper_arguments(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
    arguments: &serde_json::Value,
) -> Result<()> {
    let mut record = get_action(conn, tenant_id, action_id)?.ok_or(Error::NotFound)?;
    record.intent.arguments = arguments.clone();
    let intent_json =
        serde_json::to_string(&record.intent).map_err(|err| Error::internal("tamper json", err))?;
    conn.execute(
        "UPDATE actions SET intent_json = ?1, arguments_json = ?2 WHERE id = ?3 AND tenant_id = ?4",
        params![
            intent_json,
            arguments.to_string(),
            action_id.to_string(),
            tenant_id
        ],
    )
    .map_err(|err| Error::internal("tamper update", err))?;
    Ok(())
}

fn insert_transition(
    conn: &Connection,
    tenant_id: &str,
    action_id: Uuid,
    from: ActionStatus,
    to: ActionStatus,
    note: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO transitions (action_id, tenant_id, from_status, to_status, at, note)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            action_id.to_string(),
            tenant_id,
            from.as_str(),
            to.as_str(),
            Utc::now().to_rfc3339(),
            note
        ],
    )
    .map_err(|err| Error::internal("insert transition", err))?;
    Ok(())
}

fn insert_transition_on(
    tx: &rusqlite::Transaction<'_>,
    tenant_id: &str,
    action_id: Uuid,
    from: ActionStatus,
    to: ActionStatus,
    note: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO transitions (action_id, tenant_id, from_status, to_status, at, note)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            action_id.to_string(),
            tenant_id,
            from.as_str(),
            to.as_str(),
            Utc::now().to_rfc3339(),
            note
        ],
    )
    .map_err(|err| Error::internal("insert transition", err))?;
    Ok(())
}

fn json_opt<T: serde::Serialize>(value: &Option<T>) -> Result<Option<String>> {
    value
        .as_ref()
        .map(|item| serde_json::to_string(item).map_err(|err| Error::internal("json", err)))
        .transpose()
}

fn parse_opt<T: serde::de::DeserializeOwned>(value: &Option<String>) -> Result<Option<T>> {
    value
        .as_ref()
        .map(|item| serde_json::from_str(item).map_err(|err| Error::internal("json parse", err)))
        .transpose()
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| Error::internal("timestamp", err))
}
