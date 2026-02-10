//! GitView commands for branch diff, commits, working tree, and stash

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{
    self, FileChange, FileDiff, GitChangeSummary, GitViewCommit, StashEntry, WorkingTreeEntry,
};
use std::path::Path;

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

    git::get_git_change_summary(&repo_path, &branch, &base).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_branch_diff_files(
    project_path: String,
    branch: String,
    base_branch: String,
) -> Result<Vec<FileChange>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    git::get_branch_diff_files(&repo_path, &branch, &base_branch).map_err(|e| e.to_string())
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
    git::get_file_diff(&repo_path, &branch, &base_branch, &file_path).map_err(|e| e.to_string())
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
    git::get_branch_commits(&repo_path, &branch, &base_branch, offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_working_tree_status(project_path: String) -> Result<Vec<WorkingTreeEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    git::get_working_tree_status(&repo_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_stash_list(project_path: String) -> Result<Vec<StashEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    git::get_stash_list(&repo_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_base_branch_candidates(project_path: String) -> Result<Vec<String>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    git::list_base_branch_candidates(&repo_path).map_err(|e| e.to_string())
}
