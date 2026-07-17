//! Signing helpers for stripped AERF payloads.
//! Used by: receipt and chain verification.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::Value;

use crate::aerf::canonical::{canonicalize, canonicalize_verbatim, sha256_hex};
use crate::aerf::{AerfError, AerfResult};

pub const POST_ISSUANCE_FIELDS: [&str; 5] = [
    "signature",
    "timestamp",
    "parent_signature",
    "parent_key_id",
    "log_inclusion_proof",
];

pub fn key_id(public_key: &VerifyingKey) -> String {
    sha256_hex(&public_key.to_bytes())[..16].to_owned()
}

pub fn strip_post_issuance(value: &Value) -> Value {
    let mut stripped = value.clone();
    if let Value::Object(object) = &mut stripped {
        for field in POST_ISSUANCE_FIELDS {
            object.remove(field);
        }
    }
    stripped
}

pub fn signed_payload_bytes(value: &Value) -> AerfResult<Vec<u8>> {
    canonicalize(&strip_post_issuance(value)).map(String::into_bytes)
}

pub(crate) fn signed_payload_bytes_verbatim(value: &Value) -> AerfResult<Vec<u8>> {
    canonicalize_verbatim(&strip_post_issuance(value)).map(String::into_bytes)
}

pub fn sign_stripped(value: &Value, signing_key: &SigningKey) -> AerfResult<String> {
    let payload = signed_payload_bytes(value)?;
    Ok(hex::encode(signing_key.sign(&payload).to_bytes()))
}

pub fn verify_stripped(value: &Value, public_key: &VerifyingKey, signature_hex: &str) -> bool {
    let payload = match signed_payload_bytes_verbatim(value) {
        Ok(payload) => payload,
        Err(_) => return false,
    };

    let signature = match decode_signature(signature_hex, "signature") {
        Ok(signature) => signature,
        Err(_) => return false,
    };

    public_key.verify(&payload, &signature).is_ok()
}

pub(crate) fn decode_signature(signature_hex: &str, field: &'static str) -> AerfResult<Signature> {
    if signature_hex.to_ascii_lowercase() != signature_hex {
        return Err(AerfError::InvalidSignature { field });
    }

    let bytes = hex::decode(signature_hex).map_err(|_| AerfError::InvalidSignature { field })?;
    Signature::from_slice(&bytes).map_err(|_| AerfError::InvalidSignature { field })
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use serde_json::json;

    use super::{key_id, sign_stripped, strip_post_issuance, verify_stripped};

    #[test]
    fn strips_only_post_issuance_fields() {
        let value = json!({
            "signature": "sig",
            "timestamp": {"a": 1},
            "parent_signature": "parent",
            "parent_key_id": "kid",
            "log_inclusion_proof": {"leaf_hash": "x"},
            "action": "submit"
        });

        let stripped = strip_post_issuance(&value);

        assert_eq!(stripped, json!({ "action": "submit" }));
    }

    #[test]
    fn signs_and_verifies_integer_payloads() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let verifying_key = signing_key.verifying_key();
        let value = json!({
            "action": "submit",
            "evidence": { "count": 3 }
        });

        let signature = sign_stripped(&value, &signing_key).expect("signature");

        assert!(verify_stripped(&value, &verifying_key, &signature));
        assert_eq!(key_id(&verifying_key).len(), 16);
    }
}
