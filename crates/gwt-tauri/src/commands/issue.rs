//! GitHub Issue commands (SPEC-c6ba640a)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{
    create_linked_branch, fetch_open_issues, find_branch_for_issue, is_gh_cli_authenticated,
    is_gh_cli_available,
};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Response for fetch_github_issues (FR-010a)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchIssuesResponse {
    pub issues: Vec<IssueInfo>,
    pub has_next_page: bool,
}

/// Serializable issue info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub updated_at: String,
    pub labels: Vec<String>,
}

/// gh CLI status (FR-011a)
#[derive(Debug, Clone, Serialize)]
pub struct GhCliStatus {
    pub available: bool,
    pub authenticated: bool,
}

/// Rollback result (FR-014)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    pub local_deleted: bool,
    pub remote_deleted: bool,
    pub error: Option<String>,
}

/// Fetch GitHub issues with pagination (FR-010)
#[tauri::command]
pub fn fetch_github_issues(
    project_path: String,
    page: u32,
    per_page: u32,
) -> Result<FetchIssuesResponse, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let result = fetch_open_issues(&repo_path, page, per_page)?;

    let issues = result
        .issues
        .into_iter()
        .map(|issue| IssueInfo {
            number: issue.number,
            title: issue.title,
            updated_at: issue.updated_at,
            labels: issue.labels,
        })
        .collect();

    Ok(FetchIssuesResponse {
        issues,
        has_next_page: result.has_next_page,
    })
}

/// Check gh CLI availability and authentication (FR-011)
#[tauri::command]
pub fn check_gh_cli_status(_project_path: String) -> Result<GhCliStatus, String> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };

    Ok(GhCliStatus {
        available,
        authenticated,
    })
}

/// Find an existing branch for a given issue (FR-012)
#[tauri::command]
pub fn find_existing_issue_branch(
    project_path: String,
    issue_number: u64,
) -> Result<Option<String>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    find_branch_for_issue(&repo_path, issue_number)
}

/// Link a branch to a GitHub issue via `gh issue develop` (FR-013)
#[tauri::command]
pub fn link_branch_to_issue(
    project_path: String,
    issue_number: u64,
    branch_name: String,
) -> Result<(), String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    create_linked_branch(&repo_path, issue_number, &branch_name)
}

/// Rollback an issue-linked branch (FR-014)
///
/// Deletes local branch and optionally the remote branch.
#[tauri::command]
pub fn rollback_issue_branch(
    project_path: String,
    branch_name: String,
    delete_remote: bool,
) -> Result<RollbackResult, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    // Delete local branch
    let local_output = Command::new("git")
        .args(["branch", "-D", &branch_name])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch -D: {}", e))?;

    let local_deleted = local_output.status.success();

    // Delete remote branch if requested (FR-014a)
    let (remote_deleted, error) = if delete_remote {
        let remote_output = Command::new("git")
            .args(["push", "origin", "--delete", &branch_name])
            .current_dir(&repo_path)
            .output()
            .map_err(|e| format!("Failed to execute git push --delete: {}", e))?;

        if remote_output.status.success() {
            (true, None)
        } else {
            let stderr = String::from_utf8_lossy(&remote_output.stderr).to_string();
            // FR-029b: remote deletion failure is not fatal
            (false, Some(stderr))
        }
    } else {
        (false, None)
    };

    Ok(RollbackResult {
        local_deleted,
        remote_deleted,
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================
    // FR-010: FetchIssuesResponse serialization tests
    // ==========================================================

    #[test]
    fn test_fetch_issues_response_serialization() {
        let response = FetchIssuesResponse {
            issues: vec![IssueInfo {
                number: 42,
                title: "Fix login bug".to_string(),
                updated_at: "2025-01-25T10:00:00Z".to_string(),
                labels: vec!["bug".to_string()],
            }],
            has_next_page: true,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"hasNextPage\":true"));
        assert!(json.contains("\"number\":42"));
        assert!(json.contains("\"updatedAt\":"));
    }

    #[test]
    fn test_fetch_issues_response_empty() {
        let response = FetchIssuesResponse {
            issues: vec![],
            has_next_page: false,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"issues\":[]"));
        assert!(json.contains("\"hasNextPage\":false"));
    }

    // ==========================================================
    // FR-011: GhCliStatus serialization tests
    // ==========================================================

    #[test]
    fn test_gh_cli_status_serialization() {
        let status = GhCliStatus {
            available: true,
            authenticated: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"authenticated\":true"));
    }

    #[test]
    fn test_gh_cli_status_unavailable() {
        let status = GhCliStatus {
            available: false,
            authenticated: false,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":false"));
        assert!(json.contains("\"authenticated\":false"));
    }

    // ==========================================================
    // FR-014: RollbackResult serialization tests
    // ==========================================================

    #[test]
    fn test_rollback_result_success() {
        let result = RollbackResult {
            local_deleted: true,
            remote_deleted: true,
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"localDeleted\":true"));
        assert!(json.contains("\"remoteDeleted\":true"));
        assert!(json.contains("\"error\":null"));
    }

    #[test]
    fn test_rollback_result_with_remote_error() {
        let result = RollbackResult {
            local_deleted: true,
            remote_deleted: false,
            error: Some("remote branch not found".to_string()),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"localDeleted\":true"));
        assert!(json.contains("\"remoteDeleted\":false"));
        assert!(json.contains("remote branch not found"));
    }

    // ==========================================================
    // IssueInfo serialization tests
    // ==========================================================

    #[test]
    fn test_issue_info_with_labels() {
        let info = IssueInfo {
            number: 42,
            title: "Fix bug".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec!["bug".to_string(), "urgent".to_string()],
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"labels\":[\"bug\",\"urgent\"]"));
    }

    #[test]
    fn test_issue_info_empty_labels() {
        let info = IssueInfo {
            number: 1,
            title: "No labels".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![],
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"labels\":[]"));
    }
}
