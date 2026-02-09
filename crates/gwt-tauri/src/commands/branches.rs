//! Branch management commands

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{is_bare_repository, Branch};
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Serializable branch info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub commit: String,
    pub is_current: bool,
    pub has_remote: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub divergence_status: String,
    pub commit_timestamp: Option<i64>,
    pub is_gone: bool,
}

impl From<Branch> for BranchInfo {
    fn from(b: Branch) -> Self {
        let divergence_status = b.divergence_status().to_string();
        BranchInfo {
            name: b.name,
            commit: b.commit,
            is_current: b.is_current,
            has_remote: b.has_remote,
            upstream: b.upstream,
            ahead: b.ahead,
            behind: b.behind,
            divergence_status,
            commit_timestamp: b.commit_timestamp,
            is_gone: b.is_gone,
        }
    }
}

/// List all local branches in a repository
#[tauri::command]
pub fn list_branches(project_path: String) -> Result<Vec<BranchInfo>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let branches = Branch::list(&repo_path).map_err(|e| e.to_string())?;
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

/// List branches that currently have a local worktree (gwt "Local" view)
#[tauri::command]
pub fn list_worktree_branches(project_path: String) -> Result<Vec<BranchInfo>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

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
    Ok(branches
        .into_iter()
        .filter(|b| names.contains(&b.name))
        .map(BranchInfo::from)
        .collect())
}

/// List all remote branches in a repository
#[tauri::command]
pub fn list_remote_branches(project_path: String) -> Result<Vec<BranchInfo>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let branches = if is_bare_repository(&repo_path) {
        Branch::list_remote_from_origin(&repo_path).map_err(|e| e.to_string())?
    } else {
        Branch::list_remote(&repo_path).map_err(|e| e.to_string())?
    };
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

/// Get the current branch
#[tauri::command]
pub fn get_current_branch(project_path: String) -> Result<Option<BranchInfo>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let branch = Branch::current(&repo_path).map_err(|e| e.to_string())?;
    Ok(branch.map(BranchInfo::from))
}
