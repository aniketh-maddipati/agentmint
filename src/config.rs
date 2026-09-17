//! Startup configuration parsed once from `MINT_` environment variables.
//! Unknown enum values fail closed. Used by: main, doctor, server, tests.

use std::path::PathBuf;
use std::time::Duration;

use crate::credentials::assert_test_secret;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Development,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Fake,
    Stripe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    Local,
    Oidc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    Threshold,
    Http,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    pub bind_addr: String,
    pub database_path: PathBuf,
    pub signing_key_file: Option<PathBuf>,
    pub signing_key_env: Option<String>,
    pub kid: String,
    pub provider: ProviderKind,
    pub identity: IdentityKind,
    pub policy: PolicyKind,
    pub stripe_secret: Option<String>,
    pub auto_cents: i64,
    pub approval_cents: i64,
    pub policy_version: String,
    pub cors_origins: Vec<String>,
    pub body_limit_bytes: usize,
    pub http_timeout: Duration,
    pub oidc_issuer: Option<String>,
    pub oidc_audience: Option<String>,
    pub oidc_jwks_url: Option<String>,
    pub oidc_agent_claim: String,
    pub oidc_tenant_claim: String,
    pub pdp_url: Option<String>,
    pub credential_url: Option<String>,
    pub log_format_json: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let mode = parse_mode(&env_or("MINT_MODE", "development"))?;
        let provider = parse_provider(&env_or("MINT_PROVIDER", "fake"))?;
        let identity = parse_identity(&env_or("MINT_IDENTITY", "local"))?;
        let policy = parse_policy(&env_or("MINT_POLICY", "threshold"))?;
        let log_format = env_or("MINT_LOG_FORMAT", "text");
        let log_format_json = match log_format.to_ascii_lowercase().as_str() {
            "text" => false,
            "json" => true,
            _ => return Err(Error::Misconfigured("MINT_LOG_FORMAT must be text or json")),
        };

        let auto_cents = parse_cents("MINT_POLICY_AUTO_CENTS", 5000)?;
        let approval_cents = parse_cents("MINT_POLICY_APPROVAL_CENTS", 50_000)?;
        if auto_cents < 0 || approval_cents < 0 {
            return Err(Error::Misconfigured(
                "policy thresholds must be nonnegative",
            ));
        }
        if auto_cents > approval_cents {
            return Err(Error::Misconfigured(
                "MINT_POLICY_AUTO_CENTS must not exceed MINT_POLICY_APPROVAL_CENTS",
            ));
        }

        let body_limit_bytes = parse_usize("MINT_BODY_LIMIT", 65_536)?;
        if !(1024..=1_048_576).contains(&body_limit_bytes) {
            return Err(Error::Misconfigured(
                "MINT_BODY_LIMIT must be between 1024 and 1048576",
            ));
        }

        let timeout_ms = parse_u64("MINT_HTTP_TIMEOUT_MS", 10_000)?;
        if timeout_ms == 0 || timeout_ms > 120_000 {
            return Err(Error::Misconfigured(
                "MINT_HTTP_TIMEOUT_MS must be between 1 and 120000",
            ));
        }

        if mode == Mode::Production && identity == IdentityKind::Local {
            return Err(Error::Misconfigured(
                "local identity is disabled in production mode",
            ));
        }
        if mode == Mode::Production && provider == ProviderKind::Fake {
            return Err(Error::Misconfigured(
                "fake provider is disabled in production mode",
            ));
        }

        let stripe_secret = std::env::var("MINT_STRIPE_TEST_SECRET_KEY").ok();
        if provider == ProviderKind::Stripe {
            let secret = stripe_secret
                .as_deref()
                .ok_or(Error::Misconfigured("MINT_STRIPE_TEST_SECRET_KEY required"))?;
            assert_test_secret(secret)?;
        }

        let pdp_url = std::env::var("MINT_PDP_URL").ok();
        if policy == PolicyKind::Http && pdp_url.as_ref().is_none_or(|u| u.is_empty()) {
            return Err(Error::Misconfigured(
                "MINT_PDP_URL required when MINT_POLICY=http",
            ));
        }

        let cors_origins = std::env::var("MINT_CORS_ORIGINS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            mode,
            bind_addr: env_or("MINT_BIND_ADDR", "127.0.0.1:8787"),
            database_path: PathBuf::from(env_or("MINT_DATABASE_PATH", "mint.db")),
            signing_key_file: std::env::var("MINT_SIGNING_KEY_FILE")
                .ok()
                .map(PathBuf::from),
            signing_key_env: std::env::var("MINT_SIGNING_KEY").ok(),
            kid: env_or("MINT_KID", "mint-local-1"),
            provider,
            identity,
            policy,
            stripe_secret,
            auto_cents,
            approval_cents,
            policy_version: env_or("MINT_POLICY_VERSION", "stripe-refund-thresholds-v1"),
            cors_origins,
            body_limit_bytes,
            http_timeout: Duration::from_millis(timeout_ms),
            oidc_issuer: std::env::var("MINT_OIDC_ISSUER").ok(),
            oidc_audience: std::env::var("MINT_OIDC_AUDIENCE").ok(),
            oidc_jwks_url: std::env::var("MINT_OIDC_JWKS_URL").ok(),
            oidc_agent_claim: env_or("MINT_OIDC_AGENT_CLAIM", "agent_id"),
            oidc_tenant_claim: env_or("MINT_OIDC_TENANT_CLAIM", "tenant_id"),
            pdp_url,
            credential_url: std::env::var("MINT_CREDENTIAL_URL").ok(),
            log_format_json,
        })
    }

    pub fn for_test(database_path: PathBuf, signing_key_file: PathBuf) -> Self {
        Self {
            mode: Mode::Development,
            bind_addr: "127.0.0.1:0".into(),
            database_path,
            signing_key_file: Some(signing_key_file),
            signing_key_env: None,
            kid: "mint-test-1".into(),
            provider: ProviderKind::Fake,
            identity: IdentityKind::Local,
            policy: PolicyKind::Threshold,
            stripe_secret: None,
            auto_cents: 5000,
            approval_cents: 50_000,
            policy_version: "stripe-refund-thresholds-v1".into(),
            cors_origins: Vec::new(),
            body_limit_bytes: 65_536,
            http_timeout: Duration::from_millis(200),
            oidc_issuer: None,
            oidc_audience: None,
            oidc_jwks_url: None,
            oidc_agent_claim: "agent_id".into(),
            oidc_tenant_claim: "tenant_id".into(),
            pdp_url: None,
            credential_url: None,
            log_format_json: false,
        }
    }

    pub fn validate_relationships(&self) -> Result<()> {
        if self.auto_cents < 0 || self.approval_cents < 0 {
            return Err(Error::Misconfigured(
                "policy thresholds must be nonnegative",
            ));
        }
        if self.auto_cents > self.approval_cents {
            return Err(Error::Misconfigured(
                "automatic threshold must not exceed approval threshold",
            ));
        }
        if self.http_timeout.is_zero() {
            return Err(Error::Misconfigured("http timeout must be positive"));
        }
        if self.mode == Mode::Production && self.identity == IdentityKind::Local {
            return Err(Error::Misconfigured(
                "local identity is disabled in production mode",
            ));
        }
        if self.mode == Mode::Production && self.provider == ProviderKind::Fake {
            return Err(Error::Misconfigured(
                "fake provider is disabled in production mode",
            ));
        }
        if self.provider == ProviderKind::Stripe {
            let secret = self
                .stripe_secret
                .as_deref()
                .ok_or(Error::Misconfigured("MINT_STRIPE_TEST_SECRET_KEY required"))?;
            assert_test_secret(secret)?;
        }
        Ok(())
    }
}

