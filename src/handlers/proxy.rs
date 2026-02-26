//! Token verification proxy endpoint with latency measurement.
//! Used by: server.

use std::time::Instant;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::token::verify::verify_token;

#[derive(Deserialize)]
pub struct ProxyRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct ProxyResponse {
    pub sub: String,
    pub action: String,
    pub jti: String,
}

pub async fn proxy(
    State(state): State<AppState>,
    Json(req): Json<ProxyRequest>,
) -> Result<(HeaderMap, Json<ProxyResponse>)> {
    let total_start = Instant::now();

    let verify_start = Instant::now();
    let claims = verify_token(&req.token, &state.verifying_key)?;
    let verify_us = verify_start.elapsed().as_micros();

    let jti_start = Instant::now();
    state.jti_store.check_and_insert(&claims.jti)?;
    let jti_us = jti_start.elapsed().as_micros();

    let audit_start = Instant::now();
    state
        .audit_log
        .log(&claims.jti, &claims.sub, &claims.action, Utc::now())?;
    let audit_us = audit_start.elapsed().as_micros();

    let total_us = total_start.elapsed().as_micros();

    tracing::info!(
        "verify: {}μs | jti: {}μs | audit: {}μs | total: {}μs",
        verify_us,
        jti_us,
        audit_us,
        total_us
    );

    let mut headers = HeaderMap::new();
    let header_val = HeaderValue::from_str(&total_us.to_string())
        .map_err(|e| Error::Signing(e.to_string()))?;
    headers.insert("X-Verify-Time-Us", header_val);

    Ok((
        headers,
        Json(ProxyResponse {
            sub: claims.sub,
            action: claims.action,
            jti: claims.jti,
        }),
    ))
}
