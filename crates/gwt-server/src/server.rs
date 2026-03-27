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
        // Terminal
        .route("/list_terminals", post(handlers::terminal::list_terminals))
        .route("/write_terminal", post(handlers::terminal::write_terminal))
        .route("/resize_terminal", post(handlers::terminal::resize_terminal))
        .route("/close_terminal", post(handlers::terminal::close_terminal))
        // Project
        .route("/probe_path", post(handlers::project::probe_path))
        .route("/is_git_repo", post(handlers::project::is_git_repo))
        .route("/get_current_branch", post(handlers::project::get_current_branch))
        // Settings
        .route("/get_settings", post(handlers::settings::get_settings))
        .route("/save_settings", post(handlers::settings::save_settings))
        // WebSocket
        .route("/ws", get(ws::ws_handler).with_state(broadcaster))
        .layer(cors)
        .with_state(state)
}
