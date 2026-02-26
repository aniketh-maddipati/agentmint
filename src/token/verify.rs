//! Ed25519 token verification.
//! Used by: handlers::proxy.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::error::{Error, Result};
use crate::token::claims::Claims;

pub fn verify_token(token: &str, key: &VerifyingKey) -> Result<Claims> {
    let (payload_b64, sig_b64) = token
        .split_once('.')
        .ok_or_else(|| Error::InvalidToken("missing separator".into()))?;

    let sig_bytes = URL_SAFE_NO_PAD.decode(sig_b64)?;
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| Error::InvalidToken(e.to_string()))?;

    key.verify(payload_b64.as_bytes(), &signature)
        .map_err(|_| Error::InvalidSignature)?;

    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64)?;
    let claims: Claims = serde_json::from_slice(&payload_bytes)?;

    if claims.is_expired() {
        return Err(Error::TokenExpired);
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::sign::{generate_keypair, sign_token};

    #[test]
    fn valid_token_verifies() -> Result<()> {
        let key = generate_keypair();
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        let token = sign_token(&claims, &key)?;
        let verified = verify_token(&token, &key.verifying_key())?;
        assert_eq!(verified.sub, "agent-1");
        assert_eq!(verified.action, "deploy");
        Ok(())
    }

    #[test]
    fn expired_token_rejected() -> Result<()> {
        let key = generate_keypair();
        let claims = Claims::new("agent-1".into(), "deploy".into(), 0);
        let token = sign_token(&claims, &key)?;
        let result = verify_token(&token, &key.verifying_key());
        assert!(matches!(result, Err(Error::TokenExpired)));
        Ok(())
    }

    #[test]
    fn tampered_token_rejected() -> Result<()> {
        let key = generate_keypair();
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        let token = sign_token(&claims, &key)?;
        let tampered = format!("x{}", token);
        let result = verify_token(&tampered, &key.verifying_key());
        assert!(matches!(result, Err(Error::InvalidSignature)));
        Ok(())
    }

    #[test]
    fn wrong_key_rejected() -> Result<()> {
        let key = generate_keypair();
        let other_key = generate_keypair();
        let claims = Claims::new("agent-1".into(), "deploy".into(), 300);
        let token = sign_token(&claims, &key)?;
        let result = verify_token(&token, &other_key.verifying_key());
        assert!(matches!(result, Err(Error::InvalidSignature)));
        Ok(())
    }

    #[test]
    fn missing_separator_rejected() {
        let key = generate_keypair();
        let result = verify_token("no-dot-here", &key.verifying_key());
        assert!(matches!(result, Err(Error::InvalidToken(_))));
    }
}
