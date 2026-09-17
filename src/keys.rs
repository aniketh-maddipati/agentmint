//! Persistent Ed25519 signing keys with a stable kid.
//! Used by: receipts, CLI init, GET /v1/keys.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::Serialize;

use crate::error::{Error, Result};

pub struct KeyRing {
    pub kid: String,
    signing: SigningKey,
}

#[derive(Serialize)]
pub struct PublicJwk {
    pub kid: String,
    pub kty: &'static str,
    pub crv: &'static str,
    pub x: String,
    pub alg: &'static str,
}

impl KeyRing {
    pub fn generate() -> Self {
        Self {
            kid: "mint-local-1".into(),
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_config(kid: &str, file: Option<&Path>, env_value: Option<&str>) -> Result<Self> {
        if let Some(value) = env_value {
            return Self::parse(kid, value);
        }
        let Some(path) = file else {
            return Err(Error::Misconfigured(
                "set MINT_SIGNING_KEY_FILE or MINT_SIGNING_KEY; run `mint init` for local development",
            ));
        };
        let pem =
            fs::read_to_string(path).map_err(|err| Error::internal("read signing key", err))?;
        Self::parse(kid, &pem)
    }

    pub fn parse(kid: &str, value: &str) -> Result<Self> {
        let trimmed = value.trim();
        let signing = if trimmed.contains("BEGIN") {
            SigningKey::from_pkcs8_pem(trimmed)
                .map_err(|err| Error::internal("parse signing key pem", err))?
        } else {
            let bytes = decode_raw_key(trimmed)?;
            SigningKey::from_bytes(&bytes)
        };
        Ok(Self {
            kid: kid.to_owned(),
            signing,
        })
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.signing
    }

    pub fn public_jwk(&self) -> PublicJwk {
        PublicJwk {
            kid: self.kid.clone(),
            kty: "OKP",
            crv: "Ed25519",
            x: URL_SAFE_NO_PAD.encode(self.verifying_key().as_bytes()),
            alg: "EdDSA",
        }
    }

    pub fn write_pkcs8_pem(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(Error::Misconfigured(
                "signing key file already exists; refusing to overwrite",
            ));
        }
        let pem = self
            .signing
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .map_err(|err| Error::internal("encode signing key", err))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|err| Error::internal("create signing key file", err))?;
        file.write_all(pem.as_bytes())
            .map_err(|err| Error::internal("write signing key", err))?;
        Ok(())
    }
}

fn decode_raw_key(value: &str) -> Result<[u8; 32]> {
    let bytes = if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        hex::decode(value).map_err(|err| Error::internal("hex key", err))?
    } else {
        STANDARD
            .decode(value)
            .or_else(|_| URL_SAFE_NO_PAD.decode(value))
            .map_err(|err| Error::internal("base64 key", err))?
    };
    bytes
        .try_into()
        .map_err(|_| Error::Misconfigured("signing key must be 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_round_trips_through_pem() {
        let ring = KeyRing::generate();
        let pem = ring
            .signing
            .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .expect("pem");
        let parsed = KeyRing::parse("k1", pem.as_str()).expect("parse");
        assert_eq!(
            ring.verifying_key().as_bytes(),
            parsed.verifying_key().as_bytes()
        );
    }
}
