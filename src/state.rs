//! Shared application state for Axum handlers.
//! Used by: handlers, server, main.

use std::sync::Arc;

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::audit::sqlite::AuditLog;
use crate::error::Result;
use crate::jti::memory::JtiStore;
use crate::token::sign::generate_keypair;

pub struct AppStateInner {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub jti_store: JtiStore,
    pub audit_log: AuditLog,
}

pub type AppState = Arc<AppStateInner>;

pub fn build_state(db_path: &str) -> Result<AppState> {
    let signing_key = generate_keypair();
    let verifying_key = signing_key.verifying_key();
    let jti_store = JtiStore::new();
    let audit_log = AuditLog::open(db_path)?;
    Ok(Arc::new(AppStateInner {
        signing_key,
        verifying_key,
        jti_store,
        audit_log,
    }))
}

pub fn build_test_state() -> Result<AppState> {
    let signing_key = generate_keypair();
    let verifying_key = signing_key.verifying_key();
    let jti_store = JtiStore::new();
    let audit_log = AuditLog::open_in_memory()?;
    Ok(Arc::new(AppStateInner {
        signing_key,
        verifying_key,
        jti_store,
        audit_log,
    }))
}
