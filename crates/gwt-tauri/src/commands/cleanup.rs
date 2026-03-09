//! Worktree cleanup commands (gwt-spec issue, gwt-spec issue)

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::commands::terminal::capture_scrollback_tail_from_state;
use crate::state::AppState;
use gwt_core::config::{agent_has_hook_support, infer_agent_status, Session};
use gwt_core::git::gh_cli::PrStatus;
use gwt_core::git::Branch;
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::worktree::WorktreeManager;
use gwt_core::StructuredError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

/// Safety level for a worktree (FR-500)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyLevel {
    Safe,
    Warning,
    Danger,
    Disabled,
}

/// Worktree info for the frontend (gwt-spec issue)
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub commit: String,
    pub status: String,
    pub is_main: bool,
    pub has_changes: bool,
    pub has_unpushed: bool,
    pub is_current: bool,
    pub is_protected: bool,
    pub is_agent_running: bool,
    pub agent_status: String,
    pub ahead: usize,
    pub behind: usize,
    pub is_gone: bool,
    pub last_tool_usage: Option<String>,
    pub safety_level: SafetyLevel,
}

/// Cleanup result for a single branch (T012: remote fields added)
#[derive(Debug, Clone, Serialize)]
pub struct CleanupResult {
    pub branch: String,
    pub success: bool,
    pub error: Option<String>,
    pub remote_success: Option<bool>,
    pub remote_error: Option<String>,
}

/// Progress event payload emitted per-branch during cleanup (T015: remote_status added)
#[derive(Debug, Clone, Serialize)]
pub struct CleanupProgressPayload {
    pub branch: String,
    pub status: String,
    pub error: Option<String>,
    pub remote_status: Option<String>,
}

/// Completed event payload emitted when batch cleanup finishes
#[derive(Debug, Clone, Serialize)]
pub struct CleanupCompletedPayload {
    pub results: Vec<CleanupResult>,
}

/// Worktrees-changed event payload
#[derive(Debug, Clone, Serialize)]
pub struct WorktreesChangedPayload {
    pub project_path: String,
    pub branch: String,
}

/// Cleanup settings for a project (T018-T019)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupSettings {
    pub delete_remote_branches: bool,
}

/// Determine the safety level for a worktree (FR-500, gwt-spec issue T016-T017)
///
/// When `delete_remote` is true, PR status is integrated into the judgment:
/// - Local Safe + PR Merged/Closed → Safe
/// - Local Safe + PR Open/None → Warning
/// - Local Warning or above → kept as-is
/// - When `delete_remote` is false → legacy local-only logic
fn compute_safety_level(
    is_protected: bool,
    is_current: bool,
    is_agent_running: bool,
    has_changes: bool,
    has_unpushed: bool,
    delete_remote: bool,
    pr_status: Option<PrStatus>,
) -> SafetyLevel {
    if is_protected || is_current || is_agent_running {
        return SafetyLevel::Disabled;
    }

    let local_level = match (has_changes, has_unpushed) {
        (false, false) => SafetyLevel::Safe,
        (true, true) => SafetyLevel::Danger,
        _ => SafetyLevel::Warning,
    };

    if !delete_remote {
        return local_level;
    }

    // Integrate PR status only when local is Safe
    if local_level != SafetyLevel::Safe {
        return local_level;
    }

    match pr_status {
        Some(PrStatus::Merged) => SafetyLevel::Safe,
        Some(PrStatus::Open) | Some(PrStatus::Closed) | Some(PrStatus::None) => {
            SafetyLevel::Warning
        }
        // Unknown or no info → don't downgrade
        _ => SafetyLevel::Safe,
    }
}

/// Resolve agent status string from session file (gwt-spec issue FR-811).
/// For agents without Hook support, infers status from pane output.
fn resolve_agent_status_for_worktree(
    worktree_path: &Path,
    repo_path: &Path,
    branch_name: &str,
    state: &AppState,
) -> String {
    match Session::load_for_worktree(worktree_path) {
        Some(mut session) => {
            session.check_idle_timeout();

            let status = if agent_has_hook_support(session.agent.as_deref()) {
                session.status
            } else if let Some(pane_id) = find_pane_for_branch(state, repo_path, branch_name) {
                infer_status_from_pane(state, &pane_id)
            } else {
                session.status
            };

            match status {
                gwt_core::config::AgentStatus::Running => "running".to_string(),
                gwt_core::config::AgentStatus::WaitingInput => "waiting_input".to_string(),
                gwt_core::config::AgentStatus::Stopped => "stopped".to_string(),
                gwt_core::config::AgentStatus::Unknown => "unknown".to_string(),
            }
        }
        None => "unknown".to_string(),
    }
}