pub fn parse_mode(value: &str) -> Result<Mode> {
    match value.to_ascii_lowercase().as_str() {
        "development" | "dev" => Ok(Mode::Development),
        "production" | "prod" => Ok(Mode::Production),
        _ => Err(Error::Misconfigured(
            "MINT_MODE must be development or production",
        )),
    }
}

pub fn parse_provider(value: &str) -> Result<ProviderKind> {
    match value.to_ascii_lowercase().as_str() {
        "fake" => Ok(ProviderKind::Fake),
        "stripe" => Ok(ProviderKind::Stripe),
        _ => Err(Error::Misconfigured("MINT_PROVIDER must be fake or stripe")),
    }
}

pub fn parse_identity(value: &str) -> Result<IdentityKind> {
    match value.to_ascii_lowercase().as_str() {
        "local" => Ok(IdentityKind::Local),
        "oidc" => Ok(IdentityKind::Oidc),
        _ => Err(Error::Misconfigured("MINT_IDENTITY must be local or oidc")),
    }
}

pub fn parse_policy(value: &str) -> Result<PolicyKind> {
    match value.to_ascii_lowercase().as_str() {
        "threshold" => Ok(PolicyKind::Threshold),
        "http" => Ok(PolicyKind::Http),
        _ => Err(Error::Misconfigured(
            "MINT_POLICY must be threshold or http",
        )),
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn parse_cents(key: &str, default: i64) -> Result<i64> {
    match std::env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| Error::Misconfigured("invalid policy threshold")),
        Err(_) => Ok(default),
    }
}

