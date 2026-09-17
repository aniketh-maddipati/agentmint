//! RFC 8785 canonicalization and SHA-256 intent binding.
//! Used by: packs, execution, receipt signing, fixtures.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{ActionIntent, CanonicalAction, CANONICALIZATION_VERSION, INTENT_VERSION};
use crate::error::{Error, Result};

/// Material fields hashed for authorization binding:
/// `version`, `tenant_id`, `actor.{subject,agent_id,delegated_by,issuer}`,
/// `provider`, `operation`, `resource.{resource_type,resource_id}`,
/// `arguments`, `context.{support_ticket_id,reason}`.
///
/// Non-material operational fields excluded from the hash:
/// `action_id`, `created_at`, `expires_at`.
/// Provider idempotency is derived internally from the Mint action ID.
pub fn material_value(intent: &ActionIntent) -> Value {
    json!({
        "version": intent.version,
        "tenant_id": intent.tenant_id,
        "actor": {
            "subject": intent.actor.subject,
            "agent_id": intent.actor.agent_id,
            "delegated_by": intent.actor.delegated_by,
            "issuer": intent.actor.issuer,
        },
        "provider": intent.provider,
        "operation": intent.operation,
        "resource": {
            "resource_type": intent.resource.resource_type,
            "resource_id": intent.resource.resource_id,
        },
        "arguments": intent.arguments,
        "context": {
            "support_ticket_id": intent.context.support_ticket_id,
            "reason": intent.context.reason,
        },
    })
}

pub fn canonicalize_value(value: &Value) -> Result<String> {
    serde_json_canonicalizer::to_string(value).map_err(|err| Error::internal("canonicalize", err))
}

pub fn hash_canonical_json(canonical_json: &str) -> String {
    let digest = Sha256::digest(canonical_json.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

pub fn hash_intent(intent: &ActionIntent) -> Result<(String, String)> {
    let canonical_json = canonicalize_value(&material_value(intent))?;
    let intent_hash = hash_canonical_json(&canonical_json);
    Ok((canonical_json, intent_hash))
}

pub fn hashed_canonical(
    intent: &ActionIntent,
    refund: crate::domain::RefundAction,
) -> Result<CanonicalAction> {
    let (canonical_json, intent_hash) = hash_intent(intent)?;
    Ok(CanonicalAction {
        canonicalization_version: CANONICALIZATION_VERSION.to_owned(),
        canonical_json,
        intent_hash,
        provider: intent.provider.clone(),
        operation: intent.operation.clone(),
        refund,
    })
}

pub fn assert_intent_version(version: &str) -> Result<()> {
    if version != INTENT_VERSION {
        return Err(Error::InvalidRequest("unknown intent version"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ActionContext, ActorIdentity, ResourceRef, INTENT_VERSION};
    use chrono::Utc;
    use serde_json::{json, Value};
    use uuid::Uuid;

    fn sample() -> ActionIntent {
        ActionIntent {
            version: INTENT_VERSION.to_owned(),
            action_id: Uuid::nil(),
            tenant_id: "acme".into(),
            actor: ActorIdentity {
                subject: "user_123".into(),
                agent_id: "support-agent-7".into(),
                delegated_by: None,
                issuer: "https://identity.example.com".into(),
            },
            provider: "stripe".into(),
            operation: "refund.create".into(),
            resource: ResourceRef {
                resource_type: "charge".into(),
                resource_id: "ch_123".into(),
            },
            arguments: json!({
                "amount": 4200,
                "currency": "usd",
                "reason": "duplicate"
            }),
            context: ActionContext {
                support_ticket_id: Some("ticket_982".into()),
                reason: Some("duplicate".into()),
            },
            idempotency_key: "one".into(),
            created_at: Utc::now(),
            expires_at: Utc::now(),
        }
    }

    #[test]
    fn identical_semantic_intents_share_a_hash() {
        let mut left = sample();
        let mut right = sample();
        left.action_id = Uuid::new_v4();
        right.action_id = Uuid::new_v4();
        left.idempotency_key = "a".into();
        right.idempotency_key = "b".into();
        let left_hash = hash_intent(&left).expect("hash").1;
        let right_hash = hash_intent(&right).expect("hash").1;
        assert_eq!(left_hash, right_hash);
    }

    #[test]
    fn fixture_intents_share_a_stable_hash() {
        let base = std::fs::read_to_string("fixtures/canonicalization/support-refund-base.json")
            .expect("fixture");
        let reordered =
            std::fs::read_to_string("fixtures/canonicalization/support-refund-key-reorder.json")
                .expect("fixture");
        let base_value: Value = serde_json::from_str(&base).expect("json");
        let reordered_value: Value = serde_json::from_str(&reordered).expect("json");
        let left = canonicalize_value(&base_value["material"]).expect("canon");
        let right = canonicalize_value(&reordered_value["material"]).expect("canon");
        assert_eq!(left, right);
        let expected = base_value["expected_sha256"].as_str().expect("expected");
        assert_eq!(hash_canonical_json(&left), expected);
        assert_eq!(hash_canonical_json(&right), expected);
    }

    #[test]
    fn material_field_changes_change_the_hash() {
        let base = hash_intent(&sample()).expect("hash").1;
        let mut mutated = sample();
        mutated.tenant_id = "other".into();
        assert_ne!(base, hash_intent(&mutated).expect("hash").1);
        mutated = sample();
        mutated.actor.agent_id = "other-agent".into();
        assert_ne!(base, hash_intent(&mutated).expect("hash").1);
        mutated = sample();
        mutated.arguments["amount"] = json!(4201);
        assert_ne!(base, hash_intent(&mutated).expect("hash").1);
        mutated = sample();
        mutated.resource.resource_id = "ch_999".into();
        assert_ne!(base, hash_intent(&mutated).expect("hash").1);
        mutated = sample();
        mutated.context.support_ticket_id = Some("ticket_1".into());
        assert_ne!(base, hash_intent(&mutated).expect("hash").1);
    }
}
