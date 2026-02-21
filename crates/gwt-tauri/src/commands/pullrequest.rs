//! Pull Request status commands (SPEC-d6949f99)

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::graphql;
use gwt_core::git::{
    is_gh_cli_authenticated, is_gh_cli_available, PrCache, PrStatusInfo, Remote, ReviewComment,
    ReviewInfo, WorkflowRunInfo,
};
use gwt_core::StructuredError;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    pub merge_state_status: Option<String>,
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
    pub is_required: Option<bool>,
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
    pub merge_state_status: Option<String>,
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

/// Latest PR reference for a branch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchPrReference {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
struct LatestBranchPrCacheEntry {
    value: Option<BranchPrReference>,
    fetched_at: Instant,
}

const LATEST_BRANCH_PR_CACHE_TTL: Duration = Duration::from_secs(30);

fn latest_branch_pr_cache() -> &'static Mutex<HashMap<String, LatestBranchPrCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, LatestBranchPrCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_latest_branch_pr_cache(cache_key: &str) -> Option<Option<BranchPrReference>> {
    let cache = latest_branch_pr_cache();
    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    let entry = guard.get(cache_key)?;
    if entry.fetched_at.elapsed() < LATEST_BRANCH_PR_CACHE_TTL {
        return Some(entry.value.clone());
    }
    guard.remove(cache_key);
    None
}

fn write_latest_branch_pr_cache(cache_key: String, value: Option<BranchPrReference>) {
    let cache = latest_branch_pr_cache();
    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    guard.insert(
        cache_key,
        LatestBranchPrCacheEntry {
            value,
            fetched_at: Instant::now(),
        },
    );
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let trimmed = branch.trim();
    let Some((first, rest)) = trimmed.split_once('/') else {
        return trimmed;
    };
    if first == "origin" || remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    trimmed
}

fn to_workflow_run_summary(info: &WorkflowRunInfo) -> WorkflowRunSummary {
    WorkflowRunSummary {
        workflow_name: info.workflow_name.clone(),
        run_id: info.run_id,
        status: info.status.clone(),
        conclusion: info.conclusion.clone(),
        is_required: info.is_required,
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
        merge_state_status: info.merge_state_status.clone(),
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
        merge_state_status: info.merge_state_status.clone(),
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
) -> Result<PrStatusResponse, StructuredError> {
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
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_status"))?;

    let results = graphql::fetch_pr_statuses(&repo_path, &branches)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_status"))?;

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
pub fn fetch_pr_detail(
    project_path: String,
    pr_number: u64,
) -> Result<PrDetailResponse, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_detail"))?;

    let info = graphql::fetch_pr_detail(&repo_path, pr_number)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_detail"))?;
    Ok(to_pr_detail_response(&info))
}

/// Fetch latest branch PR: open PR first, otherwise latest closed/merged.
#[tauri::command]
pub fn fetch_latest_branch_pr(
    project_path: String,
    branch: String,
) -> Result<Option<BranchPrReference>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_latest_branch_pr"))?;
    let remotes = Remote::list(&repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(&branch, &remotes);
    if normalized.is_empty() {
        return Ok(None);
    }

    let cache_key = format!("{}::{}", repo_path.to_string_lossy(), normalized);
    if let Some(cached) = read_latest_branch_pr_cache(&cache_key) {
        return Ok(cached);
    }

    let latest = PrCache::fetch_latest_for_branch(&repo_path, normalized);
    let result = latest.map(|pr| BranchPrReference {
        number: pr.number,
        title: pr.title,
        state: pr.state,
        url: pr.url,
    });
    write_latest_branch_pr_cache(cache_key, result.clone());

    Ok(result)
}

/// Fetch CI run log for a specific check run/job ID (T011)
#[tauri::command]
pub fn fetch_ci_log(project_path: String, run_id: u64) -> Result<String, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_ci_log"))?;

    let output = gwt_core::git::graphql::gh_run_view_log(&repo_path, run_id)
        .map_err(|e| StructuredError::internal(&e, "fetch_ci_log"))?;
    Ok(output)
}

/// Update a PR branch with the latest base branch changes (SPEC-de3290fc T008)
#[tauri::command]
pub fn update_pr_branch(project_path: String, pr_number: u64) -> Result<String, String> {
    use gwt_core::git::gh_cli::gh_command;
    use gwt_core::git::resolve_repo_slug;

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let slug = resolve_repo_slug(&repo_path)
        .ok_or_else(|| "Failed to resolve repository slug".to_string())?;
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repo slug: {}", slug));
    }
    let (owner, repo) = (parts[0], parts[1]);

    let output = gh_command()
        .args([
            "api",
            "-X",
            "PUT",
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}/update-branch"),
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to update PR branch: {}", stderr));
    }

    Ok("Branch updated successfully".to_string())
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
                merge_state_status: None,
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
                    is_required: None,
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
            merge_state_status: None,
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
            merge_state_status: None,
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
                is_required: None,
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
            merge_state_status: None,
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

    #[test]
    fn test_branch_pr_reference_serialization() {
        let pr = BranchPrReference {
            number: 123,
            title: "Test PR".to_string(),
            state: "OPEN".to_string(),
            url: Some("https://github.com/example/repo/pull/123".to_string()),
        };

        let json = serde_json::to_string(&pr).unwrap();
        assert!(json.contains("\"number\":123"));
        assert!(json.contains("\"state\":\"OPEN\""));
        assert!(json.contains("\"url\":\"https://github.com/example/repo/pull/123\""));
    }

    #[test]
    fn test_strip_known_remote_prefix_for_origin_and_custom_remote() {
        let remotes = vec![
            Remote::new("origin", "git@github.com:o/r.git"),
            Remote::new("upstream", "git@github.com:o/r.git"),
        ];

        assert_eq!(
            strip_known_remote_prefix("origin/feature/x", &remotes),
            "feature/x"
        );
        assert_eq!(
            strip_known_remote_prefix("upstream/feature/x", &remotes),
            "feature/x"
        );
        assert_eq!(
            strip_known_remote_prefix("fork/feature/x", &remotes),
            "fork/feature/x"
        );
    }
}
