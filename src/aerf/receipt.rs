//! Receipt parsing and verification for AERF evidence artifacts.
//! Used by: chain verification and conformance tests.

use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::aerf::canonical::{canonicalize, sha256_hex};
use crate::aerf::merkle::{log_leaf_hash, walk_audit_path};
use crate::aerf::sign::{decode_signature, signed_payload_bytes_verbatim, verify_stripped};
use crate::aerf::{AerfError, AerfResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    Skipped,
    Passed,
    Failed,
    Missing,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyOptions<'a> {
    pub parent_public_key: Option<&'a VerifyingKey>,
    pub pdp_public_key: Option<&'a VerifyingKey>,
    pub log_public_key: Option<&'a VerifyingKey>,
    pub require_parent_signature: bool,
    pub require_pdp_signature: bool,
    pub require_log: bool,
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub ok: bool,
    pub issuer_ok: bool,
    pub parent: CheckOutcome,
    pub pdp: CheckOutcome,
    pub log: CheckOutcome,
    pub has_impact: bool,
    pub impact_tags: Vec<String>,
    pub fail_category: Option<String>,
    pub fail_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Receipt {
    pub id: String,
    #[serde(rename = "type")]
    pub receipt_type: String,
    pub plan_id: String,
    pub agent: String,
    pub action: String,
    pub in_policy: bool,
    pub policy_reason: String,
    pub evidence_hash_sha512: String,
    pub evidence: Value,
    pub observed_at: String,
    #[serde(default)]
    pub aiuc_controls: Vec<String>,
    pub key_id: String,
    #[serde(default)]
    pub agent_key_id: Option<String>,
    #[serde(default)]
    pub policy_hash: Option<String>,
    #[serde(default)]
    pub output_hash: Option<String>,
    #[serde(default)]
    pub agent_signature: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_trajectory: Vec<Value>,
    #[serde(default)]
    pub session_escalation: Option<String>,
    #[serde(default)]
    pub reasoning_hash: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub original_verdict: Option<bool>,
    #[serde(default)]
    pub previous_receipt_hash: Option<String>,
    #[serde(default)]
    pub plan_signature: Option<String>,
    #[serde(default)]
    pub seq: Option<u64>,
    #[serde(default)]
    pub compliance_tags: Vec<String>,
    #[serde(default)]
    pub impact_tags: Vec<String>,
    #[serde(default)]
    pub context_hash_sha256: Option<String>,
    #[serde(default)]
    pub pdp_signature: Option<String>,
    #[serde(default)]
    pub pdp_key_id: Option<String>,
    pub signature: String,
    #[serde(default)]
    pub timestamp: Option<Value>,
    #[serde(default)]
    pub parent_signature: Option<String>,
    #[serde(default)]
    pub parent_key_id: Option<String>,
    #[serde(default)]
    pub log_inclusion_proof: Option<LogInclusionProof>,
    #[serde(skip)]
    raw: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogInclusionProof {
    pub log_id: String,
    pub leaf_hash: String,
    #[serde(default)]
    pub leaf_index: Option<u64>,
    pub tree_size: u64,
    pub audit_path: Vec<String>,
    pub sth: SignedTreeHead,
    pub sth_signature: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignedTreeHead {
    pub tree_size: u64,
    pub root_hash: String,
    pub timestamp: String,
}

impl Receipt {
    pub fn from_value(raw: Value) -> AerfResult<Self> {
        let mut receipt: Self = serde_json::from_value(raw.clone())?;
        receipt.raw = raw;
        Ok(receipt)
    }

    pub fn from_slice(bytes: &[u8]) -> AerfResult<Self> {
        let raw: Value = serde_json::from_slice(bytes)?;
        Self::from_value(raw)
    }

    pub fn raw(&self) -> &Value {
        &self.raw
    }

    pub fn signed_payload_bytes(&self) -> AerfResult<Vec<u8>> {
        signed_payload_bytes_verbatim(&self.raw)
    }

    pub fn chain_hash(&self) -> AerfResult<String> {
        Ok(sha256_hex(&self.signed_payload_bytes()?))
    }
}

pub fn verify_aerf_receipt(
    receipt: &Receipt,
    issuer_public_key: &VerifyingKey,
    options: VerifyOptions<'_>,
) -> VerifyResult {
    let mut result = VerifyResult {
        ok: false,
        issuer_ok: false,
        parent: CheckOutcome::Skipped,
        pdp: CheckOutcome::Skipped,
        log: CheckOutcome::Skipped,
        has_impact: !receipt.impact_tags.is_empty(),
        impact_tags: receipt.impact_tags.clone(),
        fail_category: None,
        fail_reason: None,
    };

    if matches!(receipt.previous_receipt_hash.as_deref(), Some("")) {
        return fail(
            &mut result,
            "chain",
            "previous_receipt_hash present but empty",
        );
    }

    if receipt.signature.is_empty()
        || !verify_stripped(receipt.raw(), issuer_public_key, &receipt.signature)
    {
        return fail(
            &mut result,
            "issuer_signature",
            "issuer signature verification FAILED",
        );
    }
    result.issuer_ok = true;

    let parent_check = check_parent_signature(receipt, &options);
    result.parent = parent_check.outcome;
    if let Some(error) = parent_check.error {
        return fail(&mut result, "parent_signature", &error);
    }

    let pdp_check = check_pdp_signature(receipt, &options);
    result.pdp = pdp_check.outcome;
    if let Some(error) = pdp_check.error {
        return fail(&mut result, "pdp_signature", &error);
    }

    let log_check = check_log_inclusion(receipt, &options);
    result.log = log_check.outcome;
    if let Some(error) = log_check.error {
        return fail(&mut result, "log_inclusion", &error);
    }

    result.ok = true;
    result
}

pub fn context_hash_sha256(context: &Value) -> AerfResult<String> {
    let normalized = numbers_as_strings(context);
    let bytes = canonicalize(&normalized)?.into_bytes();
    Ok(sha256_hex(&bytes))
}

pub fn pdp_tuple(context_hash_sha256: &str, in_policy: bool, policy_hash: &str) -> Value {
    json!({
        "context_hash_sha256": context_hash_sha256,
        "in_policy": in_policy,
        "policy_hash": policy_hash
    })
}

struct CheckResult {
    outcome: CheckOutcome,
    error: Option<String>,
}

fn check_parent_signature(receipt: &Receipt, options: &VerifyOptions<'_>) -> CheckResult {
    let has_signature = receipt
        .parent_signature
        .as_ref()
        .is_some_and(|signature| !signature.is_empty());

    if receipt.impact_tags.len() > 0 && !has_signature {
        return missing("impact_tags non-empty but parent_signature absent");
    }

    if !has_signature {
        if options.require_parent_signature {
            return missing("require_parent_signature set but parent_signature absent");
        }
        return skipped();
    }

    let public_key = match options.parent_public_key {
        Some(public_key) => public_key,
        None if options.require_parent_signature || !receipt.impact_tags.is_empty() => {
            return missing("parent_signature present but no parent key supplied");
        }
        None => return skipped(),
    };

    if !verify_signature_bytes(
        &receipt.signed_payload_bytes().unwrap_or_default(),
        public_key,
        receipt.parent_signature.as_deref().unwrap_or_default(),
        "parent_signature",
    ) {
        return failed("parent signature verification FAILED");
    }

    passed()
}

fn check_pdp_signature(receipt: &Receipt, options: &VerifyOptions<'_>) -> CheckResult {
    let has_signature = receipt
        .pdp_signature
        .as_ref()
        .is_some_and(|signature| !signature.is_empty());

    if receipt.impact_tags.len() > 0 && !has_signature {
        return missing("impact_tags non-empty but pdp_signature absent");
    }

    if !has_signature {
        if options.require_pdp_signature {
            return missing("require_pdp_signature set but pdp_signature absent");
        }
        return skipped();
    }

    let public_key = match options.pdp_public_key {
        Some(public_key) => public_key,
        None if options.require_pdp_signature || !receipt.impact_tags.is_empty() => {
            return missing("pdp_signature present but no PDP key supplied");
        }
        None => return skipped(),
    };

    let Some(context_hash) = receipt.context_hash_sha256.as_deref() else {
        return failed("pdp_signature requires context_hash_sha256, policy_hash, in_policy");
    };
    let Some(policy_hash) = receipt.policy_hash.as_deref() else {
        return failed("pdp_signature requires context_hash_sha256, policy_hash, in_policy");
    };
    let payload = match canonicalize(&pdp_tuple(context_hash, receipt.in_policy, policy_hash)) {
        Ok(payload) => payload.into_bytes(),
        Err(_) => return failed("pdp signature verification FAILED"),
    };

    if !verify_signature_bytes(
        &payload,
        public_key,
        receipt.pdp_signature.as_deref().unwrap_or_default(),
        "pdp_signature",
    ) {
        return failed("pdp signature verification FAILED");
    }

    passed()
}

fn check_log_inclusion(receipt: &Receipt, options: &VerifyOptions<'_>) -> CheckResult {
    let Some(proof) = &receipt.log_inclusion_proof else {
        if options.require_log {
            return missing("require_log set but log_inclusion_proof absent");
        }
        return skipped();
    };

    let public_key = match options.log_public_key {
        Some(public_key) => public_key,
        None if options.require_log => return missing("require_log set but no log key supplied"),
        None => return skipped(),
    };

    let error = match verify_log_inclusion_proof(receipt, proof, public_key) {
        Ok(None) => return passed(),
        Ok(Some(error)) => error,
        Err(_) => "audit_path does not lead to STH root_hash".to_owned(),
    };

    failed(&error)
}

fn verify_log_inclusion_proof(
    receipt: &Receipt,
    proof: &LogInclusionProof,
    public_key: &VerifyingKey,
) -> AerfResult<Option<String>> {
    if !is_lower_hex_64(&proof.leaf_hash) {
        return Ok(Some("leaf_hash malformed".to_owned()));
    }

    let sth_value = raw_sth_value(receipt).ok_or_else(|| AerfError::InvalidHex { field: "sth" })?;
    let sth_payload = canonicalize_verbatim_or_strict(sth_value)?.into_bytes();
    if !verify_signature_bytes(
        &sth_payload,
        public_key,
        &proof.sth_signature,
        "sth_signature",
    ) {
        return Ok(Some("sth signature verification FAILED".to_owned()));
    }

    let expected_leaf_hash = log_leaf_hash(&receipt.signed_payload_bytes()?);
    if expected_leaf_hash != proof.leaf_hash {
        return Ok(Some(
            "leaf_hash does not match recomputed receipt leaf".to_owned(),
        ));
    }

    if proof.audit_path.iter().any(|entry| !is_lower_hex_64(entry)) {
        return Ok(Some("audit_path malformed".to_owned()));
    }

    let root = walk_audit_path(
        &proof.leaf_hash,
        &proof.audit_path,
        proof.leaf_index.unwrap_or(0),
        proof.tree_size,
    );

    if root.as_deref() != Some(proof.sth.root_hash.as_str()) {
        return Ok(Some("audit_path does not lead to STH root_hash".to_owned()));
    }

    Ok(None)
}

fn raw_sth_value(receipt: &Receipt) -> Option<&Value> {
    receipt.raw().get("log_inclusion_proof")?.get("sth")
}

fn canonicalize_verbatim_or_strict(value: &Value) -> AerfResult<String> {
    crate::aerf::canonical::canonicalize_verbatim(value)
}

fn verify_signature_bytes(
    payload: &[u8],
    public_key: &VerifyingKey,
    signature_hex: &str,
    field: &'static str,
) -> bool {
    let signature = match decode_signature(signature_hex, field) {
        Ok(signature) => signature,
        Err(_) => return false,
    };
    public_key.verify_strict(payload, &signature).is_ok()
}

fn numbers_as_strings(value: &Value) -> Value {
    match value {
        Value::Number(number) => Value::String(number.to_string()),
        Value::Array(values) => Value::Array(values.iter().map(numbers_as_strings).collect()),
        Value::Object(object) => {
            let mapped = object
                .iter()
                .map(|(key, value)| (key.clone(), numbers_as_strings(value)))
                .collect();
            Value::Object(mapped)
        }
        _ => value.clone(),
    }
}

fn fail(result: &mut VerifyResult, category: &str, reason: &str) -> VerifyResult {
    result.ok = false;
    result.fail_category = Some(category.to_owned());
    result.fail_reason = Some(reason.to_owned());
    result.clone()
}

fn passed() -> CheckResult {
    CheckResult {
        outcome: CheckOutcome::Passed,
        error: None,
    }
}

fn skipped() -> CheckResult {
    CheckResult {
        outcome: CheckOutcome::Skipped,
        error: None,
    }
}

fn missing(message: &str) -> CheckResult {
    CheckResult {
        outcome: CheckOutcome::Missing,
        error: Some(message.to_owned()),
    }
}

fn failed(message: &str) -> CheckResult {
    CheckResult {
        outcome: CheckOutcome::Failed,
        error: Some(message.to_owned()),
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{context_hash_sha256, pdp_tuple, Receipt};

    #[test]
    fn hashes_context_numbers_as_strings() {
        let context = json!({ "amount": 1250.0, "nested": [1, 2] });
        let hash = context_hash_sha256(&context).expect("hash");

        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn parses_receipt_and_keeps_raw_value() {
        let receipt = Receipt::from_value(json!({
            "id": "1",
            "type": "notarised_evidence",
            "plan_id": "p",
            "agent": "a",
            "action": "x",
            "in_policy": true,
            "policy_reason": "ok",
            "evidence_hash_sha512": "abc",
            "evidence": { "x": 1 },
            "observed_at": "2026-05-11T12:00:00+00:00",
            "key_id": "kid",
            "signature": "00"
        }))
        .expect("receipt");

        assert_eq!(receipt.raw()["type"], json!("notarised_evidence"));
        assert_eq!(pdp_tuple("ctx", true, "policy")["in_policy"], json!(true));
    }
}
