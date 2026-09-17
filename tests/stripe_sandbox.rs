//! Opt-in Stripe sandbox proof. Skipped unless credentials are present.
//!
//! Requires:
//!   MINT_STRIPE_TEST_SECRET_KEY=sk_test_...
//!   MINT_RUN_STRIPE_E2E=1
//!
//! Stripe sandbox transactions move no real funds. Default CI must not run this.

use mint_run::config::Config;
use mint_run::credentials::assert_test_secret;
use mint_run::domain::{ActionStatus, ActorIdentity};
use mint_run::identity::{encode_dev_token, AuthContext};
use mint_run::keys::KeyRing;
use mint_run::packs::{create_test_charge, retrieve_refund};
use mint_run::receipt::verify_receipt;
use mint_run::server::{build_state, run_with_listener};
use serde_json::json;
use uuid::Uuid;

fn should_run() -> bool {
    std::env::var("MINT_RUN_STRIPE_E2E").ok().as_deref() == Some("1")
        && std::env::var("MINT_STRIPE_TEST_SECRET_KEY").is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn stripe_sandbox_partial_refund_round_trip() {
    if !should_run() {
        eprintln!("skipping Stripe sandbox test (set MINT_RUN_STRIPE_E2E=1 and MINT_STRIPE_TEST_SECRET_KEY)");
        return;
    }
    let secret = std::env::var("MINT_STRIPE_TEST_SECRET_KEY").expect("secret");
    assert_test_secret(&secret).expect("test key");
    assert!(assert_test_secret("sk_live_forbidden").is_err());

    let dir = tempfile::tempdir().expect("tempdir");
    let key = dir.path().join("key.pem");
    let db = dir.path().join("mint.db");
    KeyRing::generate().write_pkcs8_pem(&key).expect("key");
    let mut config = Config::for_test(db, key);
    config.provider = mint_run::config::ProviderKind::Stripe;
    config.stripe_secret = Some(secret.clone());
    config.http_timeout = std::time::Duration::from_secs(20);
    let http = reqwest::Client::builder()
        .timeout(config.http_timeout)
        .build()
        .expect("http");
    let charge_id = create_test_charge(&http, &secret, 5000)
        .await
        .expect("sandbox charge");

    let state = build_state(config).expect("state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let spawned = state.clone();
    tokio::spawn(async move {
        let _ = run_with_listener(spawned, listener).await;
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client");
    let base = format!("http://{addr}");
    let actor = ActorIdentity {
        subject: "user_123".into(),
        agent_id: "support-agent-7".into(),
        delegated_by: None,
        issuer: "https://identity.example.com".into(),
    };
    let token = format!("Bearer {}", encode_dev_token("acme", &actor));
    let proposed: serde_json::Value = client
        .post(format!("{base}/v1/actions"))
        .header("authorization", &token)
        .json(&json!({
            "tenantId": "acme",
            "actor": {
                "subject": "user_123",
                "agentId": "support-agent-7",
                "issuer": "https://identity.example.com"
            },
            "provider": "stripe",
            "operation": "refund.create",
            "resource": { "type": "charge", "id": charge_id },
            "arguments": { "amount": 4200, "currency": "usd", "reason": "duplicate" },
            "context": { "supportTicketId": "ticket_982" }
        }))
        .send()
        .await
        .expect("propose")
        .json()
        .await
        .expect("json");
    assert_eq!(proposed["status"], "Authorized");
    let id = proposed["id"].as_str().unwrap().to_owned();
    let executed: serde_json::Value = client
        .post(format!("{base}/v1/actions/{id}/execute"))
        .header("authorization", &token)
        .json(&json!({}))
        .send()
        .await
        .expect("execute")
        .json()
        .await
        .expect("json");
    assert_eq!(executed["status"], "Succeeded");
    let refund_id = executed["providerResourceId"].as_str().unwrap().to_owned();
    let stripe_refund = retrieve_refund(&http, &secret, &refund_id)
        .await
        .expect("retrieve");
    assert_eq!(stripe_refund["amount"], 4200);
    assert_eq!(stripe_refund["metadata"]["mint_action_id"], proposed["id"]);

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/actions/{id}/execute"))
                .header("authorization", token)
                .json(&json!({}))
                .send()
                .await
                .expect("reexec")
                .json::<serde_json::Value>()
                .await
                .expect("json")
        }));
    }
    for task in tasks {
        let body = task.await.expect("join");
        assert_eq!(body["status"], "Succeeded");
        assert_eq!(body["providerResourceId"], refund_id);
    }

    let auth = AuthContext {
        tenant_id: "acme".into(),
        actor: actor.clone(),
    };
    let fail_state = {
        let dir = tempfile::tempdir().expect("tempdir2");
        let key = dir.path().join("key.pem");
        let db = dir.path().join("mint.db");
        KeyRing::generate().write_pkcs8_pem(&key).expect("key");
        let mut config = Config::for_test(db, key);
        config.provider = mint_run::config::ProviderKind::Stripe;
        config.stripe_secret = Some(secret.clone());
        config.http_timeout = std::time::Duration::from_secs(20);
        build_state(config).expect("state2")
    };
    let charge_id = create_test_charge(&http, &secret, 5000)
        .await
        .expect("charge2");
    let intent_body = mint_run::domain::ActionIntent {
        version: mint_run::domain::INTENT_VERSION.to_owned(),
        action_id: Uuid::new_v4(),
        tenant_id: "acme".into(),
        actor: actor.clone(),
        provider: "stripe".into(),
        operation: "refund.create".into(),
        resource: mint_run::domain::ResourceRef {
            resource_type: "charge".into(),
            resource_id: charge_id,
        },
        arguments: json!({"amount":4200,"currency":"usd","reason":"duplicate"}),
        context: mint_run::domain::ActionContext {
            support_ticket_id: Some("ticket_982".into()),
            reason: Some("duplicate".into()),
        },
        idempotency_key: Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
    };
    let proposed = fail_state
        .engine
        .propose(&auth, intent_body)
        .await
        .expect("propose2");
    let failed = fail_state
        .engine
        .execute(
            &auth,
            proposed.intent.action_id,
            None,
            Some(mint_run::failpoints::Failpoint::AfterProviderSuccess),
        )
        .await;
    assert!(matches!(
        failed,
        Err(mint_run::error::Error::UnknownOutcome)
    ));
    let reconciled = fail_state
        .engine
        .reconcile(&auth, proposed.intent.action_id)
        .await
        .expect("reconcile");
    assert_eq!(reconciled.status, ActionStatus::Succeeded);
    let receipt = reconciled.receipt.expect("receipt");
    verify_receipt(&receipt, &fail_state.keys).expect("verify");
    retrieve_refund(
        &http,
        &secret,
        receipt.payload.provider_resource_id.as_deref().unwrap(),
    )
    .await
    .expect("refund exists");
}
