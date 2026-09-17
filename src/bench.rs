//! Local overhead measurements excluding approval wait and provider I/O.
//! Used by: `mint bench`.

use std::time::Instant;

use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::canonical::hash_intent;
use crate::config::Config;
use crate::domain::{
    ActionContext, ActionIntent, ActionStatus, ActorIdentity, ExecutionAttempt, ExecutionGrant,
    PolicyDecision, PolicyEffect, ResourceRef, INTENT_VERSION,
};
use crate::keys::KeyRing;
use crate::packs::FakePack;
use crate::policy::StripeThresholdPolicy;
use crate::receipt::sign_receipt;
use crate::storage::Store;

pub async fn run_bench(iterations: u32) -> Result<String, crate::error::Error> {
    let dir = std::env::temp_dir().join(format!("mint-bench-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|err| crate::error::Error::internal("bench dir", err))?;
    let db = dir.join("bench.db");
    let key_path = dir.join("key.pem");
    let keys = KeyRing::generate();
    keys.write_pkcs8_pem(&key_path)?;
    let _config = Config::for_test(db.clone(), key_path);
    let store = Store::open(&db)?;
    let pack = FakePack::default();
    let policy = StripeThresholdPolicy {
        auto_cents: 5000,
        approval_cents: 50_000,
        policy_version: "stripe-refund-thresholds-v1".into(),
    };
    let intent = sample_intent();
    let mut hash_ns = 0u128;
    let mut authz_ns = 0u128;
    let mut cas_ns = 0u128;
    let mut sign_ns = 0u128;
    for _ in 0..iterations {
        let started = Instant::now();
        let (_canonical, hash) = hash_intent(&intent)?;
        hash_ns += started.elapsed().as_nanos();

        let started = Instant::now();
        let canonical = pack.canonicalize(&intent)?;
        let decision = policy.decide(&canonical)?;
        assert!(matches!(decision.effect, PolicyEffect::Automatic));
        authz_ns += started.elapsed().as_nanos();

        let mut record_intent = intent.clone();
        record_intent.action_id = Uuid::new_v4();
        record_intent.idempotency_key = record_intent.action_id.to_string();
        let record = crate::domain::ActionRecord {
            intent: record_intent.clone(),
            status: ActionStatus::Authorized,
            intent_hash: hash.clone(),
            canonical_version: canonical.canonicalization_version.clone(),
            canonical_json: canonical.canonical_json.clone(),
            policy: Some(decision.clone()),
            approval: None,
            grant: Some(ExecutionGrant {
                intent_hash: hash.clone(),
                authorized_at: Utc::now(),
                expires_at: record_intent.expires_at,
                policy_version: decision.policy_version.clone(),
            }),
            provider_result: None,
            reconciliation_required: false,
            latest_attempt: None,
            receipt: None,
        };
        store.insert_action(record).await?;
        let attempt = ExecutionAttempt {
            attempt_id: Uuid::new_v4(),
            action_id: record_intent.action_id,
            tenant_id: record_intent.tenant_id.clone(),
            attempt_no: 1,
            provider_idempotency_key: crate::domain::provider_idempotency_key(
                record_intent.action_id,
                &record_intent.operation,
            ),
            status: "started".into(),
            started_at: Utc::now(),
            completed_at: None,
        };
        let started = Instant::now();
        let claimed = store
            .claim_execution(&record_intent.tenant_id, record_intent.action_id, attempt)
            .await?;
        cas_ns += started.elapsed().as_nanos();
        assert!(claimed);

        let started = Instant::now();
        let _ = sign_receipt(
            &keys,
            crate::domain::SignedReceipt {
                format_version: crate::domain::SIGNED_OBJECT_VERSION.to_owned(),
                kid: keys.kid.clone(),
                payload: crate::domain::ReceiptPayload {
                    format_version: crate::domain::RECEIPT_VERSION.to_owned(),
                    action_id: record_intent.action_id,
                    tenant_id: record_intent.tenant_id,
                    actor: intent.actor.clone(),
                    intent_hash: hash,
                    policy: PolicyDecision {
                        effect: PolicyEffect::Automatic,
                        policy_version: policy.policy_version.clone(),
                        reason: "auto".into(),
                    },
                    approval: None,
                    attempt_id: Uuid::new_v4(),
                    provider: "fake".into(),
                    operation: "refund.create".into(),
                    provider_idempotency_key: "k".into(),
                    provider_resource_id: Some("re_bench".into()),
                    status: ActionStatus::Succeeded,
                    started_at: Utc::now(),
                    completed_at: Utc::now(),
                    reconciliation_required: false,
                    kid: keys.kid.clone(),
                },
                signature: String::new(),
            },
        )?;
        sign_ns += started.elapsed().as_nanos();
    }
    let n = iterations as f64;
    Ok(format!(
        "iterations={iterations}\ncanonicalize_hash_us={:.1}\nlocal_authorization_us={:.1}\natomic_execution_claim_us={:.1}\nreceipt_signing_us={:.1}\n",
        (hash_ns as f64 / n) / 1000.0,
        (authz_ns as f64 / n) / 1000.0,
        (cas_ns as f64 / n) / 1000.0,
        (sign_ns as f64 / n) / 1000.0,
    ))
}

fn sample_intent() -> ActionIntent {
    let now = Utc::now();
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
        provider: "fake".into(),
        operation: "refund.create".into(),
        resource: ResourceRef {
            resource_type: "charge".into(),
            resource_id: "ch_123".into(),
        },
        arguments: json!({"amount":4200,"currency":"usd","reason":"duplicate"}),
        context: ActionContext {
            support_ticket_id: Some("ticket_982".into()),
            reason: Some("duplicate".into()),
        },
        idempotency_key: "bench".into(),
        created_at: now,
        expires_at: now + Duration::seconds(300),
    }
}
