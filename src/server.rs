//! Axum router and server setup with security headers.
//! Used by: main.

use axum::http::header::{self, HeaderValue};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::cors::CorsLayer;

use crate::handlers;
use crate::state::AppState;

async fn security_headers(req: axum::extract::Request, next: middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health::health))
        .route("/mint", post(handlers::mint::mint))
        .route("/proxy", post(handlers::proxy::proxy))
        .route("/audit", get(handlers::audit::recent))
        .route("/metrics", get(handlers::metrics::metrics))
        .layer(middleware::from_fn(security_headers))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

pub async fn run(state: AppState, addr: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    run_with_listener(state, listener).await
}

pub async fn run_with_listener(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    let router = build_router(state);
    tracing::info!("listening on {:?}", listener.local_addr());
    axum::serve(listener, router).await
}
