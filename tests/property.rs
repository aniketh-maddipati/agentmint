//! Property tests for canonicalization stability and state-machine direction.

use mint_run::canonical::{canonicalize_value, hash_canonical_json};
use mint_run::domain::{can_transition, ActionStatus};
use proptest::prelude::*;
use serde_json::{json, Value};

proptest! {
    #[test]
    fn canonicalization_is_stable_under_object_rebuild(
        tenant in "[a-z]{1,8}",
        amount in 1i64..50_000,
        ticket in "[a-z0-9]{1,12}"
    ) {
        let left = json!({
            "version": "mint-intent-v1",
            "tenant_id": tenant,
            "actor": {
                "subject": "user",
                "agent_id": "agent",
                "delegated_by": Value::Null,
                "issuer": "https://identity.example.com"
            },
            "provider": "stripe",
            "operation": "refund.create",
            "resource": { "resource_type": "charge", "resource_id": "ch_1" },
            "arguments": { "amount": amount, "currency": "usd", "reason": "duplicate" },
            "context": { "support_ticket_id": ticket, "reason": "duplicate" }
        });
        let parsed: Value = serde_json::from_str(&left.to_string()).unwrap();
        let a = canonicalize_value(&left).unwrap();
        let b = canonicalize_value(&parsed).unwrap();
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(hash_canonical_json(&a), hash_canonical_json(&b));
    }

    #[test]
    fn mutating_a_material_string_changes_the_hash(
        tenant in "[a-z]{2,8}",
        other in "[0-9]{2,8}"
    ) {
        prop_assume!(tenant != other);
        let mut value = json!({
            "version": "mint-intent-v1",
            "tenant_id": tenant,
            "actor": {
                "subject": "user",
                "agent_id": "agent",
                "delegated_by": Value::Null,
                "issuer": "https://example"
            },
            "provider": "stripe",
            "operation": "refund.create",
            "resource": { "resource_type": "charge", "resource_id": "ch_1" },
            "arguments": { "amount": 4200, "currency": "usd", "reason": "duplicate" },
            "context": { "support_ticket_id": "t1", "reason": "duplicate" }
        });
        let before = hash_canonical_json(&canonicalize_value(&value).unwrap());
        value["tenant_id"] = json!(other);
        let after = hash_canonical_json(&canonicalize_value(&value).unwrap());
        prop_assert_ne!(before, after);
    }
}

#[test]
fn transitions_never_execute_twice_or_move_backward() {
    let mut status = ActionStatus::Proposed;
    let sequence = [
        ActionStatus::PendingApproval,
        ActionStatus::Authorized,
        ActionStatus::Executing,
        ActionStatus::Succeeded,
        ActionStatus::Executing,
        ActionStatus::Authorized,
        ActionStatus::Proposed,
    ];
    let mut executed = false;
    for next in sequence {
        if can_transition(status, next) {
            if status == ActionStatus::Authorized && next == ActionStatus::Executing {
                assert!(!executed);
                executed = true;
            }
            status = next;
        } else {
            assert!(
                next == ActionStatus::Executing
                    || next == ActionStatus::Authorized
                    || next == ActionStatus::Proposed
                    || status.is_terminal()
            );
        }
    }
    assert!(executed);
    assert_eq!(status, ActionStatus::Succeeded);
}
