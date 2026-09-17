//! HTTP server wiring, CORS, body limits, and security headers.
//! Used by: mint serve and integration tests.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::header::{self, HeaderValue};
use axum::http::{HeaderName, Method};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::config::{Config, ProviderKind};
use crate::credentials::CredentialSource;
use crate::error::{Error, Result};
use crate::execution::Engine;
use crate::identity::IdentityProvider;
use crate::keys::KeyRing;
use crate::packs::{FakePack, Pack, StripePack};
use crate::policy::PolicyProvider;
use crate::storage::Store;

#[derive(Clone)]
pub struct AppState {
    pub engine: Engine,
    pub keys: Arc<KeyRing>,
    pub identity: Arc<IdentityProvider>,
    pub config: Arc<Config>,
}

pub fn build_state(config: Config) -> Result<AppState> {
    let config = Arc::new(config);
    let keys = Arc::new(KeyRing::from_config(
        &config.kid,
        config.signing_key_file.as_deref(),
        config.signing_key_env.as_deref(),
    )?);
    let store = Store::open(&config.database_path)?;
    let http = reqwest::Client::builder()
        .timeout(config.http_timeout)
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| Error::internal("http client", err))?;
    let identity = Arc::new(IdentityProvider::from_config(&config, http.clone())?);
    let policy = Arc::new(PolicyProvider::from_config(&config, http.clone()));
    let credentials = Arc::new(CredentialSource::from_config(&config, http.clone())?);
    let pack = match config.provider {
        ProviderKind::Fake => Pack::Fake(Arc::new(FakePack::default())),
        ProviderKind::Stripe => Pack::Stripe(StripePack::new(http)),
    };
    let engine = Engine {
        config: config.clone(),
        store,
        keys: keys.clone(),
        policy,
        credentials,
        pack,
    };
    Ok(AppState {
        engine,
        keys,
        identity,
        config,
    })
}

pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.body_limit_bytes;
    let cors = cors_layer(&state.config.cors_origins);
    let mut router = crate::api::router(state)
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(middleware::from_fn(security_headers));
    if let Some(cors) = cors {
        router = router.layer(cors);
    }
    router
}

fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    if parsed.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(parsed))
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                HeaderName::from_static("x-mint-failpoint"),
            ]),
    )
}

async fn security_headers(req: axum::extract::Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
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
    tracing::info!(addr = ?listener.local_addr(), "mint.run listening");
    axum::serve(listener, router).await
}
