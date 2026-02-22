//! GitHub Issue commands (SPEC-c6ba640a)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::ai::{
    classify_issue_prefix as core_classify_issue_prefix, format_error_for_display, AIClient,
};
use gwt_core::config::ProfilesConfig;
use gwt_core::git::{
    create_linked_branch, fetch_issue_detail, fetch_open_issues, find_branch_for_issue,
    get_spec_issue_detail, is_gh_cli_authenticated, is_gh_cli_available,
};
use gwt_core::worktree::WorktreeManager;
use gwt_core::StructuredError;
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

/// Branch-linked issue info for Worktree Summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchLinkedIssueInfo {
    pub number: u64,
    pub title: String,
    pub updated_at: String,
    pub labels: Vec<String>,
    pub url: String,
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
        state: normalize_issue_state(&issue.state),
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

fn normalize_issue_state(state: &str) -> String {
    if state.eq_ignore_ascii_case("closed") {
        "closed".to_string()
    } else {
        "open".to_string()
    }
}

/// Fetch GitHub issues with pagination (FR-010)
#[tauri::command]
pub fn fetch_github_issues(
    project_path: String,
    page: u32,
    per_page: u32,
    state: Option<String>,
) -> Result<FetchIssuesResponse, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issues"))?;
    let state = state.unwrap_or_else(|| "open".to_string());

    let result = fetch_open_issues(&repo_path, page, per_page, &state)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issues"))?;

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
) -> Result<IssueInfo, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issue_detail"))?;

    let issue = fetch_issue_detail(&repo_path, issue_number)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issue_detail"))?;
    Ok(issue_to_info(issue))
}

fn extract_issue_number_from_branch(branch: &str) -> Option<u64> {
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        return None;
    }

    for segment in trimmed.split('/') {
        let lower = segment.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("issue-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            if let Ok(number) = digits.parse::<u64>() {
                return Some(number);
            }
        }
    }
    None
}

fn is_issue_not_found_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("could not resolve to an issue")
        || (lower.contains("issue with the number") && lower.contains("(repository.issue)"))
}

/// Fetch issue linked to branch naming pattern (`issue-<number>`).
#[tauri::command]
pub fn fetch_branch_linked_issue(
    project_path: String,
    branch: String,
) -> Result<Option<BranchLinkedIssueInfo>, StructuredError> {
    let Some(issue_number) = extract_issue_number_from_branch(&branch) else {
        return Ok(None);
    };

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_branch_linked_issue"))?;

    match get_spec_issue_detail(&repo_path, issue_number) {
        Ok(detail) => Ok(Some(BranchLinkedIssueInfo {
            number: detail.number,
            title: detail.title,
            updated_at: detail.updated_at,
            labels: detail.labels,
            url: detail.url,
        })),
        Err(err) if is_issue_not_found_error(&err) => Ok(None),
        Err(err) => Err(StructuredError::internal(&err, "fetch_branch_linked_issue")),
    }
}

/// Check gh CLI availability and authentication (FR-011)
#[tauri::command]
pub fn check_gh_cli_status(_project_path: String) -> Result<GhCliStatus, StructuredError> {
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
) -> Result<Option<String>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branch"))?;

    find_branch_for_issue(&repo_path, issue_number)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branch"))
}

/// Link a branch to a GitHub issue via `gh issue develop` (FR-013)
#[tauri::command]
pub fn link_branch_to_issue(
    project_path: String,
    issue_number: u64,
    branch_name: String,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "link_branch_to_issue"))?;

    create_linked_branch(&repo_path, issue_number, &branch_name)
        .map_err(|e| StructuredError::internal(&e, "link_branch_to_issue"))
}

/// Rollback an issue-linked branch (FR-014)
///
/// Deletes local branch and optionally the remote branch.
#[tauri::command]
pub fn rollback_issue_branch(
    project_path: String,
    branch_name: String,
    delete_remote: bool,
) -> Result<RollbackResult, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "rollback_issue_branch"))?;

    // Local rollback must remove worktree first, then delete the branch.
    let manager = WorktreeManager::new(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "rollback_issue_branch"))?;
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
            .map_err(|e| {
                StructuredError::internal(
                    &format!("Failed to execute git push --delete: {}", e),
                    "rollback_issue_branch",
                )
            })?;

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

