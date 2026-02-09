//! Session history commands (Quick Start)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::config::ToolSessionEntry;
use std::path::Path;

/// Return tool-specific latest session entries for a branch (Quick Start).
///
/// This is a read-only operation (no config/history writes).
#[tauri::command]
pub fn get_branch_quick_start(
    project_path: String,
    branch: String,
) -> Result<Vec<ToolSessionEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Branch is required".to_string());
    }

    Ok(gwt_core::config::get_branch_tool_history(&repo_path, branch))
}

