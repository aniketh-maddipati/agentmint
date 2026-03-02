//! Token minting endpoint with input validation, policy enforcement, and OIDC verification.

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
    pub id_token: Option<String>,
    // Orchestration fields (optional)
    pub scope: Option<Vec<String>>,
    pub delegates_to: Option<Vec<String>>,
    pub requires_checkpoint: Option<Vec<String>>,
    #[serde(default = "default_max_depth")]
    pub max_delegation_depth: Option<u32>,
}

fn default_ttl() -> i64 {
    60
}

fn default_max_depth() -> Option<u32> {
    None
}

#[derive(Serialize)]
pub struct MintResponse {
    pub token: String,
    pub jti: String,
    pub exp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_type: Option<String>,
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
        return Err(Error::InvalidToken(
            "action must be 1-64 chars (alphanumeric, underscore, colon, hyphen)".into(),
        ));
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

    // OIDC verification
    if let Some(ref oidc) = state.oidc {
        match &req.id_token {
            Some(token) => {
                let claims = oidc.verify(token).await.map_err(|e| {
                    crate::console::log_oidc_failure(&req.sub, &e.to_string());
                    state.metrics.record_oidc_failure();
                    Error::Unauthorized(format!("OIDC verification failed: {}", e))
                })?;

                // Verify sub matches
                let oidc_sub = claims.email.as_ref().unwrap_or(&claims.sub);
                if oidc_sub != &req.sub {
                    crate::console::log_oidc_mismatch(&req.sub, oidc_sub);
                    state.metrics.record_oidc_failure();
                    return Err(Error::Unauthorized(format!(
                        "sub mismatch: requested {} but id_token is for {}",
                        req.sub, oidc_sub
                    )));
                }

                crate::console::log_oidc_success(&req.sub);
            }
            None if state.require_oidc => {
                crate::console::log_oidc_required(&req.sub);
                state.metrics.record_oidc_failure();
                return Err(Error::Unauthorized("id_token required".into()));
            }
            None => {}
        }
    }

    // Policy check
    if let Err(v) = state.policy.check(&req.action) {
        crate::console::log_policy_denial(
            &req.sub,
            &req.action,
            v.action_type,
            v.limit,
            v.requested,
        );
        state.metrics.record_policy_denial();
        return Err(Error::PolicyViolation(format!(
            "{} limit is ${}. Requested: ${}",
            v.action_type, v.limit, v.requested
        )));
    }

    let ttl = clamp_ttl(req.ttl_seconds);

    // Build claims: plan receipt if orchestration fields present, basic receipt otherwise
    let is_plan = req.scope.is_some() || req.delegates_to.is_some();
    let claims = if is_plan {
        Claims::new_plan(
            req.sub,
            req.action,
            ttl,
            req.scope.unwrap_or_default(),
            req.delegates_to.unwrap_or_default(),
            req.requires_checkpoint.unwrap_or_default(),
            req.max_delegation_depth.unwrap_or(2),
        )
    } else {
        Claims::new(req.sub, req.action, ttl)
    };

    let jti = claims.jti.clone();
    let exp = claims.exp.to_rfc3339();
    let receipt_type = claims.receipt_type.clone();
    let token = sign_token(&claims, &state.signing_key)?;

    tracing::info!(sub = %claims.sub, action = %claims.action, jti = %jti, receipt_type = ?receipt_type, "token minted");
    crate::console::log_mint(&claims.sub, &claims.action, &jti);
    state.metrics.record_mint();

    Ok(Json(MintResponse { token, jti, exp, receipt_type }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(sub: &str, action: &str, ttl: i64) -> MintRequest {
        MintRequest {
            sub: sub.into(),
            action: action.into(),
            ttl_seconds: ttl,
            id_token: None,
            scope: None,
            delegates_to: None,
            requires_checkpoint: None,
            max_delegation_depth: None,
        }
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
    }
}
