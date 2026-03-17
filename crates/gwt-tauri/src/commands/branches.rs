//! Branch management commands

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::commands::terminal::capture_scrollback_tail_from_state;
use crate::state::AppState;
use gwt_core::config::{agent_has_hook_support, infer_agent_status, AgentStatus, Session};
use gwt_core::git::{is_bare_repository, Branch, Remote};
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::worktree::WorktreeManager;
use gwt_core::StructuredError;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use tauri::{AppHandle, Manager, State};
use tracing::error;

/// Serializable branch info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub display_name: Option<String>,
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
            display_name: None,
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

/// Per-branch metadata extracted from session files.
#[derive(Debug, Clone)]
struct SessionBranchMeta {
    agent_status: AgentStatus,
    display_name: Option<String>,
}

/// Build a map of branch name → SessionBranchMeta from session files.
/// For agents without Hook support, infers status from pane output.
fn build_session_branch_meta_map(
    repo_path: &Path,
    state: &AppState,
) -> HashMap<String, SessionBranchMeta> {
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

                let agent_status = if agent_has_hook_support(session.agent.as_deref()) {
                    // Claude Code: trust session file status
                    session.status
                } else if let Some(pane_id) = pane_map.get(branch_name) {
                    // Non-hook agent with running pane: infer from output
                    infer_status_from_pane(state, pane_id)
                } else {
                    // No running pane: use session status as-is
                    session.status
                };

                map.insert(
                    branch_name.clone(),
                    SessionBranchMeta {
                        agent_status,
                        display_name: session.display_name,
                    },
                );
            }
        }
    }
    map
}

/// Build a map of branch name → pane_id in the given repo, preferring running panes.
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
            // Fallback: use all panes without repo filtering.
            return select_preferred_branch_panes(panes_info);
        }
    };

    select_preferred_branch_panes(panes_info.into_iter().filter(|(pane_id, _, _)| {
        launch_meta
            .get(pane_id)
            .map(|meta| meta.repo_path.as_path() == repo_path)
            .unwrap_or(false)
    }))
}

fn select_preferred_branch_panes<I>(panes: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String, bool)>,
{
    let mut preferred: HashMap<String, (String, bool)> = HashMap::new();
    for (pane_id, branch, is_running) in panes {
        match preferred.get_mut(&branch) {
            Some((selected_pane_id, selected_is_running)) => {
                if !*selected_is_running && is_running {
                    *selected_pane_id = pane_id;
                    *selected_is_running = true;
                }
            }
            None => {
                preferred.insert(branch, (pane_id, is_running));
            }
        }
    }

    preferred
        .into_iter()
        .map(|(branch, (pane_id, _))| (branch, pane_id))
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

    let scrollback_tail =
        capture_scrollback_tail_from_state(state, pane_id, 4096, None).unwrap_or_default();

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
            let meta = launch_meta.get(&pane_id)?;
            if meta.repo_path.as_path() == repo_path {
                Some(branch)
            } else {
                None
            }
        })
        .collect()
}

fn with_panic_guard<T>(
    context: &str,
    command: &str,
    f: impl FnOnce() -> Result<T, StructuredError>,
) -> Result<T, StructuredError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling branch command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
        }
    }
}

#[derive(Debug)]
struct WorktreeBranchListing {
    infos: Vec<BranchInfo>,
    branch_names: Vec<String>,
}

/// Apply session branch meta (agent_status + display_name) to a BranchInfo.
/// `branch_key` is the lookup key in the meta map (may differ from info.name for remote branches).
fn apply_session_meta(
    info: &mut BranchInfo,
    branch_key: &str,
    meta_map: &HashMap<String, SessionBranchMeta>,
    summary_cache: &Option<&gwt_core::ai::SessionSummaryCache>,
) {
    if let Some(meta) = meta_map.get(branch_key) {
        info.agent_status = agent_status_to_string(meta.agent_status);
        // display_name priority: session.display_name → task_overview
        if meta.display_name.is_some() {
            info.display_name = meta.display_name.clone();
        }
    }
    if info.display_name.is_none() {
        if let Some(cache) = summary_cache {
            if let Some(summary) = cache.get(branch_key) {
                if let Some(overview) = &summary.task_overview {
                    if !overview.is_empty() {
                        info.display_name = Some(overview.clone());
                    }
                }
            }
        }
    }
}

