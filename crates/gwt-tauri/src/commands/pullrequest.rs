//! Pull Request status commands (SPEC-d6949f99)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::graphql;
use gwt_core::git::{
    is_gh_cli_authenticated, is_gh_cli_available, PrStatusInfo, ReviewComment, ReviewInfo,
    WorkflowRunInfo,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// gh CLI availability and authentication status
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhCliStatusInfo {
    pub available: bool,
    pub authenticated: bool,
}

/// Response for fetch_pr_status (T009)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusResponse {
    pub statuses: HashMap<String, Option<PrStatusSummary>>,
    pub gh_status: GhCliStatusInfo,
}

/// Serializable PR status summary for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub mergeable: String,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub linked_issues: Vec<u64>,
    pub check_suites: Vec<WorkflowRunSummary>,
    pub reviews: Vec<ReviewSummary>,
    pub changed_files_count: u64,
    pub additions: u64,
    pub deletions: u64,
}

/// Serializable workflow run info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub workflow_name: String,
    pub run_id: u64,
    pub status: String,
    pub conclusion: Option<String>,
}

/// Serializable review info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub reviewer: String,
    pub state: String,
}

/// Serializable review comment for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommentSummary {
    pub author: String,
    pub body: String,
    pub file_path: Option<String>,
    pub line: Option<u64>,
    pub code_snippet: Option<String>,
    pub created_at: String,
}

/// Response for fetch_pr_detail (T010)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetailResponse {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub mergeable: String,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub linked_issues: Vec<u64>,
    pub check_suites: Vec<WorkflowRunSummary>,
    pub reviews: Vec<ReviewSummary>,
    pub review_comments: Vec<ReviewCommentSummary>,
    pub changed_files_count: u64,
    pub additions: u64,
    pub deletions: u64,
}

fn to_workflow_run_summary(info: &WorkflowRunInfo) -> WorkflowRunSummary {
    WorkflowRunSummary {
        workflow_name: info.workflow_name.clone(),
        run_id: info.run_id,
        status: info.status.clone(),
        conclusion: info.conclusion.clone(),
    }
}

fn to_review_summary(info: &ReviewInfo) -> ReviewSummary {
    ReviewSummary {
        reviewer: info.reviewer.clone(),
        state: info.state.clone(),
    }
}

fn to_review_comment_summary(comment: &ReviewComment) -> ReviewCommentSummary {
    ReviewCommentSummary {
        author: comment.author.clone(),
        body: comment.body.clone(),
        file_path: comment.file_path.clone(),
        line: comment.line,
        code_snippet: comment.code_snippet.clone(),
        created_at: comment.created_at.clone(),
    }
}

fn to_pr_status_summary(info: &PrStatusInfo) -> PrStatusSummary {
    PrStatusSummary {
        number: info.number,
        title: info.title.clone(),
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        author: info.author.clone(),
        base_branch: info.base_branch.clone(),
        head_branch: info.head_branch.clone(),
        labels: info.labels.clone(),
        assignees: info.assignees.clone(),
        milestone: info.milestone.clone(),
        linked_issues: info.linked_issues.clone(),
        check_suites: info
            .check_suites
            .iter()
            .map(to_workflow_run_summary)
            .collect(),
        reviews: info.reviews.iter().map(to_review_summary).collect(),
        changed_files_count: info.changed_files_count,
        additions: info.additions,
        deletions: info.deletions,
    }
}

fn to_pr_detail_response(info: &PrStatusInfo) -> PrDetailResponse {
    PrDetailResponse {
        number: info.number,
        title: info.title.clone(),
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        author: info.author.clone(),
        base_branch: info.base_branch.clone(),
        head_branch: info.head_branch.clone(),
        labels: info.labels.clone(),
        assignees: info.assignees.clone(),
        milestone: info.milestone.clone(),
        linked_issues: info.linked_issues.clone(),
        check_suites: info
            .check_suites
            .iter()
            .map(to_workflow_run_summary)
            .collect(),
        reviews: info.reviews.iter().map(to_review_summary).collect(),
        review_comments: info
            .review_comments
            .iter()
            .map(to_review_comment_summary)
            .collect(),
        changed_files_count: info.changed_files_count,
        additions: info.additions,
        deletions: info.deletions,
    }
}

