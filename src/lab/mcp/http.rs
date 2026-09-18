//! Loopback HTTP JSON-RPC MCP transport.
//! Used by: `mint lab mcp-http`. Binds 127.0.0.1 only. Bearer `MINT_LAB_MCP_TOKEN`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use crate::lab::error::{LabError, LabResult};
use crate::lab::mcp::{handle_rpc, JsonRpcRequest, McpCallContext, McpState};

#[derive(Clone)]
struct HttpState {
    mcp: Arc<McpState>,
}

pub async fn serve_http(state: McpState, addr: SocketAddr) -> LabResult<()> {
    if !addr.ip().is_loopback() {
        return Err(LabError::Invalid(
            "MCP HTTP must bind 127.0.0.1 (loopback only)".into(),
        ));
    }
    let app = Router::new()
        .route("/health", get(health))
        .route("/mcp", post(rpc))
        .with_state(HttpState {
            mcp: Arc::new(state),
        });
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| LabError::Io(format!("bind {addr}: {err}")))?;
    eprintln!("mint-lab-pa-bv HTTP MCP listening on http://{addr}/mcp");
    axum::serve(listener, app)
        .await
        .map_err(|err| LabError::Io(format!("mcp http: {err}")))?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn rpc(State(state): State<HttpState>, headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let authorized = crate::lab::mcp::token_matches(provided, &state.mcp.token);
    if !authorized {
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32001, "message": "unauthorized", "data": {"error": "unauthorized"} }
        });
        return (StatusCode::UNAUTHORIZED, axum::Json(resp));
    }
    let req: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(err) => {
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {err}") }
            });
            return (StatusCode::BAD_REQUEST, axum::Json(resp));
        }
    };
    let resp = handle_rpc(&state.mcp, req, &McpCallContext { authorized: true });
    (
        StatusCode::OK,
        axum::Json(serde_json::to_value(resp).unwrap_or_else(|_| serde_json::json!({}))),
    )
}
