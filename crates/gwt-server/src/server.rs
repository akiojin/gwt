use axum::{routing::{get, post}, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::{handlers, state::AppState, ws};

/// Build the axum router with all command routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let broadcaster = Arc::new(state.broadcaster.clone());

    Router::new()
        // Health & system
        .route("/healthz", get(handlers::system::healthz))
        .route("/heartbeat", post(handlers::system::heartbeat))
        // WebSocket
        .route("/ws", get(ws::ws_handler).with_state(broadcaster))
        // TODO: Phase 2 — add all 161 command routes here
        .layer(cors)
        .with_state(state)
}
