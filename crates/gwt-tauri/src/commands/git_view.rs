//! GitView commands for branch diff, commits, working tree, and stash

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{
    self, FileChange, FileDiff, GitChangeSummary, GitViewCommit, Remote, StashEntry,
    WorkingTreeEntry,
};
use gwt_core::worktree::WorktreeManager;
use std::path::Path;
use std::path::PathBuf;

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn resolve_existing_worktree_path(
    repo_path: &Path,
    branch_ref: &str,
) -> Result<Option<PathBuf>, String> {
    let branch_ref = branch_ref.trim();
    if branch_ref.is_empty() {
        return Ok(None);
    }

    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let remotes = Remote::list(repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(branch_ref, &remotes);

    if let Ok(Some(wt)) = manager.get_by_branch_basic(normalized) {
        return Ok(Some(wt.path));
    }
    // Rare: worktree registered with the raw remote-like name.
    if normalized != branch_ref {
        if let Ok(Some(wt)) = manager.get_by_branch_basic(branch_ref) {
            return Ok(Some(wt.path));
        }
    }
    Ok(None)
}

fn resolve_any_active_worktree_path(repo_path: &Path) -> Result<Option<PathBuf>, String> {
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let worktrees = manager.list_basic().map_err(|e| e.to_string())?;
    Ok(worktrees
        .into_iter()
        .find(|wt| wt.is_active() && !wt.is_main)
        .map(|wt| wt.path))
}

fn resolve_git_view_exec_path(repo_path: &Path, branch_ref: &str) -> Result<PathBuf, String> {
    if !git::is_bare_repository(repo_path) {
        return Ok(repo_path.to_path_buf());
    }

    if let Some(wt) = resolve_existing_worktree_path(repo_path, branch_ref)? {
        return Ok(wt);
    }
    if let Some(wt) = resolve_any_active_worktree_path(repo_path)? {
        return Ok(wt);
    }

    // No worktree found. Some commands (diff/log) still work in bare, but worktree-only
    // commands (status/stash) will fail; callers should surface the error in that case.
    Ok(repo_path.to_path_buf())
}

#[tauri::command]
pub fn get_git_change_summary(
    project_path: String,
    branch: String,
    base_branch: Option<String>,
) -> Result<GitChangeSummary, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let base = match base_branch {
        Some(b) => b,
        None => git::detect_base_branch(&repo_path, &branch).map_err(|e| e.to_string())?,
    };

    let exec_path = resolve_git_view_exec_path(&repo_path, &branch)?;
    git::get_git_change_summary(&exec_path, &branch, &base).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_branch_diff_files(
    project_path: String,
    branch: String,
    base_branch: String,
) -> Result<Vec<FileChange>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let exec_path = resolve_git_view_exec_path(&repo_path, &branch)?;
    git::get_branch_diff_files(&exec_path, &branch, &base_branch).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_file_diff(
    project_path: String,
    branch: String,
    base_branch: String,
    file_path: String,
) -> Result<FileDiff, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let exec_path = resolve_git_view_exec_path(&repo_path, &branch)?;
    git::get_file_diff(&exec_path, &branch, &base_branch, &file_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_branch_commits(
    project_path: String,
    branch: String,
    base_branch: String,
    offset: usize,
    limit: usize,
) -> Result<Vec<GitViewCommit>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let exec_path = resolve_git_view_exec_path(&repo_path, &branch)?;
    git::get_branch_commits(&exec_path, &branch, &base_branch, offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_working_tree_status(
    project_path: String,
    branch: String,
) -> Result<Vec<WorkingTreeEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let branch_ref = branch.trim();
    if branch_ref.is_empty() {
        return Err("Branch is required".to_string());
    }

    let exec_path = if git::is_bare_repository(&repo_path) {
        resolve_existing_worktree_path(&repo_path, branch_ref)?
            .ok_or_else(|| format!("Worktree not found for branch: {}", branch_ref))?
    } else {
        repo_path
    };

    git::get_working_tree_status(&exec_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_stash_list(project_path: String, branch: String) -> Result<Vec<StashEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let branch_ref = branch.trim();
    let exec_path = if branch_ref.is_empty() {
        resolve_any_active_worktree_path(&repo_path)?.unwrap_or_else(|| repo_path.clone())
    } else {
        resolve_git_view_exec_path(&repo_path, branch_ref)?
    };

    git::get_stash_list(&exec_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_base_branch_candidates(project_path: String) -> Result<Vec<String>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    git::list_base_branch_candidates(&repo_path).map_err(|e| e.to_string())
}