/// AI-based issue branch prefix classification result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    /// "ok" | "ai-not-configured" | "error"
    pub status: String,
    pub prefix: Option<String>,
    pub error: Option<String>,
}

/// Classify a GitHub issue into a branch prefix using AI.
#[tauri::command]
pub fn classify_issue_branch_prefix(
    title: String,
    labels: Vec<String>,
    body: Option<String>,
) -> Result<ClassifyResult, StructuredError> {
    let profiles = ProfilesConfig::load()
        .map_err(|e| StructuredError::from_gwt_error(&e, "classify_issue_branch_prefix"))?;
    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        return Ok(ClassifyResult {
            status: "ai-not-configured".to_string(),
            prefix: None,
            error: None,
        });
    };

    let client = AIClient::new(settings)
        .map_err(|e| StructuredError::internal(&e.to_string(), "classify_issue_branch_prefix"))?;
    match core_classify_issue_prefix(&client, &title, &labels, body.as_deref()) {
        Ok(prefix) => Ok(ClassifyResult {
            status: "ok".to_string(),
            prefix: Some(prefix),
            error: None,
        }),
        Err(err) => Ok(ClassifyResult {
            status: "error".to_string(),
            prefix: None,
            error: Some(format_error_for_display(&err)),
        }),
    }
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

    #[test]
    fn test_issue_to_info_normalizes_state_to_lowercase() {
        let issue = gwt_core::git::GitHubIssue {
            number: 42,
            title: "Test".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![],
            body: None,
            state: "CLOSED".to_string(),
            html_url: "https://github.com/user/repo/issues/42".to_string(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let info = issue_to_info(issue);
        assert_eq!(info.state, "closed");
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
                LabelInfo {
                    name: "bug".to_string(),
                    color: "d73a4a".to_string(),
                },
                LabelInfo {
                    name: "urgent".to_string(),
                    color: "ff0000".to_string(),
                },
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

    #[test]
    fn test_branch_linked_issue_info_serialization() {
        let issue = BranchLinkedIssueInfo {
            number: 1097,
            title: "Rework tabs".to_string(),
            updated_at: "2026-02-17T00:00:00Z".to_string(),
            labels: vec!["enhancement".to_string()],
            url: "https://github.com/example/repo/issues/1097".to_string(),
        };

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"updatedAt\""));
        assert!(json.contains("\"url\":\"https://github.com/example/repo/issues/1097\""));
    }

    #[test]
    fn test_extract_issue_number_from_branch_variants() {
        assert_eq!(
            extract_issue_number_from_branch("feature/issue-1097"),
            Some(1097)
        );
        assert_eq!(
            extract_issue_number_from_branch("origin/bugfix/issue-42-something"),
            Some(42)
        );
        assert_eq!(extract_issue_number_from_branch("hotfix/ISSUE-9"), Some(9));
    }

    #[test]
    fn test_extract_issue_number_from_branch_absent() {
        assert_eq!(extract_issue_number_from_branch("feature/new-ui"), None);
        assert_eq!(
            extract_issue_number_from_branch("feature/noissue-123"),
            None
        );
        assert_eq!(extract_issue_number_from_branch("feature/reissue-42"), None);
        assert_eq!(extract_issue_number_from_branch(""), None);
    }

    #[test]
    fn test_is_issue_not_found_error() {
        assert!(is_issue_not_found_error(
            "gh issue view failed: could not resolve to an issue"
        ));
        assert!(is_issue_not_found_error(
            "gh issue view failed: GraphQL: Could not resolve to an issue with the number of 1097. (repository.issue)"
        ));
        assert!(!is_issue_not_found_error(
            "gh issue view failed: HTTP 404: Not Found"
        ));
        assert!(!is_issue_not_found_error(
            "gh issue view failed: GraphQL: Could not resolve to a Repository with the name 'org/repo'. (repository)"
        ));
        assert!(!is_issue_not_found_error("permission denied"));
    }
}
