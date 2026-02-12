//! Branch management commands

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;
use gwt_core::git::{is_bare_repository, Branch, Remote};
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use tauri::State;
use tracing::error;

/// Serializable branch info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub commit: String,
    pub is_current: bool,
    pub is_agent_running: bool,
    pub has_remote: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub divergence_status: String,
    pub commit_timestamp: Option<i64>,
    pub is_gone: bool,
    pub last_tool_usage: Option<String>,
}

impl From<Branch> for BranchInfo {
    fn from(b: Branch) -> Self {
        let divergence_status = b.divergence_status().to_string();
        BranchInfo {
            name: b.name,
            commit: b.commit,
            is_current: b.is_current,
            is_agent_running: false,
            has_remote: b.has_remote,
            upstream: b.upstream,
            ahead: b.ahead,
            behind: b.behind,
            divergence_status,
            commit_timestamp: b.commit_timestamp,
            is_gone: b.is_gone,
            last_tool_usage: None,
        }
    }
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn build_last_tool_usage_map(repo_path: &Path) -> HashMap<String, String> {
    gwt_core::config::get_last_tool_usage_map(repo_path)
        .into_iter()
        .map(|(branch, entry)| (branch, entry.format_tool_usage()))
        .collect()
}

fn running_agent_branches(state: &AppState, repo_path: &Path) -> HashSet<String> {
    let running: Vec<(String, String)> = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .filter(|pane| matches!(pane.status(), gwt_core::terminal::pane::PaneStatus::Running))
            .map(|pane| (pane.pane_id().to_string(), pane.branch_name().to_string()))
            .collect(),
        Err(_) => Vec::new(),
    };

    if running.is_empty() {
        return HashSet::new();
    }

    let Ok(launch_meta) = state.pane_launch_meta.lock() else {
        return running.into_iter().map(|(_, branch)| branch).collect();
    };

    running
        .into_iter()
        .filter_map(|(pane_id, branch)| {
            let Some(meta) = launch_meta.get(&pane_id) else {
                return Some(branch);
            };
            if meta.repo_path.as_path() == repo_path {
                Some(branch)
            } else {
                None
            }
        })
        .collect()
}

fn with_panic_guard<T>(context: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling branch command"
            );
            Err(format!("Unexpected error while {}", context))
        }
    }
}

/// List all local branches in a repository
#[tauri::command]
pub fn list_branches(
    project_path: String,
    state: State<AppState>,
) -> Result<Vec<BranchInfo>, String> {
    with_panic_guard("listing branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);

        let branches = Branch::list(&repo_path).map_err(|e| e.to_string())?;
        let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
        }
        Ok(infos)
    })
}

/// List branches that currently have a local worktree (gwt "Local" view)
#[tauri::command]
pub fn list_worktree_branches(
    project_path: String,
    state: State<AppState>,
) -> Result<Vec<BranchInfo>, String> {
    with_panic_guard("listing worktree branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);

        let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
        let worktrees = manager.list_basic().map_err(|e| e.to_string())?;

        let names: HashSet<String> = worktrees
            .into_iter()
            .filter(|wt| !wt.is_main && wt.is_active())
            .filter_map(|wt| wt.branch)
            .collect();

        if names.is_empty() {
            return Ok(Vec::new());
        }

        let branches = Branch::list(&repo_path).map_err(|e| e.to_string())?;
        let mut infos: Vec<BranchInfo> = branches
            .into_iter()
            .filter(|b| names.contains(&b.name))
            .map(BranchInfo::from)
            .collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
        }
        Ok(infos)
    })
}

/// List all remote branches in a repository
#[tauri::command]
pub fn list_remote_branches(
    project_path: String,
    state: State<AppState>,
) -> Result<Vec<BranchInfo>, String> {
    with_panic_guard("listing remote branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let remotes = Remote::list(&repo_path).unwrap_or_default();

        let branches = if is_bare_repository(&repo_path) {
            Branch::list_remote_from_origin(&repo_path).map_err(|e| e.to_string())?
        } else {
            Branch::list_remote(&repo_path).map_err(|e| e.to_string())?
        };
        let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
        for info in &mut infos {
            let normalized = strip_known_remote_prefix(&info.name, &remotes);
            info.last_tool_usage = last_tool.get(normalized).cloned();
            info.is_agent_running = running_branches.contains(normalized);
        }
        Ok(infos)
    })
}

/// Get the current branch
#[tauri::command]
pub fn get_current_branch(
    project_path: String,
    state: State<AppState>,
) -> Result<Option<BranchInfo>, String> {
    with_panic_guard("getting current branch", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)?;
        let branch = Branch::current(&repo_path).map_err(|e| e.to_string())?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        Ok(branch.map(|b| {
            let mut info = BranchInfo::from(b);
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            info
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::with_panic_guard;

    #[test]
    fn test_with_panic_guard_returns_error_on_panic() {
        let result: Result<(), String> = with_panic_guard("test", || -> Result<(), String> {
            panic!("boom");
        });
        assert!(result.is_err());
    }
}
