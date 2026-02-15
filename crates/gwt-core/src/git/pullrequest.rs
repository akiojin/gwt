//! Pull Request operations (FR-016)
//!
//! Provides PR information using GitHub CLI (gh) for branch-to-PR title mapping.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use super::gh_cli::{gh_command, is_gh_available};

/// Detailed PR status information retrieved via GraphQL API (SPEC-d6949f99)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusInfo {
    pub number: u64,
    pub title: String,
    /// "OPEN" | "CLOSED" | "MERGED"
    pub state: String,
    pub url: String,
    /// "MERGEABLE" | "CONFLICTING" | "UNKNOWN"
    pub mergeable: String,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub linked_issues: Vec<u64>,
    pub check_suites: Vec<WorkflowRunInfo>,
    pub reviews: Vec<ReviewInfo>,
    pub review_comments: Vec<ReviewComment>,
    pub changed_files_count: u64,
    pub additions: u64,
    pub deletions: u64,
}

/// CI/CD workflow run information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunInfo {
    pub workflow_name: String,
    pub run_id: u64,
    /// "queued" | "in_progress" | "completed"
    pub status: String,
    /// "success" | "failure" | "neutral" etc.
    pub conclusion: Option<String>,
}

/// PR review information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInfo {
    pub reviewer: String,
    /// "APPROVED" | "CHANGES_REQUESTED" | "COMMENTED" | "PENDING" | "DISMISSED"
    pub state: String,
}

/// PR review comment (inline comment)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub author: String,
    pub body: String,
    pub file_path: Option<String>,
    pub line: Option<u64>,
    pub code_snippet: Option<String>,
    pub created_at: String,
}

/// Pull Request information
#[derive(Debug, Clone)]
pub struct PullRequest {
    /// PR number
    pub number: u64,
    /// PR title
    pub title: String,
    /// Head branch name
    pub head_branch: String,
    /// PR state (open, closed, merged)
    pub state: String,
    /// PR URL (if available)
    pub url: Option<String>,
    /// PR updatedAt timestamp (ISO-8601, if available)
    pub updated_at: Option<String>,
}

/// Cache of PR information for a repository
#[derive(Debug, Default)]
pub struct PrCache {
    /// Map of branch name to PR info
    branch_to_pr: HashMap<String, PullRequest>,
    /// Whether the cache has been populated
    populated: bool,
}

impl PrCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the cache is populated
    pub fn is_populated(&self) -> bool {
        self.populated
    }

    /// Get PR info for a branch
    pub fn get(&self, branch: &str) -> Option<&PullRequest> {
        self.branch_to_pr.get(branch)
    }

    /// Get PR title for a branch (convenience method for FR-016)
    pub fn get_title(&self, branch: &str) -> Option<&str> {
        self.branch_to_pr.get(branch).map(|pr| pr.title.as_str())
    }

    /// Check if a branch has a merged PR
    pub fn is_merged(&self, branch: &str) -> bool {
        self.branch_to_pr
            .get(branch)
            .map(|pr| pr.state.eq_ignore_ascii_case("MERGED"))
            .unwrap_or(false)
    }

    /// Populate the cache with PR data from GitHub CLI
    pub fn populate(&mut self, repo_path: &Path) {
        if self.populated {
            return;
        }

        // Check if gh CLI is available
        if !is_gh_available() {
            self.populated = true;
            return;
        }

        // Fetch open and merged PRs
        if let Ok(prs) = fetch_prs(repo_path) {
            for pr in prs {
                let replace = match self.branch_to_pr.get(&pr.head_branch) {
                    Some(existing) => is_newer_pr(&pr, existing),
                    None => true,
                };
                if replace {
                    self.branch_to_pr.insert(pr.head_branch.clone(), pr);
                }
            }
        }

        self.populated = true;
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.branch_to_pr.clear();
        self.populated = false;
    }

    /// Fetch latest PR for a specific branch using GitHub CLI
    pub fn fetch_latest_for_branch(repo_path: &Path, branch: &str) -> Option<PullRequest> {
        if !is_gh_available() {
            return None;
        }

        let output = gh_command()
            .args([
                "pr",
                "list",
                "--state",
                "all",
                "--head",
                branch,
                "--limit",
                "20",
                "--json",
                "number,title,headRefName,state,url,updatedAt",
            ])
            .current_dir(repo_path)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let prs = parse_gh_pr_json(&stdout).ok()?;

        select_latest_pr(prs)
    }
}

