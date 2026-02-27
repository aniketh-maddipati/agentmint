//! Shared application state for Axum handlers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::audit::sqlite::AuditLog;
use crate::error::Result;
use crate::jti::memory::JtiStore;
use crate::oidc::OidcVerifier;
use crate::policy::PolicyEngine;
use crate::telemetry::Metrics;
use crate::token::sign::generate_keypair;

pub struct AppStateInner {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub jti_store: JtiStore,
    pub audit_log: AuditLog,
    pub metrics: Metrics,
    pub policy: PolicyEngine,
    pub oidc: Option<OidcVerifier>,
    pub require_oidc: bool,
    pub request_count: AtomicU64,
}

pub type AppState = Arc<AppStateInner>;

impl AppStateInner {
    pub fn increment_requests(&self) {
        let count = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 1000 == 0 {
            tracing::warn!(count, "high request volume");
        }
    }
}

fn build_inner(audit_log: AuditLog, policy: PolicyEngine, oidc: Option<OidcVerifier>) -> AppState {
    let signing_key = generate_keypair();
    let verifying_key = signing_key.verifying_key();
    let require_oidc = std::env::var("REQUIRE_OIDC").map(|v| v == "true").unwrap_or(false);
    
    if require_oidc && oidc.is_none() {
        tracing::warn!("REQUIRE_OIDC=true but no OIDC config provided");
    }
    
    Arc::new(AppStateInner {
        signing_key,
        verifying_key,
        jti_store: JtiStore::new(),
        audit_log,
        metrics: Metrics::new(),
        policy,
        oidc,
        require_oidc,
        request_count: AtomicU64::new(0),
    })
}

pub fn build_state(db_path: &str) -> Result<AppState> {
    let policy = PolicyEngine::from_default_file();
    let oidc = OidcVerifier::from_env();
    Ok(build_inner(AuditLog::open(db_path)?, policy, oidc))
}

pub fn build_test_state() -> Result<AppState> {
    Ok(build_inner(AuditLog::open_in_memory()?, PolicyEngine::default(), None))
}