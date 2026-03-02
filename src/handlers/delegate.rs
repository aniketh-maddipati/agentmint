//! Delegation endpoint: creates scoped delegated receipts from a parent receipt.
//! Used by: server.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::token::claims::Claims;
use crate::token::sign::sign_token;
use crate::token::verify::verify_token;

#[derive(Deserialize)]
pub struct DelegateRequest {
    pub parent_token: String,
    pub agent_id: String,
    pub action: String,
}

#[derive(Serialize)]
pub struct DelegateResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub chain: Vec<String>,
}

fn action_matches_pattern(action: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with(":*") {
        let prefix = &pattern[..pattern.len() - 2];
        return action == prefix || action.starts_with(&format!("{}:", prefix));
    }
    action == pattern
}

fn action_in_scope(action: &str, scope: &[String]) -> bool {
    scope.iter().any(|pattern| action_matches_pattern(action, pattern))
}

fn build_chain(parent: &Claims) -> Vec<String> {
    let mut chain = Vec::new();
    if let Some(ref pjti) = parent.parent_jti {
        chain.push(pjti.clone());
    }
    chain.push(parent.jti.clone());
    chain
}

pub async fn delegate(
    State(state): State<AppState>,
    Json(req): Json<DelegateRequest>,
) -> Result<Json<DelegateResponse>> {
    // Validate input
    if req.agent_id.is_empty() || req.agent_id.len() > 256 {
        return Err(Error::InvalidToken("agent_id must be 1-256 characters".into()));
    }
    if req.action.is_empty()
        || req.action.len() > 64
        || !req.action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
    {
        return Err(Error::InvalidToken(
            "action must be 1-64 chars (alphanumeric, underscore, colon, hyphen)".into(),
        ));
    }

    // Verify the parent token
    let parent = verify_token(&req.parent_token, &state.verifying_key).map_err(|e| {
        tracing::warn!(error = %e, "delegate: parent token verification failed");
        e
    })?;

    let chain = build_chain(&parent);

    // Check 1: Is this agent authorized to receive delegation?
    if let Some(ref delegates) = parent.delegates_to {
        if !delegates.contains(&req.agent_id) {
            tracing::info!(
                agent = %req.agent_id,
                action = %req.action,
                "delegate: agent not authorized"
            );
            crate::console::log_delegation_denied(&req.agent_id, &req.action, "agent_not_authorized");
            return Ok(Json(DelegateResponse {
                status: "denied".into(),
                token: None,
                jti: None,
                reason: Some("agent_not_authorized".into()),
                chain,
            }));
        }
    }

    // Check 2: Has max delegation depth been exceeded?
    let current_depth = parent.depth.unwrap_or(0);
    let max_depth = parent.max_delegation_depth.unwrap_or(1);
    if current_depth >= max_depth {
        tracing::info!(
            agent = %req.agent_id,
            depth = current_depth,
            max = max_depth,
            "delegate: max depth exceeded"
        );
        crate::console::log_delegation_denied(&req.agent_id, &req.action, "max_depth_exceeded");
        return Ok(Json(DelegateResponse {
            status: "denied".into(),
            token: None,
            jti: None,
            reason: Some("max_depth_exceeded".into()),
            chain,
        }));
    }

    // Check 3: Does this action require an explicit checkpoint?
    if let Some(ref checkpoints) = parent.requires_checkpoint {
        if action_in_scope(&req.action, checkpoints) {
            tracing::info!(
                agent = %req.agent_id,
                action = %req.action,
                "delegate: checkpoint required"
            );
            crate::console::log_checkpoint_required(&req.agent_id, &req.action);
            return Ok(Json(DelegateResponse {
                status: "checkpoint_required".into(),
                token: None,
                jti: None,
                reason: Some(format!("action '{}' requires explicit human approval", req.action)),
                chain,
            }));
        }
    }

    // Check 4: Is the action within the approved scope?
    if let Some(ref scope) = parent.scope {
        if !action_in_scope(&req.action, scope) {
            tracing::info!(
                agent = %req.agent_id,
                action = %req.action,
                "delegate: action not in scope"
            );
            crate::console::log_delegation_denied(&req.agent_id, &req.action, "action_not_in_scope");
            return Ok(Json(DelegateResponse {
                status: "denied".into(),
                token: None,
                jti: None,
                reason: Some("action_not_in_scope".into()),
                chain,
            }));
        }
    }

    // All checks passed — issue delegated receipt
    let remaining_seconds = (parent.exp - chrono::Utc::now()).num_seconds().max(1);
    let ttl = remaining_seconds.min(300);

    let claims = Claims::new_delegated(
        req.agent_id.clone(),
        req.action.clone(),
        ttl,
        &parent,
    );
    let jti = claims.jti.clone();
    let token = sign_token(&claims, &state.signing_key)?;

    // Audit log
    state.audit_log.log(&jti, &req.agent_id, &req.action, chrono::Utc::now())?;

    tracing::info!(
        agent = %req.agent_id,
        action = %req.action,
        jti = %jti,
        parent_jti = %parent.jti,
        depth = current_depth + 1,
        "delegate: receipt issued"
    );
    crate::console::log_delegation_approved(&req.agent_id, &req.action, &jti);
    state.metrics.record_mint();

    let mut full_chain = chain;
    full_chain.push(jti.clone());

    Ok(Json(DelegateResponse {
        status: "ok".into(),
        token: Some(token),
        jti: Some(jti),
        reason: None,
        chain: full_chain,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches() {
        assert!(action_matches_pattern("build:docker", "build:*"));
        assert!(action_matches_pattern("build", "build:*"));
        assert!(!action_matches_pattern("test:unit", "build:*"));
        assert!(action_matches_pattern("anything", "*"));
    }

    #[test]
    fn exact_matches() {
        assert!(action_matches_pattern("deploy:staging", "deploy:staging"));
        assert!(!action_matches_pattern("deploy:production", "deploy:staging"));
    }

    #[test]
    fn scope_check() {
        let scope = vec!["build:*".into(), "test:*".into(), "deploy:staging".into()];
        assert!(action_in_scope("build:docker", &scope));
        assert!(action_in_scope("test:integration", &scope));
        assert!(action_in_scope("deploy:staging", &scope));
        assert!(!action_in_scope("deploy:production", &scope));
    }
}
