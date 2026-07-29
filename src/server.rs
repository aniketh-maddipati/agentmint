//! Axum router and server setup with security headers.

use axum::http::header::{self, HeaderValue};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{middleware, Router};
use tower_http::cors::CorsLayer;

use crate::handlers;
use crate::state::AppState;
use crate::webauthn;

async fn security_headers(req: axum::extract::Request, next: middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Core endpoints
        .route("/health", get(handlers::health::health))
        .route("/mint", post(handlers::mint::mint))
        .route("/delegate", post(handlers::delegate::delegate))
        .route("/proxy", post(handlers::proxy::proxy))
        .route("/audit", get(handlers::audit::recent))
        .route("/metrics", get(handlers::metrics::metrics))
        // WebAuthn endpoints
        .route("/webauthn/register/start", post(webauthn::register_start))
        .route("/webauthn/register/finish", post(webauthn::register_finish))
        .route("/webauthn/auth/start", post(webauthn::auth_start))
        .route("/webauthn/auth/finish", post(webauthn::auth_finish))
        // Middleware
        .layer(middleware::from_fn(security_headers))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

pub async fn run(state: AppState, addr: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    run_with_listener(state, listener).await
}

pub async fn run_with_listener(
    state: AppState,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    let router = build_router(state);
    tracing::info!("listening on {:?}", listener.local_addr());
    axum::serve(listener, router).await
}

#[cfg(test)]
mod tests {
    use crate::state::build_test_state;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    async fn spawn_server() -> Result<(String, reqwest::Client), Box<dyn std::error::Error>> {
        let state = build_test_state()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(super::run_with_listener(state, listener));
        let client = reqwest::Client::builder().no_proxy().build()?;
        Ok((format!("http://{addr}"), client))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_route_returns_ok_with_security_headers() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client.get(format!("{base}/health")).send().await?;

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            response
                .headers()
                .get("x-frame-options")
                .and_then(|value| value.to_str().ok()),
            Some("DENY")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_route_returns_json() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client.get(format!("{base}/metrics")).send().await?;

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert!(body.is_object());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mint_rejects_malformed_body_via_extractor() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client
            .post(format!("{base}/mint"))
            .header("content-type", "application/json")
            .body("{ not json")
            .send()
            .await?;

        assert!(response.status().is_client_error());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_route_returns_not_found() -> TestResult {
        let (base, client) = spawn_server().await?;

        let response = client.get(format!("{base}/does-not-exist")).send().await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        Ok(())
    }
}