/// Cache of PR status information for branches (SPEC-d6949f99)
#[derive(Debug, Default)]
pub struct PrStatusCache {
    /// Map of branch name to PR status info
    statuses: HashMap<String, PrStatusInfo>,
    /// Whether the cache has been populated
    populated: bool,
}

impl PrStatusCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the cache is populated
    pub fn is_populated(&self) -> bool {
        self.populated
    }

    /// Get PR status info for a branch
    pub fn get(&self, branch: &str) -> Option<&PrStatusInfo> {
        self.statuses.get(branch)
    }

    /// Refresh cache by fetching PR statuses via GraphQL.
    /// On rate-limit or network errors, the existing cache is preserved.
    pub fn refresh(&mut self, repo_path: &Path, branch_names: &[String]) {
        match super::graphql::fetch_pr_statuses(repo_path, branch_names) {
            Ok(results) => {
                self.statuses.clear();
                for (branch, info) in results {
                    if let Some(info) = info {
                        self.statuses.insert(branch, info);
                    }
                }
                self.populated = true;
            }
            Err(_) => {
                // On error (rate limit, network etc.), preserve existing cache
                // but still mark as populated so we don't retry immediately
                self.populated = true;
            }
        }
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.statuses.clear();
        self.populated = false;
    }
}

/// Fetch PRs using GitHub CLI
fn fetch_prs(repo_path: &Path) -> Result<Vec<PullRequest>, std::io::Error> {
    let mut prs = Vec::new();

    // Fetch open PRs
    if let Ok(open_prs) = fetch_prs_by_state(repo_path, "open") {
        prs.extend(open_prs);
    }

    // Fetch recently merged PRs (last 50)
    if let Ok(merged_prs) = fetch_prs_by_state(repo_path, "merged") {
        prs.extend(merged_prs);
    }

    Ok(prs)
}

