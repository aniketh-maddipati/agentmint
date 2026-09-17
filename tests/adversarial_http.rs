//! Adversarial HTTP simulations against the fake provider (no Stripe, no network).
//! Covers exact-intent mutation, concurrency, lost-response, approval misuse,
//! bad inputs, tenant isolation, and receipt tampering.

use std::sync::Arc;

use mint_run::config::Config;
use mint_run::domain::{ActionStatus, ActorIdentity, SignedReceipt};
use mint_run::identity::encode_dev_token;
use mint_run::keys::KeyRing;
use mint_run::receipt::verify_receipt;
use mint_run::server::{build_state, run_with_listener, AppState};
use serde_json::{json, Value};
use uuid::Uuid;

struct Env {
    base: String,
    state: AppState,
    dir: tempfile::TempDir,
    client: reqwest::Client,
}

impl Env {
    async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = dir.path().join("key.pem");
        let db = dir.path().join("mint.db");
        KeyRing::generate().write_pkcs8_pem(&key).expect("key");
        let state = build_state(Config::for_test(db, key)).expect("state");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let spawned = state.clone();
        tokio::spawn(async move {
            let _ = run_with_listener(spawned, listener).await;
        });
        let env = Self {
            base: format!("http://{addr}"),
            state,
            dir,
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("client"),
        };
        for _ in 0..50 {
            if env
                .client
                .get(format!("{}/health", env.base))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        env
    }

    fn token(&self, tenant: &str, subject: &str) -> String {
        let actor = ActorIdentity {
            subject: subject.into(),
            agent_id: "support-agent-7".into(),
            delegated_by: None,
            issuer: "https://identity.example.com".into(),
        };
        format!("Bearer {}", encode_dev_token(tenant, &actor))
    }

    fn effects(&self) -> u64 {
        self.state.engine.pack.fake().expect("fake").effect_count()
    }

    fn calls(&self) -> u64 {
        self.state.engine.pack.fake().expect("fake").call_count()
    }

    async fn propose(
        &self,
        tenant: &str,
        subject: &str,
        body: Value,
    ) -> (reqwest::StatusCode, Value) {
        let response = self
            .client
            .post(format!("{}/v1/actions", self.base))
            .header("authorization", self.token(tenant, subject))
            .json(&body)
            .send()
            .await
            .expect("propose");
        let status = response.status();
        let body = response.json().await.expect("json");
        (status, body)
    }

    async fn execute(
        &self,
        tenant: &str,
        subject: &str,
        id: &str,
        body: Option<Value>,
        failpoint: Option<&str>,
    ) -> (reqwest::StatusCode, Value) {
        let mut req = self
            .client
            .post(format!("{}/v1/actions/{id}/execute", self.base))
            .header("authorization", self.token(tenant, subject));
        if let Some(name) = failpoint {
            req = req.header("x-mint-failpoint", name);
        }
        if let Some(body) = body {
            req = req.json(&body);
        } else {
            req = req.json(&json!({}));
        }
        let response = req.send().await.expect("execute");
        let status = response.status();
        let body = response.json().await.expect("json");
        (status, body)
    }

    async fn get(&self, tenant: &str, subject: &str, id: &str) -> (reqwest::StatusCode, Value) {
        let response = self
            .client
            .get(format!("{}/v1/actions/{id}", self.base))
            .header("authorization", self.token(tenant, subject))
            .send()
            .await
            .expect("get");
        let status = response.status();
        let body = response.json().await.expect("json");
        (status, body)
    }
}

fn refund_for(tenant: &str, subject: &str, amount: i64, charge: &str) -> Value {
    json!({
        "tenantId": tenant,
        "actor": {
            "subject": subject,
            "agentId": "support-agent-7",
            "issuer": "https://identity.example.com"
        },
        "provider": "fake",
        "operation": "refund.create",
        "resource": { "type": "charge", "id": charge },
        "arguments": { "amount": amount, "currency": "usd", "reason": "duplicate" },
        "context": { "supportTicketId": "ticket_982", "reason": "duplicate" }
    })
}

fn refund(amount: i64, charge: &str) -> Value {
    refund_for("acme", "user_123", amount, charge)
}

fn assert_client_safe(err: &Value) {
    let code = err["error"]["code"].as_str().expect("code");
    let message = err["error"]["message"].as_str().expect("message");
    assert!(!code.is_empty());
    assert!(!message.to_lowercase().contains("sk_"));
    assert!(!message.to_lowercase().contains("panic"));
    assert!(!message.contains("sql"));
}

