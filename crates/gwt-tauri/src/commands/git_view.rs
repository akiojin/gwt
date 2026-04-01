//! GitView commands for branch diff, commits, working tree, and stash

use std::path::Path;

#[cfg(test)]
use gwt_core::git::Remote;
use gwt_core::{
    git::{
        self, FileChange, FileDiff, GitChangeSummary, GitViewCommit, StashEntry, WorkingTreeEntry,
    },
    StructuredError,
};
use tracing::instrument;

use crate::commands::project::resolve_repo_path_for_project_root;

#[cfg(test)]
fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_known_remote_prefix ---

    #[test]
    fn strip_prefix_removes_origin() {
        let remotes = vec![Remote {
            name: "origin".to_string(),
            fetch_url: "https://example.com/repo".to_string(),
            push_url: "https://example.com/repo".to_string(),
        }];
        assert_eq!(
            strip_known_remote_prefix("origin/feature/x", &remotes),
            "feature/x"
        );
    }

    #[test]
    fn strip_prefix_preserves_unknown_prefix() {
        let remotes = vec![Remote {
            name: "origin".to_string(),
            fetch_url: "https://example.com/repo".to_string(),
            push_url: "https://example.com/repo".to_string(),
        }];
        assert_eq!(
            strip_known_remote_prefix("fork/feature/x", &remotes),
            "fork/feature/x"
        );
    }

    #[test]
    fn strip_prefix_no_slash_returns_same() {
        let remotes = vec![Remote {
            name: "origin".to_string(),
            fetch_url: "https://example.com/repo".to_string(),
            push_url: "https://example.com/repo".to_string(),
        }];
        assert_eq!(strip_known_remote_prefix("main", &remotes), "main");
    }

    #[test]
    fn strip_prefix_empty_remotes_preserves_all() {
        let remotes: Vec<Remote> = vec![];
        assert_eq!(
            strip_known_remote_prefix("origin/main", &remotes),
            "origin/main"
        );
    }

    #[test]
    fn strip_prefix_upstream_remote() {
        let remotes = vec![
            Remote {
                name: "origin".to_string(),
                fetch_url: "https://example.com/repo".to_string(),
                push_url: "https://example.com/repo".to_string(),
            },
            Remote {
                name: "upstream".to_string(),
                fetch_url: "https://example.com/upstream".to_string(),
                push_url: "https://example.com/upstream".to_string(),
            },
        ];
        assert_eq!(strip_known_remote_prefix("upstream/main", &remotes), "main");
    }

    #[test]
    fn strip_prefix_nested_slashes() {
        let remotes = vec![Remote {
            name: "origin".to_string(),
            fetch_url: "https://example.com/repo".to_string(),
            push_url: "https://example.com/repo".to_string(),
        }];
        // "origin/feature/deep/nested" -> splits on first / -> rest = "feature/deep/nested"
        assert_eq!(
            strip_known_remote_prefix("origin/feature/deep/nested", &remotes),
            "feature/deep/nested"
        );
    }

    #[test]
    fn strip_prefix_empty_branch() {
        let remotes = vec![Remote {
            name: "origin".to_string(),
            fetch_url: "https://example.com/repo".to_string(),
            push_url: "https://example.com/repo".to_string(),
        }];
        assert_eq!(strip_known_remote_prefix("", &remotes), "");
    }

    #[test]
    fn strip_prefix_branch_with_only_slash() {
        let remotes: Vec<Remote> = vec![];
        assert_eq!(strip_known_remote_prefix("a/b", &remotes), "a/b");
    }
}

// ---------------------------------------------------------------------------
// Shared _impl functions (used by both Tauri commands and HTTP IPC handlers)
// ---------------------------------------------------------------------------

pub(crate) fn get_git_change_summary_impl(
    project_path: &str,
    branch: &str,
    base_branch: Option<&str>,
) -> Result<GitChangeSummary, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_git_change_summary"))?;

    let base = match base_branch {
        Some(b) => b.to_string(),
        None => git::detect_base_branch(&repo_path, branch)
            .map_err(|e| StructuredError::from_gwt_error(&e, "get_git_change_summary"))?,
    };

    git::get_git_change_summary(&repo_path, branch, &base)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_git_change_summary"))
}

