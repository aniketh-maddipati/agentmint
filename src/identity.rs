//! Identity verification: local development identity or generic JWT/OIDC.
//! Used by: API authentication.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;

use crate::config::{Config, IdentityKind, Mode};
use crate::domain::ActorIdentity;
use crate::error::{Error, Result};

const MAX_TOKEN_BYTES: usize = 8192;
const JWKS_TTL: Duration = Duration::from_secs(300);

pub struct AuthContext {
    pub tenant_id: String,
    pub actor: ActorIdentity,
}

pub enum IdentityProvider {
    Local,
    Oidc(OidcVerifier),
}

impl IdentityProvider {
    pub fn from_config(config: &Config, http: reqwest::Client) -> Result<Self> {
        match config.identity {
            IdentityKind::Local => {
                if config.mode != Mode::Development {
                    return Err(Error::Misconfigured(
                        "local identity is disabled outside development",
                    ));
                }
                Ok(Self::Local)
            }
            IdentityKind::Oidc => {
                let issuer = config
                    .oidc_issuer
                    .clone()
                    .ok_or(Error::Misconfigured("MINT_OIDC_ISSUER required"))?;
                let audience = config
                    .oidc_audience
                    .clone()
                    .ok_or(Error::Misconfigured("MINT_OIDC_AUDIENCE required"))?;
                let jwks_url = config
                    .oidc_jwks_url
                    .clone()
                    .ok_or(Error::Misconfigured("MINT_OIDC_JWKS_URL required"))?;
                if !jwks_url.starts_with("https://")
                    && !(config.mode == Mode::Development
                        && (jwks_url.starts_with("http://127.0.0.1")
                            || jwks_url.starts_with("http://localhost")))
                {
                    return Err(Error::Misconfigured("OIDC JWKS URL must be https"));
                }
                Ok(Self::Oidc(OidcVerifier {
                    issuer,
                    audience,
                    jwks_url,
                    agent_claim: config.oidc_agent_claim.clone(),
                    tenant_claim: config.oidc_tenant_claim.clone(),
                    http,
                    cache: Mutex::new(JwksCache::default()),
                }))
            }
        }
    }

    pub async fn warmup(&self) -> Result<()> {
        match self {
            Self::Local => Ok(()),
            Self::Oidc(oidc) => oidc.refresh().await,
        }
    }

    pub async fn authenticate(&self, authorization: Option<&str>) -> Result<AuthContext> {
        let value = authorization.ok_or(Error::Unauthenticated)?;
        let token = value
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthenticated)?;
        if token.len() > MAX_TOKEN_BYTES {
            return Err(Error::InvalidRequest("token exceeds size limit"));
        }
        match self {
            Self::Local => parse_local(token),
            Self::Oidc(oidc) => oidc.verify(token).await,
        }
    }
}

fn parse_local(token: &str) -> Result<AuthContext> {
    let encoded = token.strip_prefix("dev.").ok_or(Error::Unauthenticated)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| Error::IdentityFailed)?;
    if bytes.len() > MAX_TOKEN_BYTES {
        return Err(Error::InvalidRequest("token exceeds size limit"));
    }
    let value: LocalIdentity = serde_json::from_slice(&bytes).map_err(|_| Error::IdentityFailed)?;
    if value.tenant_id.is_empty()
        || value.subject.is_empty()
        || value.agent_id.is_empty()
        || value.issuer.is_empty()
    {
        return Err(Error::IdentityFailed);
    }
    Ok(AuthContext {
        tenant_id: value.tenant_id,
        actor: ActorIdentity {
            subject: value.subject,
            agent_id: value.agent_id,
            delegated_by: value.delegated_by,
            issuer: value.issuer,
        },
    })
}

#[derive(Deserialize)]
struct LocalIdentity {
    #[serde(alias = "tenantId")]
    tenant_id: String,
    subject: String,
    #[serde(alias = "agentId")]
    agent_id: String,
    issuer: String,
    #[serde(default, alias = "delegatedBy")]
    delegated_by: Option<String>,
}

pub struct OidcVerifier {
    issuer: String,
    audience: String,
    jwks_url: String,
    agent_claim: String,
    tenant_claim: String,
    http: reqwest::Client,
    cache: Mutex<JwksCache>,
}

#[derive(Default)]
struct JwksCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

#[derive(Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    n: Option<String>,
    e: Option<String>,
    x: Option<String>,
    y: Option<String>,
    crv: Option<String>,
}

impl OidcVerifier {
    async fn verify(&self, token: &str) -> Result<AuthContext> {
        let header = decode_header(token).map_err(|_| Error::IdentityFailed)?;
        let kid = header.kid.ok_or(Error::IdentityFailed)?;
        let key = self.key_for(&kid).await?;
        let algorithm = header.alg;
        if !matches!(algorithm, Algorithm::RS256 | Algorithm::ES256) {
            return Err(Error::IdentityFailed);
        }
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.validate_exp = true;
        let data = decode::<Value>(token, &key, &validation).map_err(|_| Error::IdentityFailed)?;
        let claims = data.claims;
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .ok_or(Error::IdentityFailed)?;
        let agent_id = claims
            .get(&self.agent_claim)
            .and_then(Value::as_str)
            .ok_or(Error::IdentityFailed)?;
        let tenant_id = claims
            .get(&self.tenant_claim)
            .and_then(Value::as_str)
            .ok_or(Error::IdentityFailed)?;
        Ok(AuthContext {
            tenant_id: tenant_id.to_owned(),
            actor: ActorIdentity {
                subject: subject.to_owned(),
                agent_id: agent_id.to_owned(),
                delegated_by: claims
                    .get("delegated_by")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                issuer: self.issuer.clone(),
            },
        })
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey> {
        {
            let cache = self
                .cache
                .lock()
                .map_err(|err| Error::internal("jwks cache", err))?;
            if let Some(fetched_at) = cache.fetched_at {
                if fetched_at.elapsed() < JWKS_TTL {
                    return cache.keys.get(kid).cloned().ok_or(Error::IdentityFailed);
                }
            }
        }
        self.refresh().await?;
        let cache = self
            .cache
            .lock()
            .map_err(|err| Error::internal("jwks cache", err))?;
        cache.keys.get(kid).cloned().ok_or(Error::IdentityFailed)
    }

    async fn refresh(&self) -> Result<()> {
        let response = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| Error::IdentityFailed)?;
        let jwks: JwksResponse = response.json().await.map_err(|_| Error::IdentityFailed)?;
        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            let Some(kid) = jwk.kid else { continue };
            if jwk.kty == "RSA" {
                if let (Some(n), Some(e)) = (jwk.n, jwk.e) {
                    if let Ok(key) = DecodingKey::from_rsa_components(&n, &e) {
                        keys.insert(kid, key);
                    }
                }
            } else if jwk.kty == "EC" && jwk.crv.as_deref() == Some("P-256") {
                if let (Some(x), Some(y)) = (jwk.x, jwk.y) {
                    if let Ok(key) = DecodingKey::from_ec_components(&x, &y) {
                        keys.insert(kid, key);
                    }
                }
            }
        }
        let mut cache = self
            .cache
            .lock()
            .map_err(|err| Error::internal("jwks cache", err))?;
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }
}

pub fn encode_dev_token(tenant_id: &str, actor: &ActorIdentity) -> String {
    let payload = serde_json::json!({
        "tenant_id": tenant_id,
        "subject": actor.subject,
        "agent_id": actor.agent_id,
        "issuer": actor.issuer,
        "delegated_by": actor.delegated_by,
    });
    format!(
        "dev.{}",
        URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes())
    )
}
