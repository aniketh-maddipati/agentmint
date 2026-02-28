//! Shared application state.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::audit::sqlite::AuditLog;
use crate::error::Result;
use crate::jti::memory::JtiStore;
use crate::oidc::OidcVerifier;
use crate::policy::PolicyEngine;
use crate::ratelimit::{RateLimiter, RateLimitConfig};
use crate::telemetry::Metrics;
use crate::token::sign::generate_keypair;
use crate::webauthn::WebAuthnState;

pub struct AppStateInner {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub jti_store: JtiStore,
    pub audit_log: AuditLog,
    pub metrics: Metrics,
    pub policy: PolicyEngine,
    pub oidc: Option<OidcVerifier>,
    pub webauthn: Option<WebAuthnState>,
    pub rate_limiter: RateLimiter,
    pub require_oidc: bool,
    pub request_count: AtomicU64,
}

pub type AppState = Arc<AppStateInner>;

impl AppStateInner {
    pub fn increment_requests(&self) {
        let n = self.request_count.fetch_add(1, Relaxed) + 1;
        if n % 1000 == 0 {
            tracing::warn!(count = n, "high request volume");
        }
    }
}

struct StateBuilder {
    audit: AuditLog,
    policy: PolicyEngine,
    oidc: Option<OidcVerifier>,
    webauthn: Option<WebAuthnState>,
}

impl StateBuilder {
    fn build(self) -> AppState {
        let signing_key = generate_keypair();
        let verifying_key = signing_key.verifying_key();
        let require_oidc = std::env::var("REQUIRE_OIDC").map(|v| v == "true").unwrap_or(false);

        if require_oidc && self.oidc.is_none() {
            tracing::warn!("REQUIRE_OIDC=true but no OIDC configured");
        }

        Arc::new(AppStateInner {
            signing_key,
            verifying_key,
            jti_store: JtiStore::new(),
            audit_log: self.audit,
            metrics: Metrics::new(),
            policy: self.policy,
            oidc: self.oidc,
            webauthn: self.webauthn,
            rate_limiter: RateLimiter::new(RateLimitConfig::default()),
            require_oidc,
            request_count: AtomicU64::new(0),
        })
    }
}

pub fn build_state(db_path: &str) -> Result<AppState> {
    Ok(StateBuilder {
        audit: AuditLog::open(db_path)?,
        policy: PolicyEngine::from_default_file(),
        oidc: OidcVerifier::from_env(),
        webauthn: WebAuthnState::from_env(),
    }.build())
}

pub fn build_test_state() -> Result<AppState> {
    Ok(StateBuilder {
        audit: AuditLog::open_in_memory()?,
        policy: PolicyEngine::default(),
        oidc: None,
        webauthn: None,
    }.build())
}
