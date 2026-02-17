//! GitHub Issue commands (SPEC-c6ba640a)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{
    create_linked_branch, fetch_issue_detail, fetch_open_issues, find_branch_for_issue,
    is_gh_cli_authenticated, is_gh_cli_available,
};
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::path::Path;

/// Response for fetch_github_issues (FR-010a)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchIssuesResponse {
    pub issues: Vec<IssueInfo>,
    pub has_next_page: bool,
}

/// Serializable label info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelInfo {
    pub name: String,
    pub color: String,
}

/// Serializable assignee info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssigneeInfo {
    pub login: String,
    pub avatar_url: String,
}

/// Serializable milestone info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneInfo {
    pub title: String,
    pub number: u32,
}

/// Serializable issue info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub updated_at: String,
    pub labels: Vec<LabelInfo>,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub assignees: Vec<AssigneeInfo>,
    pub comments_count: u32,
    pub milestone: Option<MilestoneInfo>,
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

/// Convert a core GitHubIssue to the serializable IssueInfo
fn issue_to_info(issue: gwt_core::git::GitHubIssue) -> IssueInfo {
    IssueInfo {
        number: issue.number,
        title: issue.title,
        updated_at: issue.updated_at,
        labels: issue
            .labels
            .into_iter()
            .map(|l| LabelInfo {
                name: l.name,
                color: l.color,
            })
            .collect(),
        body: issue.body,
        state: issue.state,
        html_url: issue.html_url,
        assignees: issue
            .assignees
            .into_iter()
            .map(|a| AssigneeInfo {
                login: a.login,
                avatar_url: a.avatar_url,
            })
            .collect(),
        comments_count: issue.comments_count,
        milestone: issue.milestone.map(|m| MilestoneInfo {
            title: m.title,
            number: m.number,
        }),
    }
}

/// Fetch GitHub issues with pagination (FR-010)
#[tauri::command]
pub fn fetch_github_issues(
    project_path: String,
    page: u32,
    per_page: u32,
    state: String,
) -> Result<FetchIssuesResponse, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let result = fetch_open_issues(&repo_path, page, per_page, &state)?;

    let issues = result.issues.into_iter().map(issue_to_info).collect();

    Ok(FetchIssuesResponse {
        issues,
        has_next_page: result.has_next_page,
    })
}

/// Fetch a single GitHub issue detail
#[tauri::command]
pub fn fetch_github_issue_detail(
    project_path: String,
    issue_number: u64,
) -> Result<IssueInfo, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let issue = fetch_issue_detail(&repo_path, issue_number)?;
    Ok(issue_to_info(issue))
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

    // Local rollback must remove worktree first, then delete the branch.
    let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
    let (local_deleted, local_error) = match manager.cleanup_branch(&branch_name, true, true) {
        Ok(()) => (true, None),
        Err(err) => (false, Some(err.to_string())),
    };

    // Delete remote branch if requested (FR-014a)
    let (remote_deleted, remote_error) = if delete_remote {
        let remote_output = gwt_core::process::command("git")
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

    let error = match (local_error, remote_error) {
        (None, None) => None,
        (Some(local), None) => Some(format!("Local cleanup warning: {}", local)),
        (None, Some(remote)) => Some(remote),
        (Some(local), Some(remote)) => Some(format!(
            "Local cleanup warning: {}\nRemote cleanup warning: {}",
            local, remote
        )),
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
                labels: vec![LabelInfo {
                    name: "bug".to_string(),
                    color: "d73a4a".to_string(),
                }],
                body: Some("Issue body".to_string()),
                state: "OPEN".to_string(),
                html_url: "https://github.com/user/repo/issues/42".to_string(),
                assignees: vec![],
                comments_count: 0,
                milestone: None,
            }],
            has_next_page: true,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"hasNextPage\":true"));
        assert!(json.contains("\"number\":42"));
        assert!(json.contains("\"updatedAt\":"));
        assert!(json.contains("\"state\":\"OPEN\""));
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

    #[test]
    fn test_issue_info_extended_fields_serialization() {
        let info = IssueInfo {
            number: 42,
            title: "Test".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![LabelInfo {
                name: "bug".to_string(),
                color: "d73a4a".to_string(),
            }],
            body: Some("body".to_string()),
            state: "OPEN".to_string(),
            html_url: "https://github.com/user/repo/issues/42".to_string(),
            assignees: vec![AssigneeInfo {
                login: "octocat".to_string(),
                avatar_url: "https://avatars.example.com/1".to_string(),
            }],
            comments_count: 5,
            milestone: Some(MilestoneInfo {
                title: "v1.0".to_string(),
                number: 1,
            }),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"body\":\"body\""));
        assert!(json.contains("\"state\":\"OPEN\""));
        assert!(json.contains("\"htmlUrl\":"));
        assert!(json.contains("\"commentsCount\":5"));
        assert!(json.contains("\"login\":\"octocat\""));
        assert!(json.contains("\"avatarUrl\":"));
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
            labels: vec![
                LabelInfo { name: "bug".to_string(), color: "d73a4a".to_string() },
                LabelInfo { name: "urgent".to_string(), color: "ff0000".to_string() },
            ],
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"bug\""));
        assert!(json.contains("\"name\":\"urgent\""));
    }

    #[test]
    fn test_issue_info_empty_labels() {
        let info = IssueInfo {
            number: 1,
            title: "No labels".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![],
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"labels\":[]"));
    }
}
