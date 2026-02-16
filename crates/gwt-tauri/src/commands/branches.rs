//! Branch management commands

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::commands::terminal::capture_scrollback_tail_from_state;
use crate::state::AppState;
use gwt_core::config::{agent_has_hook_support, infer_agent_status, AgentStatus, Session};
use gwt_core::git::{is_bare_repository, Branch, Remote};
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use tauri::{AppHandle, State};
use tracing::error;

/// Serializable branch info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub commit: String,
    pub is_current: bool,
    pub is_agent_running: bool,
    pub agent_status: String,
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
            agent_status: "unknown".to_string(),
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

/// Build a map of branch name → AgentStatus from session files.
/// For agents without Hook support, infers status from pane output.
fn build_agent_status_map(repo_path: &Path, state: &AppState) -> HashMap<String, AgentStatus> {
    let manager = match WorktreeManager::new(repo_path) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };
    let worktrees = match manager.list_basic() {
        Ok(wts) => wts,
        Err(_) => return HashMap::new(),
    };

    // Build branch → pane_id mapping for running panes
    let pane_map = build_branch_pane_map(state, repo_path);

    let mut map = HashMap::new();
    for wt in &worktrees {
        if let Some(branch_name) = &wt.branch {
            if let Some(mut session) = Session::load_for_worktree(&wt.path) {
                session.check_idle_timeout();

                if agent_has_hook_support(session.agent.as_deref()) {
                    // Claude Code: trust session file status
                    map.insert(branch_name.clone(), session.status);
                } else if let Some(pane_id) = pane_map.get(branch_name) {
                    // Non-hook agent with running pane: infer from output
                    let status = infer_status_from_pane(state, pane_id);
                    map.insert(branch_name.clone(), status);
                } else {
                    // No running pane: use session status as-is
                    map.insert(branch_name.clone(), session.status);
                }
            }
        }
    }
    map
}

/// Build a map of branch name → pane_id for running panes in the given repo.
fn build_branch_pane_map(state: &AppState, repo_path: &Path) -> HashMap<String, String> {
    let panes_info: Vec<(String, String, bool)> = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .map(|pane| {
                (
                    pane.pane_id().to_string(),
                    pane.branch_name().to_string(),
                    matches!(pane.status(), PaneStatus::Running),
                )
            })
            .collect(),
        Err(_) => return HashMap::new(),
    };

    let launch_meta = match state.pane_launch_meta.lock() {
        Ok(meta) => meta,
        Err(_) => {
            // Fallback: use all panes without repo filtering
            return panes_info
                .into_iter()
                .map(|(pane_id, branch, _)| (branch, pane_id))
                .collect();
        }
    };

    panes_info
        .into_iter()
        .filter(|(pane_id, _, _)| {
            launch_meta
                .get(pane_id)
                .map(|meta| meta.repo_path.as_path() == repo_path)
                .unwrap_or(true)
        })
        .map(|(pane_id, branch, _)| (branch, pane_id))
        .collect()
}

/// Infer agent status from a pane's scrollback tail.
fn infer_status_from_pane(state: &AppState, pane_id: &str) -> AgentStatus {
    let process_alive = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .map(|p| matches!(p.status(), PaneStatus::Running))
            .unwrap_or(false),
        Err(_) => false,
    };

    let scrollback_tail = capture_scrollback_tail_from_state(state, pane_id, 4096)
        .unwrap_or_default();

    infer_agent_status(&scrollback_tail, process_alive)
}

fn agent_status_to_string(status: AgentStatus) -> String {
    match status {
        AgentStatus::Unknown => "unknown".to_string(),
        AgentStatus::Running => "running".to_string(),
        AgentStatus::WaitingInput => "waiting_input".to_string(),
        AgentStatus::Stopped => "stopped".to_string(),
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
        let agent_statuses = build_agent_status_map(&repo_path, &state);

        let branches = Branch::list(&repo_path).map_err(|e| e.to_string())?;
        let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            if let Some(status) = agent_statuses.get(&info.name) {
                info.agent_status = agent_status_to_string(*status);
            }
        }
        Ok(infos)
    })
}

/// List branches that currently have a local worktree (gwt "Local" view)
#[tauri::command]
pub fn list_worktree_branches(
    project_path: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<Vec<BranchInfo>, String> {
    with_panic_guard("listing worktree branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let agent_statuses = build_agent_status_map(&repo_path, &state);

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

        let branch_names = names.iter().cloned().collect::<Vec<_>>();

        let branches = Branch::list(&repo_path).map_err(|e| e.to_string())?;
        let mut infos: Vec<BranchInfo> = branches
            .into_iter()
            .filter(|b| names.contains(&b.name))
            .map(BranchInfo::from)
            .collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            if let Some(status) = agent_statuses.get(&info.name) {
                info.agent_status = agent_status_to_string(*status);
            }
        }

        let prewarm_project_path = project_path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::commands::sessions::prewarm_missing_worktree_summaries(
                prewarm_project_path,
                branch_names,
                app_handle.clone(),
            );
        });

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
        let agent_statuses = build_agent_status_map(&repo_path, &state);
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
            if let Some(status) = agent_statuses.get(normalized) {
                info.agent_status = agent_status_to_string(*status);
            }
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
        let agent_statuses = build_agent_status_map(&repo_path, &state);
        Ok(branch.map(|b| {
            let mut info = BranchInfo::from(b);
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            if let Some(status) = agent_statuses.get(&info.name) {
                info.agent_status = agent_status_to_string(*status);
            }
            info
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::config::AgentStatus;

    #[test]
    fn test_with_panic_guard_returns_error_on_panic() {
        let result: Result<(), String> = with_panic_guard("test", || -> Result<(), String> {
            panic!("boom");
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_status_to_string_unknown() {
        assert_eq!(agent_status_to_string(AgentStatus::Unknown), "unknown");
    }

    #[test]
    fn test_agent_status_to_string_running() {
        assert_eq!(agent_status_to_string(AgentStatus::Running), "running");
    }

    #[test]
    fn test_agent_status_to_string_waiting_input() {
        assert_eq!(
            agent_status_to_string(AgentStatus::WaitingInput),
            "waiting_input"
        );
    }

    #[test]
    fn test_agent_status_to_string_stopped() {
        assert_eq!(agent_status_to_string(AgentStatus::Stopped), "stopped");
    }

    #[test]
    fn test_branch_info_default_agent_status() {
        let branch = gwt_core::git::Branch {
            name: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            is_current: false,
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };
        let info = BranchInfo::from(branch);
        assert_eq!(info.agent_status, "unknown");
        assert!(!info.is_agent_running);
    }
}
