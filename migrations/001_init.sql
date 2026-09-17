CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS actions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    status TEXT NOT NULL,
    intent_json TEXT NOT NULL,
    intent_hash TEXT NOT NULL,
    canonical_version TEXT NOT NULL,
    canonical_json TEXT NOT NULL,
    provider TEXT NOT NULL,
    operation TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    policy_json TEXT,
    approval_json TEXT,
    grant_json TEXT,
    provider_result_json TEXT,
    reconciliation_required INTEGER NOT NULL DEFAULT 0,
    UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_actions_tenant_status ON actions (tenant_id, status);

CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    attempt_no INTEGER NOT NULL,
    provider_idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    provider_result_json TEXT,
    UNIQUE (action_id, attempt_no),
    FOREIGN KEY (action_id) REFERENCES actions(id)
);

CREATE TABLE IF NOT EXISTS transitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    at TEXT NOT NULL,
    note TEXT
);

CREATE TABLE IF NOT EXISTS receipts (
    action_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    signed_json TEXT NOT NULL,
    kid TEXT NOT NULL,
    signed_at TEXT NOT NULL,
    FOREIGN KEY (action_id) REFERENCES actions(id)
);