#[tokio::test(flavor = "multi_thread")]
async fn exact_action_mutations_rejected_before_provider() {
    let env = Env::spawn().await;
    let (_, proposed) = env
        .propose("acme", "user_123", refund(4217, "ch_test_original"))
        .await;
    assert_eq!(proposed["status"], "Authorized");
    let id = proposed["id"].as_str().unwrap();
    let original_hash = proposed["intentHash"].clone();

    let mutations = [
        json!({"arguments":{"amount":4218,"currency":"usd","reason":"duplicate"}}),
        json!({"arguments":{"amount":4217,"currency":"eur","reason":"duplicate"}}),
        json!({"arguments":{"amount":4217,"currency":"usd","reason":"fraudulent"}}),
    ];
    for body in mutations {
        let (status, err) = env.execute("acme", "user_123", id, Some(body), None).await;
        assert!(
            status == reqwest::StatusCode::CONFLICT || status == reqwest::StatusCode::BAD_REQUEST,
            "unexpected {status} {err}"
        );
        let code = err["error"]["code"].as_str().unwrap();
        assert!(
            code == "intent_hash_mismatch" || code == "invalid_request",
            "unexpected code {code}"
        );
        assert_client_safe(&err);
        assert_eq!(env.effects(), 0);
        assert_eq!(env.calls(), 0);
        let receipt = env
            .client
            .get(format!("{}/v1/actions/{id}/receipt", env.base))
            .header("authorization", env.token("acme", "user_123"))
            .send()
            .await
            .expect("receipt");
        assert_eq!(receipt.status(), reqwest::StatusCode::NOT_FOUND);
    }

    let action_id = Uuid::parse_str(id).unwrap();
    env.state
        .engine
        .store
        .tamper_resource("acme", action_id, "ch_test_other")
        .await
        .expect("tamper");
    let (status, err) = env
        .execute("acme", "user_123", id, Some(json!({})), None)
        .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert_eq!(err["error"]["code"], "intent_hash_mismatch");
    assert_eq!(env.effects(), 0);

    let (ok_status, current) = env.get("acme", "user_123", id).await;
    assert_eq!(ok_status, reqwest::StatusCode::OK);
    assert_eq!(current["intentHash"], original_hash);
    assert_eq!(current["status"], "Authorized");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_http_executions_converge_on_one_effect() {
    let env = Env::spawn().await;
    let (_, proposed) = env
        .propose("acme", "user_123", refund(4217, "ch_test_original"))
        .await;
    let id = Arc::new(proposed["id"].as_str().unwrap().to_owned());
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let base = env.base.clone();
        let token = env.token("acme", "user_123");
        let client = env.client.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/actions/{id}/execute"))
                .header("authorization", token)
                .json(&json!({}))
                .send()
                .await
                .expect("exec")
                .json::<Value>()
                .await
                .expect("json")
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        let body = task.await.expect("join");
        assert_eq!(body["status"], "Succeeded");
        ids.push(body["providerResourceId"].as_str().unwrap().to_owned());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 1, "all callers must share one provider result");
    assert_eq!(env.effects(), 1);
    assert_eq!(env.calls(), 1);
    let (status, again) = env
        .execute("acme", "user_123", &id, Some(json!({})), None)
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(again["providerResourceId"], ids[0]);
    assert_eq!(env.effects(), 1);
    eprintln!(
        "32 callers\n1 execution winner\n1 provider effect\n1 provider result ID {}",
        ids[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn simulated_lost_provider_response_reconciles_without_second_effect() {
    let env = Env::spawn().await;
    let (_, proposed) = env
        .propose("acme", "user_123", refund(4217, "ch_test_original"))
        .await;
    let id = proposed["id"].as_str().unwrap();
    let (status, err) = env
        .execute(
            "acme",
            "user_123",
            id,
            Some(json!({})),
            Some("after_provider_success"),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert_eq!(err["error"]["code"], "unknown_outcome");
    assert_eq!(env.effects(), 1);
    let (_, current) = env.get("acme", "user_123", id).await;
    assert_eq!(current["status"], "Unknown");

    let (exec_status, exec_err) = env
        .execute("acme", "user_123", id, Some(json!({})), None)
        .await;
    assert_eq!(exec_status, reqwest::StatusCode::CONFLICT);
    assert_eq!(exec_err["error"]["code"], "reconciliation_required");
    assert_eq!(env.effects(), 1);

    let reconciled = env
        .client
        .post(format!("{}/v1/actions/{id}/reconcile", env.base))
        .header("authorization", env.token("acme", "user_123"))
        .json(&json!({}))
        .send()
        .await
        .expect("reconcile");
    let body: Value = reconciled.json().await.expect("json");
    assert_eq!(body["status"], "Succeeded");
    let provider_id = body["providerResourceId"].as_str().unwrap().to_owned();
    assert_eq!(env.effects(), 1);

    let receipt: SignedReceipt = env
        .client
        .get(format!("{}/v1/actions/{id}/receipt", env.base))
        .header("authorization", env.token("acme", "user_123"))
        .send()
        .await
        .expect("receipt")
        .json()
        .await
        .expect("json");
    verify_receipt(&receipt, &env.state.keys).expect("verify");
    assert_eq!(
        receipt.payload.provider_resource_id.as_deref(),
        Some(provider_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_misuse_and_denial_fail_closed() {
    let env = Env::spawn().await;
    let mut body = refund(12_000, "ch_test_original");
    let (_, proposed) = env.propose("acme", "user_123", body.clone()).await;
    assert_eq!(proposed["status"], "PendingApproval");
    let id = proposed["id"].as_str().unwrap();
    let hash = proposed["intentHash"].as_str().unwrap();

    let self_approve = env
        .client
        .post(format!("{}/v1/actions/{id}/approve", env.base))
        .header("authorization", env.token("acme", "user_123"))
        .json(&json!({ "intentHash": hash }))
        .send()
        .await
        .expect("self");
    assert_eq!(self_approve.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        self_approve.json::<Value>().await.unwrap()["error"]["code"],
        "self_approval"
    );

    let cross = env
        .client
        .post(format!("{}/v1/actions/{id}/approve", env.base))
        .header("authorization", env.token("globex", "other"))
        .json(&json!({ "intentHash": hash }))
        .send()
        .await
        .expect("cross");
    assert_eq!(cross.status(), reqwest::StatusCode::NOT_FOUND);

    let unauth = env
        .client
        .post(format!("{}/v1/actions/{id}/approve", env.base))
        .json(&json!({ "intentHash": hash }))
        .send()
        .await
        .expect("unauth");
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let approved = env
        .client
        .post(format!("{}/v1/actions/{id}/approve", env.base))
        .header("authorization", env.token("acme", "supervisor_1"))
        .json(&json!({ "intentHash": hash }))
        .send()
        .await
        .expect("approve");
    assert_eq!(approved.status(), reqwest::StatusCode::OK);
    assert_eq!(
        approved.json::<Value>().await.unwrap()["approval"]["subject"],
        "supervisor_1"
    );
    assert_eq!(env.effects(), 0);

    body["ttlSeconds"] = json!(0);
    let (exp_status, expired) = env.propose("acme", "user_123", body).await;
    assert_eq!(
        exp_status,
        reqwest::StatusCode::OK,
        "expired propose {expired}"
    );
    let expired_id = expired["id"].as_str().unwrap();
    let expired_hash = expired["intentHash"].as_str().unwrap();
    assert_eq!(expired["status"], "PendingApproval");
    let expired_approve = env
        .client
        .post(format!("{}/v1/actions/{expired_id}/approve", env.base))
        .header("authorization", env.token("acme", "supervisor_1"))
        .json(&json!({ "intentHash": expired_hash }))
        .send()
        .await
        .expect("expired approve");
    assert_eq!(
        expired_approve.status(),
        reqwest::StatusCode::FORBIDDEN,
        "expired approve body={}",
        expired_approve.text().await.unwrap_or_default()
    );
    let (exec_status, exec_err) = env
        .execute("acme", "user_123", expired_id, Some(json!({})), None)
        .await;
    assert!(
        exec_status == reqwest::StatusCode::FORBIDDEN
            || exec_status == reqwest::StatusCode::CONFLICT,
        "expired execute {exec_status} {exec_err}"
    );
    assert_eq!(env.effects(), 0);

    let (_, deny_prop) = env
        .propose("acme", "user_123", refund(12_000, "ch_deny"))
        .await;
    let deny_id = deny_prop["id"].as_str().unwrap();
    let deny_hash = deny_prop["intentHash"].as_str().unwrap();
    let denied = env
        .client
        .post(format!("{}/v1/actions/{deny_id}/deny", env.base))
        .header("authorization", env.token("acme", "supervisor_1"))
        .json(&json!({ "intentHash": deny_hash }))
        .send()
        .await
        .expect("deny");
    assert_eq!(denied.status(), reqwest::StatusCode::OK);
    let (deny_exec_status, deny_err) = env
        .execute("acme", "user_123", deny_id, Some(json!({})), None)
        .await;
    assert!(
        deny_exec_status == reqwest::StatusCode::CONFLICT
            || deny_exec_status == reqwest::StatusCode::OK
    );
    if deny_exec_status.is_success() {
        assert_eq!(deny_err["status"], "Denied");
    } else {
        assert_eq!(deny_err["error"]["code"], "not_executable");
    }
    assert_eq!(env.effects(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_refund_inputs_fail_closed_without_provider_effect() {
    let env = Env::spawn().await;
    let cases = [
        json!({"amount":0,"currency":"usd","reason":"duplicate"}),
        json!({"amount":-5,"currency":"usd","reason":"duplicate"}),
        json!({"amount":"4217","currency":"usd","reason":"duplicate"}),
        json!({"amount":1.5,"currency":"usd","reason":"duplicate"}),
        json!({"amount":4217,"currency":"eur","reason":"duplicate"}),
        json!({"amount":4217,"currency":"usd","reason":"nope"}),
        json!({"amount":4217,"currency":"usd","reason":"duplicate","metadata":{"x":1}}),
    ];
    for arguments in cases {
        let mut body = refund(4217, "ch_test_original");
        body["arguments"] = arguments;
        let (status, err) = env.propose("acme", "user_123", body).await;
        assert!(
            status.is_client_error(),
            "expected client error, got {status} {err}"
        );
        assert_client_safe(&err);
        assert_eq!(env.effects(), 0);
        assert_eq!(env.calls(), 0);
    }

    let mut expired = refund(4217, "ch_test_original");
    expired["ttlSeconds"] = json!(0);
    let (_, proposed) = env.propose("acme", "user_123", expired).await;
    let id = proposed["id"].as_str().unwrap();
    let (status, err) = env
        .execute("acme", "user_123", id, Some(json!({})), None)
        .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert_eq!(err["error"]["code"], "authorization_expired");
    assert_eq!(env.effects(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_isolation_is_non_disclosing() {
    let env = Env::spawn().await;
    let (_, proposed) = env
        .propose("acme", "user_a", refund_for("acme", "user_a", 4217, "ch_a"))
        .await;
    let id = proposed["id"].as_str().expect("id");
    let hash = proposed["intentHash"].as_str().expect("hash");
    assert_eq!(proposed["status"], "Authorized");
    assert_eq!(env.effects(), 0);

    for (method, suffix) in [
        ("GET", ""),
        ("POST", "/approve"),
        ("POST", "/deny"),
        ("POST", "/execute"),
        ("POST", "/reconcile"),
        ("GET", "/receipt"),
    ] {
        let url = format!("{}/v1/actions/{id}{suffix}", env.base);
        let builder = if method == "GET" {
            env.client.get(url)
        } else {
            env.client.post(url).json(&json!({ "intentHash": hash }))
        };
        let response = builder
            .header("authorization", env.token("globex", "user_b"))
            .send()
            .await
            .expect("cross");
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "method={method} suffix={suffix}"
        );
    }
    assert_eq!(env.effects(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn receipt_tampering_fails_verification_original_survives_reopen() {
    let env = Env::spawn().await;
    let (_, proposed) = env
        .propose("acme", "user_123", refund(4217, "ch_test_original"))
        .await;
    let id = proposed["id"].as_str().unwrap();
    let (_, executed) = env
        .execute("acme", "user_123", id, Some(json!({})), None)
        .await;
    assert_eq!(executed["status"], "Succeeded");
    let receipt: SignedReceipt = env
        .client
        .get(format!("{}/v1/actions/{id}/receipt", env.base))
        .header("authorization", env.token("acme", "user_123"))
        .send()
        .await
        .expect("receipt")
        .json()
        .await
        .expect("json");
    verify_receipt(&receipt, &env.state.keys).expect("original");

    let mut action_id = receipt.clone();
    action_id.payload.action_id = Uuid::new_v4();
    assert!(verify_receipt(&action_id, &env.state.keys).is_err());

    let mut intent_hash = receipt.clone();
    intent_hash.payload.intent_hash = "sha256:deadbeef".into();
    assert!(verify_receipt(&intent_hash, &env.state.keys).is_err());

    let mut operation = receipt.clone();
    operation.payload.operation = "refund.destroy".into();
    assert!(verify_receipt(&operation, &env.state.keys).is_err());

    let mut provider = receipt.clone();
    provider.payload.provider_resource_id = Some("re_evil".into());
    assert!(verify_receipt(&provider, &env.state.keys).is_err());

    let mut status = receipt.clone();
    status.payload.status = ActionStatus::Failed;
    assert!(verify_receipt(&status, &env.state.keys).is_err());

    let mut kid = receipt.clone();
    kid.kid = "evil-kid".into();
    kid.payload.kid = "evil-kid".into();
    assert!(verify_receipt(&kid, &env.state.keys).is_err());

    let mut signature = receipt.clone();
    signature.signature.push('A');
    assert!(verify_receipt(&signature, &env.state.keys).is_err());

    let db = env.dir.path().join("mint.db");
    let key = env.dir.path().join("key.pem");
    let restarted = build_state(Config::for_test(db, key)).expect("reopen");
    verify_receipt(&receipt, &restarted.keys).expect("after reopen");
}