/// Find pane_id for a branch, preferring a running pane when multiple panes match.
fn find_pane_for_branch(state: &AppState, repo_path: &Path, branch_name: &str) -> Option<String> {
    let panes_info: Vec<(String, String, bool)> = {
        let manager = state.pane_manager.lock().ok()?;
        manager
            .panes()
            .iter()
            .map(|pane| {
                (
                    pane.pane_id().to_string(),
                    pane.branch_name().to_string(),
                    matches!(pane.status(), PaneStatus::Running),
                )
            })
            .collect()
    };

    let launch_meta = state.pane_launch_meta.lock().ok()?;
    let panes = panes_info.into_iter().map(|(pane_id, branch, is_running)| {
        let same_repo = launch_meta
            .get(&pane_id)
            .map(|meta| meta.repo_path.as_path() == repo_path)
            .unwrap_or(false);
        (pane_id, branch, is_running, same_repo)
    });

    select_preferred_pane_for_branch(panes, branch_name)
}

fn select_preferred_pane_for_branch<I>(panes: I, branch_name: &str) -> Option<String>
where
    I: IntoIterator<Item = (String, String, bool, bool)>,
{
    let mut fallback: Option<String> = None;
    for (pane_id, pane_branch, is_running, same_repo) in panes {
        if pane_branch != branch_name || !same_repo {
            continue;
        }

        if is_running {
            return Some(pane_id);
        }

        if fallback.is_none() {
            fallback = Some(pane_id);
        }
    }

    fallback
}

/// Infer agent status from a pane's scrollback tail.
fn infer_status_from_pane(state: &AppState, pane_id: &str) -> gwt_core::config::AgentStatus {
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

/// Get the set of branch names that have a running agent pane
fn running_agent_branches(state: &AppState) -> HashSet<String> {
    let mut branches = HashSet::new();
    if let Ok(manager) = state.pane_manager.lock() {
        for pane in manager.panes() {
            if matches!(pane.status(), gwt_core::terminal::pane::PaneStatus::Running) {
                branches.insert(pane.branch_name().to_string());
            }
        }
    }
    branches
}

fn list_worktrees_impl(project_path: &str, state: &AppState) -> Result<Vec<WorktreeInfo>, String> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let last_tool = build_last_tool_usage_map(&repo_path);

    let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
    let worktrees = manager.list().map_err(|e| e.to_string())?;

    // Get branch info for ahead/behind/is_gone/is_current
    let branches = Branch::list(&repo_path).unwrap_or_default();
    let current_branch = branches
        .iter()
        .find(|b| b.is_current)
        .map(|b| b.name.clone());

    let agent_branches = running_agent_branches(state);

    let mut infos: Vec<WorktreeInfo> = worktrees
        .into_iter()
        .filter_map(|wt| {
            let branch_name = wt.branch.as_deref()?;
            let branch_info = branches.iter().find(|b| b.name == branch_name);

            let is_current = current_branch.as_deref() == Some(branch_name);
            let is_protected = WorktreeManager::is_protected(branch_name);
            let is_agent_running = agent_branches.contains(branch_name);

            // Read agent status from session file (gwt-spec issue FR-811)
            let agent_status =
                resolve_agent_status_for_worktree(&wt.path, &repo_path, branch_name, state);

            let ahead = branch_info.map(|b| b.ahead).unwrap_or(0);
            let behind = branch_info.map(|b| b.behind).unwrap_or(0);
            let is_gone = branch_info.map(|b| b.is_gone).unwrap_or(false);

            let safety_level = compute_safety_level(
                is_protected,
                is_current,
                is_agent_running,
                wt.has_changes,
                wt.has_unpushed,
                false,
                None,
            );

            Some(WorktreeInfo {
                path: wt.path.to_string_lossy().to_string(),
                branch: branch_name.to_string(),
                commit: wt.commit.clone(),
                status: wt.status.to_string(),
                is_main: wt.is_main,
                has_changes: wt.has_changes,
                has_unpushed: wt.has_unpushed,
                is_current,
                is_protected,
                is_agent_running,
                agent_status,
                ahead,
                behind,
                is_gone,
                last_tool_usage: last_tool.get(branch_name).cloned(),
                safety_level,
            })
        })
        .collect();

    // Sort by safety level: safe → warning → danger → disabled (FR-503)
    infos.sort_by_key(|w| w.safety_level);

    Ok(infos)
}

/// List all worktrees with safety info (gwt-spec issue T1)
#[tauri::command]
pub async fn list_worktrees(
    project_path: String,
    app_handle: AppHandle,
) -> Result<Vec<WorktreeInfo>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        list_worktrees_impl(&project_path, &state)
            .map_err(|e| StructuredError::internal(&e, "list_worktrees"))
    })
    .await
    .map_err(|e| {
        StructuredError::internal(&format!("Failed to list worktrees: {e}"), "list_worktrees")
    })?
}

