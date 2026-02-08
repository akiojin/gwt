//! Branch management commands

use gwt_core::git::Branch;
use serde::Serialize;
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
    let path = Path::new(&project_path);
    let branches = Branch::list(path).map_err(|e| e.to_string())?;
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

/// List all remote branches in a repository
#[tauri::command]
pub fn list_remote_branches(project_path: String) -> Result<Vec<BranchInfo>, String> {
    let path = Path::new(&project_path);
    let branches = Branch::list_remote(path).map_err(|e| e.to_string())?;
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

/// Get the current branch
#[tauri::command]
pub fn get_current_branch(project_path: String) -> Result<Option<BranchInfo>, String> {
    let path = Path::new(&project_path);
    let branch = Branch::current(path).map_err(|e| e.to_string())?;
    Ok(branch.map(BranchInfo::from))
}
