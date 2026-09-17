//! HTTP runtime invariants for propose, authorize, execute, and reconcile.

use std::sync::Arc;

use mint_run::config::Config;
use mint_run::domain::{ActionStatus, ActorIdentity};
use mint_run::identity::encode_dev_token;
use mint_run::keys::KeyRing;
use mint_run::packs::FakeScript;
use mint_run::receipt::verify_receipt;
use mint_run::server::{build_state, run_with_listener, AppState};
use serde_json::{json, Value};
use uuid::Uuid;

struct TestEnv {
    base: String,
    state: AppState,
    dir: tempfile::TempDir,
    client: reqwest::Client,
}

impl TestEnv {
    async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = dir.path().join("key.pem");
        let db = dir.path().join("mint.db");
        KeyRing::generate().write_pkcs8_pem(&key).expect("key");
        let config = Config::for_test(db, key);
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
        let env = Self {
            base: format!("http://{addr}"),
            state,
            dir,
            client,
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

    fn token(&self, tenant: &str) -> String {
        format!("Bearer {}", encode_dev_token(tenant, &actor()))
    }

    fn token_for(&self, tenant: &str, subject: &str) -> String {
        let mut identity = actor();
        identity.subject = subject.to_owned();
        format!("Bearer {}", encode_dev_token(tenant, &identity))
    }

    async fn propose(&self, tenant: &str, body: Value) -> reqwest::Response {
        self.client
            .post(format!("{}/v1/actions", self.base))
            .header("authorization", self.token(tenant))
            .json(&body)
            .send()
            .await
            .expect("propose")
    }

    async fn execute(
        &self,
        tenant: &str,
        id: &str,
        body: Option<Value>,
        failpoint: Option<&str>,
    ) -> reqwest::Response {
        let mut req = self
            .client
            .post(format!("{}/v1/actions/{id}/execute", self.base))
            .header("authorization", self.token(tenant));
        if let Some(name) = failpoint {
            req = req.header("x-mint-failpoint", name);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.expect("execute")
    }
}

fn actor() -> ActorIdentity {
    ActorIdentity {
        subject: "user_123".into(),
        agent_id: "support-agent-7".into(),
        delegated_by: None,
        issuer: "https://identity.example.com".into(),
    }
}

fn refund_body(amount: i64) -> Value {
    json!({
        "tenantId": "acme",
        "actor": {
            "subject": "user_123",
            "agentId": "support-agent-7",
            "issuer": "https://identity.example.com"
        },
        "provider": "fake",
        "operation": "refund.create",
        "resource": { "type": "charge", "id": "ch_123" },
        "arguments": { "amount": amount, "currency": "usd", "reason": "duplicate" },
        "context": { "supportTicketId": "ticket_982", "reason": "duplicate" }
    })
}

async fn json_body(response: reqwest::Response) -> Value {
    response.json().await.expect("json")
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_authorization_cannot_execute() {
    let env = TestEnv::spawn().await;
    let mut body = refund_body(4200);
    body["ttlSeconds"] = json!(0);
    let proposed = json_body(env.propose("acme", body).await).await;
    let id = proposed["id"].as_str().expect("id");
    assert_eq!(proposed["status"], "Authorized");
    let executed = env.execute("acme", id, None, None).await;
    assert_eq!(executed.status(), reqwest::StatusCode::FORBIDDEN);
    let err = json_body(executed).await;
    assert_eq!(err["error"]["code"], "authorization_expired");
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_access_is_denied() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let other = env
        .client
        .get(format!("{}/v1/actions/{id}", env.base))
        .header("authorization", env.token("globex"))
        .send()
        .await
        .expect("get");
    assert_eq!(other.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_succeeds_with_valid_receipt() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    assert_eq!(proposed["status"], "Authorized");
    let id = proposed["id"].as_str().expect("id");
    let executed = json_body(env.execute("acme", id, Some(json!({})), None).await).await;
    assert_eq!(executed["status"], "Succeeded");
    assert!(executed["providerResourceId"]
        .as_str()
        .unwrap()
        .starts_with("re_fake_"));
    let receipt_resp = env
        .client
        .get(format!("{}/v1/actions/{id}/receipt", env.base))
        .header("authorization", env.token("acme"))
        .send()
        .await
        .expect("receipt");
    assert_eq!(receipt_resp.status(), reqwest::StatusCode::OK);
    let receipt: mint_run::domain::SignedReceipt = receipt_resp.json().await.expect("receipt json");
    verify_receipt(&receipt, &env.state.keys).expect("verify");
}

#[tokio::test(flavor = "multi_thread")]
async fn modified_arguments_after_approval_do_not_call_provider() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(12_000)).await).await;
    assert_eq!(proposed["status"], "PendingApproval");
    let id = proposed["id"].as_str().expect("id");
    let hash = proposed["intentHash"].as_str().expect("hash").to_owned();
    let approved = json_body(
        env.client
            .post(format!("{}/v1/actions/{id}/approve", env.base))
            .header("authorization", env.token_for("acme", "supervisor_1"))
            .json(&json!({ "intentHash": hash }))
            .send()
            .await
            .expect("approve"),
    )
    .await;
    assert_eq!(approved["status"], "Authorized");
    assert_eq!(approved["approval"]["subject"], "supervisor_1");
    let action_id = Uuid::parse_str(id).expect("uuid");
    env.state
        .engine
        .store
        .tamper_arguments(
            "acme",
            action_id,
            json!({"amount": 1, "currency": "usd", "reason": "duplicate"}),
        )
        .await
        .expect("tamper");
    let executed = env.execute("acme", id, None, None).await;
    assert_eq!(executed.status(), reqwest::StatusCode::CONFLICT);
    let err = json_body(executed).await;
    assert_eq!(err["error"]["code"], "intent_hash_mismatch");
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        0
    );
    assert_eq!(env.state.engine.pack.fake().expect("fake").call_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_body_amount_mismatch_is_rejected_before_provider() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let executed = env
        .execute(
            "acme",
            id,
            Some(json!({
                "arguments": {
                    "amount": 9999,
                    "currency": "usd",
                    "reason": "duplicate"
                }
            })),
            None,
        )
        .await;
    assert_eq!(executed.status(), reqwest::StatusCode::CONFLICT);
    let err = json_body(executed).await;
    assert_eq!(err["error"]["code"], "intent_hash_mismatch");
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        0
    );
    assert_eq!(env.state.engine.pack.fake().expect("fake").call_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn modified_resource_after_approval_do_not_call_provider() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    assert_eq!(proposed["status"], "Authorized");
    let id = proposed["id"].as_str().expect("id");
    let action_id = Uuid::parse_str(id).expect("uuid");
    env.state
        .engine
        .store
        .tamper_resource("acme", action_id, "ch_other")
        .await
        .expect("tamper resource");
    let executed = env.execute("acme", id, Some(json!({})), None).await;
    assert_eq!(executed.status(), reqwest::StatusCode::CONFLICT);
    let err = json_body(executed).await;
    assert_eq!(err["error"]["code"], "intent_hash_mismatch");
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        0
    );
    assert_eq!(env.state.engine.pack.fake().expect("fake").call_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_execute_results_in_one_provider_call() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = Arc::new(proposed["id"].as_str().expect("id").to_owned());
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let env_base = env.base.clone();
        let token = env.token("acme");
        let client = env.client.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{env_base}/v1/actions/{id}/execute"))
                .header("authorization", token)
                .json(&json!({}))
                .send()
                .await
                .expect("execute")
                .json::<Value>()
                .await
                .expect("json")
        }));
    }
    let mut statuses = Vec::new();
    for task in tasks {
        let body = task.await.expect("join");
        statuses.push(body["status"].as_str().unwrap().to_owned());
    }
    assert!(statuses.iter().all(|status| status == "Succeeded"));
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        1
    );
    assert_eq!(env.state.engine.pack.fake().expect("fake").call_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_does_not_restore_consumed_authorization() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id").to_owned();
    let executed = json_body(env.execute("acme", &id, Some(json!({})), None).await).await;
    assert_eq!(executed["status"], "Succeeded");
    let db = env.dir.path().join("mint.db");
    let key = env.dir.path().join("key.pem");
    let restarted = build_state(Config::for_test(db, key)).expect("reopen");
    let auth = mint_run::identity::AuthContext {
        tenant_id: "acme".into(),
        actor: actor(),
    };
    let again = restarted
        .engine
        .execute(&auth, Uuid::parse_str(&id).expect("uuid"), None, None)
        .await
        .expect("replay");
    assert_eq!(again.status, ActionStatus::Succeeded);
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_rejection_becomes_failed() {
    let env = TestEnv::spawn().await;
    env.state
        .engine
        .pack
        .fake()
        .expect("fake")
        .set_next(FakeScript::Reject)
        .expect("script");
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let executed = json_body(env.execute("acme", id, Some(json!({})), None).await).await;
    assert_eq!(executed["status"], "Failed");
    let receipt_resp = env
        .client
        .get(format!("{}/v1/actions/{id}/receipt", env.base))
        .header("authorization", env.token("acme"))
        .send()
        .await
        .expect("receipt");
    assert_eq!(receipt_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_timeout_becomes_unknown_without_retry() {
    let env = TestEnv::spawn().await;
    env.state
        .engine
        .pack
        .fake()
        .expect("fake")
        .set_next(FakeScript::Timeout)
        .expect("script");
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let executed = env.execute("acme", id, Some(json!({})), None).await;
    assert_eq!(executed.status(), reqwest::StatusCode::CONFLICT);
    let err = json_body(executed).await;
    assert_eq!(err["error"]["code"], "unknown_outcome");
    let current = env
        .client
        .get(format!("{}/v1/actions/{id}", env.base))
        .header("authorization", env.token("acme"))
        .send()
        .await
        .expect("get");
    let body = json_body(current).await;
    assert_eq!(body["status"], "Unknown");
    assert_eq!(env.state.engine.pack.fake().expect("fake").call_count(), 1);
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_after_provider_success_is_reconciled_to_one_effect() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let executed = env
        .execute("acme", id, Some(json!({})), Some("after_provider_success"))
        .await;
    assert_eq!(executed.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        1
    );
    let reconciled = env
        .client
        .post(format!("{}/v1/actions/{id}/reconcile", env.base))
        .header("authorization", env.token("acme"))
        .json(&json!({}))
        .send()
        .await
        .expect("reconcile");
    let body = json_body(reconciled).await;
    assert_eq!(body["status"], "Succeeded");
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        1
    );
    let refund_id = body["providerResourceId"].as_str().unwrap().to_owned();
    let receipt: mint_run::domain::SignedReceipt = env
        .client
        .get(format!("{}/v1/actions/{id}/receipt", env.base))
        .header("authorization", env.token("acme"))
        .send()
        .await
        .expect("receipt")
        .json()
        .await
        .expect("json");
    assert_eq!(
        receipt.payload.provider_resource_id.as_deref(),
        Some(refund_id.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn later_caller_receives_terminal_result() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(4200)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let first = json_body(env.execute("acme", id, Some(json!({})), None).await).await;
    assert_eq!(first["status"], "Succeeded");
    let second = json_body(env.execute("acme", id, Some(json!({})), None).await).await;
    assert_eq!(second["status"], "Succeeded");
    assert_eq!(first["providerResourceId"], second["providerResourceId"]);
    assert_eq!(
        env.state.engine.pack.fake().expect("fake").effect_count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn health_does_not_leak_configuration() {
    let env = TestEnv::spawn().await;
    let response = env
        .client
        .get(format!("{}/health", env.base))
        .send()
        .await
        .expect("health");
    let body = json_body(response).await;
    assert_eq!(body, json!({"status":"ok"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn self_approval_is_rejected_separate_principal_can_approve() {
    let env = TestEnv::spawn().await;
    let proposed = json_body(env.propose("acme", refund_body(12_000)).await).await;
    let id = proposed["id"].as_str().expect("id");
    let hash = proposed["intentHash"].as_str().expect("hash");
    let self_approve = env
        .client
        .post(format!("{}/v1/actions/{id}/approve", env.base))
        .header("authorization", env.token("acme"))
        .json(&json!({ "intentHash": hash }))
        .send()
        .await
        .expect("self approve");
    assert_eq!(self_approve.status(), reqwest::StatusCode::FORBIDDEN);
    let err = json_body(self_approve).await;
    assert_eq!(err["error"]["code"], "self_approval");

    let approved = json_body(
        env.client
            .post(format!("{}/v1/actions/{id}/approve", env.base))
            .header("authorization", env.token_for("acme", "supervisor_1"))
            .json(&json!({ "intentHash": hash }))
            .send()
            .await
            .expect("approve"),
    )
    .await;
    assert_eq!(approved["status"], "Authorized");
    assert_eq!(approved["approval"]["subject"], "supervisor_1");
    assert_eq!(approved["approval"]["decision"], "approved");
}