/// Check gh CLI availability (gwt-spec issue T010)
#[tauri::command]
pub async fn check_gh_available(
    state: tauri::State<'_, AppState>,
) -> Result<bool, StructuredError> {
    let available = tauri::async_runtime::spawn_blocking(gwt_core::git::gh_cli::check_auth)
        .await
        .map_err(|e| {
            StructuredError::internal(
                &format!("Failed to check gh availability: {e}"),
                "check_gh_available",
            )
        })?;
    state.gh_available.store(available, Ordering::Relaxed);
    Ok(available)
}

/// Get PR statuses for cleanup (gwt-spec issue T011)
#[tauri::command]
pub async fn get_cleanup_pr_statuses(
    project_path: String,
) -> Result<HashMap<String, String>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_cleanup_pr_statuses"))?;

    let result = tauri::async_runtime::spawn_blocking(move || {
        gwt_core::git::gh_cli::get_pr_statuses(&repo_path)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Failed to get PR statuses: {e}"),
            "get_cleanup_pr_statuses",
        )
    })?;

    // Convert PrStatus enum to string for frontend
    Ok(result
        .into_iter()
        .map(|(branch, status)| {
            let status_str = match status {
                PrStatus::Merged => "merged",
                PrStatus::Open => "open",
                PrStatus::Closed => "closed",
                PrStatus::None => "none",
                PrStatus::Unknown => "unknown",
            };
            (branch, status_str.to_string())
        })
        .collect())
}

/// Get branch deletion protection info for cleanup (#1404).
///
/// Returns branch names that cannot be deleted remotely due to repository rules.
#[tauri::command]
pub async fn get_cleanup_branch_protection(
    project_path: String,
    branches: Vec<String>,
) -> Result<Vec<String>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_cleanup_branch_protection"))?;

    let result = tauri::async_runtime::spawn_blocking(move || {
        let branch_refs: Vec<&str> = branches.iter().map(|s| s.as_str()).collect();
        gwt_core::git::gh_cli::get_branch_deletion_rules(&repo_path, &branch_refs)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Failed to get branch protection: {e}"),
            "get_cleanup_branch_protection",
        )
    })?;

    Ok(result.into_iter().collect())
}

/// Get cleanup settings for a project (gwt-spec issue T019)
#[tauri::command]
pub async fn get_cleanup_settings(
    project_path: String,
) -> Result<CleanupSettings, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_cleanup_settings"))?;
    Ok(load_cleanup_settings(&repo_path))
}

/// Set cleanup settings for a project (gwt-spec issue T019)
#[tauri::command]
pub async fn set_cleanup_settings(
    project_path: String,
    settings: CleanupSettings,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "set_cleanup_settings"))?;
    save_cleanup_settings(&repo_path, &settings)
        .map_err(|e| StructuredError::internal(&e, "set_cleanup_settings"))
}

/// Cleanup multiple worktrees (gwt-spec issue T2, gwt-spec issue T013-T014)
#[tauri::command]
pub async fn cleanup_worktrees(
    project_path: String,
    branches: Vec<String>,
    force: bool,
    delete_remote: bool,
    state: tauri::State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Vec<CleanupResult>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "cleanup_worktrees"))?;

    // Collect gone branches for skipping remote deletion
    let branch_list = Branch::list(&repo_path).unwrap_or_default();
    let gone_branches: HashSet<String> = branch_list
        .iter()
        .filter(|b| b.is_gone)
        .map(|b| b.name.clone())
        .collect();

    let agent_branches = running_agent_branches(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let manager = WorktreeManager::new(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "cleanup_worktrees"))?;
        let mut results = Vec::with_capacity(branches.len());

        for branch in &branches {
            // Emit deleting progress
            let _ = app_handle.emit(
                "cleanup-progress",
                &CleanupProgressPayload {
                    branch: branch.clone(),
                    status: "deleting".to_string(),
                    error: None,
                    remote_status: None,
                },
            );

            let local_result =
                cleanup_single_branch(&manager, &repo_path, branch, force, &agent_branches);

            let cleanup_result = match local_result {
                Ok(()) => {
                    // Local deletion succeeded; attempt remote if requested
                    let (remote_success, remote_error, remote_status) =
                        if delete_remote && !gone_branches.contains(branch.as_str()) {
                            let _ = app_handle.emit(
                                "cleanup-progress",
                                &CleanupProgressPayload {
                                    branch: branch.clone(),
                                    status: "deleting".to_string(),
                                    error: None,
                                    remote_status: Some("deleting".to_string()),
                                },
                            );

                            match gwt_core::git::gh_cli::delete_remote_branch(&repo_path, branch) {
                                Ok(()) => (Some(true), None, Some("deleted".to_string())),
                                Err(e)
                                    if e.starts_with(
                                        gwt_core::git::gh_cli::PROTECTED_BRANCH_PREFIX,
                                    ) =>
                                {
                                    (Some(true), Some(e), Some("skipped".to_string()))
                                }
                                Err(e) => (Some(false), Some(e), Some("failed".to_string())),
                            }
                        } else if delete_remote && gone_branches.contains(branch.as_str()) {
                            // gone branch: remote already deleted
                            (Some(true), None, Some("skipped_gone".to_string()))
                        } else {
                            (None, None, None)
                        };

                    let _ = app_handle.emit(
                        "cleanup-progress",
                        &CleanupProgressPayload {
                            branch: branch.clone(),
                            status: "deleted".to_string(),
                            error: None,
                            remote_status: remote_status.clone(),
                        },
                    );

                    CleanupResult {
                        branch: branch.clone(),
                        success: true,
                        error: None,
                        remote_success,
                        remote_error,
                    }
                }
                Err(err) => {
                    let _ = app_handle.emit(
                        "cleanup-progress",
                        &CleanupProgressPayload {
                            branch: branch.clone(),
                            status: "failed".to_string(),
                            error: Some(err.clone()),
                            remote_status: None,
                        },
                    );
                    CleanupResult {
                        branch: branch.clone(),
                        success: false,
                        error: Some(err),
                        remote_success: None,
                        remote_error: None,
                    }
                }
            };

            results.push(cleanup_result);
        }

        // Emit cleanup-completed
        let _ = app_handle.emit(
            "cleanup-completed",
            &CleanupCompletedPayload {
                results: results.clone(),
            },
        );

        // Emit worktrees-changed
        let _ = app_handle.emit(
            "worktrees-changed",
            &WorktreesChangedPayload {
                project_path: project_path.clone(),
                branch: String::new(),
            },
        );

        Ok(results)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Failed to execute cleanup task: {e}"),
            "cleanup_worktrees",
        )
    })?
}

