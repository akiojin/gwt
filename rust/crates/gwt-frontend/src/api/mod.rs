//! API client for backend communication

use gloo_net::http::Request;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
}

/// Fetch worktrees from the API
pub async fn fetch_worktrees() -> Result<Vec<Worktree>, gloo_net::Error> {
    Request::get("/api/worktrees")
        .send()
        .await?
        .json()
        .await
}

/// Fetch branches from the API
pub async fn fetch_branches() -> Result<Vec<Branch>, gloo_net::Error> {
    Request::get("/api/branches")
        .send()
        .await?
        .json()
        .await
}
