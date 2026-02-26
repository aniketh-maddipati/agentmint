//! Ed25519 token signing.
//! Used by: handlers::mint.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{SigningKey, Signer};

use crate::error::Result;
use crate::token::claims::Claims;

pub fn sign_token(claims: &Claims, key: &SigningKey) -> Result<String> {
    let payload = serde_json::to_vec(claims)?;
    let encoded_payload = URL_SAFE_NO_PAD.encode(&payload);
    let signature = key.sign(encoded_payload.as_bytes());
    let encoded_signature = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    Ok(format!("{}.{}", encoded_payload, encoded_signature))
}

pub fn generate_keypair() -> SigningKey {
    SigningKey::generate(&mut rand::thread_rng())
}
