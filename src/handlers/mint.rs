//! Token minting endpoint with input validation.
//! Used by: server.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::token::claims::Claims;
use crate::token::sign::sign_token;

#[derive(Deserialize)]
pub struct MintRequest {
    pub sub: String,
    pub action: String,
    #[serde(default = "default_ttl")]
    pub ttl_seconds: i64,
}

fn default_ttl() -> i64 {
    300
}

#[derive(Serialize)]
pub struct MintResponse {
    pub token: String,
    pub jti: String,
    pub exp: String,
}

fn validate_request(req: &MintRequest) -> Result<()> {
    if req.sub.is_empty() || req.sub.len() > 256 {
        return Err(Error::Validation("sub must be 1-256 characters".into()));
    }
    if req.sub.chars().any(|c| c.is_control()) {
        return Err(Error::Validation("sub contains control characters".into()));
    }
    if req.action.is_empty()
        || req.action.len() > 64
        || !req.action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(Error::Validation("action must be 1-64 alphanumeric/underscore chars".into()));
    }
    if !(1..=300).contains(&req.ttl_seconds) {
        return Err(Error::Validation("ttl_seconds must be 1-300".into()));
    }
    Ok(())
}

pub async fn mint(
    State(state): State<AppState>,
    Json(req): Json<MintRequest>,
) -> Result<Json<MintResponse>> {
    validate_request(&req)?;
    let claims = Claims::new(req.sub, req.action, req.ttl_seconds);
    let jti = claims.jti.clone();
    let exp = claims.exp.to_rfc3339();
    let token = sign_token(&claims, &state.signing_key)?;
    tracing::info!(sub = %claims.sub, action = %claims.action, jti = %jti, "token minted");
    state.metrics.record_mint();
    Ok(Json(MintResponse { token, jti, exp }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(sub: &str, action: &str, ttl: i64) -> MintRequest {
        MintRequest { sub: sub.into(), action: action.into(), ttl_seconds: ttl }
    }

    #[test]
    fn valid_request_passes() {
        assert!(validate_request(&req("agent-1", "deploy", 60)).is_ok());
    }

    #[test]
    fn empty_sub_rejected() {
        assert!(validate_request(&req("", "deploy", 60)).is_err());
    }

    #[test]
    fn long_sub_rejected() {
        assert!(validate_request(&req(&"a".repeat(257), "deploy", 60)).is_err());
    }

    #[test]
    fn control_chars_in_sub_rejected() {
        assert!(validate_request(&req("agent\x00", "deploy", 60)).is_err());
    }

    #[test]
    fn invalid_action_rejected() {
        assert!(validate_request(&req("a", "deploy!", 60)).is_err());
    }

    #[test]
    fn empty_action_rejected() {
        assert!(validate_request(&req("a", "", 60)).is_err());
    }

    #[test]
    fn ttl_bounds_enforced() {
        assert!(validate_request(&req("a", "x", 0)).is_err());
        assert!(validate_request(&req("a", "x", 301)).is_err());
        assert!(validate_request(&req("a", "x", 1)).is_ok());
        assert!(validate_request(&req("a", "x", 300)).is_ok());
    }
}