/// Cleanup a single worktree (gwt-spec issue T3)
///
/// **Deprecated**: Use `cleanup_worktrees` with a single-element `branches` list instead.
/// This command is retained for backward compatibility but the frontend now routes
/// all deletions through `CleanupModal` → `cleanup_worktrees` (gwt-spec issue FR-612).
#[deprecated(note = "Use cleanup_worktrees with a single branch instead (gwt-spec issue FR-612)")]
#[tauri::command]
pub async fn cleanup_single_worktree(
    project_path: String,
    branch: String,
    force: bool,
    state: tauri::State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "cleanup_single_worktree"))?;

    let agent_branches = running_agent_branches(&state);
    let branch_for_event = branch.clone();
    let project_path_for_event = project_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = WorktreeManager::new(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "cleanup_single_worktree"))?;
        cleanup_single_branch(&manager, &repo_path, &branch, force, &agent_branches)
            .map_err(|e| StructuredError::internal(&e, "cleanup_single_worktree"))
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Failed to execute cleanup task: {e}"),
            "cleanup_single_worktree",
        )
    })??;

    // Emit worktrees-changed
    let _ = app_handle.emit(
        "worktrees-changed",
        &WorktreesChangedPayload {
            project_path: project_path_for_event,
            branch: branch_for_event,
        },
    );

    Ok(())
}

/// Internal: cleanup a single branch (worktree + local branch)
fn cleanup_single_branch(
    manager: &WorktreeManager,
    repo_path: &Path,
    branch: &str,
    force: bool,
    agent_branches: &HashSet<String>,
) -> Result<(), String> {
    // Force mode only applies to unsafe local state (e.g. uncommitted changes).
    // It must never bypass protected/current/running-agent guards.

    // Check if this is the current worktree
    let branches = Branch::list(repo_path).unwrap_or_default();
    if branches.iter().any(|b| b.name == branch && b.is_current) {
        return Err(format!("Cannot delete current worktree: {}", branch));
    }

    // Reject protected branches
    if WorktreeManager::is_protected(branch) {
        return Err(format!("Cannot delete protected branch: {}", branch));
    }

    // Reject agent-running branches
    if agent_branches.contains(branch) {
        return Err(format!(
            "Cannot delete branch with running agent: {}",
            branch
        ));
    }

    // Use cleanup_branch which handles worktree + branch deletion
    manager
        .cleanup_branch(branch, force, force)
        .map_err(|e| e.to_string())
}

/// Load cleanup settings from `.gwt/cleanup_settings.json` (T018)
fn load_cleanup_settings(repo_path: &Path) -> CleanupSettings {
    let settings_path = repo_path.join(".gwt").join("cleanup_settings.json");
    match std::fs::read_to_string(&settings_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => CleanupSettings::default(),
    }
}

/// Save cleanup settings to `.gwt/cleanup_settings.json` (T018)
fn save_cleanup_settings(repo_path: &Path, settings: &CleanupSettings) -> Result<(), String> {
    let gwt_dir = repo_path.join(".gwt");
    if !gwt_dir.exists() {
        std::fs::create_dir_all(&gwt_dir)
            .map_err(|e| format!("Failed to create .gwt directory: {}", e))?;
    }
    let settings_path = gwt_dir.join("cleanup_settings.json");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&settings_path, json)
        .map_err(|e| format!("Failed to write cleanup settings: {}", e))
}

