//! Audit log query endpoint.
//! Used by: server.

use axum::extract::State;
use axum::Json;

use crate::audit::sqlite::AuditEntry;
use crate::error::Result;
use crate::state::AppState;

pub async fn recent(State(state): State<AppState>) -> Result<Json<Vec<AuditEntry>>> {
    let entries = state.audit_log.recent(100)?;
    Ok(Json(entries))
}