/// Fetch PR statuses for all given branches via GraphQL (T009)
///
/// Also returns gh CLI availability/authentication status.
#[tauri::command]
pub fn fetch_pr_status(
    project_path: String,
    branches: Vec<String>,
) -> Result<PrStatusResponse, String> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };
    let gh_status = GhCliStatusInfo {
        available,
        authenticated,
    };

    if !available || !authenticated {
        // Return empty statuses with gh_status indicating the problem
        let statuses = branches.into_iter().map(|branch| (branch, None)).collect();
        return Ok(PrStatusResponse {
            statuses,
            gh_status,
        });
    }

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let results = graphql::fetch_pr_statuses(&repo_path, &branches)?;

    let statuses = results
        .into_iter()
        .map(|(branch, info)| (branch, info.as_ref().map(to_pr_status_summary)))
        .collect();

    Ok(PrStatusResponse {
        statuses,
        gh_status,
    })
}

/// Fetch detailed PR information for a single PR (T010)
#[tauri::command]
pub fn fetch_pr_detail(project_path: String, pr_number: u64) -> Result<PrDetailResponse, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let info = graphql::fetch_pr_detail(&repo_path, pr_number)?;
    Ok(to_pr_detail_response(&info))
}

/// Fetch CI run log for a specific check run/job ID (T011)
#[tauri::command]
pub fn fetch_ci_log(project_path: String, run_id: u64) -> Result<String, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let output = gwt_core::git::graphql::gh_run_view_log(&repo_path, run_id)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================
    // T012: GhCliStatusInfo serialization tests
    // ==========================================================

    #[test]
    fn test_gh_cli_status_info_serialization() {
        let status = GhCliStatusInfo {
            available: true,
            authenticated: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"authenticated\":true"));
    }

    #[test]
    fn test_gh_cli_status_info_unavailable() {
        let status = GhCliStatusInfo {
            available: false,
            authenticated: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":false"));
        assert!(json.contains("\"authenticated\":false"));
    }

    // ==========================================================
    // T012: PrStatusResponse serialization tests
    // ==========================================================

    #[test]
    fn test_pr_status_response_serialization() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "feature/x".to_string(),
            Some(PrStatusSummary {
                number: 42,
                title: "Add feature X".to_string(),
                state: "OPEN".to_string(),
                url: "https://github.com/o/r/pull/42".to_string(),
                mergeable: "MERGEABLE".to_string(),
                author: "alice".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/x".to_string(),
                labels: vec!["enhancement".to_string()],
                assignees: vec!["bob".to_string()],
                milestone: Some("v2.0".to_string()),
                linked_issues: vec![10],
                check_suites: vec![WorkflowRunSummary {
                    workflow_name: "CI".to_string(),
                    run_id: 12345,
                    status: "completed".to_string(),
                    conclusion: Some("success".to_string()),
                }],
                reviews: vec![ReviewSummary {
                    reviewer: "charlie".to_string(),
                    state: "APPROVED".to_string(),
                }],
                changed_files_count: 5,
                additions: 100,
                deletions: 20,
            }),
        );
        statuses.insert("feature/y".to_string(), None);

        let response = PrStatusResponse {
            statuses,
            gh_status: GhCliStatusInfo {
                available: true,
                authenticated: true,
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\""));
        assert!(json.contains("\"ghStatus\""));
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"number\":42"));
        assert!(json.contains("\"baseBranch\":\"main\""));
        assert!(json.contains("\"checkSuites\""));
        assert!(json.contains("\"workflowName\":\"CI\""));
        assert!(json.contains("\"changedFilesCount\":5"));
    }

    #[test]
    fn test_pr_status_response_empty() {
        let response = PrStatusResponse {
            statuses: HashMap::new(),
            gh_status: GhCliStatusInfo {
                available: false,
                authenticated: false,
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\":{}"));
        assert!(json.contains("\"ghStatus\""));
        assert!(json.contains("\"available\":false"));
    }

    // ==========================================================
    // T012: PrDetailResponse serialization tests
    // ==========================================================

    #[test]
    fn test_pr_detail_response_serialization() {
        let response = PrDetailResponse {
            number: 42,
            title: "Detailed PR".to_string(),
            state: "OPEN".to_string(),
            url: "https://github.com/o/r/pull/42".to_string(),
            mergeable: "MERGEABLE".to_string(),
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/detail".to_string(),
            labels: vec!["bug".to_string()],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![ReviewSummary {
                reviewer: "bob".to_string(),
                state: "CHANGES_REQUESTED".to_string(),
            }],
            review_comments: vec![ReviewCommentSummary {
                author: "bob".to_string(),
                body: "Fix this line".to_string(),
                file_path: Some("src/main.rs".to_string()),
                line: Some(42),
                code_snippet: None,
                created_at: "2025-01-01T00:00:00Z".to_string(),
            }],
            changed_files_count: 3,
            additions: 50,
            deletions: 10,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"reviewComments\""));
        assert!(json.contains("\"filePath\":\"src/main.rs\""));
        assert!(json.contains("\"createdAt\":\"2025-01-01T00:00:00Z\""));
        assert!(json.contains("\"changedFilesCount\":3"));
    }

    // ==========================================================
    // T012: Conversion function tests
    // ==========================================================

    #[test]
    fn test_to_pr_status_summary() {
        let info = PrStatusInfo {
            number: 1,
            title: "Test".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com".to_string(),
            mergeable: "UNKNOWN".to_string(),
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/test".to_string(),
            labels: vec!["label".to_string()],
            assignees: vec!["a".to_string()],
            milestone: Some("m1".to_string()),
            linked_issues: vec![5],
            check_suites: vec![WorkflowRunInfo {
                workflow_name: "CI".to_string(),
                run_id: 100,
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
            }],
            reviews: vec![ReviewInfo {
                reviewer: "r1".to_string(),
                state: "APPROVED".to_string(),
            }],
            review_comments: vec![],
            changed_files_count: 2,
            additions: 10,
            deletions: 3,
        };

        let summary = to_pr_status_summary(&info);
        assert_eq!(summary.number, 1);
        assert_eq!(summary.labels, vec!["label"]);
        assert_eq!(summary.check_suites.len(), 1);
        assert_eq!(summary.check_suites[0].workflow_name, "CI");
        assert_eq!(summary.reviews.len(), 1);
        assert_eq!(summary.reviews[0].reviewer, "r1");
    }

    #[test]
    fn test_to_pr_detail_response() {
        let info = PrStatusInfo {
            number: 10,
            title: "Detail".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com/10".to_string(),
            mergeable: "MERGEABLE".to_string(),
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "fix/bug".to_string(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![],
            review_comments: vec![ReviewComment {
                author: "reviewer".to_string(),
                body: "Comment".to_string(),
                file_path: Some("file.rs".to_string()),
                line: Some(5),
                code_snippet: None,
                created_at: "2025-01-01T00:00:00Z".to_string(),
            }],
            changed_files_count: 1,
            additions: 5,
            deletions: 0,
        };

        let detail = to_pr_detail_response(&info);
        assert_eq!(detail.number, 10);
        assert_eq!(detail.review_comments.len(), 1);
        assert_eq!(detail.review_comments[0].author, "reviewer");
        assert_eq!(
            detail.review_comments[0].file_path,
            Some("file.rs".to_string())
        );
    }
}