fn list_worktree_branches_impl(
    project_path: &str,
    state: &AppState,
) -> Result<WorktreeBranchListing, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_worktree_branches"))?;
    let last_tool = build_last_tool_usage_map(&repo_path);
    let running_branches = running_agent_branches(state, &repo_path);
    let meta_map = build_session_branch_meta_map(&repo_path, state);
    let repo_key = repo_path.to_string_lossy().to_string();
    let summary_cache_guard = state.session_summary_cache.lock().ok();
    let summary_cache = summary_cache_guard
        .as_ref()
        .and_then(|g| g.get(&repo_key));

    let manager = WorktreeManager::new(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;
    let worktrees = manager
        .list_basic()
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;

    let names: HashSet<String> = worktrees
        .into_iter()
        .filter(|wt| !wt.is_main && wt.is_active())
        .filter_map(|wt| wt.branch)
        .collect();

    if names.is_empty() {
        return Ok(WorktreeBranchListing {
            infos: Vec::new(),
            branch_names: Vec::new(),
        });
    }

    let branch_names = names.iter().cloned().collect::<Vec<_>>();

    let branches = Branch::list(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;
    let mut infos: Vec<BranchInfo> = branches
        .into_iter()
        .filter(|b| names.contains(&b.name))
        .map(BranchInfo::from)
        .collect();
    for info in &mut infos {
        info.last_tool_usage = last_tool.get(&info.name).cloned();
        info.is_agent_running = running_branches.contains(&info.name);
        apply_session_meta(info, &info.name.clone(), &meta_map, &summary_cache);
    }

    Ok(WorktreeBranchListing {
        infos,
        branch_names,
    })
}

fn list_remote_branches_impl(
    project_path: &str,
    state: &AppState,
) -> Result<Vec<BranchInfo>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_remote_branches"))?;
    let last_tool = build_last_tool_usage_map(&repo_path);
    let running_branches = running_agent_branches(state, &repo_path);
    let meta_map = build_session_branch_meta_map(&repo_path, state);
    let repo_key = repo_path.to_string_lossy().to_string();
    let summary_cache_guard = state.session_summary_cache.lock().ok();
    let summary_cache = summary_cache_guard
        .as_ref()
        .and_then(|g| g.get(&repo_key));
    let remotes = Remote::list(&repo_path).unwrap_or_default();

    let branches = if is_bare_repository(&repo_path) {
        Branch::list_remote_from_origin(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_remote_branches"))?
    } else {
        Branch::list_remote(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_remote_branches"))?
    };

    let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
    for info in &mut infos {
        let normalized = strip_known_remote_prefix(&info.name, &remotes).to_string();
        info.last_tool_usage = last_tool.get(&normalized).cloned();
        info.is_agent_running = running_branches.contains(&normalized);
        apply_session_meta(info, &normalized, &meta_map, &summary_cache);
    }

    Ok(infos)
}

/// List all local branches in a repository
#[tauri::command]
pub fn list_branches(
    project_path: String,
    state: State<AppState>,
) -> Result<Vec<BranchInfo>, StructuredError> {
    with_panic_guard("listing branches", "list_branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)
            .map_err(|e| StructuredError::internal(&e, "list_branches"))?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let meta_map = build_session_branch_meta_map(&repo_path, &state);
        let repo_key = repo_path.to_string_lossy().to_string();
        let summary_cache_guard = state.session_summary_cache.lock().ok();
        let summary_cache = summary_cache_guard
            .as_ref()
            .and_then(|g| g.get(&repo_key));

        let branches = Branch::list(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_branches"))?;
        let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            apply_session_meta(info, &info.name.clone(), &meta_map, &summary_cache);
        }
        Ok(infos)
    })
}

/// List branches that currently have a local worktree (gwt "Local" view)
#[tauri::command]
pub async fn list_worktree_branches(
    project_path: String,
    app_handle: AppHandle,
) -> Result<Vec<BranchInfo>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        with_panic_guard(
            "listing worktree branches",
            "list_worktree_branches",
            || {
                let state = app_handle.state::<AppState>();
                let listing = list_worktree_branches_impl(&project_path, &state)?;

                let prewarm_project_path = project_path.clone();
                let prewarm_handle = app_handle.clone();
                let branch_names = listing.branch_names;
                tauri::async_runtime::spawn_blocking(move || {
                    crate::commands::sessions::prewarm_missing_worktree_summaries(
                        prewarm_project_path,
                        branch_names,
                        prewarm_handle,
                    );
                });

                Ok(listing.infos)
            },
        )
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while listing worktree branches: {e}"),
            "list_worktree_branches",
        )
    })?
}

