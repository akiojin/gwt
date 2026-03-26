//! Lightweight axum HTTP server that offloads heavy IPC commands off the
//! WKWebView main-thread URL scheme handler.
//!
//! The server is started on a **dedicated OS thread** with its own tokio
//! runtime so it never contends with the Tauri async runtime.

use std::sync::Arc;

use axum::{
    extract::State as AxumState,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use crate::state::AppState;

/// Shared state accessible from axum handlers.
type SharedState = Arc<AppState>;

// ── Request payloads ────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectPathRequest {
    project_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchInventoryRequest {
    project_path: String,
    refresh_key: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchInventoryDetailRequest {
    project_path: String,
    canonical_name: String,
    #[serde(default)]
    force_refresh: bool,
}

// ── Handlers ────────────────────────────────────────────────────────

async fn handle_list_worktree_branches(
    AxumState(state): AxumState<SharedState>,
    Json(req): Json<ProjectPathRequest>,
) -> impl IntoResponse {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::commands::branches::list_worktree_branches_impl(&req.project_path, &state)
    })
    .await;

    match result {
        Ok(Ok(listing)) => Json(listing.infos).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_list_worktrees(
    AxumState(state): AxumState<SharedState>,
    Json(req): Json<ProjectPathRequest>,
) -> impl IntoResponse {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::commands::cleanup::list_worktrees_impl(&req.project_path, &state)
    })
    .await;

    match result {
        Ok(Ok(worktrees)) => Json(worktrees).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_list_branch_inventory(
    AxumState(state): AxumState<SharedState>,
    Json(req): Json<BranchInventoryRequest>,
) -> impl IntoResponse {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::commands::branches::list_branch_inventory_impl(
            &req.project_path,
            req.refresh_key,
            &state,
        )
    })
    .await;

    match result {
        Ok(Ok(entries)) => Json(entries).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_branch_inventory_detail(
    AxumState(state): AxumState<SharedState>,
    Json(req): Json<BranchInventoryDetailRequest>,
) -> impl IntoResponse {
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::commands::branches::get_branch_inventory_detail_impl(
            &req.project_path,
            &req.canonical_name,
            req.force_refresh,
            &state,
        )
    })
    .await;

    match result {
        Ok(Ok(detail)) => Json(detail).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(e)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Server bootstrap ────────────────────────────────────────────────

fn build_router(state: SharedState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/ipc/list_worktree_branches", post(handle_list_worktree_branches))
        .route("/ipc/list_worktrees", post(handle_list_worktrees))
        .route("/ipc/list_branch_inventory", post(handle_list_branch_inventory))
        .route(
            "/ipc/get_branch_inventory_detail",
            post(handle_get_branch_inventory_detail),
        )
        .layer(cors)
        .with_state(state)
}

/// Start the HTTP IPC server on a dedicated OS thread with its own tokio
/// runtime.  Returns the ephemeral port the server is listening on.
pub fn start_http_server(app_state: Arc<AppState>) -> u16 {
    let (tx, rx) = std::sync::mpsc::channel::<u16>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build HTTP IPC tokio runtime");

        rt.block_on(async move {
            let router = build_router(app_state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("failed to bind HTTP IPC server");
            let port = listener.local_addr().unwrap().port();

            info!(
                category = "http_ipc",
                port = port,
                "HTTP IPC server listening"
            );

            if tx.send(port).is_err() {
                warn!(
                    category = "http_ipc",
                    "Failed to send port back to main thread"
                );
                return;
            }

            axum::serve(listener, router)
                .await
                .expect("HTTP IPC server failed");
        });
    });

    rx.recv().expect("failed to receive HTTP IPC port")
}
