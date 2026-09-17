//! Signed execution receipts. A signature proves what Mint recorded, not Stripe internals.
//! Used by: execution engine, CLI verify, GET /v1/actions/{id}/receipt.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use serde_json::Value;

use crate::canonical::{canonicalize_value, hash_canonical_json};
use crate::domain::{SignedReceipt, SIGNED_OBJECT_VERSION};
use crate::error::{Error, Result};
use crate::keys::KeyRing;

pub fn sign_receipt(keys: &KeyRing, mut receipt: SignedReceipt) -> Result<SignedReceipt> {
    receipt.format_version = SIGNED_OBJECT_VERSION.to_owned();
    receipt.kid = keys.kid.clone();
    receipt.payload.kid = keys.kid.clone();
    let canonical = payload_canonical(&receipt.payload)?;
    let signature = keys.signing_key().sign(canonical.as_bytes());
    receipt.signature = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    Ok(receipt)
}

pub fn verify_receipt(receipt: &SignedReceipt, keys: &KeyRing) -> Result<()> {
    if receipt.format_version != SIGNED_OBJECT_VERSION
        || receipt.payload.format_version != crate::domain::RECEIPT_VERSION
    {
        return Err(Error::UnknownReceiptFormat);
    }
    if receipt.kid != keys.kid || receipt.payload.kid != keys.kid {
        return Err(Error::UnknownKeyId);
    }
    let canonical = payload_canonical(&receipt.payload)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&receipt.signature)
        .map_err(|_| Error::InvalidSignature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| Error::InvalidSignature)?;
    keys.verifying_key()
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| Error::InvalidSignature)?;
    Ok(())
}

pub fn verify_with_public_key(
    receipt: &SignedReceipt,
    kid: &str,
    verifying: &VerifyingKey,
) -> Result<()> {
    if receipt.format_version != SIGNED_OBJECT_VERSION
        || receipt.payload.format_version != crate::domain::RECEIPT_VERSION
    {
        return Err(Error::UnknownReceiptFormat);
    }
    if receipt.kid != kid || receipt.payload.kid != kid {
        return Err(Error::UnknownKeyId);
    }
    let canonical = payload_canonical(&receipt.payload)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&receipt.signature)
        .map_err(|_| Error::InvalidSignature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| Error::InvalidSignature)?;
    verifying
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| Error::InvalidSignature)?;
    Ok(())
}

fn payload_canonical(payload: &crate::domain::ReceiptPayload) -> Result<String> {
    let value =
        serde_json::to_value(payload).map_err(|err| Error::internal("receipt json", err))?;
    canonicalize_value(&value)
}

pub fn payload_hash(payload: &crate::domain::ReceiptPayload) -> Result<String> {
    Ok(hash_canonical_json(&payload_canonical(payload)?))
}

pub fn verifying_key_from_jwk_x(x: &str) -> Result<VerifyingKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(x)
        .map_err(|_| Error::InvalidRequest("invalid public key"))?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::InvalidRequest("invalid public key length"))?;
    VerifyingKey::from_bytes(&array).map_err(|_| Error::InvalidRequest("invalid public key"))
}

pub fn signed_from_value(value: &Value) -> Result<SignedReceipt> {
    serde_json::from_value(value.clone()).map_err(|_| Error::UnknownReceiptFormat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ActorIdentity, PolicyDecision, PolicyEffect, ReceiptPayload, RECEIPT_VERSION,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn sample(keys: &KeyRing) -> SignedReceipt {
        SignedReceipt {
            format_version: SIGNED_OBJECT_VERSION.to_owned(),
            kid: keys.kid.clone(),
            payload: ReceiptPayload {
                format_version: RECEIPT_VERSION.to_owned(),
                action_id: Uuid::new_v4(),
                tenant_id: "acme".into(),
                actor: ActorIdentity {
                    subject: "user_123".into(),
                    agent_id: "support-agent-7".into(),
                    delegated_by: None,
                    issuer: "https://identity.example.com".into(),
                },
                intent_hash: "sha256:abc".into(),
                policy: PolicyDecision {
                    effect: PolicyEffect::Automatic,
                    policy_version: "v1".into(),
                    reason: "auto".into(),
                },
                approval: None,
                attempt_id: Uuid::new_v4(),
                provider: "fake".into(),
                operation: "refund.create".into(),
                provider_idempotency_key: "mint:1".into(),
                provider_resource_id: Some("re_123".into()),
                status: crate::domain::ActionStatus::Succeeded,
                started_at: Utc::now(),
                completed_at: Utc::now(),
                reconciliation_required: false,
                kid: keys.kid.clone(),
            },
            signature: String::new(),
        }
    }

    #[test]
    fn receipt_verifies_and_tampering_fails() {
        let keys = KeyRing::generate();
        let signed = sign_receipt(&keys, sample(&keys)).expect("sign");
        verify_receipt(&signed, &keys).expect("verify");
        let mut payload_tamper = signed.clone();
        payload_tamper.payload.provider_resource_id = Some("re_evil".into());
        assert!(verify_receipt(&payload_tamper, &keys).is_err());
        let mut sig_tamper = signed;
        sig_tamper.signature.push('A');
        assert!(verify_receipt(&sig_tamper, &keys).is_err());
    }
}
