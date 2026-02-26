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
    60
}

#[derive(Serialize)]
pub struct MintResponse {
    pub token: String,
    pub jti: String,
    pub exp: String,
}

fn validate_request(req: &MintRequest) -> Result<()> {
    if req.sub.is_empty() || req.sub.len() > 256 {
        return Err(Error::InvalidToken("sub must be 1-256 characters".into()));
    }
    if req.sub.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidToken("sub contains control characters".into()));
    }
    if req.action.is_empty()
        || req.action.len() > 64
        || !req.action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
    {
        return Err(Error::InvalidToken("action must be 1-64 chars (alphanumeric, underscore, colon, hyphen)".into()));
    }
    Ok(())
}

fn clamp_ttl(ttl: i64) -> i64 {
    ttl.clamp(1, 300)
}

pub async fn mint(
    State(state): State<AppState>,
    Json(req): Json<MintRequest>,
) -> Result<Json<MintResponse>> {
    validate_request(&req)?;
    let ttl = clamp_ttl(req.ttl_seconds);
    let claims = Claims::new(req.sub, req.action, ttl);
    let jti = claims.jti.clone();
    let exp = claims.exp.to_rfc3339();
    let token = sign_token(&claims, &state.signing_key)?;
    tracing::info!(sub = %claims.sub, action = %claims.action, jti = %jti, "token minted");
    crate::console::log_mint(&claims.sub, &claims.action, &jti);
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
    fn action_allows_colons_and_hyphens() {
        assert!(validate_request(&req("a", "refund:order:123", 60)).is_ok());
        assert!(validate_request(&req("a", "deploy-prod", 60)).is_ok());
    }

    #[test]
    fn empty_action_rejected() {
        assert!(validate_request(&req("a", "", 60)).is_err());
    }

    #[test]
    fn ttl_clamped_to_bounds() {
        assert_eq!(clamp_ttl(0), 1);
        assert_eq!(clamp_ttl(-5), 1);
        assert_eq!(clamp_ttl(500), 300);
        assert_eq!(clamp_ttl(60), 60);
        assert_eq!(clamp_ttl(1), 1);
        assert_eq!(clamp_ttl(300), 300);
    }
}
