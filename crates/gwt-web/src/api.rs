//! REST API handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use gwt_core::{
    config::{load_ts_session, Settings},
    error::GwtError,
    git::Branch,
    worktree::{WorktreeManager, WorktreeStatus},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub repo_path: PathBuf,
}

impl AppState {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
pub struct WorktreeResponse {
    pub path: String,
    pub branch: Option<String>,
    pub commit: String,
    pub status: String,
    pub is_main: bool,
    pub has_changes: bool,
    pub has_unpushed: bool,
}

#[derive(Serialize)]
pub struct BranchResponse {
    pub name: String,
    pub is_current: bool,
    pub has_remote: bool,
    pub has_worktree: bool,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

#[derive(Deserialize)]
pub struct CreateWorktreeRequest {
    pub branch: String,
    #[serde(default)]
    pub new_branch: bool,
    #[serde(default)]
    pub base_branch: Option<String>,
}

#[derive(Serialize)]
pub struct CreateWorktreeResponse {
    pub path: String,
    pub branch: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateBranchRequest {
    pub name: String,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Serialize)]
pub struct SettingsResponse {
    pub default_base_branch: String,
    pub worktree_root: String,
    pub protected_branches: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub default_base_branch: Option<String>,
    #[serde(default)]
    pub worktree_root: Option<String>,
    #[serde(default)]
    pub protected_branches: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub branch: String,
    pub tool_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: i64,
}

#[derive(Serialize)]
pub struct SessionHistoryResponse {
    pub last_branch: Option<String>,
    pub last_tool: Option<String>,
    pub last_session_id: Option<String>,
    pub history: Vec<SessionResponse>,
}

fn map_err(e: GwtError) -> (StatusCode, Json<ErrorResponse>) {
    let code = e.code().to_string();
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: e.to_string(),
            code,
        }),
    )
}

/// Health check endpoint
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// List worktrees endpoint
pub async fn list_worktrees(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WorktreeResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let manager = WorktreeManager::new(&state.repo_path).map_err(map_err)?;
    let worktrees = manager.list().map_err(map_err)?;

    let responses: Vec<WorktreeResponse> = worktrees
        .iter()
        .map(|wt| {
            let status_str = match wt.status {
                WorktreeStatus::Active => "active",
                WorktreeStatus::Locked => "locked",
                WorktreeStatus::Prunable => "prunable",
                WorktreeStatus::Missing => "missing",
            };

            WorktreeResponse {
                path: wt.path.display().to_string(),
                branch: wt.branch.clone(),
                commit: wt.commit.clone(),
                status: status_str.to_string(),
                is_main: wt.is_main,
                has_changes: wt.has_changes,
                has_unpushed: wt.has_unpushed,
            }
        })
        .collect();

    Ok(Json(responses))
}

/// Create worktree endpoint
pub async fn create_worktree(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWorktreeRequest>,
) -> Result<Json<CreateWorktreeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let manager = WorktreeManager::new(&state.repo_path).map_err(map_err)?;

    let worktree = if req.new_branch {
        manager
            .create_new_branch(&req.branch, req.base_branch.as_deref())
            .map_err(map_err)?
    } else {
        manager.create_for_branch(&req.branch).map_err(map_err)?
    };

    Ok(Json(CreateWorktreeResponse {
        path: worktree.path.display().to_string(),
        branch: worktree.branch,
    }))
}

/// Delete worktree endpoint
pub async fn delete_worktree(
    State(state): State<Arc<AppState>>,
    Path(branch): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let manager = WorktreeManager::new(&state.repo_path).map_err(map_err)?;

    // Find worktree by branch
    let wt = manager.get_by_branch(&branch).map_err(map_err)?;
    let wt = wt.ok_or_else(|| {
        map_err(GwtError::BranchNotFound {
            name: branch.clone(),
        })
    })?;

    manager.remove(&wt.path, false).map_err(map_err)?;

    Ok(StatusCode::NO_CONTENT)
}

/// List branches endpoint
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BranchResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let branches = Branch::list(&state.repo_path).map_err(map_err)?;

    // Get worktrees to check which branches have worktrees
    let worktree_manager = WorktreeManager::new(&state.repo_path).ok();
    let worktree_branches: Vec<String> = worktree_manager
        .and_then(|m| m.list().ok())
        .map(|wts| wts.iter().filter_map(|wt| wt.branch.clone()).collect())
        .unwrap_or_default();

    let responses: Vec<BranchResponse> = branches
        .iter()
        .map(|b| BranchResponse {
            name: b.name.clone(),
            is_current: b.is_current,
            has_remote: b.has_remote,
            has_worktree: worktree_branches.contains(&b.name),
            ahead: b.ahead,
            behind: b.behind,
        })
        .collect();

    Ok(Json(responses))
}

/// Create branch endpoint
pub async fn create_branch(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateBranchRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let base = req.base.as_deref().unwrap_or("HEAD");
    Branch::create(&state.repo_path, &req.name, base).map_err(map_err)?;
    Ok(StatusCode::CREATED)
}

/// Delete branch endpoint
pub async fn delete_branch(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    Branch::delete(&state.repo_path, &name, false).map_err(map_err)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get settings endpoint
pub async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let settings = Settings::load(&state.repo_path).unwrap_or_default();

    Ok(Json(SettingsResponse {
        default_base_branch: settings.default_base_branch,
        worktree_root: settings.worktree_root,
        protected_branches: settings.protected_branches,
    }))
}

/// Update settings endpoint
pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<Json<SettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let mut settings = Settings::load(&state.repo_path).unwrap_or_default();

    // Apply updates
    if let Some(base) = req.default_base_branch {
        settings.default_base_branch = base;
    }
    if let Some(root) = req.worktree_root {
        settings.worktree_root = root;
    }
    if let Some(protected) = req.protected_branches {
        settings.protected_branches = protected;
    }

    // Save settings
    settings.save(&state.repo_path).map_err(map_err)?;

    Ok(Json(SettingsResponse {
        default_base_branch: settings.default_base_branch,
        worktree_root: settings.worktree_root,
        protected_branches: settings.protected_branches,
    }))
}

/// Get session history endpoint
pub async fn get_sessions(State(state): State<Arc<AppState>>) -> Json<SessionHistoryResponse> {
    let session = load_ts_session(&state.repo_path);

    match session {
        Some(data) => {
            let history: Vec<SessionResponse> = data
                .history
                .iter()
                .map(|entry| SessionResponse {
                    branch: entry.branch.clone(),
                    tool_id: entry.tool_id.clone(),
                    tool_label: entry.tool_label.clone(),
                    model: entry.model.clone(),
                    session_id: entry.session_id.clone(),
                    timestamp: entry.timestamp,
                })
                .collect();

            Json(SessionHistoryResponse {
                last_branch: data.last_branch,
                last_tool: data.last_used_tool,
                last_session_id: data.last_session_id,
                history,
            })
        }
        None => Json(SessionHistoryResponse {
            last_branch: None,
            last_tool: None,
            last_session_id: None,
            history: vec![],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_creation() {
        let state = AppState::new("/tmp/test");
        assert_eq!(state.repo_path, PathBuf::from("/tmp/test"));
    }
}
