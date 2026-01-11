//! REST API handlers

use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
pub struct WorktreeResponse {
    pub path: String,
    pub branch: String,
}

#[derive(Serialize)]
pub struct BranchResponse {
    pub name: String,
    pub is_current: bool,
}

/// Health check endpoint
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// List worktrees endpoint
pub async fn list_worktrees() -> Json<Vec<WorktreeResponse>> {
    // TODO: Get from gwt-core
    Json(vec![])
}

/// List branches endpoint
pub async fn list_branches() -> Json<Vec<BranchResponse>> {
    // TODO: Get from gwt-core
    Json(vec![])
}
