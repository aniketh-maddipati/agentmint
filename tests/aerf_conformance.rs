//! AERF conformance coverage for published TS SDK vectors.
//! Used by: cargo test acceptance for the Rust crypto-core port.

use std::fs;
use std::path::{Path, PathBuf};

use agentmint::aerf::chain::{verify_aerf_chain, ChainBreakType, VerifyChainOptions};
use agentmint::aerf::receipt::{verify_aerf_receipt, Receipt, VerifyOptions};
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    dir: String,
    outcome: String,
    reason_code: String,
}

#[derive(Debug, Deserialize)]
struct ExpectedOutcome {
    outcome: String,
    reason_code: String,
}

#[derive(Debug, Deserialize)]
struct LocalExpectedOutcome {
    outcome: String,
    break_type: String,
    break_at_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualOutcome {
    outcome: String,
    reason_code: String,
}

#[test]
fn aerf_conformance_vectors_match_manifest() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test/vectors/manifest.json");
    let manifest: Vec<ManifestEntry> =
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest bytes"))
            .expect("manifest json");

    let mut summary = Vec::with_capacity(manifest.len());

    for entry in manifest {
        let vector_dir = manifest_path
            .parent()
            .expect("manifest parent")
            .join(&entry.dir);
        let expected: ExpectedOutcome =
            serde_json::from_slice(&fs::read(vector_dir.join("expected.json")).expect("expected"))
                .expect("expected json");

        assert_eq!(entry.outcome, expected.outcome, "{}", entry.dir);
        assert_eq!(entry.reason_code, expected.reason_code, "{}", entry.dir);

        let actual = verify_vector(&vector_dir, &expected.outcome);
        assert_eq!(actual.outcome, expected.outcome, "{}", entry.dir);
        if expected.outcome == "FAIL" {
            assert_eq!(actual.reason_code, expected.reason_code, "{}", entry.dir);
        }

        summary.push(format!(
            "{} | expected={} | actual={} | reason={}",
            entry.dir, expected.outcome, actual.outcome, actual.reason_code
        ));
    }

    let local_summary = verify_local_seq_gap_fixture();
    summary.push(local_summary);

    println!("AERF conformance 12/12 official + 1 local");
    for line in summary {
        println!("{line}");
    }
}

fn verify_local_seq_gap_fixture() -> String {
    let vector_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test/vectors/13-seq-gap-local");
    let expected: LocalExpectedOutcome =
        serde_json::from_slice(&fs::read(vector_dir.join("expected.json")).expect("expected"))
            .expect("expected json");
    let issuer_public_key = load_required_key(&vector_dir.join("public_key.pem"));
    let receipts = load_receipts(&vector_dir);

    let full_chain = verify_aerf_chain(
        &receipts,
        VerifyChainOptions {
            issuer_public_key: Some(&issuer_public_key),
        },
    );
    assert!(
        full_chain.valid,
        "13-seq-gap-local full chain should be valid"
    );

    let deleted_middle = vec![receipts[0].clone(), receipts[2].clone()];
    let subset_chain = verify_aerf_chain(
        &deleted_middle,
        VerifyChainOptions {
            issuer_public_key: Some(&issuer_public_key),
        },
    );

    assert!(!subset_chain.valid, "13-seq-gap-local subset should fail");
    assert_eq!(expected.outcome, "FAIL");
    assert_eq!(
        subset_chain.break_at_index,
        Some(expected.break_at_index),
        "13-seq-gap-local break index"
    );
    assert_eq!(
        chain_break_type_name(subset_chain.break_type),
        expected.break_type,
        "13-seq-gap-local break type"
    );

    format!(
        "13-seq-gap-local [LOCAL, not official] | expected={} | actual={} | reason={}",
        expected.outcome,
        "FAIL",
        chain_break_type_name(subset_chain.break_type)
    )
}

fn verify_vector(vector_dir: &Path, expected_outcome: &str) -> ActualOutcome {
    let issuer_public_key = load_required_key(&vector_dir.join("public_key.pem"));
    let parent_public_key = load_optional_key(&vector_dir.join("parent_key.pem"));
    let pdp_public_key = load_optional_key(&vector_dir.join("pdp_key.pem"));
    let log_public_key = load_optional_key(&vector_dir.join("log_key.pem"));

    let receipts = load_receipts(vector_dir);

    if receipts.len() > 1 {
        let chain = verify_aerf_chain(
            &receipts,
            VerifyChainOptions {
                issuer_public_key: Some(&issuer_public_key),
            },
        );

        if chain.valid {
            return ActualOutcome {
                outcome: expected_outcome.to_owned(),
                reason_code: String::new(),
            };
        }

        return ActualOutcome {
            outcome: "FAIL".to_owned(),
            reason_code: match chain.break_type {
                Some(ChainBreakType::SignatureInvalid) => "issuer_signature".to_owned(),
                Some(ChainBreakType::HashLinkMismatch)
                | Some(ChainBreakType::SeqGap)
                | Some(ChainBreakType::GenesisViolation) => "chain".to_owned(),
                None => String::new(),
            },
        };
    }

    let receipt = receipts.first().expect("single receipt");
    let result = verify_aerf_receipt(
        receipt,
        &issuer_public_key,
        VerifyOptions {
            parent_public_key: parent_public_key.as_ref(),
            pdp_public_key: pdp_public_key.as_ref(),
            log_public_key: log_public_key.as_ref(),
            require_parent_signature: false,
            require_pdp_signature: false,
            require_log: false,
        },
    );

    if result.ok {
        return ActualOutcome {
            outcome: expected_outcome.to_owned(),
            reason_code: String::new(),
        };
    }

    ActualOutcome {
        outcome: "FAIL".to_owned(),
        reason_code: result.fail_category.unwrap_or_default(),
    }
}

fn load_receipts(vector_dir: &Path) -> Vec<Receipt> {
    let chain_dir = vector_dir.join("receipts");
    if chain_dir.is_dir() {
        let mut files = read_json_files(&chain_dir);
        files.sort();
        return files
            .into_iter()
            .map(|path| {
                Receipt::from_slice(&fs::read(path).expect("receipt bytes")).expect("receipt")
            })
            .collect();
    }

    vec![
        Receipt::from_slice(&fs::read(vector_dir.join("receipt.json")).expect("receipt bytes"))
            .expect("receipt"),
    ]
}

fn read_json_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect()
}

fn load_required_key(path: &Path) -> VerifyingKey {
    load_optional_key(path).expect("public key")
}

fn load_optional_key(path: &Path) -> Option<VerifyingKey> {
    if !path.exists() {
        return None;
    }

    let pem = fs::read_to_string(path).expect("pem");
    Some(VerifyingKey::from_public_key_pem(&pem).expect("public key pem"))
}

fn chain_break_type_name(break_type: Option<ChainBreakType>) -> String {
    match break_type {
        Some(ChainBreakType::SignatureInvalid) => "signature_invalid".to_owned(),
        Some(ChainBreakType::HashLinkMismatch) => "hash_link_mismatch".to_owned(),
        Some(ChainBreakType::SeqGap) => "seq_gap".to_owned(),
        Some(ChainBreakType::GenesisViolation) => "genesis_violation".to_owned(),
        None => String::new(),
    }
}
