//! Loopback HTTP: JSON-RPC MCP for agents and REST for buyers/UI.
//! Used by: `mint lab mcp-http`. Binds 127.0.0.1 only. Bearer `MINT_LAB_MCP_TOKEN`.
//! Does not add routes to mint refund `/v1`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::lab::error::{LabError, LabResult};
use crate::lab::mcp::{
    dispatch_bv_tool, handle_rpc, inspect_bound_run, map_tool_error, token_matches,
    trace_bound_run, JsonRpcRequest, McpCallContext, McpState,
};
use crate::lab::schema::openapi_document;

#[derive(Clone)]
struct HttpState {
    mcp: Arc<McpState>,
}

pub async fn serve_http(state: McpState, addr: SocketAddr) -> LabResult<()> {
    ensure_loopback(addr)?;
    let app = lab_http_router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| LabError::Io(format!("bind {addr}: {err}")))?;
    eprintln!("mint-lab-pa-bv HTTP MCP listening on http://{addr}/mcp");
    eprintln!("mint-lab-pa-bv REST listening on http://{addr}/lab");
    axum::serve(listener, app)
        .await
        .map_err(|err| LabError::Io(format!("mcp http: {err}")))?;
    Ok(())
}

pub(crate) fn ensure_loopback(addr: SocketAddr) -> LabResult<()> {
    if !addr.ip().is_loopback() {
        return Err(LabError::Invalid(
            "MCP HTTP must bind 127.0.0.1 (loopback only)".into(),
        ));
    }
    Ok(())
}

pub(crate) fn lab_http_router(mcp: Arc<McpState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/mcp", post(rpc))
        .route("/lab/openapi.json", get(openapi))
        .route("/lab/bv/{tool}", post(rest_tool))
        .route("/lab/runs/{run_id}/inspect", get(rest_inspect))
        .route("/lab/runs/{run_id}/trace", get(rest_trace))
        .with_state(HttpState { mcp })
}

async fn health() -> &'static str {
    "ok"
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn unauthorized_json() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized", "message": "unauthorized"})),
    )
}

fn require_bearer(state: &HttpState, headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    if token_matches(bearer(headers), &state.mcp.token) {
        Ok(())
    } else {
        Err(unauthorized_json())
    }
}

fn rest_err(err: LabError) -> (StatusCode, Json<Value>) {
    let (code, message, _) = map_tool_error(&err);
    let status = match (&err, code) {
        (_, "unauthorized") => StatusCode::UNAUTHORIZED,
        (LabError::NotFound(_), _) => StatusCode::NOT_FOUND,
        (LabError::Conflict(_), _) | (LabError::Cancelled, _) => StatusCode::CONFLICT,
        (LabError::Invalid(_), _) => StatusCode::BAD_REQUEST,
        (LabError::Unverified(_), _) => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({"error": code, "message": message})))
}

fn json_ok(body: Value) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(body))
}

async fn rpc(State(state): State<HttpState>, headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    let authorized = token_matches(bearer(&headers), &state.mcp.token);
    if !authorized {
        let resp = json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32001, "message": "unauthorized", "data": {"error": "unauthorized"} }
        });
        return (StatusCode::UNAUTHORIZED, Json(resp));
    }
    let req: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(err) => {
            let resp = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {err}") }
            });
            return (StatusCode::BAD_REQUEST, Json(resp));
        }
    };
    let resp = handle_rpc(&state.mcp, req, &McpCallContext { authorized: true });
    match serde_json::to_value(resp) {
        Ok(value) => json_ok(value),
        Err(err) => rest_err(LabError::Serialization(err.to_string())),
    }
}

async fn openapi(State(state): State<HttpState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(resp) = require_bearer(&state, &headers) {
        return resp;
    }
    json_ok(openapi_document())
}

