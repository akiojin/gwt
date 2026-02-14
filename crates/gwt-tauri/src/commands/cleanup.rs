//! Worktree cleanup commands (SPEC-c4e8f210)

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;
use gwt_core::git::Branch;
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use tauri::{AppHandle, Emitter};

/// Safety level for a worktree (FR-500)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyLevel {
    Safe,
    Warning,
    Danger,
    Disabled,
}

/// Worktree info for the frontend (SPEC-c4e8f210)
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
    pub ahead: usize,
    pub behind: usize,
    pub is_gone: bool,
    pub last_tool_usage: Option<String>,
    pub safety_level: SafetyLevel,
}

/// Cleanup result for a single branch
#[derive(Debug, Clone, Serialize)]
pub struct CleanupResult {
    pub branch: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Progress event payload emitted per-branch during cleanup
#[derive(Debug, Clone, Serialize)]
pub struct CleanupProgressPayload {
    pub branch: String,
    pub status: String,
    pub error: Option<String>,
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

/// Determine the safety level for a worktree (FR-500)
fn compute_safety_level(
    is_protected: bool,
    is_current: bool,
    is_agent_running: bool,
    has_changes: bool,
    has_unpushed: bool,
) -> SafetyLevel {
    if is_protected || is_current || is_agent_running {
        return SafetyLevel::Disabled;
    }
    match (has_changes, has_unpushed) {
        (false, false) => SafetyLevel::Safe,
        (true, true) => SafetyLevel::Danger,
        _ => SafetyLevel::Warning,
    }
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

/// List all worktrees with safety info (SPEC-c4e8f210 T1)
#[tauri::command]
pub fn list_worktrees(
    project_path: String,
    state: tauri::State<AppState>,
) -> Result<Vec<WorktreeInfo>, String> {
    let project_root = Path::new(&project_path);
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

    let agent_branches = running_agent_branches(&state);

    let mut infos: Vec<WorktreeInfo> = worktrees
        .into_iter()
        .filter_map(|wt| {
            let branch_name = wt.branch.as_deref()?;
            let branch_info = branches.iter().find(|b| b.name == branch_name);

            let is_current = current_branch.as_deref() == Some(branch_name);
            let is_protected = WorktreeManager::is_protected(branch_name);
            let is_agent_running = agent_branches.contains(branch_name);

            let ahead = branch_info.map(|b| b.ahead).unwrap_or(0);
            let behind = branch_info.map(|b| b.behind).unwrap_or(0);
            let is_gone = branch_info.map(|b| b.is_gone).unwrap_or(false);

            let safety_level = compute_safety_level(
                is_protected,
                is_current,
                is_agent_running,
                wt.has_changes,
                wt.has_unpushed,
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

/// Cleanup multiple worktrees (SPEC-c4e8f210 T2)
#[tauri::command]
pub async fn cleanup_worktrees(
    project_path: String,
    branches: Vec<String>,
    force: bool,
    state: tauri::State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Vec<CleanupResult>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let agent_branches = running_agent_branches(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
        let mut results = Vec::with_capacity(branches.len());

        for branch in &branches {
            // Emit deleting progress
            let _ = app_handle.emit(
                "cleanup-progress",
                &CleanupProgressPayload {
                    branch: branch.clone(),
                    status: "deleting".to_string(),
                    error: None,
                },
            );

            let result =
                cleanup_single_branch(&manager, &repo_path, branch, force, &agent_branches);

            let cleanup_result = match result {
                Ok(()) => {
                    let _ = app_handle.emit(
                        "cleanup-progress",
                        &CleanupProgressPayload {
                            branch: branch.clone(),
                            status: "deleted".to_string(),
                            error: None,
                        },
                    );
                    CleanupResult {
                        branch: branch.clone(),
                        success: true,
                        error: None,
                    }
                }
                Err(err) => {
                    let _ = app_handle.emit(
                        "cleanup-progress",
                        &CleanupProgressPayload {
                            branch: branch.clone(),
                            status: "failed".to_string(),
                            error: Some(err.clone()),
                        },
                    );
                    CleanupResult {
                        branch: branch.clone(),
                        success: false,
                        error: Some(err),
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
    .map_err(|e| format!("Failed to execute cleanup task: {e}"))?
}

/// Cleanup a single worktree (SPEC-c4e8f210 T3)
#[tauri::command]
pub async fn cleanup_single_worktree(
    project_path: String,
    branch: String,
    force: bool,
    state: tauri::State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let agent_branches = running_agent_branches(&state);
    let branch_for_event = branch.clone();
    let project_path_for_event = project_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
        cleanup_single_branch(&manager, &repo_path, &branch, force, &agent_branches)
    })
    .await
    .map_err(|e| format!("Failed to execute cleanup task: {e}"))??;

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

fn build_last_tool_usage_map(repo_path: &Path) -> std::collections::HashMap<String, String> {
    gwt_core::config::get_last_tool_usage_map(repo_path)
        .into_iter()
        .map(|(branch, entry)| (branch, entry.format_tool_usage()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Serialization contract tests (SPEC-d7f2a1b3) --

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

    // -- SafetyLevel computation tests (T1) --

    #[test]
    fn safe_when_no_changes_and_no_unpushed() {
        assert_eq!(
            compute_safety_level(false, false, false, false, false),
            SafetyLevel::Safe
        );
    }

    #[test]
    fn warning_when_unpushed_only() {
        assert_eq!(
            compute_safety_level(false, false, false, false, true),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn warning_when_changes_only() {
        assert_eq!(
            compute_safety_level(false, false, false, true, false),
            SafetyLevel::Warning
        );
    }

    #[test]
    fn danger_when_both_changes_and_unpushed() {
        assert_eq!(
            compute_safety_level(false, false, false, true, true),
            SafetyLevel::Danger
        );
    }

    #[test]
    fn disabled_when_protected() {
        assert_eq!(
            compute_safety_level(true, false, false, false, false),
            SafetyLevel::Disabled
        );
    }

    #[test]
    fn disabled_when_current() {
        assert_eq!(
            compute_safety_level(false, true, false, false, false),
            SafetyLevel::Disabled
        );
    }

    #[test]
    fn disabled_when_agent_running() {
        assert_eq!(
            compute_safety_level(false, false, true, false, false),
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
