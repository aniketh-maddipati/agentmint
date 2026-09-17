//! Startup configuration parsed once from `MINT_` environment variables.
//! Used by: main, server, tests.

use std::path::PathBuf;
use std::time::Duration;

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
        let mode = match env_or("MINT_MODE", "development")
            .to_ascii_lowercase()
            .as_str()
        {
            "production" => Mode::Production,
            _ => Mode::Development,
        };
        let provider = match env_or("MINT_PROVIDER", "fake")
            .to_ascii_lowercase()
            .as_str()
        {
            "stripe" => ProviderKind::Stripe,
            _ => ProviderKind::Fake,
        };
        let identity = match env_or("MINT_IDENTITY", "local")
            .to_ascii_lowercase()
            .as_str()
        {
            "oidc" => IdentityKind::Oidc,
            _ => IdentityKind::Local,
        };
        if mode == Mode::Production && identity == IdentityKind::Local {
            return Err(Error::Misconfigured(
                "local identity is disabled outside development mode",
            ));
        }
        if mode == Mode::Production && provider == ProviderKind::Fake {
            return Err(Error::Misconfigured(
                "fake provider is disabled outside development mode",
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
            stripe_secret: std::env::var("MINT_STRIPE_TEST_SECRET_KEY").ok(),
            auto_cents: parse_cents("MINT_POLICY_AUTO_CENTS", 5000)?,
            approval_cents: parse_cents("MINT_POLICY_APPROVAL_CENTS", 50_000)?,
            policy_version: env_or("MINT_POLICY_VERSION", "stripe-refund-thresholds-v1"),
            cors_origins,
            body_limit_bytes: parse_usize("MINT_BODY_LIMIT", 65_536)?,
            http_timeout: Duration::from_millis(parse_u64("MINT_HTTP_TIMEOUT_MS", 10_000)?),
            oidc_issuer: std::env::var("MINT_OIDC_ISSUER").ok(),
            oidc_audience: std::env::var("MINT_OIDC_AUDIENCE").ok(),
            oidc_jwks_url: std::env::var("MINT_OIDC_JWKS_URL").ok(),
            oidc_agent_claim: env_or("MINT_OIDC_AGENT_CLAIM", "agent_id"),
            oidc_tenant_claim: env_or("MINT_OIDC_TENANT_CLAIM", "tenant_id"),
            pdp_url: std::env::var("MINT_PDP_URL").ok(),
            credential_url: std::env::var("MINT_CREDENTIAL_URL").ok(),
            log_format_json: env_or("MINT_LOG_FORMAT", "text") == "json",
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
