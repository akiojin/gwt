use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;

use crate::state::AppState;

/// GET /healthz — simple health check.
pub async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /heartbeat — frontend liveness ping.
pub async fn heartbeat(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Ok(mut hb) = state.last_heartbeat.lock() {
        *hb = Some(std::time::Instant::now());
    }
    Json(serde_json::json!({ "ok": true }))
}