/// Fetch PRs by state using GitHub CLI
fn fetch_prs_by_state(repo_path: &Path, state: &str) -> Result<Vec<PullRequest>, std::io::Error> {
    // gh pr list --state open --json number,title,headRefName,state --limit 100
    let output = gh_command()
        .args([
            "pr",
            "list",
            "--state",
            state,
            "--json",
            "number,title,headRefName,state,url,updatedAt",
            "--limit",
            "100",
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_gh_pr_json(&stdout)
}

/// Parse GitHub CLI JSON output
fn parse_gh_pr_json(json_str: &str) -> Result<Vec<PullRequest>, std::io::Error> {
    // Simple JSON parsing without adding serde_json dependency to this module
    // Format: [{"headRefName":"branch","number":123,"state":"OPEN","title":"Title"}]
    let mut prs = Vec::new();

    // Use serde_json for parsing
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(arr) = value.as_array() {
            for item in arr {
                if let (Some(number), Some(title), Some(head_branch), Some(state)) = (
                    item.get("number").and_then(|n| n.as_u64()),
                    item.get("title").and_then(|t| t.as_str()),
                    item.get("headRefName").and_then(|h| h.as_str()),
                    item.get("state").and_then(|s| s.as_str()),
                ) {
                    let url = item
                        .get("url")
                        .and_then(|u| u.as_str())
                        .map(|u| u.to_string());
                    let updated_at = item
                        .get("updatedAt")
                        .and_then(|u| u.as_str())
                        .map(|u| u.to_string());
                    prs.push(PullRequest {
                        number,
                        title: title.to_string(),
                        head_branch: head_branch.to_string(),
                        state: state.to_string(),
                        url,
                        updated_at,
                    });
                }
            }
        }
    }

    Ok(prs)
}

fn select_latest_pr(prs: Vec<PullRequest>) -> Option<PullRequest> {
    let mut selected: Option<PullRequest> = None;
    for pr in prs {
        selected = match selected {
            Some(current) => {
                if is_newer_pr(&pr, &current) {
                    Some(pr)
                } else {
                    Some(current)
                }
            }
            None => Some(pr),
        };
    }
    selected
}

fn is_newer_pr(candidate: &PullRequest, current: &PullRequest) -> bool {
    let candidate_open = candidate.state.eq_ignore_ascii_case("OPEN");
    let current_open = current.state.eq_ignore_ascii_case("OPEN");

    if candidate_open != current_open {
        return candidate_open;
    }

    match (&candidate.updated_at, &current.updated_at) {
        (Some(candidate_ts), Some(current_ts)) => candidate_ts > current_ts,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pr_cache_new() {
        let cache = PrCache::new();
        assert!(!cache.is_populated());
        assert!(cache.get("main").is_none());
    }

    #[test]
    fn test_parse_gh_pr_json() {
        let json = r#"[
            {"number": 123, "title": "Fix bug", "headRefName": "fix/bug", "state": "OPEN", "url": "https://github.com/a/b/pull/123", "updatedAt": "2024-01-01T00:00:00Z"},
            {"number": 456, "title": "Add feature", "headRefName": "feature/new", "state": "MERGED", "url": "https://github.com/a/b/pull/456", "updatedAt": "2024-01-02T00:00:00Z"}
        ]"#;

        let prs = parse_gh_pr_json(json).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 123);
        assert_eq!(prs[0].title, "Fix bug");
        assert_eq!(prs[0].head_branch, "fix/bug");
        assert_eq!(
            prs[0].url.as_deref(),
            Some("https://github.com/a/b/pull/123")
        );
        assert_eq!(prs[1].head_branch, "feature/new");
    }

    #[test]
    fn test_parse_gh_pr_json_empty() {
        let json = "[]";
        let prs = parse_gh_pr_json(json).unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn test_parse_gh_pr_json_invalid() {
        let json = "not json";
        let prs = parse_gh_pr_json(json).unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn test_pr_cache_is_merged() {
        let mut cache = PrCache::new();
        cache.branch_to_pr.insert(
            "feature/merged".to_string(),
            PullRequest {
                number: 1,
                title: "Merged".to_string(),
                head_branch: "feature/merged".to_string(),
                state: "MERGED".to_string(),
                url: None,
                updated_at: None,
            },
        );
        cache.branch_to_pr.insert(
            "feature/open".to_string(),
            PullRequest {
                number: 2,
                title: "Open".to_string(),
                head_branch: "feature/open".to_string(),
                state: "OPEN".to_string(),
                url: None,
                updated_at: None,
            },
        );

        assert!(cache.is_merged("feature/merged"));
        assert!(!cache.is_merged("feature/open"));
        assert!(!cache.is_merged("feature/missing"));
    }

    #[test]
    fn test_is_newer_pr_open_priority() {
        let current = PullRequest {
            number: 1,
            title: "Merged".to_string(),
            head_branch: "feature/test".to_string(),
            state: "MERGED".to_string(),
            url: None,
            updated_at: Some("2024-02-01T00:00:00Z".to_string()),
        };
        let candidate = PullRequest {
            number: 2,
            title: "Open".to_string(),
            head_branch: "feature/test".to_string(),
            state: "OPEN".to_string(),
            url: None,
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
        };

        assert!(is_newer_pr(&candidate, &current));
        assert!(!is_newer_pr(&current, &candidate));
    }

    #[test]
    fn test_is_newer_pr_updated_at_same_state() {
        let current = PullRequest {
            number: 1,
            title: "Old".to_string(),
            head_branch: "feature/test".to_string(),
            state: "OPEN".to_string(),
            url: None,
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
        };
        let candidate = PullRequest {
            number: 2,
            title: "New".to_string(),
            head_branch: "feature/test".to_string(),
            state: "OPEN".to_string(),
            url: None,
            updated_at: Some("2024-02-01T00:00:00Z".to_string()),
        };

        assert!(is_newer_pr(&candidate, &current));
        assert!(!is_newer_pr(&current, &candidate));
    }

    // ==========================================================
    // PrStatusInfo serialization/deserialization round-trip tests
    // ==========================================================

    #[test]
    fn test_pr_status_info_serialize_deserialize_roundtrip() {
        let info = PrStatusInfo {
            number: 42,
            title: "Add feature X".to_string(),
            state: "OPEN".to_string(),
            url: "https://github.com/owner/repo/pull/42".to_string(),
            mergeable: "MERGEABLE".to_string(),
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            labels: vec!["enhancement".to_string(), "ready".to_string()],
            assignees: vec!["bob".to_string()],
            milestone: Some("v2.0".to_string()),
            linked_issues: vec![10, 20],
            check_suites: vec![WorkflowRunInfo {
                workflow_name: "CI".to_string(),
                run_id: 12345,
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
            }],
            reviews: vec![ReviewInfo {
                reviewer: "charlie".to_string(),
                state: "APPROVED".to_string(),
            }],
            review_comments: vec![ReviewComment {
                author: "dave".to_string(),
                body: "Looks good".to_string(),
                file_path: Some("src/main.rs".to_string()),
                line: Some(42),
                code_snippet: Some("fn main()".to_string()),
                created_at: "2025-01-01T00:00:00Z".to_string(),
            }],
            changed_files_count: 5,
            additions: 100,
            deletions: 20,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PrStatusInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.number, 42);
        assert_eq!(deserialized.title, "Add feature X");
        assert_eq!(deserialized.state, "OPEN");
        assert_eq!(deserialized.mergeable, "MERGEABLE");
        assert_eq!(deserialized.author, "alice");
        assert_eq!(deserialized.base_branch, "main");
        assert_eq!(deserialized.head_branch, "feature/x");
        assert_eq!(deserialized.labels, vec!["enhancement", "ready"]);
        assert_eq!(deserialized.assignees, vec!["bob"]);
        assert_eq!(deserialized.milestone, Some("v2.0".to_string()));
        assert_eq!(deserialized.linked_issues, vec![10, 20]);
        assert_eq!(deserialized.check_suites.len(), 1);
        assert_eq!(deserialized.check_suites[0].workflow_name, "CI");
        assert_eq!(
            deserialized.check_suites[0].conclusion,
            Some("success".to_string())
        );
        assert_eq!(deserialized.reviews.len(), 1);
        assert_eq!(deserialized.reviews[0].reviewer, "charlie");
        assert_eq!(deserialized.review_comments.len(), 1);
        assert_eq!(
            deserialized.review_comments[0].file_path,
            Some("src/main.rs".to_string())
        );
        assert_eq!(deserialized.changed_files_count, 5);
        assert_eq!(deserialized.additions, 100);
        assert_eq!(deserialized.deletions, 20);
    }

    #[test]
    fn test_pr_status_info_camel_case_serialization() {
        let info = PrStatusInfo {
            number: 1,
            title: "Test".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com".to_string(),
            mergeable: "UNKNOWN".to_string(),
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/test".to_string(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![],
            review_comments: vec![],
            changed_files_count: 0,
            additions: 0,
            deletions: 0,
        };

        let json = serde_json::to_string(&info).unwrap();
        // Verify camelCase field names
        assert!(json.contains("\"baseBranch\""));
        assert!(json.contains("\"headBranch\""));
        assert!(json.contains("\"checkSuites\""));
        assert!(json.contains("\"linkedIssues\""));
        assert!(json.contains("\"changedFilesCount\""));
        assert!(json.contains("\"reviewComments\""));
        // Should NOT contain snake_case
        assert!(!json.contains("\"base_branch\""));
        assert!(!json.contains("\"head_branch\""));
    }

    #[test]
    fn test_pr_status_info_optional_fields_none() {
        let info = PrStatusInfo {
            number: 1,
            title: "No milestone".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com".to_string(),
            mergeable: "UNKNOWN".to_string(),
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/test".to_string(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![],
            review_comments: vec![],
            changed_files_count: 0,
            additions: 0,
            deletions: 0,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PrStatusInfo = serde_json::from_str(&json).unwrap();
        assert!(deserialized.milestone.is_none());
    }

    #[test]
    fn test_workflow_run_info_roundtrip() {
        let info = WorkflowRunInfo {
            workflow_name: "Build & Test".to_string(),
            run_id: 999,
            status: "in_progress".to_string(),
            conclusion: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: WorkflowRunInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.workflow_name, "Build & Test");
        assert_eq!(deserialized.run_id, 999);
        assert_eq!(deserialized.status, "in_progress");
        assert!(deserialized.conclusion.is_none());
    }

    #[test]
    fn test_review_info_roundtrip() {
        let info = ReviewInfo {
            reviewer: "reviewer1".to_string(),
            state: "CHANGES_REQUESTED".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ReviewInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.reviewer, "reviewer1");
        assert_eq!(deserialized.state, "CHANGES_REQUESTED");
    }

    #[test]
    fn test_review_comment_roundtrip() {
        let comment = ReviewComment {
            author: "commenter".to_string(),
            body: "Please fix this".to_string(),
            file_path: Some("src/lib.rs".to_string()),
            line: Some(10),
            code_snippet: Some("let x = 1;".to_string()),
            created_at: "2025-06-01T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&comment).unwrap();
        let deserialized: ReviewComment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.author, "commenter");
        assert_eq!(deserialized.body, "Please fix this");
        assert_eq!(deserialized.file_path, Some("src/lib.rs".to_string()));
        assert_eq!(deserialized.line, Some(10));
        assert_eq!(deserialized.code_snippet, Some("let x = 1;".to_string()));
    }

    #[test]
    fn test_review_comment_optional_fields_none() {
        let comment = ReviewComment {
            author: "user".to_string(),
            body: "General comment".to_string(),
            file_path: None,
            line: None,
            code_snippet: None,
            created_at: "2025-06-01T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&comment).unwrap();
        let deserialized: ReviewComment = serde_json::from_str(&json).unwrap();
        assert!(deserialized.file_path.is_none());
        assert!(deserialized.line.is_none());
        assert!(deserialized.code_snippet.is_none());
    }

    // ==========================================================
    // T008: PrStatusCache tests
    // ==========================================================

    #[test]
    fn test_pr_status_cache_new() {
        let cache = PrStatusCache::new();
        assert!(!cache.is_populated());
        assert!(cache.get("main").is_none());
    }

    #[test]
    fn test_pr_status_cache_clear() {
        let mut cache = PrStatusCache::new();
        cache.statuses.insert(
            "feature/x".to_string(),
            PrStatusInfo {
                number: 1,
                title: "Test".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.com".to_string(),
                mergeable: "UNKNOWN".to_string(),
                author: "user".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/x".to_string(),
                labels: vec![],
                assignees: vec![],
                milestone: None,
                linked_issues: vec![],
                check_suites: vec![],
                reviews: vec![],
                review_comments: vec![],
                changed_files_count: 0,
                additions: 0,
                deletions: 0,
            },
        );
        cache.populated = true;

        assert!(cache.get("feature/x").is_some());
        assert!(cache.is_populated());

        cache.clear();
        assert!(cache.get("feature/x").is_none());
        assert!(!cache.is_populated());
    }

    #[test]
    fn test_pr_status_cache_get_returns_correct_entry() {
        let mut cache = PrStatusCache::new();
        cache.statuses.insert(
            "feature/a".to_string(),
            PrStatusInfo {
                number: 10,
                title: "PR A".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.com/10".to_string(),
                mergeable: "MERGEABLE".to_string(),
                author: "alice".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/a".to_string(),
                labels: vec!["bug".to_string()],
                assignees: vec![],
                milestone: None,
                linked_issues: vec![],
                check_suites: vec![],
                reviews: vec![],
                review_comments: vec![],
                changed_files_count: 2,
                additions: 30,
                deletions: 5,
            },
        );

        let info = cache.get("feature/a").unwrap();
        assert_eq!(info.number, 10);
        assert_eq!(info.title, "PR A");
        assert_eq!(info.labels, vec!["bug"]);

        assert!(cache.get("feature/b").is_none());
    }

    #[test]
    fn test_select_latest_pr_prefers_open() {
        let merged = PullRequest {
            number: 1,
            title: "Merged".to_string(),
            head_branch: "feature/test".to_string(),
            state: "MERGED".to_string(),
            url: None,
            updated_at: Some("2024-02-01T00:00:00Z".to_string()),
        };
        let open = PullRequest {
            number: 2,
            title: "Open".to_string(),
            head_branch: "feature/test".to_string(),
            state: "OPEN".to_string(),
            url: None,
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let selected = select_latest_pr(vec![merged, open]).unwrap();
        assert_eq!(selected.state, "OPEN");
        assert_eq!(selected.number, 2);
    }
}
