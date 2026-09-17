//! HTTP API for mint.run `/v1` actions, keys, and health.
//! Used by: server.

mod actions;
mod health;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::Router;

use crate::error::Error;
use crate::failpoints::{self, Failpoint};
use crate::identity::{AuthContext, IdentityProvider};
use crate::server::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/v1/keys", get(actions::keys))
        .route("/v1/actions", post(actions::propose))
        .route("/v1/actions/{id}", get(actions::get_action))
        .route("/v1/actions/{id}/approve", post(actions::approve))
        .route("/v1/actions/{id}/deny", post(actions::deny))
        .route("/v1/actions/{id}/execute", post(actions::execute))
        .route("/v1/actions/{id}/reconcile", post(actions::reconcile))
        .route("/v1/actions/{id}/receipt", get(actions::receipt))
        .route("/v1/actions/{id}/approval", get(actions::approval_page))
        .with_state(state)
}

pub struct Auth(pub AuthContext);

impl FromRequestParts<AppState> for Auth {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        let identity: &IdentityProvider = &state.identity;
        Ok(Auth(identity.authenticate(header).await?))
    }
}

pub fn failpoint_from(headers: &HeaderMap) -> Option<Failpoint> {
    let value = headers
        .get("x-mint-failpoint")
        .and_then(|value| value.to_str().ok())?;
    failpoints::parse_name(value)
}
