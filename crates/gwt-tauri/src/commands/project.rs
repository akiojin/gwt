//! Project/repo management commands

use crate::state::AppState;
use gwt_core::git::{self, Branch};
use serde::Serialize;
use std::path::Path;
use tauri::State;

/// Serializable project info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub path: String,
    pub repo_name: String,
    pub current_branch: Option<String>,
}

/// Open a project (set project_path in AppState)
#[tauri::command]
pub fn open_project(path: String, state: State<AppState>) -> Result<ProjectInfo, String> {
    let p = Path::new(&path);

    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    if !git::is_git_repo(p) {
        return Err(format!("Not a git repository: {}", path));
    }

    // Get repo name from the directory name
    let repo_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    // Get current branch
    let current_branch = Branch::current(p)
        .ok()
        .flatten()
        .map(|b| b.name);

    // Update state
    if let Ok(mut project_path) = state.project_path.lock() {
        *project_path = Some(path.clone());
    }

    Ok(ProjectInfo {
        path,
        repo_name,
        current_branch,
    })
}

/// Get current project info from state
#[tauri::command]
pub fn get_project_info(state: State<AppState>) -> Option<ProjectInfo> {
    let project_path = state.project_path.lock().ok()?;
    let path_str = project_path.as_ref()?;
    let p = Path::new(path_str);

    let repo_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_str.clone());

    let current_branch = Branch::current(p)
        .ok()
        .flatten()
        .map(|b| b.name);

    Some(ProjectInfo {
        path: path_str.clone(),
        repo_name,
        current_branch,
    })
}

/// Check if a path is a git repository
#[tauri::command]
pub fn is_git_repo(path: String) -> bool {
    git::is_git_repo(Path::new(&path))
}
