//! Narrow readiness checks. Never prints secrets. Used by: `mint doctor`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use crate::canonical::{canonicalize_value, hash_canonical_json};
use crate::config::{Config, ProviderKind};
use crate::credentials::assert_test_secret;
use crate::keys::KeyRing;
use crate::storage::Store;
use serde_json::Value;

pub struct CheckResult {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

pub async fn run_doctor(config: &Config) -> Vec<CheckResult> {
    let mut results = Vec::new();
    results.push(check("config.parse", true, "configuration parsed"));
    match config.validate_relationships() {
        Ok(()) => results.push(check(
            "config.relationships",
            true,
            format!(
                "mode={:?} provider={:?} identity={:?} policy={:?}",
                config.mode, config.provider, config.identity, config.policy
            ),
        )),
        Err(err) => results.push(check("config.relationships", false, err.to_string())),
    }
    results.push(check_signing_key(config));
    results.push(check_database(config));
    results.push(check(
        "policy.thresholds",
        config.auto_cents >= 0 && config.auto_cents <= config.approval_cents,
        format!(
            "auto={} approval={}",
            config.auto_cents, config.approval_cents
        ),
    ));
    results.push(check_canonical_fixture());
    results.push(check_stripe_prefix(config));
    results.push(check_stripe_connectivity(config).await);
    results
}

pub fn exit_code(results: &[CheckResult]) -> i32 {
    if results.iter().all(|r| r.ok) {
        0
    } else {
        1
    }
}

pub fn print_results(results: &[CheckResult]) {
    for result in results {
        let mark = if result.ok { "PASS" } else { "FAIL" };
        println!("{mark}  {} — {}", result.name, result.detail);
    }
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>) -> CheckResult {
    CheckResult {
        name,
        ok,
        detail: detail.into(),
    }
}

fn check_signing_key(config: &Config) -> CheckResult {
    match KeyRing::from_config(
        &config.kid,
        config.signing_key_file.as_deref(),
        config.signing_key_env.as_deref(),
    ) {
        Ok(keys) => {
            let mut detail = format!("kid={} parseable", keys.kid);
            if let Some(path) = &config.signing_key_file {
                if let Ok(meta) = fs::metadata(path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode & 0o077 != 0 {
                        return check(
                            "signing.key",
                            false,
                            format!(
                                "{} mode {:o} is group/world readable; use chmod 600",
                                path.display(),
                                mode
                            ),
                        );
                    }
                    detail.push_str(&format!(" file_mode={mode:o}"));
                }
            }
            check("signing.key", true, detail)
        }
        Err(err) => check("signing.key", false, err.to_string()),
    }
}

fn check_database(config: &Config) -> CheckResult {
    if let Some(parent) = config.database_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if let Err(err) = fs::create_dir_all(parent) {
                return check(
                    "database",
                    false,
                    format!("cannot create parent {}: {err}", parent.display()),
                );
            }
        }
    }
    match Store::open(&config.database_path) {
        Ok(_) => check(
            "database",
            true,
            format!(
                "{} writable + migrations ok",
                config.database_path.display()
            ),
        ),
        Err(err) => check("database", false, err.to_string()),
    }
}

fn check_canonical_fixture() -> CheckResult {
    let path = Path::new("fixtures/canonicalization/support-refund-base.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return check("canonical.fixture", false, "fixture file missing");
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return check("canonical.fixture", false, "fixture json invalid");
    };
    let Some(material) = value.get("material") else {
        return check("canonical.fixture", false, "material missing");
    };
    let Ok(canonical) = canonicalize_value(material) else {
        return check("canonical.fixture", false, "canonicalize failed");
    };
    let hash = hash_canonical_json(&canonical);
    let expected = value
        .get("expected_sha256")
        .and_then(Value::as_str)
        .unwrap_or("");
    if hash == expected {
        check("canonical.fixture", true, hash)
    } else {
        check(
            "canonical.fixture",
            false,
            format!("hash mismatch got={hash} expected={expected}"),
        )
    }
}

fn check_stripe_prefix(config: &Config) -> CheckResult {
    match config.provider {
        ProviderKind::Fake => check("stripe.credential", true, "skipped — provider=fake"),
        ProviderKind::Stripe => match config.stripe_secret.as_deref() {
            None => check("stripe.credential", false, "missing test secret"),
            Some(secret) => match assert_test_secret(secret) {
                Ok(()) => check(
                    "stripe.credential",
                    true,
                    "accepted sk_test_/rk_test_ prefix (secret not shown)",
                ),
                Err(_) => check(
                    "stripe.credential",
                    false,
                    "live or unrecognized Stripe credential refused",
                ),
            },
        },
    }
}

async fn check_stripe_connectivity(config: &Config) -> CheckResult {
    if config.provider != ProviderKind::Stripe {
        return check("stripe.connectivity", true, "skipped — provider!=stripe");
    }
    let Some(secret) = config.stripe_secret.as_deref() else {
        return check("stripe.connectivity", false, "no credential");
    };
    if assert_test_secret(secret).is_err() {
        return check("stripe.connectivity", false, "live credentials refused");
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return check("stripe.connectivity", false, "http client failed");
    };
    match client
        .get("https://api.stripe.com/v1/balance")
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => check(
            "stripe.connectivity",
            true,
            "test account reachable via /v1/balance",
        ),
        Ok(response) => check(
            "stripe.connectivity",
            false,
            format!("stripe returned HTTP {}", response.status()),
        ),
        Err(_) => check(
            "stripe.connectivity",
            false,
            "network error reaching api.stripe.com",
        ),
    }
}
