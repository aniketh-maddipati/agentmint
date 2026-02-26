//! Metrics snapshot endpoint.
//! Used by: server.

use axum::extract::State;
use axum::Json;

use crate::state::AppState;
use crate::telemetry::MetricsSnapshot;

pub async fn metrics(State(state): State<AppState>) -> Json<MetricsSnapshot> {
    Json(state.metrics.snapshot())
}
