//! Health check endpoint.
//! Used by: server.

use axum::http::StatusCode;

pub async fn health() -> StatusCode {
    StatusCode::OK
}
