//! Token minting endpoint.
//! Used by: server.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::AppState;
use crate::token::claims::Claims;
use crate::token::sign::sign_token;

#[derive(Deserialize)]
pub struct MintRequest {
    pub sub: String,
    pub action: String,
    #[serde(default = "default_ttl")]
    pub ttl_seconds: i64,
}

fn default_ttl() -> i64 {
    300
}

#[derive(Serialize)]
pub struct MintResponse {
    pub token: String,
    pub jti: String,
    pub exp: String,
}

pub async fn mint(
    State(state): State<AppState>,
    Json(req): Json<MintRequest>,
) -> Result<Json<MintResponse>> {
    let claims = Claims::new(req.sub, req.action, req.ttl_seconds);
    let jti = claims.jti.clone();
    let exp = claims.exp.to_rfc3339();
    let token = sign_token(&claims, &state.signing_key)?;
    Ok(Json(MintResponse { token, jti, exp }))
}
