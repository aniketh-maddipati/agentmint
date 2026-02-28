use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use url::Url;
use webauthn_rs::prelude::*;

use crate::error::{Error, Result};
use crate::state::AppState;

// Hardening constants
const MAX_CHALLENGES: usize = 10_000;
const CHALLENGE_TTL: Duration = Duration::from_secs(300);
const LOCKOUT_THRESHOLD: u32 = 5;
const LOCKOUT_DURATION: Duration = Duration::from_secs(900);

pub struct WebAuthnState {
    core: Webauthn,
    reg_challenges: RwLock<HashMap<Box<str>, ChallengeEntry<PasskeyRegistration>>>,
    auth_challenges: RwLock<HashMap<Box<str>, ChallengeEntry<PasskeyAuthentication>>>,
    credentials: RwLock<HashMap<Box<str>, Passkey>>,
    failures: RwLock<HashMap<Box<str>, FailureRecord>>,
}

struct ChallengeEntry<T> {
    data: T,
    created: Instant,
}

struct FailureRecord {
    count: u32,
    last_failure: Instant,
}

impl WebAuthnState {
    pub fn new(rp_id: &str, rp_origin: &str) -> std::result::Result<Self, WebauthnError> {
        let origin = Url::parse(rp_origin).map_err(|_| WebauthnError::Configuration)?;
        let core = WebauthnBuilder::new(rp_id, &origin)?.build()?;

        Ok(Self {
            core,
            reg_challenges: RwLock::new(HashMap::new()),
            auth_challenges: RwLock::new(HashMap::new()),
            credentials: RwLock::new(HashMap::new()),
            failures: RwLock::new(HashMap::new()),
        })
    }

    pub fn from_env() -> Option<Self> {
        let rp_id = std::env::var("WEBAUTHN_RP_ID").ok()?;
        let rp_origin = std::env::var("WEBAUTHN_RP_ORIGIN").ok()?;

        Self::new(&rp_id, &rp_origin)
            .inspect(|_| tracing::info!(rp_id = %rp_id, "WebAuthn enabled"))
            .inspect_err(|e| tracing::warn!(error = ?e, "WebAuthn config failed"))
            .ok()
    }

    #[inline]
    fn require(opt: Option<&Self>) -> Result<&Self> {
        opt.ok_or_else(|| Error::Unauthorized("WebAuthn not configured".into()))
    }

    fn is_locked_out(&self, user_id: &str) -> bool {
        let failures = self.failures.read().unwrap();
        if let Some(record) = failures.get(user_id) {
            if record.count >= LOCKOUT_THRESHOLD {
                return record.last_failure.elapsed() < LOCKOUT_DURATION;
            }
        }
        false
    }

    fn record_failure(&self, user_id: &str) {
        let mut failures = self.failures.write().unwrap();
        let record = failures.entry(user_id.into()).or_insert(FailureRecord {
            count: 0,
            last_failure: Instant::now(),
        });
        record.count += 1;
        record.last_failure = Instant::now();
    }

    fn clear_failures(&self, user_id: &str) {
        self.failures.write().unwrap().remove(user_id);
    }

    fn cleanup_expired<T>(map: &mut HashMap<Box<str>, ChallengeEntry<T>>) {
        map.retain(|_, entry| entry.created.elapsed() < CHALLENGE_TTL);
    }

    fn check_capacity<T>(map: &HashMap<Box<str>, ChallengeEntry<T>>) -> Result<()> {
        if map.len() >= MAX_CHALLENGES {
            return Err(Error::ServiceUnavailable("challenge store at capacity".into()));
        }
        Ok(())
    }
}

// === Types ===

#[derive(Deserialize)]
pub struct RegStartReq {
    pub user_id: String,
    pub user_name: String,
}

#[derive(Serialize)]
pub struct RegStartRes {
    pub challenge: CreationChallengeResponse,
}

#[derive(Deserialize)]
pub struct RegFinishReq {
    pub user_id: String,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Deserialize)]
pub struct AuthStartReq {
    pub user_id: String,
}

#[derive(Serialize)]
pub struct AuthStartRes {
    pub challenge: RequestChallengeResponse,
}

#[derive(Deserialize)]
pub struct AuthFinishReq {
    pub user_id: String,
    pub credential: PublicKeyCredential,
}

#[derive(Serialize)]
pub struct SuccessRes {
    pub success: bool,
}

// === Handlers ===

pub async fn register_start(
    State(state): State<AppState>,
    Json(req): Json<RegStartReq>,
) -> Result<Json<RegStartRes>> {
    let wa = WebAuthnState::require(state.webauthn.as_ref())?;

    // Rate limit per user
    state.rate_limiter.check_user(&req.user_id)
        .map_err(|e| Error::RateLimited(e.to_string()))?;

    let user_id = Uuid::parse_str(&req.user_id).unwrap_or_else(|_| Uuid::new_v4());

    let (challenge, reg_state) = wa.core
        .start_passkey_registration(user_id, &req.user_name, &req.user_name, None)
        .map_err(|e| Error::Unauthorized(format!("{:?}", e)))?;

    {
        let mut challenges = wa.reg_challenges.write().unwrap();
        WebAuthnState::cleanup_expired(&mut challenges);
        WebAuthnState::check_capacity(&challenges)?;
        challenges.insert(req.user_id.into_boxed_str(), ChallengeEntry {
            data: reg_state,
            created: Instant::now(),
        });
    }

    Ok(Json(RegStartRes { challenge }))
}

