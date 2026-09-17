//! Opt-in Stripe sandbox proof. Ignored by default so a skip is never a silent pass.
//!
//! Run only with:
//!   MINT_STRIPE_TEST_SECRET_KEY=sk_test_...
//!   MINT_RUN_STRIPE_E2E=1
//!   cargo test --test stripe_sandbox -- --ignored --nocapture
//!
//! Stripe sandbox transactions move no real funds.

use mint_run::config::Config;
use mint_run::credentials::assert_test_secret;
use mint_run::domain::{ActionStatus, ActorIdentity};
use mint_run::identity::{encode_dev_token, AuthContext};
use mint_run::keys::KeyRing;
use mint_run::packs::{create_test_charge, retrieve_charge, retrieve_refund, StripePack};
use mint_run::receipt::verify_receipt;
use mint_run::server::{build_state, run_with_listener};
use serde_json::json;
use uuid::Uuid;

fn require_opt_in() {
    assert_eq!(
        std::env::var("MINT_RUN_STRIPE_E2E").ok().as_deref(),
        Some("1"),
        "SKIPPED — set MINT_RUN_STRIPE_E2E=1 to run Stripe sandbox"
    );
    assert!(
        std::env::var("MINT_STRIPE_TEST_SECRET_KEY").is_ok(),
        "SKIPPED — credential not provided"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Stripe sandbox: requires MINT_RUN_STRIPE_E2E=1 and MINT_STRIPE_TEST_SECRET_KEY"]
async fn stripe_sandbox_partial_refund_round_trip() {
    require_opt_in();
    let secret = std::env::var("MINT_STRIPE_TEST_SECRET_KEY").expect("secret");
    assert_test_secret(&secret).expect("test key required");
    assert!(
        assert_test_secret("sk_live_forbidden").is_err(),
        "live keys must be refused"
    );
    eprintln!("STRIPE_SANDBOX: starting against test-mode Stripe");

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
    let distinctive_amount = 4_217_i64;
    let charge_id = create_test_charge(&http, &secret, 10_000)
        .await
        .expect("sandbox charge");
    eprintln!("STRIPE_SANDBOX: created charge {charge_id}");

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
    for _ in 0..50 {
        if client
            .get(format!("{base}/health"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

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
            "arguments": { "amount": distinctive_amount, "currency": "usd", "reason": "duplicate" },
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
    eprintln!("STRIPE_SANDBOX: proposed action {id}");

    let mut tasks = Vec::new();
    for _ in 0..16 {
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
                .expect("exec")
                .json::<serde_json::Value>()
                .await
                .expect("json")
        }));
    }
    let mut refund_ids = Vec::new();
    for task in tasks {
        let body = task.await.expect("join");
        assert_eq!(body["status"], "Succeeded", "body={body}");
        refund_ids.push(body["providerResourceId"].as_str().unwrap().to_owned());
    }
    refund_ids.sort();
    refund_ids.dedup();
    assert_eq!(
        refund_ids.len(),
        1,
        "concurrent callers must converge on one refund id"
    );
    let refund_id = refund_ids[0].clone();
    eprintln!("STRIPE_SANDBOX: concurrent execute converged on {refund_id}");

    let stripe_refund = retrieve_refund(&http, &secret, &refund_id)
        .await
        .expect("retrieve");
    // Stripe Refund objects do not expose `livemode`; test mode is on the Charge.
    // See https://docs.stripe.com/api/refunds/object and
    // https://docs.stripe.com/api/charges/object
    // Do not assert refund["livemode"] — it is Null and is not a Stripe Refund field.
    let refund_charge_id = stripe_refund["charge"]
        .as_str()
        .expect("refund.charge must reference the source Charge");
    assert_eq!(
        refund_charge_id, charge_id,
        "refund.charge must match the charge Mint refunded"
    );
    eprintln!(
        "STRIPE_SANDBOX: asserting Charge.livemode via retrieve_charge (not Refund.livemode)"
    );
    let stripe_charge = retrieve_charge(&http, &secret, refund_charge_id)
        .await
        .expect("retrieve charge for livemode check");
    let charge_livemode = stripe_charge
        .get("livemode")
        .and_then(|v| v.as_bool())
        .expect("Charge.livemode must be present on Stripe Charge objects");
    assert!(
        !charge_livemode,
        "Charge.livemode must be false (test mode); got true — refuse live mode"
    );
    assert_eq!(stripe_refund["amount"], distinctive_amount);
    assert_eq!(stripe_refund["currency"], "usd");
    assert_eq!(stripe_refund["metadata"]["mint_action_id"], proposed["id"]);

    let pack = StripePack::new(http.clone());
    let credential = mint_run::credentials::ProviderCredential {
        secret: secret.clone(),
    };
    let canonical = pack
        .canonicalize(&mint_run::domain::ActionIntent {
            version: mint_run::domain::INTENT_VERSION.to_owned(),
            action_id: Uuid::parse_str(&id).unwrap(),
            tenant_id: "acme".into(),
            actor: actor.clone(),
            provider: "stripe".into(),
            operation: "refund.create".into(),
            resource: mint_run::domain::ResourceRef {
                resource_type: "charge".into(),
                resource_id: charge_id.clone(),
            },
            arguments: json!({"amount": distinctive_amount, "currency": "usd", "reason": "duplicate"}),
            context: mint_run::domain::ActionContext {
                support_ticket_id: Some("ticket_982".into()),
                reason: Some("duplicate".into()),
            },
            idempotency_key: id.clone(),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        })
        .expect("canon");
    let listed = pack
        .list_refunds_for_action(&canonical, &credential)
        .await
        .expect("list");
    assert_eq!(
        listed.len(),
        1,
        "exactly one Stripe refund must exist for this Mint action"
    );

    let again: serde_json::Value = client
        .post(format!("{base}/v1/actions/{id}/execute"))
        .header("authorization", &token)
        .json(&json!({}))
        .send()
        .await
        .expect("reexec")
        .json()
        .await
        .expect("json");
    assert_eq!(again["status"], "Succeeded");
    assert_eq!(again["providerResourceId"], refund_id);

    let auth = AuthContext {
        tenant_id: "acme".into(),
        actor: actor.clone(),
    };
    let fail_dir = tempfile::tempdir().expect("tempdir2");
    let fail_key = fail_dir.path().join("key.pem");
    let fail_db = fail_dir.path().join("mint.db");
    KeyRing::generate().write_pkcs8_pem(&fail_key).expect("key");
    let mut fail_config = Config::for_test(fail_db, fail_key);
    fail_config.provider = mint_run::config::ProviderKind::Stripe;
    fail_config.stripe_secret = Some(secret.clone());
    fail_config.http_timeout = std::time::Duration::from_secs(20);
    let fail_state = build_state(fail_config).expect("state2");
    let charge2 = create_test_charge(&http, &secret, 10_000)
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
            resource_id: charge2,
        },
        arguments: json!({"amount": distinctive_amount, "currency": "usd", "reason": "duplicate"}),
        context: mint_run::domain::ActionContext {
            support_ticket_id: Some("ticket_982".into()),
            reason: Some("duplicate".into()),
        },
        idempotency_key: Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
    };
    let proposed2 = fail_state
        .engine
        .propose(&auth, intent_body)
        .await
        .expect("propose2");
    let failed = fail_state
        .engine
        .execute(
            &auth,
            proposed2.intent.action_id,
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
        .reconcile(&auth, proposed2.intent.action_id)
        .await
        .expect("reconcile");
    assert_eq!(reconciled.status, ActionStatus::Succeeded);
    let receipt = reconciled.receipt.expect("receipt");
    verify_receipt(&receipt, &fail_state.keys).expect("verify");
    let reconciled_id = receipt
        .payload
        .provider_resource_id
        .as_deref()
        .expect("refund id");
    retrieve_refund(&http, &secret, reconciled_id)
        .await
        .expect("refund exists");
    let pack2 = StripePack::new(http.clone());
    let canonical2 = pack2.canonicalize(&proposed2.intent).expect("canon2");
    let rows = pack2
        .list_refunds_for_action(&canonical2, &credential)
        .await
        .expect("list after");
    assert_eq!(rows.len(), 1, "reconcile must not create a second refund");
    assert_eq!(rows[0]["id"], reconciled_id);
    eprintln!("STRIPE_SANDBOX: PASS — one refund, reconcile recovered same id, receipt valid");
}
