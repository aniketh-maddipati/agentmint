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
    state.increment_requests();
    let total_start = Instant::now();

    let verify_start = Instant::now();
    let claims = match verify_token(&req.token, &state.verifying_key) {
        Ok(c) => c,
        Err(e) => {
            state.metrics.record_rejection();
            tracing::warn!(reason = %e, "token rejected");
            return Err(e);
        }
    };
    let verify_us = verify_start.elapsed().as_micros();

    let jti_start = Instant::now();
    if let Err(e) = state.jti_store.check_and_insert(&claims.jti, claims.exp) {
        state.metrics.record_replay();
        tracing::warn!(jti = %claims.jti, "replay blocked");
        return Err(e);
    }
    let jti_us = jti_start.elapsed().as_micros();

    let audit_start = Instant::now();
    state.audit_log.log(&claims.jti, &claims.sub, &claims.action, Utc::now())?;
    let audit_us = audit_start.elapsed().as_micros();

    let total_us = total_start.elapsed().as_micros();
    state.metrics.record_verify(total_us as u64);

    tracing::info!(
        jti = %claims.jti,
        verify_us = %verify_us,
        "verify: {}μs | jti: {}μs | audit: {}μs | total: {}μs",
        verify_us, jti_us, audit_us, total_us
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Verify-Time-Us",
        HeaderValue::from_str(&total_us.to_string()).map_err(|e| Error::Signing(e.to_string()))?,
    );

    Ok((headers, Json(ProxyResponse {
        sub: claims.sub,
        action: claims.action,
        jti: claims.jti,
    })))
}