pub async fn register_finish(
    State(state): State<AppState>,
    Json(req): Json<RegFinishReq>,
) -> Result<Json<SuccessRes>> {
    let wa = WebAuthnState::require(state.webauthn.as_ref())?;

    let entry = wa.reg_challenges
        .write()
        .unwrap()
        .remove(req.user_id.as_str())
        .ok_or_else(|| Error::Unauthorized("no pending registration".into()))?;

    // Check TTL
    if entry.created.elapsed() > CHALLENGE_TTL {
        return Err(Error::Unauthorized("challenge expired".into()));
    }

    let passkey = wa.core
        .finish_passkey_registration(&req.credential, &entry.data)
        .map_err(|e| Error::Unauthorized(format!("{:?}", e)))?;

    wa.credentials
        .write()
        .unwrap()
        .insert(req.user_id.clone().into_boxed_str(), passkey);

    crate::console::log_webauthn_register(&req.user_id);
    state.metrics.record_webauthn_register();

    Ok(Json(SuccessRes { success: true }))
}

pub async fn auth_start(
    State(state): State<AppState>,
    Json(req): Json<AuthStartReq>,
) -> Result<Json<AuthStartRes>> {
    let wa = WebAuthnState::require(state.webauthn.as_ref())?;

    // Check lockout
    if wa.is_locked_out(&req.user_id) {
        crate::console::log_webauthn_lockout(&req.user_id);
        state.metrics.record_webauthn_lockout();
        return Err(Error::RateLimited("account temporarily locked".into()));
    }

    // Rate limit per user
    state.rate_limiter.check_user(&req.user_id)
        .map_err(|e| Error::RateLimited(e.to_string()))?;

    let passkey = wa.credentials
        .read()
        .unwrap()
        .get(req.user_id.as_str())
        .cloned()
        .ok_or_else(|| Error::Unauthorized("user not registered".into()))?;

    let (challenge, auth_state) = wa.core
        .start_passkey_authentication(&[passkey])
        .map_err(|e| Error::Unauthorized(format!("{:?}", e)))?;

    {
        let mut challenges = wa.auth_challenges.write().unwrap();
        WebAuthnState::cleanup_expired(&mut challenges);
        WebAuthnState::check_capacity(&challenges)?;
        challenges.insert(req.user_id.into_boxed_str(), ChallengeEntry {
            data: auth_state,
            created: Instant::now(),
        });
    }

    Ok(Json(AuthStartRes { challenge }))
}

pub async fn auth_finish(
    State(state): State<AppState>,
    Json(req): Json<AuthFinishReq>,
) -> Result<Json<SuccessRes>> {
    let wa = WebAuthnState::require(state.webauthn.as_ref())?;

    // Check lockout
    if wa.is_locked_out(&req.user_id) {
        return Err(Error::RateLimited("account temporarily locked".into()));
    }

    let entry = wa.auth_challenges
        .write()
        .unwrap()
        .remove(req.user_id.as_str())
        .ok_or_else(|| Error::Unauthorized("no pending auth".into()))?;

    // Check TTL
    if entry.created.elapsed() > CHALLENGE_TTL {
        return Err(Error::Unauthorized("challenge expired".into()));
    }

    match wa.core.finish_passkey_authentication(&req.credential, &entry.data) {
        Ok(_) => {
            wa.clear_failures(&req.user_id);
            crate::console::log_webauthn_auth(&req.user_id);
            state.metrics.record_webauthn_success();
            Ok(Json(SuccessRes { success: true }))
        }
        Err(e) => {
            wa.record_failure(&req.user_id);
            crate::console::log_webauthn_failure(&req.user_id);
            state.metrics.record_webauthn_failure();
            Err(Error::Unauthorized(format!("{:?}", e)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_when_not_set() {
        std::env::remove_var("WEBAUTHN_RP_ID");
        std::env::remove_var("WEBAUTHN_RP_ORIGIN");
        assert!(WebAuthnState::from_env().is_none());
    }

    #[test]
    fn lockout_after_threshold() {
        let wa = WebAuthnState::new("test.com", "https://test.com").unwrap();
        
        for _ in 0..LOCKOUT_THRESHOLD {
            wa.record_failure("alice");
        }
        
        assert!(wa.is_locked_out("alice"));
        assert!(!wa.is_locked_out("bob"));
    }

    #[test]
    fn clear_failures_removes_lockout() {
        let wa = WebAuthnState::new("test.com", "https://test.com").unwrap();
        
        for _ in 0..LOCKOUT_THRESHOLD {
            wa.record_failure("alice");
        }
        
        assert!(wa.is_locked_out("alice"));
        wa.clear_failures("alice");
        assert!(!wa.is_locked_out("alice"));
    }
}