/// List all remote branches in a repository
#[tauri::command]
pub async fn list_remote_branches(
    project_path: String,
    app_handle: AppHandle,
) -> Result<Vec<BranchInfo>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        with_panic_guard("listing remote branches", "list_remote_branches", || {
            let state = app_handle.state::<AppState>();
            list_remote_branches_impl(&project_path, &state)
        })
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while listing remote branches: {e}"),
            "list_remote_branches",
        )
    })?
}

/// Get the current branch
#[tauri::command]
pub fn get_current_branch(
    project_path: String,
    state: State<AppState>,
) -> Result<Option<BranchInfo>, StructuredError> {
    with_panic_guard("getting current branch", "get_current_branch", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)
            .map_err(|e| StructuredError::internal(&e, "get_current_branch"))?;
        let branch = Branch::current(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "get_current_branch"))?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let meta_map = build_session_branch_meta_map(&repo_path, &state);
        let repo_key = repo_path.to_string_lossy().to_string();
        let summary_cache_guard = state.session_summary_cache.lock().ok();
        let summary_cache = summary_cache_guard
            .as_ref()
            .and_then(|g| g.get(&repo_key));
        Ok(branch.map(|b| {
            let mut info = BranchInfo::from(b);
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            let name_key = info.name.clone();
            apply_session_meta(&mut info, &name_key, &meta_map, &summary_cache);
            info
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use gwt_core::config::AgentStatus;
    use gwt_core::process::command;
    use tempfile::TempDir;

    fn init_git_repo(path: &Path) {
        let init = command("git").args(["init"]).current_dir(path).output();
        assert!(init.is_ok(), "git init failed to run");
        assert!(init.unwrap().status.success(), "git init failed");

        let _ = command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output();
        let _ = command("git")
            .args(["config", "user.name", "test"])
            .current_dir(path)
            .output();

        std::fs::write(path.join("README.md"), "init\n").expect("failed to write README");
        let add = command("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .expect("git add should run");
        assert!(add.status.success(), "git add failed");

        let commit = command("git")
            .args(["commit", "-m", "init"])
            .current_dir(path)
            .output()
            .expect("git commit should run");
        assert!(commit.status.success(), "git commit failed");
    }

    #[test]
    fn test_with_panic_guard_returns_error_on_panic() {
        let result: Result<(), StructuredError> =
            with_panic_guard("test", "test_cmd", || -> Result<(), StructuredError> {
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

    #[test]
    fn test_select_preferred_branch_panes_prefers_running_pane() {
        let panes = vec![
            ("pane-completed".to_string(), "feature/a".to_string(), false),
            ("pane-running".to_string(), "feature/a".to_string(), true),
        ];

        let map = select_preferred_branch_panes(panes);
        assert_eq!(
            map.get("feature/a").map(String::as_str),
            Some("pane-running")
        );
    }

    #[test]
    fn test_select_preferred_branch_panes_keeps_first_when_not_running() {
        let panes = vec![
            ("pane-old".to_string(), "feature/a".to_string(), false),
            ("pane-new".to_string(), "feature/a".to_string(), false),
        ];

        let map = select_preferred_branch_panes(panes);
        assert_eq!(map.get("feature/a").map(String::as_str), Some("pane-old"));
    }

    #[test]
    fn test_list_worktree_branches_impl_returns_consistent_branch_mapping() {
        let repo = TempDir::new().expect("temp dir");
        init_git_repo(repo.path());
        let project_path = repo.path().to_string_lossy().to_string();
        let state = AppState::new();

        let out = list_worktree_branches_impl(&project_path, &state).expect("listing should work");
        let names: HashSet<String> = out.branch_names.iter().cloned().collect();
        assert_eq!(names.len(), out.branch_names.len());
        for info in &out.infos {
            assert!(names.contains(&info.name));
        }
    }

    #[test]
    fn test_list_remote_branches_impl_returns_empty_without_remotes() {
        let repo = TempDir::new().expect("temp dir");
        init_git_repo(repo.path());
        let project_path = repo.path().to_string_lossy().to_string();
        let state = AppState::new();

        let out = list_remote_branches_impl(&project_path, &state).expect("listing should work");
        assert!(out.is_empty());
    }
}