fn parse_usize(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| Error::Misconfigured("invalid integer config")),
        Err(_) => Ok(default),
    }
}

fn parse_u64(key: &str, default: u64) -> Result<u64> {
    match std::env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| Error::Misconfigured("invalid integer config")),
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_env<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().expect("env lock");
        for key in [
            "MINT_MODE",
            "MINT_PROVIDER",
            "MINT_IDENTITY",
            "MINT_POLICY",
            "MINT_LOG_FORMAT",
            "MINT_POLICY_AUTO_CENTS",
            "MINT_POLICY_APPROVAL_CENTS",
            "MINT_BODY_LIMIT",
            "MINT_HTTP_TIMEOUT_MS",
            "MINT_STRIPE_TEST_SECRET_KEY",
            "MINT_PDP_URL",
        ] {
            std::env::remove_var(key);
        }
        f();
    }

    #[test]
    fn unknown_provider_fails_closed() {
        with_clean_env(|| {
            std::env::set_var("MINT_PROVIDER", "strpie");
            let err = Config::from_env().expect_err("typo");
            assert!(matches!(err, Error::Misconfigured(_)));
        });
    }

    #[test]
    fn unknown_mode_identity_policy_fail_closed() {
        assert!(parse_mode("staging").is_err());
        assert!(parse_identity("webauthn").is_err());
        assert!(parse_policy("opa").is_err());
        assert!(parse_provider("paypal").is_err());
    }

    #[test]
    fn auto_threshold_cannot_exceed_approval() {
        with_clean_env(|| {
            std::env::set_var("MINT_POLICY_AUTO_CENTS", "6000");
            std::env::set_var("MINT_POLICY_APPROVAL_CENTS", "5000");
            assert!(Config::from_env().is_err());
        });
    }

    #[test]
    fn stripe_requires_test_secret() {
        with_clean_env(|| {
            std::env::set_var("MINT_PROVIDER", "stripe");
            assert!(Config::from_env().is_err());
            std::env::set_var("MINT_STRIPE_TEST_SECRET_KEY", "sk_live_x");
            assert!(Config::from_env().is_err());
            std::env::set_var("MINT_STRIPE_TEST_SECRET_KEY", "sk_test_x");
            assert!(Config::from_env().is_ok());
        });
    }

    #[test]
    fn production_rejects_local_and_fake() {
        with_clean_env(|| {
            std::env::set_var("MINT_MODE", "production");
            std::env::set_var("MINT_PROVIDER", "fake");
            std::env::set_var("MINT_IDENTITY", "local");
            assert!(Config::from_env().is_err());
        });
    }
}