pub(crate) fn get_branch_diff_files_impl(
    project_path: &str,
    branch: &str,
    base_branch: &str,
) -> Result<Vec<FileChange>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_branch_diff_files"))?;
    git::get_branch_diff_files(&repo_path, branch, base_branch)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_branch_diff_files"))
}

pub(crate) fn get_branch_commits_impl(
    project_path: &str,
    branch: &str,
    base_branch: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<GitViewCommit>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_branch_commits"))?;
    git::get_branch_commits(&repo_path, branch, base_branch, offset, limit)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_branch_commits"))
}

pub(crate) fn get_working_tree_status_impl(
    project_path: &str,
    branch: &str,
) -> Result<Vec<WorkingTreeEntry>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_working_tree_status"))?;
    let branch_ref = branch.trim();
    if branch_ref.is_empty() {
        return Err(StructuredError::internal(
            "Branch is required",
            "get_working_tree_status",
        ));
    }

    git::get_working_tree_status(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_working_tree_status"))
}

pub(crate) fn get_stash_list_impl(
    project_path: &str,
    _branch: &str,
) -> Result<Vec<StashEntry>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_stash_list"))?;
    git::get_stash_list(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_stash_list"))
}

// ---------------------------------------------------------------------------
// Tauri command wrappers (thin delegates to _impl functions)
// ---------------------------------------------------------------------------

#[instrument(
    skip_all,
    fields(command = "get_git_change_summary", project_path, branch)
)]
#[tauri::command]
pub fn get_git_change_summary(
    project_path: String,
    branch: String,
    base_branch: Option<String>,
) -> Result<GitChangeSummary, StructuredError> {
    get_git_change_summary_impl(&project_path, &branch, base_branch.as_deref())
}

#[instrument(
    skip_all,
    fields(command = "get_branch_diff_files", project_path, branch)
)]
#[tauri::command]
pub fn get_branch_diff_files(
    project_path: String,
    branch: String,
    base_branch: String,
) -> Result<Vec<FileChange>, StructuredError> {
    get_branch_diff_files_impl(&project_path, &branch, &base_branch)
}

#[instrument(skip_all, fields(command = "get_file_diff", project_path, branch))]
#[tauri::command]
pub fn get_file_diff(
    project_path: String,
    branch: String,
    base_branch: String,
    file_path: String,
) -> Result<FileDiff, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_file_diff"))?;
    git::get_file_diff(&repo_path, &branch, &base_branch, &file_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_file_diff"))
}

#[instrument(skip_all, fields(command = "get_branch_commits", project_path, branch))]
#[tauri::command]
pub fn get_branch_commits(
    project_path: String,
    branch: String,
    base_branch: String,
    offset: usize,
    limit: usize,
) -> Result<Vec<GitViewCommit>, StructuredError> {
    get_branch_commits_impl(&project_path, &branch, &base_branch, offset, limit)
}

#[instrument(
    skip_all,
    fields(command = "get_working_tree_status", project_path, branch)
)]
#[tauri::command]
pub fn get_working_tree_status(
    project_path: String,
    branch: String,
) -> Result<Vec<WorkingTreeEntry>, StructuredError> {
    get_working_tree_status_impl(&project_path, &branch)
}

#[instrument(skip_all, fields(command = "get_stash_list", project_path, branch))]
#[tauri::command]
pub fn get_stash_list(
    project_path: String,
    branch: String,
) -> Result<Vec<StashEntry>, StructuredError> {
    get_stash_list_impl(&project_path, &branch)
}

#[instrument(skip_all, fields(command = "get_base_branch_candidates", project_path))]
#[tauri::command]
pub fn get_base_branch_candidates(project_path: String) -> Result<Vec<String>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_base_branch_candidates"))?;
    git::list_base_branch_candidates(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "get_base_branch_candidates"))
}
