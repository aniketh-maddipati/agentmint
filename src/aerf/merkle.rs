//! RFC 6962 hashing primitives for AERF log inclusion proofs.
//! Used by: receipt verification and conformance tests.

use sha2::{Digest, Sha256};

use crate::aerf::{AerfError, AerfResult};

pub fn log_leaf_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update(data);
    hex::encode(hasher.finalize())
}

pub fn hash_internal(left_hex: &str, right_hex: &str) -> AerfResult<String> {
    let left = hex::decode(left_hex).map_err(|_| AerfError::InvalidHex {
        field: "merkle_left",
    })?;
    let right = hex::decode(right_hex).map_err(|_| AerfError::InvalidHex {
        field: "merkle_right",
    })?;

    let mut hasher = Sha256::new();
    hasher.update([0x01]);
    hasher.update(left);
    hasher.update(right);
    Ok(hex::encode(hasher.finalize()))
}

pub fn walk_audit_path(
    leaf_hash_hex: &str,
    path_hex: &[String],
    leaf_index: u64,
    tree_size: u64,
) -> Option<String> {
    if tree_size == 0 || leaf_index >= tree_size {
        return None;
    }

    let mut fn_index = leaf_index;
    let mut sn_index = tree_size - 1;
    let mut root = leaf_hash_hex.to_owned();

    for sibling in path_hex {
        if sn_index == 0 {
            return None;
        }

        let next = if !fn_index.is_multiple_of(2) || fn_index == sn_index {
            let hashed = hash_internal(sibling, &root).ok()?;
            if fn_index.is_multiple_of(2) {
                while fn_index.is_multiple_of(2) && fn_index != 0 {
                    fn_index /= 2;
                    sn_index /= 2;
                }
            }
            hashed
        } else {
            hash_internal(&root, sibling).ok()?
        };

        root = next;
        fn_index /= 2;
        sn_index /= 2;
    }

    if sn_index == 0 {
        Some(root)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{log_leaf_hash, walk_audit_path};

    #[test]
    fn rejects_invalid_leaf_index() {
        assert!(walk_audit_path("00", &[], 1, 1).is_none());
    }

    #[test]
    fn produces_known_leaf_hash() {
        assert_eq!(
            log_leaf_hash(br#"{"a":1}"#),
            "c7261463ebd776f4650b6d0fe942d9cc38c925d90f77d440ab6df8d5dd258c5f"
        );
    }
}
