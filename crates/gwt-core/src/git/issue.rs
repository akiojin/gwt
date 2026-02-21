//! GitHub Issue operations (SPEC-e4798383)
//!
//! Provides Issue information using GitHub CLI (gh) for branch creation from issues.

use std::path::Path;

use super::gh_cli::{gh_command, is_gh_available};
use super::remote::Remote;
use super::repository::{find_bare_repo_in_dir, is_git_repo};

// `gh issue list --json comments` returns at most this many comments per issue.
const GH_COMMENTS_PREVIEW_LIMIT: u32 = 100;

/// Result of fetching issues with pagination info
#[derive(Debug, Clone)]
pub struct FetchIssuesResult {
    /// Fetched issues
    pub issues: Vec<GitHubIssue>,
    /// Whether there are more issues available on the next page
    pub has_next_page: bool,
}

/// GitHub label information
#[derive(Debug, Clone)]
pub struct GitHubLabel {
    /// Label name
    pub name: String,
    /// Label color (hex without #)
    pub color: String,
}

/// GitHub assignee information
#[derive(Debug, Clone)]
pub struct GitHubAssignee {
    /// GitHub login
    pub login: String,
    /// Avatar URL
    pub avatar_url: String,
}

/// GitHub milestone information
#[derive(Debug, Clone)]
pub struct GitHubMilestone {
    /// Milestone title
    pub title: String,
    /// Milestone number
    pub number: u32,
}

/// GitHub Issue information
#[derive(Debug, Clone)]
pub struct GitHubIssue {
    /// Issue number
    pub number: u64,
    /// Issue title
    pub title: String,
    /// Issue updatedAt timestamp (ISO-8601)
    pub updated_at: String,
    /// Issue labels
    pub labels: Vec<GitHubLabel>,
    /// Issue body (markdown)
    pub body: Option<String>,
    /// Issue state ("OPEN" or "CLOSED")
    pub state: String,
    /// Issue HTML URL
    pub html_url: String,
    /// Assignees
    pub assignees: Vec<GitHubAssignee>,
    /// Number of comments
    pub comments_count: u32,
    /// Milestone
    pub milestone: Option<GitHubMilestone>,
}

impl GitHubIssue {
    /// Create a new GitHubIssue with default values for extended fields
    pub fn new(number: u64, title: String, updated_at: String) -> Self {
        Self {
            number,
            title,
            updated_at,
            labels: Vec::new(),
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: Vec::new(),
            comments_count: 0,
            milestone: None,
        }
    }

    /// Create a new GitHubIssue with labels
    pub fn with_labels(
        number: u64,
        title: String,
        updated_at: String,
        labels: Vec<GitHubLabel>,
    ) -> Self {
        Self {
            number,
            title,
            updated_at,
            labels,
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: Vec::new(),
            comments_count: 0,
            milestone: None,
        }
    }

    /// Format issue for display: "#42: Fix login bug"
    pub fn display(&self) -> String {
        format!("#{}: {}", self.number, self.title)
    }

    /// Format issue for display with truncation
    /// Returns "#42: Title..." if title exceeds max_width
    pub fn display_truncated(&self, max_width: usize) -> String {
        let prefix = format!("#{}: ", self.number);
        let prefix_len = prefix.len();

        if prefix_len >= max_width {
            return format!("#{}", self.number);
        }

        let available_width = max_width - prefix_len;
        if self.title.len() <= available_width {
            format!("{}{}", prefix, self.title)
        } else if available_width <= 3 {
            format!("{}...", prefix)
        } else {
            let truncated: String = self.title.chars().take(available_width - 3).collect();
            format!("{}{}...", prefix, truncated)
        }
    }

    /// Generate branch name suffix from issue: "issue-42"
    pub fn branch_name_suffix(&self) -> String {
        format!("issue-{}", self.number)
    }
}

/// Check if GitHub CLI (gh) is available
pub fn is_gh_cli_available() -> bool {
    is_gh_available()
}

