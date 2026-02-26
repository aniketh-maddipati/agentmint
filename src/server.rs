//! Axum router and server setup.
//! Used by: main.

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::handlers;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health::health))
        .route("/mint", post(handlers::mint::mint))
        .route("/proxy", post(handlers::proxy::proxy))
        .route("/audit", get(handlers::audit::recent))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

pub async fn run(state: AppState, addr: &str) -> std::io::Result<()> {
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {}", addr);
    axum::serve(listener, router).await
}
