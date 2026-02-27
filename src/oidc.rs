use jsonwebtoken::{decode, decode_header, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

const JWKS_CACHE_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    pub email: Option<String>,
    pub aud: String,
    pub iss: String,
    pub exp: u64,
    pub iat: u64,
}


pub struct OidcVerifier {
    issuer: String,
    audience: String,
    jwks_uri: String,
    cache: RwLock<JwksCache>,
}

#[derive(Default)]
struct JwksCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    n: Option<String>,
    e: Option<String>,
}

impl OidcVerifier {
    pub fn new(issuer: &str, audience: &str, jwks_uri: &str) -> Self {
        Self {
            issuer: issuer.to_string(),
            audience: audience.to_string(),
            jwks_uri: jwks_uri.to_string(),
            cache: RwLock::new(JwksCache::default()),
        }
    }

    pub fn from_env() -> Option<Self> {
        let issuer = std::env::var("OIDC_ISSUER").ok()?;
        let audience = std::env::var("OIDC_AUDIENCE").ok()?;
        let jwks_uri = std::env::var("OIDC_JWKS_URI").ok()?;
        
        tracing::info!(issuer = %issuer, "OIDC enabled");
        Some(Self::new(&issuer, &audience, &jwks_uri))
    }

    pub async fn verify(&self, token: &str) -> Result<IdTokenClaims, Error> {
        let header = decode_header(token).map_err(|_| Error::InvalidToken)?;
        
        let kid = header.kid.ok_or(Error::MissingKid)?;
        
        let key = self.get_key(&kid).await?;
        
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        
        let data = decode::<IdTokenClaims>(token, &key, &validation)
            .map_err(|e| Error::ValidationFailed(e.to_string()))?;
        
        Ok(data.claims)
    }

    async fn get_key(&self, kid: &str) -> Result<DecodingKey, Error> {
        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(fetched_at) = cache.fetched_at {
                if fetched_at.elapsed() < JWKS_CACHE_TTL {
                    if let Some(key) = cache.keys.get(kid) {
                        return Ok(key.clone());
                    }
                }
            }
        }

        // Fetch fresh JWKS
        self.refresh_jwks().await?;

        // Try again
        let cache = self.cache.read().unwrap();
        cache.keys.get(kid).cloned().ok_or(Error::KeyNotFound)
    }

    async fn refresh_jwks(&self) -> Result<(), Error> {
        let response = reqwest::get(&self.jwks_uri)
            .await
            .map_err(|e| Error::FetchFailed(e.to_string()))?;

        let jwks: JwksResponse = response
            .json()
            .await
            .map_err(|e| Error::FetchFailed(e.to_string()))?;

        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            if jwk.kty == "RSA" {
                if let (Some(n), Some(e)) = (jwk.n, jwk.e) {
                    if let Ok(key) = DecodingKey::from_rsa_components(&n, &e) {
                        keys.insert(jwk.kid, key);
                    }
                }
            }
        }

        let mut cache = self.cache.write().unwrap();
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());

        tracing::info!(keys = cache.keys.len(), "JWKS refreshed");
        Ok(())
    }
}


#[derive(Debug)]
pub enum Error {
    InvalidToken,
    MissingKid,
    KeyNotFound,
    FetchFailed(String),
    ValidationFailed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken => write!(f, "invalid id_token"),
            Self::MissingKid => write!(f, "missing kid in token header"),
            Self::KeyNotFound => write!(f, "signing key not found"),
            Self::FetchFailed(e) => write!(f, "failed to fetch JWKS: {}", e),
            Self::ValidationFailed(e) => write!(f, "token validation failed: {}", e),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_from_env_returns_none_when_not_set() {
        // Clear env vars if set
        std::env::remove_var("OIDC_ISSUER");
        std::env::remove_var("OIDC_AUDIENCE");
        std::env::remove_var("OIDC_JWKS_URI");
        
        assert!(OidcVerifier::from_env().is_none());
    }
}