/// Check if GitHub CLI (gh) is authenticated (FR-003)
///
/// Runs `gh auth status` and returns true if the exit code is 0.
pub fn is_gh_cli_authenticated() -> bool {
    gh_command()
        .args(["auth", "status"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Fetch issues from GitHub using gh CLI with pagination support (FR-001)
///
/// Returns issues sorted by updated_at descending (most recently updated first).
/// Uses `page` and `per_page` to control pagination.
/// `state` controls which issues to fetch ("open" or "closed").
/// `has_next_page` is determined by requesting `per_page * page + 1` items
/// and checking if more exist beyond the current page.
pub fn fetch_open_issues(
    repo_path: &Path,
    page: u32,
    per_page: u32,
    state: &str,
) -> Result<FetchIssuesResult, String> {
    if page == 0 {
        return Err("page must be greater than 0".to_string());
    }
    if per_page == 0 {
        return Err("per_page must be greater than 0".to_string());
    }

    let repo_slug = resolve_repo_slug(repo_path);
    let args = issue_list_args(repo_slug.as_deref(), page, per_page, state);

    let output = gh_command()
        .args(args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh CLI: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all_issues = parse_gh_issues_json(&stdout)?;

    // Skip items from previous pages. Conversion is checked to avoid platform-size overflow.
    let skip_u64 = u64::from(page - 1) * u64::from(per_page);
    let skip = usize::try_from(skip_u64)
        .map_err(|_| "Pagination values are too large for this platform".to_string())?;
    let remaining: Vec<GitHubIssue> = all_issues.into_iter().skip(skip).collect();

    // If we got more than per_page items after skipping, there's a next page
    let has_next_page = remaining.len() > per_page as usize;
    let mut issues: Vec<GitHubIssue> = remaining.into_iter().take(per_page as usize).collect();

    for issue in &mut issues {
        hydrate_comments_count_from_rest_if_needed(repo_path, repo_slug.as_deref(), issue);
    }

    Ok(FetchIssuesResult {
        issues,
        has_next_page,
    })
}

fn issue_list_args(repo_slug: Option<&str>, page: u32, per_page: u32, state: &str) -> Vec<String> {
    // Request enough items to cover the current page plus one extra to detect next page
    let limit = u64::from(per_page) * u64::from(page) + 1;

    let limit_str = limit.to_string();
    let state_value = if state == "closed" { "closed" } else { "open" };
    let mut args = vec![
        "issue",
        "list",
        "--state",
        state_value,
        "--json",
        "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone",
        "--limit",
        &limit_str,
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<String>>();

    if let Some(slug) = repo_slug {
        args.push("--repo".to_string());
        args.push(slug.to_string());
    }

    args
}

fn hydrate_comments_count_from_rest_if_needed(
    repo_path: &Path,
    repo_slug: Option<&str>,
    issue: &mut GitHubIssue,
) {
    if issue.comments_count < GH_COMMENTS_PREVIEW_LIMIT {
        return;
    }

    let slug = repo_slug
        .map(|value| value.to_string())
        .or_else(|| parse_repo_slug_from_issue_html_url(&issue.html_url));
    let Some(slug) = slug else {
        return;
    };

    if let Ok(total_count) = fetch_issue_comments_total_count(repo_path, &slug, issue.number) {
        issue.comments_count = total_count;
    }
}

fn fetch_issue_comments_total_count(
    repo_path: &Path,
    repo_slug: &str,
    issue_number: u64,
) -> Result<u32, String> {
    let endpoint = format!("repos/{}/issues/{}", repo_slug, issue_number);
    let output = gh_command()
        .args(["api", &endpoint, "--jq", ".comments"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh api for issue comments: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api comments count failed: {}", stderr));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let parsed = raw
        .trim()
        .parse::<u64>()
        .map_err(|e| format!("Failed to parse comments count: {}", e))?;
    u32::try_from(parsed).map_err(|_| "comments count exceeds u32".to_string())
}

pub fn resolve_repo_slug(repo_path: &Path) -> Option<String> {
    let candidate_repo = if is_git_repo(repo_path) {
        Some(repo_path.to_path_buf())
    } else {
        find_bare_repo_in_dir(repo_path)
    }?;

    let remote = Remote::default(&candidate_repo).ok().flatten()?;
    parse_repo_slug_from_remote_url(&remote.fetch_url)
        .or_else(|| parse_repo_slug_from_remote_url(&remote.push_url))
}

fn parse_repo_slug_from_remote_url(remote_url: &str) -> Option<String> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("file://") {
        return None;
    }

    if let Some(rest) = trimmed.split("://").nth(1) {
        // Strip userinfo if present (e.g., git@host)
        let rest = rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest);
        let path_idx = rest.find('/').or_else(|| rest.find(':'))?;
        let path = &rest[path_idx + 1..];
        return normalize_repo_slug(path);
    }

    if let Some(at_pos) = trimmed.find('@') {
        let after_at = &trimmed[at_pos + 1..];
        if let Some(colon_pos) = after_at.find(':') {
            let path = &after_at[colon_pos + 1..];
            return normalize_repo_slug(path);
        }
        if let Some(slash_pos) = after_at.find('/') {
            let path = &after_at[slash_pos + 1..];
            return normalize_repo_slug(path);
        }
    }

    None
}

fn normalize_repo_slug(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{}/{}", owner, repo))
}

fn parse_repo_slug_from_issue_html_url(html_url: &str) -> Option<String> {
    let rest = html_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(html_url);
    let mut parts = rest.split('/');
    let _host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{}/{}", owner, repo))
}

fn parse_comments_count(item: &serde_json::Value) -> u32 {
    if let Some(count) = item.get("commentsCount").and_then(|v| v.as_u64()) {
        return u32::try_from(count).unwrap_or(u32::MAX);
    }

    let Some(comments) = item.get("comments") else {
        return 0;
    };

    if let Some(count) = comments.as_u64() {
        return u32::try_from(count).unwrap_or(u32::MAX);
    }

    if let Some(count) = comments.get("totalCount").and_then(|v| v.as_u64()) {
        return u32::try_from(count).unwrap_or(u32::MAX);
    }

    comments.as_array().map(|arr| arr.len() as u32).unwrap_or(0)
}

/// Parse gh issue list/view JSON output
pub fn parse_gh_issues_json(json: &str) -> Result<Vec<GitHubIssue>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let issues = parsed
        .as_array()
        .ok_or_else(|| "Expected JSON array".to_string())?;

    let mut result: Vec<GitHubIssue> = issues
        .iter()
        .filter_map(|item| {
            let number = item.get("number")?.as_u64()?;
            let title = item.get("title")?.as_str()?.to_string();
            let updated_at = item.get("updatedAt")?.as_str()?.to_string();
            let labels = item
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|label| {
                            let name = label.get("name")?.as_str()?.to_string();
                            let color = label
                                .get("color")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            Some(GitHubLabel { name, color })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let body = item.get("body").and_then(|v| v.as_str()).map(String::from);
            let state = item
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("OPEN")
                .to_string();
            let html_url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let assignees = item
                .get("assignees")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| {
                            let login = a.get("login")?.as_str()?.to_string();
                            let avatar_url = a
                                .get("avatarUrl")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            Some(GitHubAssignee { login, avatar_url })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let comments_count = parse_comments_count(item);
            let milestone = item.get("milestone").and_then(|v| {
                if v.is_null() {
                    return None;
                }
                let title = v.get("title")?.as_str()?.to_string();
                let number = v.get("number")?.as_u64()? as u32;
                Some(GitHubMilestone { title, number })
            });

            Some(GitHubIssue {
                number,
                title,
                updated_at,
                labels,
                body,
                state,
                html_url,
                assignees,
                comments_count,
                milestone,
            })
        })
        .collect();

    // Sort by updated_at descending (FR-006)
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(result)
}

/// Parse gh issue view JSON output (single issue)
fn parse_gh_issue_json(json: &str) -> Result<GitHubIssue, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Wrap in array and reuse parse_gh_issues_json logic
    let array_json = format!("[{}]", json.trim());
    let mut issues = parse_gh_issues_json(&array_json)?;

    if issues.is_empty() {
        // Try parsing the original value for a better error message
        if parsed.is_object() {
            return Err("Failed to extract issue fields from JSON".to_string());
        }
        return Err("Expected JSON object for issue detail".to_string());
    }

    Ok(issues.remove(0))
}

/// Fetch a single issue detail from GitHub using gh CLI
pub fn fetch_issue_detail(repo_path: &Path, issue_number: u64) -> Result<GitHubIssue, String> {
    let repo_slug = resolve_repo_slug(repo_path);

    let mut args = vec![
        "issue".to_string(),
        "view".to_string(),
        issue_number.to_string(),
        "--json".to_string(),
        "number,title,body,state,url,labels,assignees,comments,milestone,updatedAt".to_string(),
    ];

    if let Some(slug) = repo_slug.as_deref() {
        args.push("--repo".to_string());
        args.push(slug.to_string());
    }

    let output = gh_command()
        .args(&args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh CLI: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut issue = parse_gh_issue_json(&stdout)?;
    hydrate_comments_count_from_rest_if_needed(repo_path, repo_slug.as_deref(), &mut issue);
    Ok(issue)
}

/// Filter issues by title (case-insensitive substring match)
pub fn filter_issues_by_title<'a>(issues: &'a [GitHubIssue], query: &str) -> Vec<&'a GitHubIssue> {
    if query.is_empty() {
        return issues.iter().collect();
    }

    let query_lower = query.to_lowercase();
    issues
        .iter()
        .filter(|issue| issue.title.to_lowercase().contains(&query_lower))
        .collect()
}

/// Check if a branch for the given issue already exists
/// Searches for branches containing "issue-{number}" pattern
pub fn find_branch_for_issue(
    repo_path: &Path,
    issue_number: u64,
) -> Result<Option<String>, String> {
    let pattern = format!("issue-{}", issue_number);

    let output = crate::process::command("git")
        .args(["branch", "--list", &format!("*{}*", pattern)])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch: {}", e))?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let branches: Vec<&str> = stdout
        .lines()
        .map(|line| line.trim().trim_start_matches("* "))
        .filter(|branch| branch.contains(&pattern))
        .collect();

    Ok(branches.first().map(|s| s.to_string()))
}

/// Generate full branch name from type and issue
/// Format: "{type_prefix}issue-{number}" (e.g., "feature/issue-42")
pub fn generate_branch_name(type_prefix: &str, issue_number: u64) -> String {
    format!("{}issue-{}", type_prefix, issue_number)
}

fn issue_develop_args(issue_number: u64, branch_name: &str) -> Vec<String> {
    vec![
        "issue".to_string(),
        "develop".to_string(),
        issue_number.to_string(),
        "--name".to_string(),
        branch_name.to_string(),
        "--checkout=false".to_string(),
    ]
}

/// Create a branch linked to a GitHub Issue using `gh issue develop` (FR-016)
///
/// This creates a branch on GitHub that is officially linked to the issue,
/// appearing in the issue's "Development" section.
///
/// # Arguments
/// * `repo_path` - Path to the git repository
/// * `issue_number` - The GitHub issue number to link
/// * `branch_name` - Full branch name (e.g., "feature/issue-42")
///
/// # Returns
/// * `Ok(())` - Branch was successfully created and linked on GitHub
/// * `Err(String)` - Error message if the command failed
pub fn create_linked_branch(
    repo_path: &Path,
    issue_number: u64,
    branch_name: &str,
) -> Result<(), String> {
    // FR-016a: Use --name to specify branch name
    // FR-016b: Use --checkout=false so worktree handles checkout
    let output = gh_command()
        .args(issue_develop_args(issue_number, branch_name))
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh issue develop: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue develop failed: {}", stderr.trim()));
    }

    // FR-019: Log success (caller should handle actual logging)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================
    // FR-007: Display format tests
    // ==========================================================

    #[test]
    fn test_issue_display_format() {
        let issue = GitHubIssue::new(
            42,
            "Fix login bug".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        assert_eq!(issue.display(), "#42: Fix login bug");
    }

    #[test]
    fn test_issue_display_with_special_characters() {
        let issue = GitHubIssue::new(
            123,
            "Fix \"quoted\" text & <tags>".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        assert_eq!(issue.display(), "#123: Fix \"quoted\" text & <tags>");
    }

    // ==========================================================
    // FR-008b: Truncation tests
    // ==========================================================

    #[test]
    fn test_issue_display_truncated_short_title() {
        let issue = GitHubIssue::new(42, "Short".to_string(), "2025-01-25T10:00:00Z".to_string());
        // "#42: Short" = 10 chars, max_width=20 should not truncate
        assert_eq!(issue.display_truncated(20), "#42: Short");
    }

    #[test]
    fn test_issue_display_truncated_long_title() {
        let issue = GitHubIssue::new(
            42,
            "This is a very long title that needs truncation".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        // "#42: " = 5 chars, available = 15 chars, title gets truncated to 12 + "..."
        let result = issue.display_truncated(20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 20);
        assert!(result.starts_with("#42: "));
    }

    #[test]
    fn test_issue_display_truncated_exact_fit() {
        let issue = GitHubIssue::new(1, "Exact".to_string(), "2025-01-25T10:00:00Z".to_string());
        // "#1: Exact" = 9 chars
        assert_eq!(issue.display_truncated(9), "#1: Exact");
    }

    // ==========================================================
    // FR-009: Branch name generation tests
    // ==========================================================

    #[test]
    fn test_branch_name_suffix() {
        let issue = GitHubIssue::new(
            42,
            "Fix login bug".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        assert_eq!(issue.branch_name_suffix(), "issue-42");
    }

    #[test]
    fn test_generate_branch_name_feature() {
        assert_eq!(generate_branch_name("feature/", 42), "feature/issue-42");
    }

    #[test]
    fn test_generate_branch_name_bugfix() {
        assert_eq!(generate_branch_name("bugfix/", 10), "bugfix/issue-10");
    }

    #[test]
    fn test_generate_branch_name_hotfix() {
        assert_eq!(generate_branch_name("hotfix/", 5), "hotfix/issue-5");
    }

    #[test]
    fn test_generate_branch_name_release() {
        assert_eq!(generate_branch_name("release/", 100), "release/issue-100");
    }

    // ==========================================================
    // FR-016b: gh issue develop args tests
    // ==========================================================

    // ==========================================================
    // FR-003: gh CLI authentication tests
    // ==========================================================

    #[test]
    fn test_is_gh_cli_authenticated_returns_bool() {
        // This test verifies the function runs without panic.
        // The actual return value depends on the environment.
        let _result: bool = is_gh_cli_authenticated();
    }

    // ==========================================================
    // FR-016b: gh issue develop args tests
    // ==========================================================

    #[test]
    fn test_issue_develop_args_includes_checkout_false() {
        let args = issue_develop_args(42, "feature/issue-42");
        assert_eq!(
            args,
            vec![
                "issue",
                "develop",
                "42",
                "--name",
                "feature/issue-42",
                "--checkout=false"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    // ==========================================================
    // FR-006: JSON parsing and sorting tests
    // ==========================================================

    #[test]
    fn test_parse_gh_issues_json_valid() {
        let json = r#"[
            {"number": 42, "title": "Fix login bug", "updatedAt": "2025-01-25T10:00:00Z"},
            {"number": 10, "title": "Update docs", "updatedAt": "2025-01-24T08:00:00Z"}
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 42); // More recent first
        assert_eq!(issues[1].number, 10);
    }

    #[test]
    fn test_parse_gh_issues_json_sorted_by_updated_at() {
        let json = r#"[
            {"number": 1, "title": "Oldest", "updatedAt": "2025-01-01T00:00:00Z"},
            {"number": 2, "title": "Newest", "updatedAt": "2025-01-25T00:00:00Z"},
            {"number": 3, "title": "Middle", "updatedAt": "2025-01-15T00:00:00Z"}
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues[0].number, 2); // Newest first
        assert_eq!(issues[1].number, 3); // Middle
        assert_eq!(issues[2].number, 1); // Oldest last
    }

    #[test]
    fn test_parse_gh_issues_json_with_labels() {
        let json = r#"[
            {
                "number": 42,
                "title": "Fix login bug",
                "updatedAt": "2025-01-25T10:00:00Z",
                "labels": [
                    {"name": "bug", "color": "d73a4a"},
                    {"name": "priority: high", "color": "ff0000"}
                ]
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].labels.len(), 2);
        assert_eq!(issues[0].labels[0].name, "bug");
        assert_eq!(issues[0].labels[0].color, "d73a4a");
        assert_eq!(issues[0].labels[1].name, "priority: high");
        assert_eq!(issues[0].labels[1].color, "ff0000");
    }

    #[test]
    fn test_parse_gh_issues_json_without_labels_field() {
        let json = r#"[
            {"number": 42, "title": "Fix login bug", "updatedAt": "2025-01-25T10:00:00Z"}
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert!(issues[0].labels.is_empty());
    }

    #[test]
    fn test_parse_gh_issues_json_empty_labels() {
        let json = r#"[
            {"number": 42, "title": "Fix login bug", "updatedAt": "2025-01-25T10:00:00Z", "labels": []}
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert!(issues[0].labels.is_empty());
    }

    #[test]
    fn test_github_issue_with_labels_constructor() {
        let labels = vec![
            GitHubLabel {
                name: "bug".to_string(),
                color: "d73a4a".to_string(),
            },
            GitHubLabel {
                name: "urgent".to_string(),
                color: "ff0000".to_string(),
            },
        ];
        let issue = GitHubIssue::with_labels(
            42,
            "Fix bug".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
            labels,
        );
        assert_eq!(issue.number, 42);
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.labels[1].name, "urgent");
    }

    #[test]
    fn test_github_issue_new_has_empty_labels() {
        let issue = GitHubIssue::new(1, "Test".to_string(), "2025-01-25T10:00:00Z".to_string());
        assert!(issue.labels.is_empty());
    }

    #[test]
    fn test_parse_gh_issues_json_empty() {
        let json = "[]";
        let issues = parse_gh_issues_json(json).unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn test_parse_gh_issues_json_invalid() {
        let json = "not valid json";
        let result = parse_gh_issues_json(json);
        assert!(result.is_err());
    }

    // ==========================================================
    // FR-008: Filter tests
    // ==========================================================

    #[test]
    fn test_filter_issues_by_title_match() {
        let issues = vec![
            GitHubIssue::new(
                1,
                "Fix login bug".to_string(),
                "2025-01-25T10:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                2,
                "Update documentation".to_string(),
                "2025-01-24T08:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                3,
                "Login page redesign".to_string(),
                "2025-01-23T06:00:00Z".to_string(),
            ),
        ];

        let filtered = filter_issues_by_title(&issues, "login");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].number, 1);
        assert_eq!(filtered[1].number, 3);
    }

    #[test]
    fn test_filter_issues_by_title_case_insensitive() {
        let issues = vec![
            GitHubIssue::new(
                1,
                "Fix LOGIN Bug".to_string(),
                "2025-01-25T10:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                2,
                "login fix".to_string(),
                "2025-01-24T08:00:00Z".to_string(),
            ),
        ];

        let filtered = filter_issues_by_title(&issues, "LOGIN");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_issues_by_title_no_match() {
        let issues = vec![GitHubIssue::new(
            1,
            "Fix bug".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        )];

        let filtered = filter_issues_by_title(&issues, "nonexistent");
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_issues_by_title_empty_query() {
        let issues = vec![
            GitHubIssue::new(
                1,
                "Issue one".to_string(),
                "2025-01-25T10:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                2,
                "Issue two".to_string(),
                "2025-01-24T08:00:00Z".to_string(),
            ),
        ];

        let filtered = filter_issues_by_title(&issues, "");
        assert_eq!(filtered.len(), 2);
    }

    // ==========================================================
    // FR-011: Duplicate detection tests
    // ==========================================================

    // Note: find_branch_for_issue requires git repo, tested via integration tests

    // ==========================================================
    // Edge cases
    // ==========================================================

    #[test]
    fn test_issue_with_large_number() {
        let issue = GitHubIssue::new(
            999999,
            "Large number".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        assert_eq!(issue.display(), "#999999: Large number");
        assert_eq!(issue.branch_name_suffix(), "issue-999999");
    }

    #[test]
    fn test_issue_with_empty_title() {
        let issue = GitHubIssue::new(1, "".to_string(), "2025-01-25T10:00:00Z".to_string());
        assert_eq!(issue.display(), "#1: ");
    }

    #[test]
    fn test_issue_with_unicode_title() {
        let issue = GitHubIssue::new(
            1,
            "日本語タイトル".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        );
        assert_eq!(issue.display(), "#1: 日本語タイトル");
    }

    // ==========================================================
    // FR-005d: gh issue list repo resolution tests
    // ==========================================================

    #[test]
    fn test_issue_list_args_without_repo_page1() {
        // page=1, per_page=50 → limit = 50*1+1 = 51
        let args = issue_list_args(None, 1, 50, "open");
        assert_eq!(
            args,
            vec![
                "issue",
                "list",
                "--state",
                "open",
                "--json",
                "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone",
                "--limit",
                "51"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_issue_list_args_with_repo_page1() {
        let args = issue_list_args(Some("owner/repo"), 1, 50, "open");
        assert_eq!(
            args,
            vec![
                "issue",
                "list",
                "--state",
                "open",
                "--json",
                "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone",
                "--limit",
                "51",
                "--repo",
                "owner/repo"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_issue_list_args_page2() {
        // page=2, per_page=50 → limit = 50*2+1 = 101
        let args = issue_list_args(None, 2, 50, "open");
        assert_eq!(
            args,
            vec![
                "issue",
                "list",
                "--state",
                "open",
                "--json",
                "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone",
                "--limit",
                "101"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_issue_list_args_custom_per_page() {
        // page=1, per_page=10 → limit = 10*1+1 = 11
        let args = issue_list_args(None, 1, 10, "open");
        assert_eq!(
            args,
            vec![
                "issue",
                "list",
                "--state",
                "open",
                "--json",
                "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone",
                "--limit",
                "11"
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_issue_list_args_closed_state() {
        let args = issue_list_args(None, 1, 10, "closed");
        assert!(args.contains(&"closed".to_string()));
    }

    #[test]
    fn test_issue_list_args_large_values_do_not_overflow() {
        let args = issue_list_args(None, u32::MAX, u32::MAX, "open");
        let expected_limit = (u64::from(u32::MAX) * u64::from(u32::MAX) + 1).to_string();

        assert!(args
            .windows(2)
            .any(|w| w[0] == "--limit" && w[1] == expected_limit));
    }

    #[test]
    fn test_fetch_open_issues_rejects_page_zero() {
        let err = fetch_open_issues(std::path::Path::new("."), 0, 50, "open").unwrap_err();
        assert!(err.contains("page must be greater than 0"));
    }

    #[test]
    fn test_fetch_open_issues_rejects_per_page_zero() {
        let err = fetch_open_issues(std::path::Path::new("."), 1, 0, "open").unwrap_err();
        assert!(err.contains("per_page must be greater than 0"));
    }

    // ==========================================================
    // Extended fields parsing tests
    // ==========================================================

    #[test]
    fn test_parse_gh_issues_json_extended_fields() {
        let json = r#"[
            {
                "number": 42,
                "title": "Fix login bug",
                "updatedAt": "2025-01-25T10:00:00Z",
                "labels": [{"name": "bug", "color": "d73a4a"}],
                "body": "This is the issue body",
                "state": "OPEN",
                "url": "https://github.com/user/repo/issues/42",
                "assignees": [{"login": "octocat", "avatarUrl": "https://avatars.example.com/1"}],
                "comments": [{"body": "comment1"}, {"body": "comment2"}],
                "milestone": {"title": "v1.0", "number": 1}
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert_eq!(issue.body.as_deref(), Some("This is the issue body"));
        assert_eq!(issue.state, "OPEN");
        assert_eq!(issue.html_url, "https://github.com/user/repo/issues/42");
        assert_eq!(issue.assignees.len(), 1);
        assert_eq!(issue.assignees[0].login, "octocat");
        assert_eq!(
            issue.assignees[0].avatar_url,
            "https://avatars.example.com/1"
        );
        assert_eq!(issue.comments_count, 2);
        assert!(issue.milestone.is_some());
        let ms = issue.milestone.as_ref().unwrap();
        assert_eq!(ms.title, "v1.0");
        assert_eq!(ms.number, 1);
    }

    #[test]
    fn test_parse_gh_issues_json_missing_optional_fields() {
        let json = r#"[
            {
                "number": 1,
                "title": "Simple issue",
                "updatedAt": "2025-01-25T10:00:00Z"
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        let issue = &issues[0];
        assert!(issue.body.is_none());
        assert_eq!(issue.state, "OPEN");
        assert_eq!(issue.html_url, "");
        assert!(issue.assignees.is_empty());
        assert_eq!(issue.comments_count, 0);
        assert!(issue.milestone.is_none());
    }

    #[test]
    fn test_parse_gh_issues_json_with_comment_total_count_object() {
        let json = r#"[
            {
                "number": 7,
                "title": "High traffic issue",
                "updatedAt": "2025-01-25T10:00:00Z",
                "comments": {"totalCount": 150}
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].comments_count, 150);
    }

    #[test]
    fn test_parse_gh_issues_json_with_numeric_comments() {
        let json = r#"[
            {
                "number": 8,
                "title": "REST-backed issue",
                "updatedAt": "2025-01-25T10:00:00Z",
                "comments": 12
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].comments_count, 12);
    }

    #[test]
    fn test_parse_gh_issue_json_single() {
        let json = r#"{
            "number": 42,
            "title": "Fix login bug",
            "updatedAt": "2025-01-25T10:00:00Z",
            "labels": [],
            "body": "body text",
            "state": "OPEN",
            "url": "https://github.com/user/repo/issues/42",
            "assignees": [],
            "comments": [],
            "milestone": null
        }"#;

        let issue = parse_gh_issue_json(json).unwrap();
        assert_eq!(issue.number, 42);
        assert_eq!(issue.body.as_deref(), Some("body text"));
        assert!(issue.milestone.is_none());
    }

    #[test]
    fn test_parse_repo_slug_https() {
        let slug = parse_repo_slug_from_remote_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(slug, "user/repo");
    }

    #[test]
    fn test_parse_repo_slug_https_no_git_suffix() {
        let slug = parse_repo_slug_from_remote_url("https://github.com/user/repo").unwrap();
        assert_eq!(slug, "user/repo");
    }

    #[test]
    fn test_parse_repo_slug_https_trailing_slash() {
        let slug = parse_repo_slug_from_remote_url("https://github.com/user/repo/").unwrap();
        assert_eq!(slug, "user/repo");
    }

    #[test]
    fn test_parse_repo_slug_from_issue_html_url() {
        let slug = parse_repo_slug_from_issue_html_url("https://github.com/user/repo/issues/42");
        assert_eq!(slug.as_deref(), Some("user/repo"));
    }

    #[test]
    fn test_parse_repo_slug_ssh_scp_style() {
        let slug = parse_repo_slug_from_remote_url("git@github.com:user/repo.git").unwrap();
        assert_eq!(slug, "user/repo");
    }

    #[test]
    fn test_parse_repo_slug_ssh_url() {
        let slug = parse_repo_slug_from_remote_url("ssh://git@github.com/user/repo.git").unwrap();
        assert_eq!(slug, "user/repo");
    }

    #[test]
    fn test_parse_repo_slug_invalid_local_path() {
        assert!(parse_repo_slug_from_remote_url("/tmp/repo.git").is_none());
    }

    #[test]
    fn test_parse_repo_slug_rejects_extra_segments() {
        assert!(parse_repo_slug_from_remote_url("https://github.com/owner/repo/extra").is_none());
    }
}
