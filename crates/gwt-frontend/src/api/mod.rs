//! API client for backend communication

#![allow(dead_code)] // API functions for frontend use

use gloo_net::http::Request;
use serde::Deserialize;

/// Worktree response from API
#[derive(Debug, Clone, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub commit: String,
    pub status: String,
    pub is_main: bool,
    pub has_changes: bool,
    pub has_unpushed: bool,
}

/// Branch response from API
#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub has_remote: bool,
    pub has_worktree: bool,
    pub ahead: usize,
    pub behind: usize,
}

/// Settings response from API
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub default_base_branch: String,
    pub worktree_root: String,
    pub protected_branches: Vec<String>,
}

/// Session history entry from API
#[derive(Debug, Clone, Deserialize)]
pub struct SessionEntry {
    pub branch: String,
    pub tool_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: i64,
}

/// Session history response from API
#[derive(Debug, Clone, Deserialize)]
pub struct SessionHistory {
    pub last_branch: Option<String>,
    pub last_tool: Option<String>,
    pub last_session_id: Option<String>,
    pub history: Vec<SessionEntry>,
}

/// Fetch worktrees from the API
pub async fn fetch_worktrees() -> Result<Vec<Worktree>, gloo_net::Error> {
    Request::get("/api/worktrees").send().await?.json().await
}

/// Fetch branches from the API
pub async fn fetch_branches() -> Result<Vec<Branch>, gloo_net::Error> {
    Request::get("/api/branches").send().await?.json().await
}

/// Fetch settings from the API
pub async fn fetch_settings() -> Result<Settings, gloo_net::Error> {
    Request::get("/api/settings").send().await?.json().await
}

/// Fetch session history from the API
pub async fn fetch_sessions() -> Result<SessionHistory, gloo_net::Error> {
    Request::get("/api/sessions").send().await?.json().await
}
