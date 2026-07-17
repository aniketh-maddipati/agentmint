//! Verification for ordered AERF receipt chains.
//! Used by: conformance tests and future offline verification.

use ed25519_dalek::VerifyingKey;

use crate::aerf::receipt::Receipt;
use crate::aerf::sign::verify_stripped;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainBreakType {
    SignatureInvalid,
    HashLinkMismatch,
    SeqGap,
    GenesisViolation,
}

#[derive(Debug, Clone)]
pub struct ChainVerification {
    pub valid: bool,
    pub length: usize,
    pub root_hash: String,
    pub break_at_index: Option<usize>,
    pub break_type: Option<ChainBreakType>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyChainOptions<'a> {
    pub issuer_public_key: Option<&'a VerifyingKey>,
}

pub fn verify_aerf_chain(
    receipts: &[Receipt],
    options: VerifyChainOptions<'_>,
) -> ChainVerification {
    if receipts.is_empty() {
        return ChainVerification {
            valid: true,
            length: 0,
            root_hash: String::new(),
            break_at_index: None,
            break_type: None,
            reason: None,
        };
    }

    let mut expected_previous_hash = None;
    let mut previous_seq = None;

    for (index, receipt) in receipts.iter().enumerate() {
        if let Some(public_key) = options.issuer_public_key {
            if receipt.signature.is_empty()
                || !verify_stripped(receipt.raw(), public_key, &receipt.signature)
            {
                return fail(
                    receipts.len(),
                    index,
                    ChainBreakType::SignatureInvalid,
                    "signature verification failed",
                );
            }
        }

        if index == 0 {
            if receipt.previous_receipt_hash.is_some() {
                return fail(
                    receipts.len(),
                    index,
                    ChainBreakType::GenesisViolation,
                    "first receipt carries previous_receipt_hash",
                );
            }
        } else if receipt.previous_receipt_hash.as_deref() != expected_previous_hash.as_deref() {
            return fail(
                receipts.len(),
                index,
                ChainBreakType::HashLinkMismatch,
                "previous_receipt_hash does not match predecessor",
            );
        }

        if let Some(seq) = receipt.seq {
            match previous_seq {
                None if index == 0 => {
                    if seq < 1 {
                        return fail(
                            receipts.len(),
                            index,
                            ChainBreakType::SeqGap,
                            "genesis seq must be at least 1",
                        );
                    }
                }
                Some(previous) if seq != previous + 1 => {
                    return fail(
                        receipts.len(),
                        index,
                        ChainBreakType::SeqGap,
                        "sequence gap detected",
                    );
                }
                _ => {}
            }
            previous_seq = Some(seq);
        }

        expected_previous_hash = receipt.chain_hash().ok();
    }

    ChainVerification {
        valid: true,
        length: receipts.len(),
        root_hash: expected_previous_hash.unwrap_or_default(),
        break_at_index: None,
        break_type: None,
        reason: None,
    }
}

fn fail(
    length: usize,
    break_at_index: usize,
    break_type: ChainBreakType,
    reason: &str,
) -> ChainVerification {
    ChainVerification {
        valid: false,
        length,
        root_hash: String::new(),
        break_at_index: Some(break_at_index),
        break_type: Some(break_type),
        reason: Some(reason.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{verify_aerf_chain, ChainBreakType, VerifyChainOptions};
    use crate::aerf::receipt::Receipt;

    #[test]
    fn flags_genesis_previous_hash() {
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
            "previous_receipt_hash": "bad",
            "signature": "00"
        }))
        .expect("receipt");

        let verification = verify_aerf_chain(&[receipt], VerifyChainOptions::default());

        assert_eq!(
            verification.break_type,
            Some(ChainBreakType::GenesisViolation)
        );
    }
}
