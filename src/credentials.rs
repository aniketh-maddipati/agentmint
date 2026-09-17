//! Provider credentials. Never returned on agent-facing APIs.
//! Used by: execution engine.

use serde::Deserialize;
use serde_json::json;

use crate::config::{Config, ProviderKind};
use crate::error::{Error, Result};

#[derive(Clone)]
pub struct ProviderCredential {
    pub secret: String,
}

pub enum CredentialSource {
    Env(ProviderCredential),
    Http(HttpCredentials),
    None,
}

impl CredentialSource {
    pub fn from_config(config: &Config, http: reqwest::Client) -> Result<Self> {
        if let Some(url) = &config.credential_url {
            return Ok(Self::Http(HttpCredentials {
                url: url.clone(),
                http,
            }));
        }
        match config.provider {
            ProviderKind::Fake => Ok(Self::None),
            ProviderKind::Stripe => {
                let secret = config
                    .stripe_secret
                    .clone()
                    .ok_or(Error::Misconfigured("MINT_STRIPE_TEST_SECRET_KEY required"))?;
                assert_test_secret(&secret)?;
                Ok(Self::Env(ProviderCredential { secret }))
            }
        }
    }

    pub async fn credential(&self, tenant_id: &str, provider: &str) -> Result<ProviderCredential> {
        match self {
            Self::None => Ok(ProviderCredential {
                secret: "fake".into(),
            }),
            Self::Env(value) => {
                assert_test_secret(&value.secret)?;
                Ok(value.clone())
            }
            Self::Http(http) => http.fetch(tenant_id, provider).await,
        }
    }
}

pub fn assert_test_secret(secret: &str) -> Result<()> {
    let trimmed = secret.trim();
    if trimmed.starts_with("sk_live_") || trimmed.starts_with("rk_live_") {
        return Err(Error::LiveStripeRefused);
    }
    if trimmed.starts_with("sk_test_") || trimmed.starts_with("rk_test_") {
        return Ok(());
    }
    Err(Error::LiveStripeRefused)
}

pub struct HttpCredentials {
    url: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct CredentialResponse {
    secret: String,
}

impl HttpCredentials {
    async fn fetch(&self, tenant_id: &str, provider: &str) -> Result<ProviderCredential> {
        let response = self
            .http
            .post(&self.url)
            .json(&json!({
                "tenant_id": tenant_id,
                "provider": provider
            }))
            .send()
            .await
            .map_err(|_| Error::Internal)?;
        if !response.status().is_success() {
            return Err(Error::Internal);
        }
        let parsed: CredentialResponse = response.json().await.map_err(|_| Error::Internal)?;
        assert_test_secret(&parsed.secret)?;
        Ok(ProviderCredential {
            secret: parsed.secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_keys_are_refused() {
        assert!(assert_test_secret("sk_live_123").is_err());
        assert!(assert_test_secret("rk_live_123").is_err());
        assert!(assert_test_secret("not-a-key").is_err());
        assert!(assert_test_secret("sk_test_123").is_ok());
    }
}
