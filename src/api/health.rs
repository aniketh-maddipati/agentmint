//! Liveness endpoint. Does not leak configuration.
//! Used by: api router.

use axum::Json;
use serde_json::{json, Value};

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