async fn rest_tool(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(tool): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(resp) = require_bearer(&state, &headers) {
        return resp;
    }
    let args: Value = match serde_json::from_slice(&body) {
        Ok(args) => args,
        Err(err) => {
            return rest_err(LabError::Invalid(format!("invalid_schema: {err}")));
        }
    };
    match dispatch_bv_tool(&state.mcp, &tool, &args) {
        Ok(body) => json_ok(body),
        Err(err) => rest_err(err),
    }
}

async fn rest_inspect(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(resp) = require_bearer(&state, &headers) {
        return resp;
    }
    match inspect_bound_run(&state.mcp, run_id) {
        Ok(report) => match serde_json::to_value(report) {
            Ok(body) => json_ok(body),
            Err(err) => rest_err(LabError::Serialization(err.to_string())),
        },
        Err(err) => rest_err(err),
    }
}

async fn rest_trace(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(resp) = require_bearer(&state, &headers) {
        return resp;
    }
    match trace_bound_run(&state.mcp, run_id) {
        Ok(trace) => match serde_json::to_value(trace) {
            Ok(body) => json_ok(body),
            Err(err) => rest_err(LabError::Serialization(err.to_string())),
        },
        Err(err) => rest_err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::domain::CaseStage;
    use crate::lab::mcp::test_mcp_state;
    use crate::lab::tools::{TOOL_ASK_PAYER, TOOL_READ_ASSIGNED_CONTEXT, TOOL_REPORT_OBSERVATIONS};

    async fn spawn_server(state: McpState) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        let app = lab_http_router(Arc::new(state));
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (format!("http://{addr}"), handle)
    }

    async fn json_request(
        method: reqwest::Method,
        url: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let http = reqwest::Client::new();
        let mut req = http.request(method, url);
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.expect("http");
        let status = StatusCode::from_u16(response.status().as_u16()).expect("status");
        let payload = response.json::<Value>().await.unwrap_or(Value::Null);
        (status, payload)
    }

    #[test]
    fn refuse_non_loopback_bind() {
        let addr: SocketAddr = "0.0.0.0:8787".parse().expect("addr");
        let err = ensure_loopback(addr).expect_err("non-loopback");
        assert!(matches!(err, LabError::Invalid(_)));
        let loopback: SocketAddr = "127.0.0.1:8787".parse().expect("addr");
        ensure_loopback(loopback).expect("loopback ok");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rest_and_mcp_share_read_and_ask_payer_bodies() {
        let (_dir, state) = test_mcp_state("unclear_bv", true, true);
        let token = state.token.clone();
        let run_id = state.run_id;
        let task_id = state.task_id;
        let (base, handle) = spawn_server(state).await;

        let ids = json!({ "run_id": run_id, "task_id": task_id });
        let (rest_status, rest_ctx) = json_request(
            reqwest::Method::POST,
            &format!("{base}/lab/bv/{TOOL_READ_ASSIGNED_CONTEXT}"),
            Some(&token),
            Some(ids.clone()),
        )
        .await;
        assert_eq!(rest_status, StatusCode::OK);
        assert_eq!(rest_ctx["service"]["cpt"], "72148");

        let mcp_payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": TOOL_READ_ASSIGNED_CONTEXT, "arguments": ids }
        });
        let (mcp_status, mcp_resp) = json_request(
            reqwest::Method::POST,
            &format!("{base}/mcp"),
            Some(&token),
            Some(mcp_payload),
        )
        .await;
        assert_eq!(mcp_status, StatusCode::OK);
        assert_eq!(mcp_resp["result"]["structuredContent"], rest_ctx);

        let ask = json!({
            "run_id": run_id,
            "task_id": task_id,
            "question": "Is prior authorization required for CPT 72148?",
            "evidence_hint": "payer_bv_response"
        });
        let (ask_status, ask_body) = json_request(
            reqwest::Method::POST,
            &format!("{base}/lab/bv/{TOOL_ASK_PAYER}"),
            Some(&token),
            Some(ask),
        )
        .await;
        assert_eq!(ask_status, StatusCode::OK);
        assert_eq!(ask_body["status"], "answered");
        assert_eq!(ask_body["mode"], "auto_payer");
        assert!(ask_body["jsonrpc"].is_null());

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rest_requires_bearer_and_rejects_forbidden_tools() {
        let (_dir, state) = test_mcp_state("approval", true, false);
        let token = state.token.clone();
        let run_id = state.run_id;
        let task_id = state.task_id;
        let stage_before = state.engine.require_case(run_id).expect("case").stage;
        let (base, handle) = spawn_server(state).await;

        let (unauth, body) = json_request(
            reqwest::Method::POST,
            &format!("{base}/lab/bv/{TOOL_READ_ASSIGNED_CONTEXT}"),
            None,
            Some(json!({ "run_id": run_id, "task_id": task_id })),
        )
        .await;
        assert_eq!(unauth, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");

        let (forbidden, forbidden_body) = json_request(
            reqwest::Method::POST,
            &format!("{base}/lab/bv/set_stage"),
            Some(&token),
            Some(json!({
                "run_id": run_id,
                "task_id": task_id,
                "stage": "handoff"
            })),
        )
        .await;
        assert_eq!(forbidden, StatusCode::BAD_REQUEST);
        assert_eq!(forbidden_body["error"], "invalid_schema");
        assert!(forbidden_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("set_stage"));

        let inspect_url = format!("{base}/lab/runs/{run_id}/inspect");
        let (inspect_status, inspect) =
            json_request(reqwest::Method::GET, &inspect_url, Some(&token), None).await;
        assert_eq!(inspect_status, StatusCode::OK);
        assert_eq!(inspect["run_id"], json!(run_id));
        assert_eq!(inspect["stage"], json!(stage_before));
        assert_ne!(inspect["stage"], json!(CaseStage::Handoff));

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rest_openapi_inspect_and_trace() {
        let (_dir, state) = test_mcp_state("unclear_bv", true, true);
        let token = state.token.clone();
        let run_id = state.run_id;
        let task_id = state.task_id;
        let engine = Arc::clone(&state.engine);
        let (base, handle) = spawn_server(state).await;

        let (spec_status, spec) = json_request(
            reqwest::Method::GET,
            &format!("{base}/lab/openapi.json"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(spec_status, StatusCode::OK);
        assert_eq!(spec["openapi"], "3.1.0");
        for tool in [
            TOOL_READ_ASSIGNED_CONTEXT,
            TOOL_ASK_PAYER,
            TOOL_REPORT_OBSERVATIONS,
        ] {
            assert!(
                spec["paths"]
                    .get(format!("/lab/bv/{tool}"))
                    .and_then(|p| p.get("post"))
                    .is_some(),
                "openapi missing {tool}"
            );
        }
        assert!(spec["paths"].get("/lab/bv/set_stage").is_none());

        json_request(
            reqwest::Method::POST,
            &format!("{base}/lab/bv/{TOOL_READ_ASSIGNED_CONTEXT}"),
            Some(&token),
            Some(json!({ "run_id": run_id, "task_id": task_id })),
        )
        .await;

        let (trace_status, trace) = json_request(
            reqwest::Method::GET,
            &format!("{base}/lab/runs/{run_id}/trace"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(trace_status, StatusCode::OK);
        assert_eq!(trace["trace_version"], "bv-tools-v1");
        assert_eq!(trace["calls"][0]["tool"], TOOL_READ_ASSIGNED_CONTEXT);

        let other = Uuid::new_v4();
        let (cross, cross_body) = json_request(
            reqwest::Method::GET,
            &format!("{base}/lab/runs/{other}/inspect"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(cross, StatusCode::BAD_REQUEST);
        assert_eq!(cross_body["error"], "invalid_schema");

        let stage = engine.require_case(run_id).expect("case").stage;
        assert_ne!(stage, CaseStage::Handoff);
        handle.abort();
    }
}