fn build_last_tool_usage_map(repo_path: &Path) -> std::collections::HashMap<String, String> {
    gwt_core::config::get_last_tool_usage_map(repo_path)
        .into_iter()
        .map(|(branch, entry)| (branch, entry.format_tool_usage()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Serialization contract tests (gwt-spec issue) --

    #[test]
    fn worktree_info_serializes_with_snake_case_keys() {
        let info = WorktreeInfo {
            path: "/tmp/wt".to_string(),
            branch: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            status: "active".to_string(),
            is_main: false,
            has_changes: true,
            has_unpushed: false,
            is_current: false,
            is_protected: false,
            is_agent_running: false,
            agent_status: "unknown".to_string(),
            ahead: 1,
            behind: 0,
            is_gone: true,
            last_tool_usage: Some("Claude 2m ago".to_string()),
            safety_level: SafetyLevel::Warning,
        };
        let json = serde_json::to_value(&info).unwrap();
        let obj = json.as_object().unwrap();

        // All multi-word fields must be snake_case (matching TypeScript types.ts)
        assert!(
            obj.contains_key("safety_level"),
            "expected snake_case key 'safety_level'"
        );
        assert!(
            obj.contains_key("has_changes"),
            "expected snake_case key 'has_changes'"
        );
        assert!(
            obj.contains_key("has_unpushed"),
            "expected snake_case key 'has_unpushed'"
        );
        assert!(
            obj.contains_key("is_main"),
            "expected snake_case key 'is_main'"
        );
        assert!(
            obj.contains_key("is_current"),
            "expected snake_case key 'is_current'"
        );
        assert!(
            obj.contains_key("is_protected"),
            "expected snake_case key 'is_protected'"
        );
        assert!(
            obj.contains_key("is_agent_running"),
            "expected snake_case key 'is_agent_running'"
        );
        assert!(
            obj.contains_key("is_gone"),
            "expected snake_case key 'is_gone'"
        );
        assert!(
            obj.contains_key("last_tool_usage"),
            "expected snake_case key 'last_tool_usage'"
        );

        // camelCase keys must NOT exist
        assert!(
            !obj.contains_key("safetyLevel"),
            "unexpected camelCase key 'safetyLevel'"
        );
        assert!(
            !obj.contains_key("hasChanges"),
            "unexpected camelCase key 'hasChanges'"
        );
        assert!(
            !obj.contains_key("isGone"),
            "unexpected camelCase key 'isGone'"
        );

        // SafetyLevel enum value must be lowercase
        assert_eq!(json["safety_level"], "warning");
    }

    #[test]
    fn worktrees_changed_payload_serializes_with_snake_case_keys() {
        let payload = WorktreesChangedPayload {
            project_path: "/tmp/project".to_string(),
            branch: "main".to_string(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        let obj = json.as_object().unwrap();

        assert!(
            obj.contains_key("project_path"),
            "expected snake_case key 'project_path'"
        );
        assert!(
            !obj.contains_key("projectPath"),
            "unexpected camelCase key 'projectPath'"
        );
    }

    // -- agent_status field serialization tests (gwt-spec issue) --

    #[test]
    fn worktree_info_agent_status_serializes_in_json() {
        let info = WorktreeInfo {
            path: "/tmp/wt".to_string(),
            branch: "feature/agent".to_string(),
            commit: "def5678".to_string(),
            status: "active".to_string(),
            is_main: false,
            has_changes: false,
            has_unpushed: false,
            is_current: false,
            is_protected: false,
            is_agent_running: true,
            agent_status: "running".to_string(),
            ahead: 0,
            behind: 0,
            is_gone: false,
            last_tool_usage: None,
            safety_level: SafetyLevel::Disabled,
        };
        let json = serde_json::to_value(&info).unwrap();
        let obj = json.as_object().unwrap();

        // agent_status field must exist and be snake_case
        assert!(
            obj.contains_key("agent_status"),
            "expected snake_case key 'agent_status'"
        );
        assert!(
            !obj.contains_key("agentStatus"),
            "unexpected camelCase key 'agentStatus'"
        );
        assert_eq!(json["agent_status"], "running");
    }

    #[test]
    fn worktree_info_agent_status_all_values() {
        for (status_str, expected) in [
            ("unknown", "unknown"),
            ("running", "running"),
            ("waiting_input", "waiting_input"),
            ("stopped", "stopped"),
        ] {
            let info = WorktreeInfo {
                path: "/tmp/wt".to_string(),
                branch: "test".to_string(),
                commit: "abc".to_string(),
                status: "active".to_string(),
                is_main: false,
                has_changes: false,
                has_unpushed: false,
                is_current: false,
                is_protected: false,
                is_agent_running: false,
                agent_status: status_str.to_string(),
                ahead: 0,
                behind: 0,
                is_gone: false,
                last_tool_usage: None,
                safety_level: SafetyLevel::Safe,
            };
            let json = serde_json::to_value(&info).unwrap();
            assert_eq!(
                json["agent_status"], expected,
                "agent_status mismatch for '{}'",
                status_str
            );
        }
    }

    #[test]
    fn select_preferred_pane_for_branch_prefers_running() {
        let panes = vec![
            (
                "pane-stopped".to_string(),
                "feature/a".to_string(),
                false,
                true,
            ),
            (
                "pane-running".to_string(),
                "feature/a".to_string(),
                true,
                true,
            ),
        ];

        let selected = select_preferred_pane_for_branch(panes, "feature/a");
        assert_eq!(selected.as_deref(), Some("pane-running"));
    }

    #[test]
    fn select_preferred_pane_for_branch_falls_back_to_first_non_running() {
        let panes = vec![
            ("pane-old".to_string(), "feature/a".to_string(), false, true),
            ("pane-new".to_string(), "feature/a".to_string(), false, true),
        ];

        let selected = select_preferred_pane_for_branch(panes, "feature/a");
        assert_eq!(selected.as_deref(), Some("pane-old"));
    }

    #[test]
    fn select_preferred_pane_for_branch_returns_none_for_unknown_branch() {
        let panes = vec![("pane".to_string(), "feature/a".to_string(), true, true)];

        let selected = select_preferred_pane_for_branch(panes, "feature/b");
        assert!(selected.is_none());
    }

    #[test]
    fn select_preferred_pane_for_branch_ignores_other_repo() {
        let panes = vec![
            (
                "pane-other-repo-running".to_string(),
                "feature/a".to_string(),
                true,
                false,
            ),
            (
                "pane-this-repo".to_string(),
                "feature/a".to_string(),
                false,
                true,
            ),
        ];

        let selected = select_preferred_pane_for_branch(panes, "feature/a");
        assert_eq!(selected.as_deref(), Some("pane-this-repo"));
    }

    // -- SafetyLevel computation tests (T1) — backward compatible (delete_remote=false) --

    #[test]
    fn safe_when_no_changes_and_no_unpushed() {
        assert_eq!(
            compute_safety_level(false, false, false, false, false, false, None),
            SafetyLevel::Safe
        );
    }

    #[test]
    fn warning_when_unpushed_only() {
        assert_eq!(
            compute_safety_level(false, false, false, false, true, false, None),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn warning_when_changes_only() {
        assert_eq!(
            compute_safety_level(false, false, false, true, false, false, None),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn danger_when_both_changes_and_unpushed() {
        assert_eq!(
            compute_safety_level(false, false, false, true, true, false, None),
            SafetyLevel::Danger
        );
    }

    #[test]
    fn disabled_when_protected() {
        assert_eq!(
            compute_safety_level(true, false, false, false, false, false, None),
            SafetyLevel::Disabled
        );
    }

    #[test]
    fn disabled_when_current() {
        assert_eq!(
            compute_safety_level(false, true, false, false, false, false, None),
            SafetyLevel::Disabled
        );
    }

    #[test]
    fn disabled_when_agent_running() {
        assert_eq!(
            compute_safety_level(false, false, true, false, false, false, None),
            SafetyLevel::Disabled
        );
    }

    #[test]
    fn safety_level_sort_order() {
        let mut levels = vec![
            SafetyLevel::Danger,
            SafetyLevel::Safe,
            SafetyLevel::Disabled,
            SafetyLevel::Warning,
        ];
        levels.sort();
        assert_eq!(
            levels,
            vec![
                SafetyLevel::Safe,
                SafetyLevel::Warning,
                SafetyLevel::Danger,
                SafetyLevel::Disabled,
            ]
        );
    }

    // -- T016-T017: Integrated safety level with PR status --

    #[test]
    fn integrated_safe_with_merged_pr() {
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                false,
                false,
                true,
                Some(PrStatus::Merged)
            ),
            SafetyLevel::Safe
        );
    }

    #[test]
    fn integrated_safe_with_closed_pr_downgrades_to_warning() {
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                false,
                false,
                true,
                Some(PrStatus::Closed)
            ),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn integrated_safe_with_open_pr_downgrades_to_warning() {
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                false,
                false,
                true,
                Some(PrStatus::Open)
            ),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn integrated_safe_with_no_pr_downgrades_to_warning() {
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                false,
                false,
                true,
                Some(PrStatus::None)
            ),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn integrated_warning_stays_warning_regardless_of_pr() {
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                true,
                false,
                true,
                Some(PrStatus::Merged)
            ),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn integrated_danger_stays_danger_regardless_of_pr() {
        assert_eq!(
            compute_safety_level(false, false, false, true, true, true, Some(PrStatus::Open)),
            SafetyLevel::Danger
        );
    }

    #[test]
    fn toggle_off_ignores_pr_status() {
        // With delete_remote=false, PR status is irrelevant
        assert_eq!(
            compute_safety_level(
                false,
                false,
                false,
                false,
                false,
                false,
                Some(PrStatus::Open)
            ),
            SafetyLevel::Safe
        );
    }

    #[test]
    fn disabled_stays_disabled_regardless_of_pr() {
        assert_eq!(
            compute_safety_level(
                true,
                false,
                false,
                false,
                false,
                true,
                Some(PrStatus::Merged)
            ),
            SafetyLevel::Disabled
        );
    }

    // -- T012: CleanupResult serialization tests --

    #[test]
    fn cleanup_result_serializes_with_remote_fields() {
        let result = CleanupResult {
            branch: "feature/test".to_string(),
            success: true,
            error: None,
            remote_success: Some(true),
            remote_error: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("remote_success"));
        assert!(obj.contains_key("remote_error"));
        assert_eq!(json["remote_success"], true);
        assert!(json["remote_error"].is_null());
    }

    #[test]
    fn cleanup_result_remote_none_when_toggle_off() {
        let result = CleanupResult {
            branch: "feature/test".to_string(),
            success: true,
            error: None,
            remote_success: None,
            remote_error: None,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json["remote_success"].is_null());
        assert!(json["remote_error"].is_null());
    }

    #[test]
    fn cleanup_result_remote_failure() {
        let result = CleanupResult {
            branch: "feature/test".to_string(),
            success: true,
            error: None,
            remote_success: Some(false),
            remote_error: Some("Permission denied".to_string()),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["remote_success"], false);
        assert_eq!(json["remote_error"], "Permission denied");
    }

    // -- T015: CleanupProgressPayload includes remote_status --

    #[test]
    fn progress_event_includes_remote_status() {
        let payload = CleanupProgressPayload {
            branch: "feature/test".to_string(),
            status: "deleted".to_string(),
            error: None,
            remote_status: Some("deleted".to_string()),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["remote_status"], "deleted");
    }

    #[test]
    fn progress_event_remote_status_none() {
        let payload = CleanupProgressPayload {
            branch: "feature/test".to_string(),
            status: "deleted".to_string(),
            error: None,
            remote_status: None,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json["remote_status"].is_null());
    }

    // -- T018-T019: Cleanup settings persistence --

    #[test]
    fn cleanup_settings_default() {
        let settings = CleanupSettings::default();
        assert!(!settings.delete_remote_branches);
    }

    #[test]
    fn cleanup_settings_serialization_roundtrip() {
        let settings = CleanupSettings {
            delete_remote_branches: true,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: CleanupSettings = serde_json::from_str(&json).unwrap();
        assert!(deserialized.delete_remote_branches);
    }

    #[test]
    fn load_cleanup_settings_returns_default_when_missing() {
        let temp = tempfile::TempDir::new().unwrap();
        let settings = load_cleanup_settings(temp.path());
        assert!(!settings.delete_remote_branches);
    }

    #[test]
    fn save_and_load_cleanup_settings() {
        let temp = tempfile::TempDir::new().unwrap();
        let settings = CleanupSettings {
            delete_remote_branches: true,
        };
        save_cleanup_settings(temp.path(), &settings).unwrap();

        let loaded = load_cleanup_settings(temp.path());
        assert!(loaded.delete_remote_branches);
    }

    #[test]
    fn save_cleanup_settings_creates_gwt_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        let gwt_dir = temp.path().join(".gwt");
        assert!(!gwt_dir.exists());

        save_cleanup_settings(
            temp.path(),
            &CleanupSettings {
                delete_remote_branches: false,
            },
        )
        .unwrap();
        assert!(gwt_dir.exists());
    }

    // -- Integration tests (existing) --

    #[test]
    fn list_worktrees_impl_returns_main_worktree_for_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let state = AppState::new();
        let project_path = temp.path().to_string_lossy().to_string();
        let current = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);

        let infos = list_worktrees_impl(&project_path, &state).unwrap();
        assert!(!infos.is_empty());
        assert!(infos.iter().any(|wt| wt.branch == current));
    }

    #[test]
    fn protected_branch_detected() {
        assert!(WorktreeManager::is_protected("main"));
        assert!(WorktreeManager::is_protected("master"));
        assert!(WorktreeManager::is_protected("develop"));
        assert!(WorktreeManager::is_protected("release"));
        assert!(!WorktreeManager::is_protected("feature/foo"));
    }

    // -- cleanup_single_branch guard tests (T2/T3) --

    #[test]
    fn rejects_protected_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        // Create "develop" (protected) branch so it exists but isn't current
        gwt_core::git::Branch::create(temp.path(), "develop", "HEAD").unwrap();
        let result = cleanup_single_branch(&manager, temp.path(), "develop", false, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("protected"));
    }

    #[test]
    fn force_still_rejects_protected_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        gwt_core::git::Branch::create(temp.path(), "develop", "HEAD").unwrap();
        let result = cleanup_single_branch(&manager, temp.path(), "develop", true, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("protected"));
    }

    #[test]
    fn rejects_agent_running_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let mut agents = HashSet::new();
        agents.insert("feature/test".to_string());

        // Create branch + worktree
        gwt_core::git::Branch::create(temp.path(), "feature/test", "HEAD").unwrap();
        let _wt = manager.create_for_branch("feature/test").unwrap();

        let result = cleanup_single_branch(&manager, temp.path(), "feature/test", false, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("running agent"));
    }

    #[test]
    fn force_still_rejects_agent_running_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let mut agents = HashSet::new();
        agents.insert("feature/test".to_string());

        gwt_core::git::Branch::create(temp.path(), "feature/test", "HEAD").unwrap();
        let _wt = manager.create_for_branch("feature/test").unwrap();

        let result = cleanup_single_branch(&manager, temp.path(), "feature/test", true, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("running agent"));
    }

    #[test]
    fn rejects_current_worktree() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        // The default branch is the current one
        let current = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        let result = cleanup_single_branch(&manager, temp.path(), &current, false, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("current"));
    }

    #[test]
    fn force_still_rejects_current_worktree() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        let current = git_stdout(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
        let result = cleanup_single_branch(&manager, temp.path(), &current, true, &agents);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("current"));
    }

    #[test]
    fn successful_cleanup_removes_worktree_and_branch() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        gwt_core::git::Branch::create(temp.path(), "feature/done", "HEAD").unwrap();
        let wt = manager.create_for_branch("feature/done").unwrap();
        assert!(wt.path.exists());

        let result = cleanup_single_branch(&manager, temp.path(), "feature/done", false, &agents);
        assert!(result.is_ok());
        assert!(!gwt_core::git::Branch::exists(temp.path(), "feature/done").unwrap());
    }

    #[test]
    fn skips_failure_and_continues_in_batch() {
        // Simulate batch: protected branch fails, normal branch succeeds
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        gwt_core::git::Branch::create(temp.path(), "feature/ok", "HEAD").unwrap();
        manager.create_for_branch("feature/ok").unwrap();

        let r1 = cleanup_single_branch(&manager, temp.path(), "main", false, &agents);
        let r2 = cleanup_single_branch(&manager, temp.path(), "feature/ok", false, &agents);

        assert!(r1.is_err()); // protected
        assert!(r2.is_ok()); // cleaned up
    }

    #[test]
    fn force_deletes_unsafe_worktree() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        gwt_core::git::Branch::create(temp.path(), "feature/wip", "HEAD").unwrap();
        let wt = manager.create_for_branch("feature/wip").unwrap();

        // Make the worktree dirty (uncommitted changes)
        std::fs::write(wt.path.join("dirty.txt"), "unsaved work").unwrap();

        // force=true should succeed even with uncommitted changes
        let result = cleanup_single_branch(&manager, temp.path(), "feature/wip", true, &agents);
        assert!(result.is_ok());
        assert!(!gwt_core::git::Branch::exists(temp.path(), "feature/wip").unwrap());
    }

    #[test]
    fn cleanup_single_branch_auto_forces_unmerged_when_force_false() {
        let temp = tempfile::TempDir::new().unwrap();
        create_test_repo(temp.path());
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let agents = HashSet::new();

        gwt_core::git::Branch::create(temp.path(), "feature/unmerged", "HEAD").unwrap();
        let wt = manager.create_for_branch("feature/unmerged").unwrap();

        std::fs::write(wt.path.join("unmerged.txt"), "unmerged").unwrap();
        let add_output = gwt_core::process::git_command()
            .args(["add", "."])
            .current_dir(&wt.path)
            .output()
            .unwrap();
        assert!(add_output.status.success());

        let commit_output = gwt_core::process::git_command()
            .args(["commit", "-m", "unmerged commit"])
            .current_dir(&wt.path)
            .output()
            .unwrap();
        assert!(commit_output.status.success());

        let result =
            cleanup_single_branch(&manager, temp.path(), "feature/unmerged", false, &agents);
        assert!(result.is_ok());
        assert!(!gwt_core::git::Branch::exists(temp.path(), "feature/unmerged").unwrap());
    }

    // -- Test helpers --

    fn create_test_repo(path: &std::path::Path) {
        gwt_core::process::command("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        gwt_core::process::command("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .unwrap();
        gwt_core::process::command("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .unwrap();
        std::fs::write(path.join("test.txt"), "hello").unwrap();
        gwt_core::process::command("git")
            .args(["add", "."])
            .current_dir(path)
            .output()
            .unwrap();
        gwt_core::process::command("git")
            .args(["commit", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    fn git_stdout(dir: &std::path::Path, args: &[&str]) -> String {
        let output = gwt_core::process::command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
