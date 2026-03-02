//! JWT-like claims for agent authorization tokens.
//! Used by: token::sign, token::verify, handlers::mint, handlers::delegate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    pub jti: String,
    pub sub: String,
    pub action: String,
    pub iat: DateTime<Utc>,
    pub exp: DateTime<Utc>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegates_to: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_checkpoint: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_delegation_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_approver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

impl Claims {
    pub fn new(sub: String, action: String, ttl_seconds: i64) -> Self {
        let now = Utc::now();
        Self {
            jti: uuid::Uuid::new_v4().to_string(),
            sub,
            action,
            iat: now,
            exp: now + chrono::Duration::seconds(ttl_seconds),
            receipt_type: None,
            scope: None,
            delegates_to: None,
            requires_checkpoint: None,
            max_delegation_depth: None,
            parent_jti: None,
            original_approver: None,
            depth: None,
        }
    }

    pub fn new_plan(
        sub: String,
        action: String,
        ttl_seconds: i64,
        scope: Vec<String>,
        delegates_to: Vec<String>,
        requires_checkpoint: Vec<String>,
        max_delegation_depth: u32,
    ) -> Self {
        let mut claims = Self::new(sub, action, ttl_seconds);
        claims.receipt_type = Some("plan".into());
        claims.scope = Some(scope);
        claims.delegates_to = Some(delegates_to);
        claims.requires_checkpoint = Some(requires_checkpoint);
        claims.max_delegation_depth = Some(max_delegation_depth);
        claims.depth = Some(0);
        claims
    }

    pub fn new_delegated(
        agent_id: String,
        action: String,
        ttl_seconds: i64,
        parent: &Claims,
    ) -> Self {
        let mut claims = Self::new(agent_id, action, ttl_seconds);
        claims.receipt_type = Some("delegated".into());
        claims.parent_jti = Some(parent.jti.clone());
        claims.original_approver = Some(
            parent.original_approver.clone().unwrap_or_else(|| parent.sub.clone())
        );
        claims.depth = Some(parent.depth.unwrap_or(0) + 1);
        claims.scope = parent.scope.clone();
        claims.delegates_to = parent.delegates_to.clone();
        claims.requires_checkpoint = parent.requires_checkpoint.clone();
        claims.max_delegation_depth = parent.max_delegation_depth;
        claims
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.exp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_claims_have_valid_fields() {
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        assert_eq!(claims.sub, "agent-1");
        assert_eq!(claims.action, "deploy");
        assert!(!claims.jti.is_empty());
        assert!(claims.exp > claims.iat);
        assert!(claims.receipt_type.is_none());
    }

    #[test]
    fn claims_with_zero_ttl_are_expired() {
        let claims = Claims::new("agent-1".into(), "deploy".into(), 0);
        assert!(claims.is_expired());
    }

    #[test]
    fn claims_roundtrip_through_json() -> crate::error::Result<()> {
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        let json = serde_json::to_string(&claims)?;
        let decoded: Claims = serde_json::from_str(&json)?;
        assert_eq!(claims, decoded);
        Ok(())
    }

    #[test]
    fn plan_claims_have_orchestration_fields() {
        let claims = Claims::new_plan(
            "aniketh@company.com".into(),
            "deploy:api-v2".into(),
            3600,
            vec!["build:*".into(), "test:*".into()],
            vec!["build-agent".into()],
            vec!["deploy:production".into()],
            2,
        );
        assert_eq!(claims.receipt_type, Some("plan".into()));
        assert_eq!(claims.depth, Some(0));
    }

    #[test]
    fn delegated_claims_chain_to_parent() {
        let parent = Claims::new_plan(
            "aniketh@company.com".into(),
            "deploy:api-v2".into(),
            3600,
            vec!["build:*".into()],
            vec!["build-agent".into()],
            vec![],
            2,
        );
        let child = Claims::new_delegated(
            "build-agent".into(),
            "build:docker".into(),
            300,
            &parent,
        );
        assert_eq!(child.receipt_type, Some("delegated".into()));
        assert_eq!(child.parent_jti, Some(parent.jti.clone()));
        assert_eq!(child.original_approver, Some("aniketh@company.com".into()));
        assert_eq!(child.depth, Some(1));
    }

    #[test]
    fn old_style_claims_skip_none_fields() -> crate::error::Result<()> {
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        let json = serde_json::to_string(&claims)?;
        assert!(!json.contains("scope"));
        assert!(!json.contains("parent_jti"));
        Ok(())
    }
}
