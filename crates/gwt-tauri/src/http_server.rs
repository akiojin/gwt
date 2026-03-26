//! Lightweight axum HTTP server for heavy IPC commands.
//!
//! Offloads expensive Git queries from the WKWebView main thread by letting
//! the frontend `fetch()` directly to a local HTTP endpoint instead of going
//! through the Tauri invoke bridge.

use axum::{extract::Json, http::StatusCode, response::IntoResponse, routing::post, Router};
use gwt_core::StructuredError;
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::commands::git_view;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeSummaryRequest {
    project_path: String,
    branch: String,
    base_branch: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchDiffFilesRequest {
    project_path: String,
    branch: String,
    base_branch: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchCommitsRequest {
    project_path: String,
    branch: String,
    base_branch: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkingTreeStatusRequest {
    project_path: String,
    branch: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StashListRequest {
    project_path: String,
    branch: String,
}

// ---------------------------------------------------------------------------
// Error response helper
// ---------------------------------------------------------------------------

struct HttpError(StructuredError);

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::to_value(&self.0).unwrap_or_default();
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

impl From<StructuredError> for HttpError {
    fn from(e: StructuredError) -> Self {
        Self(e)
    }
}

// ---------------------------------------------------------------------------
// Blocking dispatch helper
// ---------------------------------------------------------------------------

/// Run a blocking closure on the tokio blocking pool, converting JoinError
/// to StructuredError.
async fn blocking<T, F>(cmd: &'static str, f: F) -> Result<Json<T>, HttpError>
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce() -> Result<T, StructuredError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(StructuredError::internal(&e.to_string(), cmd)))
        .map(Json)
        .map_err(HttpError::from)
}

// ---------------------------------------------------------------------------
// Route handlers (thin delegates to git_view::*_impl)
// ---------------------------------------------------------------------------

async fn handle_get_git_change_summary(
    Json(req): Json<ChangeSummaryRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("get_git_change_summary", move || {
        git_view::get_git_change_summary_impl(
            &req.project_path,
            &req.branch,
            req.base_branch.as_deref(),
        )
    })
    .await
}

async fn handle_get_branch_diff_files(
    Json(req): Json<BranchDiffFilesRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("get_branch_diff_files", move || {
        git_view::get_branch_diff_files_impl(&req.project_path, &req.branch, &req.base_branch)
    })
    .await
}

async fn handle_get_branch_commits(
    Json(req): Json<BranchCommitsRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("get_branch_commits", move || {
        git_view::get_branch_commits_impl(
            &req.project_path,
            &req.branch,
            &req.base_branch,
            req.offset,
            req.limit,
        )
    })
    .await
}

async fn handle_get_working_tree_status(
    Json(req): Json<WorkingTreeStatusRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("get_working_tree_status", move || {
        git_view::get_working_tree_status_impl(&req.project_path, &req.branch)
    })
    .await
}

async fn handle_get_stash_list(
    Json(req): Json<StashListRequest>,
) -> Result<impl IntoResponse, HttpError> {
    blocking("get_stash_list", move || {
        git_view::get_stash_list_impl(&req.project_path, &req.branch)
    })
    .await
}

// ---------------------------------------------------------------------------
// Router & server startup
// ---------------------------------------------------------------------------

fn build_router() -> Router {
    Router::new()
        .route(
            "/get_git_change_summary",
            post(handle_get_git_change_summary),
        )
        .route(
            "/get_branch_diff_files",
            post(handle_get_branch_diff_files),
        )
        .route("/get_branch_commits", post(handle_get_branch_commits))
        .route(
            "/get_working_tree_status",
            post(handle_get_working_tree_status),
        )
        .route("/get_stash_list", post(handle_get_stash_list))
}

/// Start the HTTP IPC server on a random port and return the port number.
///
/// The server runs as a background tokio task and lives for the lifetime of
/// the process.
#[cfg_attr(test, allow(dead_code))]
pub async fn start_http_server() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind HTTP IPC server: {e}"))?;

    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {e}"))?
        .port();

    info!(port, "HTTP IPC server listening");

    let router = build_router();

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            warn!(error = %e, "HTTP IPC server exited with error");
        }
    });

    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_builds_without_panic() {
        let _ = build_router();
    }
}
