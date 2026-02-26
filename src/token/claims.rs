//! JWT-like claims for agent authorization tokens.
//! Used by: token::sign, token::verify, handlers::mint.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    pub jti: String,
    pub sub: String,
    pub action: String,
    pub iat: DateTime<Utc>,
    pub exp: DateTime<Utc>,
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
        }
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
}
