//! GitHub Issue operations (SPEC-e4798383)
//!
//! Provides Issue information using GitHub CLI (gh) for branch creation from issues.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::gh_cli::{gh_command, is_gh_available, run_gh_output_with_repair};
use super::remote::Remote;
use super::repository::{find_bare_repo_in_dir, is_git_repo};

// `gh issue list --json comments` returns at most this many comments per issue.
const GH_COMMENTS_PREVIEW_LIMIT: u32 = 100;
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
const SPEC_LABEL: &str = "gwt-spec";
const SEARCH_API_MAX_RESULTS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueCategory {
    All,
    Issues,
    Specs,
}

/// Result of fetching issues with pagination info
#[derive(Debug, Clone)]
pub struct FetchIssuesResult {
    /// Fetched issues
    pub issues: Vec<GitHubIssue>,
    /// Whether there are more issues available on the next page
    pub has_next_page: bool,
}

/// Result of ensuring an issue-linked branch via `gh issue develop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueLinkedBranchStatus {
    /// The branch was newly created and linked to the issue.
    Created,
    /// The branch already existed and was confirmed as linked to the issue.
    AlreadyLinked,
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
/// `has_next_page` is derived from API metadata and pagination parameters.
pub fn fetch_open_issues(
    repo_path: &Path,
    page: u32,
    per_page: u32,
    state: &str,
) -> Result<FetchIssuesResult, String> {
    fetch_issues_with_options(repo_path, page, per_page, state, true, "all")
}

/// Fetch issues with category/body options.
///
/// `category` can be `"all"`, `"issues"` (exclude `gwt-spec`), or `"specs"` (only `gwt-spec`).
///
/// When a repo slug is available, uses the GitHub Search API for O(1) per-page pagination.
/// Falls back to `gh issue list` when the slug cannot be resolved.
pub fn fetch_issues_with_options(
    repo_path: &Path,
    page: u32,
    per_page: u32,
    state: &str,
    include_body: bool,
    category: &str,
) -> Result<FetchIssuesResult, String> {
    if page == 0 {
        return Err("page must be greater than 0".to_string());
    }
    if per_page == 0 {
        return Err("per_page must be greater than 0".to_string());
    }

    let repo_slug = resolve_repo_slug(repo_path);

    let cat = parse_issue_category(category);

    // Use Search API when repo slug is available (O(1) pagination)
    if let Some(ref slug) = repo_slug {
        return fetch_issues_via_search_api(
            repo_path,
            slug,
            page,
            per_page,
            state,
            include_body,
            cat,
        );
    }

    // Fallback to gh issue list (no repo slug available)
    let args = issue_list_args_with_options(
        None,
        page,
        per_page,
        state,
        include_body,
        cat,
    );

    let output = run_gh_output_with_repair(repo_path, args)
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
        // Keep list retrieval lightweight. Exact comment counts are resolved in detail view.
        if include_body {
            hydrate_comments_count_from_rest_if_needed(repo_path, None, issue);
        }
    }

    Ok(FetchIssuesResult {
        issues,
        has_next_page,
    })
}

/// Fetch issues using GitHub REST Search API (O(1) per page).
fn fetch_issues_via_search_api(
    repo_path: &Path,
    repo_slug: &str,
    page: u32,
    per_page: u32,
    state: &str,
    include_body: bool,
    category: IssueCategory,
) -> Result<FetchIssuesResult, String> {
    let endpoint = issue_search_api_endpoint(repo_slug, page, per_page, state, category);
    let output = run_gh_output_with_repair(repo_path, ["api", &endpoint])
        .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api search/issues failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = parse_search_issues_json(&stdout, page, per_page, include_body)?;

    if include_body {
        for issue in &mut result.issues {
            hydrate_comments_count_from_rest_if_needed(repo_path, Some(repo_slug), issue);
        }
    }

    Ok(result)
}

/// Build the Search API endpoint URL for issue listing.
fn issue_search_api_endpoint(
    repo_slug: &str,
    page: u32,
    per_page: u32,
    state: &str,
    category: IssueCategory,
) -> String {
    let state_value = if state == "closed" { "closed" } else { "open" };

    let mut query_parts = vec![
        format!("repo:{}", repo_slug),
        "is:issue".to_string(),
        format!("state:{}", state_value),
        "sort:updated-desc".to_string(),
    ];

    match category {
        IssueCategory::Specs => {
            query_parts.push(format!("label:{}", SPEC_LABEL));
        }
        IssueCategory::Issues => {
            query_parts.push(format!("-label:{}", SPEC_LABEL));
        }
        IssueCategory::All => {}
    }

    let q = query_parts.join("+");
    format!("search/issues?q={q}&per_page={per_page}&page={page}")
}

/// Parse a GitHub REST Search API response into `FetchIssuesResult`.
fn parse_search_issues_json(
    json: &str,
    page: u32,
    per_page: u32,
    include_body: bool,
) -> Result<FetchIssuesResult, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let items = parsed
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Expected 'items' array in search response".to_string())?;

    let total_count = parsed
        .get("total_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(items.len() as u64);
    let capped_total_count = total_count.min(SEARCH_API_MAX_RESULTS);
    let shown_count = u64::from(page).saturating_mul(u64::from(per_page));
    let has_next_page = shown_count < capped_total_count;

    let issues: Vec<GitHubIssue> = items
        .iter()
        .take(per_page as usize)
        .filter_map(|item| {
            let number = item.get("number")?.as_u64()?;
            let title = item.get("title")?.as_str()?.to_string();
            let updated_at = item.get("updated_at")?.as_str()?.to_string();
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
            let body = if include_body {
                item.get("body").and_then(|v| v.as_str()).map(String::from)
            } else {
                None
            };
            let state = item
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("open")
                .to_string();
            let html_url = item
                .get("html_url")
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
                                .get("avatar_url")
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

    Ok(FetchIssuesResult {
        issues,
        has_next_page,
    })
}

fn parse_issue_category(category: &str) -> IssueCategory {
    match category {
        "issues" => IssueCategory::Issues,
        "specs" => IssueCategory::Specs,
        _ => IssueCategory::All,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn issue_list_args(repo_slug: Option<&str>, page: u32, per_page: u32, state: &str) -> Vec<String> {
    issue_list_args_with_options(repo_slug, page, per_page, state, true, IssueCategory::All)
}

fn issue_list_args_with_options(
    repo_slug: Option<&str>,
    page: u32,
    per_page: u32,
    state: &str,
    include_body: bool,
    category: IssueCategory,
) -> Vec<String> {
    // Request enough items to cover the current page plus one extra to detect next page
    let limit = u64::from(per_page) * u64::from(page) + 1;

    let limit_str = limit.to_string();
    let state_value = if state == "closed" { "closed" } else { "open" };
    let json_fields = if include_body {
        "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone"
    } else {
        "number,title,updatedAt,labels,state,url,assignees,comments,milestone"
    };
    let mut args = vec![
        "issue",
        "list",
        "--state",
        state_value,
        "--json",
        json_fields,
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

    match category {
        IssueCategory::Specs => {
            args.push("--label".to_string());
            args.push(SPEC_LABEL.to_string());
        }
        IssueCategory::Issues => {
            // Exclude spec-management issues from regular issue lists.
            args.push("--search".to_string());
            args.push(format!("-label:{SPEC_LABEL}"));
        }
        IssueCategory::All => {}
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
    let output =
        run_gh_output_with_repair(repo_path, ["api", endpoint.as_str(), "--jq", ".comments"])
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

    let output = run_gh_output_with_repair(repo_path, &args)
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
/// Searches local branches first, then remote tracking branches.
pub fn find_branch_for_issue(
    repo_path: &Path,
    issue_number: u64,
) -> Result<Option<String>, String> {
    let pattern = format!("issue-{}", issue_number);

    // Search local branches first
    let output = crate::process::command("git")
        .args(["branch", "--list", &format!("*{}*", pattern)])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let local: Vec<&str> = stdout
            .lines()
            .map(|line| line.trim().trim_start_matches("* "))
            .filter(|branch| branch.contains(&pattern))
            .collect();

        if let Some(branch) = local.first() {
            return Ok(Some(branch.to_string()));
        }
    }

    // Fallback: search remote tracking branches
    let remote_output = crate::process::command("git")
        .args(["branch", "-r", "--list", &format!("*{}*", pattern)])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch -r: {}", e))?;

    if !remote_output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&remote_output.stdout);
    let mut checked = HashSet::new();

    for remote_branch in stdout
        .lines()
        .map(|line| line.trim())
        .filter(|branch| branch.contains(&pattern))
    {
        let Some((remote_name, branch_name)) = split_remote_tracking_branch(remote_branch) else {
            continue;
        };

        // Avoid duplicate lookups when remote output contains repeated refs.
        let key = format!("{remote_name}/{branch_name}");
        if !checked.insert(key) {
            continue;
        }

        if remote_branch_exists_on_remote(repo_path, remote_name, branch_name)? {
            return Ok(Some(strip_remote_prefix(remote_branch).to_string()));
        }
    }

    Ok(None)
}

fn extract_issue_number_from_branch_name(branch: &str) -> Option<u64> {
    for segment in branch.trim().split('/') {
        let lower = segment.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("issue-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(number) = digits.parse::<u64>() {
            return Some(number);
        }
    }
    None
}

/// Bulk lookup for issue-linked branches.
///
/// This is optimized for list UIs while preserving remote existence checks
/// so stale remote-tracking refs are not treated as real issue branches.
pub fn find_branches_for_issues(
    repo_path: &Path,
    issue_numbers: &[u64],
) -> Result<HashMap<u64, String>, String> {
    let mut found = HashMap::new();
    let targets: HashSet<u64> = issue_numbers.iter().copied().collect();
    if targets.is_empty() {
        return Ok(found);
    }

    let local_output = crate::process::command("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch: {}", e))?;

    if local_output.status.success() {
        let stdout = String::from_utf8_lossy(&local_output.stdout);
        for branch in stdout
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
        {
            let Some(number) = extract_issue_number_from_branch_name(branch) else {
                continue;
            };
            if !targets.contains(&number) || found.contains_key(&number) {
                continue;
            }
            found.insert(number, branch.to_string());
            if found.len() == targets.len() {
                return Ok(found);
            }
        }
    }

    let remote_output = crate::process::command("git")
        .args(["branch", "-r", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git branch -r: {}", e))?;

    if !remote_output.status.success() {
        return Ok(found);
    }

    let stdout = String::from_utf8_lossy(&remote_output.stdout);
    let mut checked = HashSet::new();
    for remote_branch in stdout
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
    {
        let Some((remote_name, branch_name)) = split_remote_tracking_branch(remote_branch) else {
            continue;
        };
        let Some(number) = extract_issue_number_from_branch_name(branch_name) else {
            continue;
        };
        if !targets.contains(&number) || found.contains_key(&number) {
            continue;
        }

        let key = format!("{remote_name}/{branch_name}");
        if !checked.insert(key) {
            continue;
        }

        if !remote_branch_exists_on_remote(repo_path, remote_name, branch_name)? {
            continue;
        }

        found.insert(number, strip_remote_prefix(remote_branch).to_string());
        if found.len() == targets.len() {
            break;
        }
    }

    Ok(found)
}

/// Strip remote prefix from a remote branch name.
/// e.g., "origin/feature/issue-42" -> "feature/issue-42"
fn strip_remote_prefix(remote_branch: &str) -> &str {
    remote_branch
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(remote_branch)
}

fn split_remote_tracking_branch(remote_branch: &str) -> Option<(&str, &str)> {
    let (remote_name, branch_name) = remote_branch.split_once('/')?;
    if remote_name.is_empty() || branch_name.is_empty() || branch_name.contains(" -> ") {
        return None;
    }
    Some((remote_name, branch_name))
}

fn remote_branch_exists_on_remote(
    repo_path: &Path,
    remote_name: &str,
    branch_name: &str,
) -> Result<bool, String> {
    let output = run_git_with_timeout(
        repo_path,
        &["ls-remote", "--heads", remote_name, branch_name],
        LS_REMOTE_TIMEOUT,
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git ls-remote failed for remote '{}' and branch '{}': {}",
            remote_name,
            branch_name,
            stderr.trim()
        ));
    }

    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

fn run_git_with_timeout(
    repo_path: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = crate::process::command("git")
        .args(args)
        .current_dir(repo_path)
        // Avoid hanging on interactive auth prompts.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn git command: {}", e))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_handle = thread::spawn(move || {
        if let Some(mut stdout) = stdout {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        } else {
            Vec::new()
        }
    });
    let stderr_handle = thread::spawn(move || {
        if let Some(mut stderr) = stderr {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            buf
        } else {
            Vec::new()
        }
    });

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle.join().unwrap_or_else(|_| Vec::new());
                let stderr = stderr_handle.join().unwrap_or_else(|_| Vec::new());
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(format!(
                        "git {} timed out after {}s",
                        args.join(" "),
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(format!("Failed while waiting for git command: {}", e));
            }
        }
    }
}

/// Generate full branch name from type and issue
/// Format: "{type_prefix}issue-{number}" (e.g., "feature/issue-42")
pub fn generate_branch_name(type_prefix: &str, issue_number: u64) -> String {
    format!("{}issue-{}", type_prefix, issue_number)
}

fn issue_develop_args(issue_number: u64, branch_name: &str, base: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "issue".to_string(),
        "develop".to_string(),
        issue_number.to_string(),
        "--name".to_string(),
        branch_name.to_string(),
        "--checkout=false".to_string(),
    ];

    if let Some(base_branch) = base.filter(|b| !b.trim().is_empty()) {
        args.push("--base".to_string());
        args.push(base_branch.trim().to_string());
    }

    args
}

fn issue_develop_list_args(issue_number: u64, repo_slug: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "issue".to_string(),
        "develop".to_string(),
        "--list".to_string(),
        issue_number.to_string(),
    ];

    if let Some(slug) = repo_slug {
        args.push("--repo".to_string());
        args.push(slug.to_string());
    }

    args
}

fn contains_already_exists_message(text: &str) -> bool {
    text.to_ascii_lowercase().contains("already exists")
}

fn token_matches_branch(token: &str, branch_name: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | ';'
                    | '.'
                    | ':'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '"'
                    | '\''
                    | '`'
                    | '*'
                    | '-'
                    | '|'
            )
    });

    if trimmed.is_empty() {
        return false;
    }

    trimmed == branch_name
        || trimmed.ends_with(&format!(":{branch_name}"))
        || trimmed.ends_with(&format!("/{branch_name}"))
        || trimmed.contains(&format!("refs/heads/{branch_name}"))
}

fn issue_develop_list_mentions_branch(output: &str, branch_name: &str) -> bool {
    output.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }

        token_matches_branch(trimmed, branch_name)
            || trimmed
                .split_whitespace()
                .any(|token| token_matches_branch(token, branch_name))
    })
}

fn verify_issue_branch_linked(
    repo_path: &Path,
    issue_number: u64,
    branch_name: &str,
) -> Result<bool, String> {
    let repo_slug = resolve_repo_slug(repo_path);
    let output = run_gh_output_with_repair(
        repo_path,
        issue_develop_list_args(issue_number, repo_slug.as_deref()),
    )
    .map_err(|e| format!("Failed to execute gh issue develop --list: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue develop --list failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(issue_develop_list_mentions_branch(&stdout, branch_name))
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
pub fn create_or_verify_linked_branch(
    repo_path: &Path,
    issue_number: u64,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<IssueLinkedBranchStatus, String> {
    let normalized_branch_name = branch_name.trim();
    if normalized_branch_name.is_empty() {
        return Err("[E1012] Branch name is required".to_string());
    }

    let output = run_gh_output_with_repair(
        repo_path,
        issue_develop_args(issue_number, normalized_branch_name, base_branch),
    )
    .map_err(|e| format!("Failed to execute gh issue develop: {}", e))?;

    if output.status.success() {
        return Ok(IssueLinkedBranchStatus::Created);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}\n{}", stderr, stdout);
    if contains_already_exists_message(&combined) {
        if verify_issue_branch_linked(repo_path, issue_number, normalized_branch_name)? {
            return Ok(IssueLinkedBranchStatus::AlreadyLinked);
        }
        return Err(format!(
            "[E1012] Issue branch exists but is not linked: {}",
            normalized_branch_name
        ));
    }

    Err(format!("gh issue develop failed: {}", stderr.trim()))
}

pub fn create_linked_branch(
    repo_path: &Path,
    issue_number: u64,
    branch_name: &str,
) -> Result<(), String> {
    create_or_verify_linked_branch(repo_path, issue_number, branch_name, None).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(repo_path: &Path, args: &[&str]) -> String {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        std::fs::write(temp.path().join("README.md"), "initial").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        temp
    }

    fn current_branch_name(repo_path: &Path) -> String {
        git_stdout(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])
    }

    fn create_repo_with_origin() -> (TempDir, TempDir) {
        let repo = create_test_repo();
        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare"]);
        run_git(
            repo.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        let base = current_branch_name(repo.path());
        run_git(repo.path(), &["push", "-u", "origin", &base]);
        (repo, origin)
    }

    fn create_remote_issue_branch_without_local_copy(repo_path: &Path, branch_name: &str) {
        let base = current_branch_name(repo_path);
        run_git(repo_path, &["checkout", "-b", branch_name, &base]);
        std::fs::write(repo_path.join("issue.txt"), branch_name).unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(repo_path, &["commit", "-m", "issue branch"]);
        run_git(repo_path, &["push", "-u", "origin", branch_name]);
        run_git(repo_path, &["checkout", &base]);
        run_git(repo_path, &["branch", "-D", branch_name]);
    }

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
        let args = issue_develop_args(42, "feature/issue-42", None);
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

    #[test]
    fn test_issue_develop_args_includes_base_when_provided() {
        let args = issue_develop_args(42, "feature/issue-42", Some("develop"));
        assert_eq!(
            args,
            vec![
                "issue",
                "develop",
                "42",
                "--name",
                "feature/issue-42",
                "--checkout=false",
                "--base",
                "develop",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_issue_develop_args_omits_base_when_blank() {
        let args = issue_develop_args(42, "feature/issue-42", Some("  "));
        assert!(!args.iter().any(|arg| arg == "--base"));
    }

    #[test]
    fn test_create_or_verify_linked_branch_rejects_blank_branch_name() {
        let repo = create_test_repo();
        let err = create_or_verify_linked_branch(repo.path(), 42, "   ", None).unwrap_err();
        assert!(err.contains("[E1012]"));
        assert!(err.contains("Branch name is required"));
    }

    #[test]
    fn test_issue_develop_list_mentions_branch_owner_repo_format() {
        let output = "akiojin/gwt:feature/issue-42";
        assert!(issue_develop_list_mentions_branch(
            output,
            "feature/issue-42"
        ));
    }

    #[test]
    fn test_issue_develop_list_mentions_branch_refs_heads_format() {
        let output = "refs/heads/feature/issue-42";
        assert!(issue_develop_list_mentions_branch(
            output,
            "feature/issue-42"
        ));
    }

    #[test]
    fn test_issue_develop_list_mentions_branch_exact_token() {
        let output = "- feature/issue-42";
        assert!(issue_develop_list_mentions_branch(
            output,
            "feature/issue-42"
        ));
    }

    #[test]
    fn test_issue_develop_list_does_not_match_adjacent_name() {
        let output = "akiojin/gwt:feature/issue-420";
        assert!(!issue_develop_list_mentions_branch(
            output,
            "feature/issue-42"
        ));
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
    fn test_issue_list_args_with_options_excludes_body_for_lightweight_list() {
        let args = issue_list_args_with_options(None, 1, 10, "open", false, IssueCategory::All);
        let json_index = args.iter().position(|v| v == "--json").unwrap();
        assert_eq!(
            args.get(json_index + 1).map(String::as_str),
            Some("number,title,updatedAt,labels,state,url,assignees,comments,milestone")
        );
    }

    #[test]
    fn test_issue_list_args_with_options_specs_category_uses_label_filter() {
        let args = issue_list_args_with_options(
            Some("owner/repo"),
            1,
            10,
            "open",
            false,
            IssueCategory::Specs,
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--label" && w[1] == SPEC_LABEL));
    }

    #[test]
    fn test_issue_list_args_with_options_issues_category_uses_negative_label_search() {
        let args = issue_list_args_with_options(
            Some("owner/repo"),
            1,
            10,
            "open",
            false,
            IssueCategory::Issues,
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--search" && w[1] == format!("-label:{SPEC_LABEL}")));
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

    // ==========================================================
    // SPEC-rb01a2f3: Remote branch prefix stripping tests
    // ==========================================================

    #[test]
    fn test_strip_remote_prefix_origin() {
        assert_eq!(
            strip_remote_prefix("origin/feature/issue-42"),
            "feature/issue-42"
        );
    }

    #[test]
    fn test_strip_remote_prefix_upstream() {
        assert_eq!(
            strip_remote_prefix("upstream/bugfix/issue-10"),
            "bugfix/issue-10"
        );
    }

    #[test]
    fn test_strip_remote_prefix_simple_branch() {
        assert_eq!(strip_remote_prefix("origin/issue-42"), "issue-42");
    }

    #[test]
    fn test_strip_remote_prefix_no_prefix() {
        // Edge case: no slash means no remote prefix, return as-is
        assert_eq!(strip_remote_prefix("issue-42"), "issue-42");
    }

    #[test]
    fn test_split_remote_tracking_branch_parses_valid_ref() {
        let parsed = split_remote_tracking_branch("origin/feature/issue-42");
        assert_eq!(parsed, Some(("origin", "feature/issue-42")));
    }

    #[test]
    fn test_split_remote_tracking_branch_rejects_symbolic_ref() {
        let parsed = split_remote_tracking_branch("origin/HEAD -> origin/main");
        assert!(parsed.is_none());
    }

    // ==========================================================
    // SPEC-rb01a2f3: find_branch_for_issue remote search
    // ==========================================================

    #[test]
    fn test_find_branch_for_issue_finds_local_branch() {
        let repo = create_test_repo();
        let base = current_branch_name(repo.path());
        run_git(
            repo.path(),
            &["checkout", "-b", "feature/issue-1029", &base],
        );

        let found = find_branch_for_issue(repo.path(), 1029).unwrap();
        assert_eq!(found.as_deref(), Some("feature/issue-1029"));
    }

    #[test]
    fn test_find_branch_for_issue_confirms_remote_branch_with_ls_remote() {
        let (repo, _origin) = create_repo_with_origin();
        create_remote_issue_branch_without_local_copy(repo.path(), "bugfix/issue-1029");
        run_git(repo.path(), &["fetch", "origin"]);

        let remote_tracking = git_stdout(repo.path(), &["branch", "-r", "--list", "*issue-1029*"]);
        assert!(
            remote_tracking.contains("origin/bugfix/issue-1029"),
            "expected remote-tracking ref to exist in test setup"
        );

        let found = find_branch_for_issue(repo.path(), 1029).unwrap();
        assert_eq!(found.as_deref(), Some("bugfix/issue-1029"));
    }

    #[test]
    fn test_find_branch_for_issue_ignores_stale_remote_tracking_ref() {
        let (repo, origin) = create_repo_with_origin();
        create_remote_issue_branch_without_local_copy(repo.path(), "bugfix/issue-1029");
        run_git(repo.path(), &["fetch", "origin"]);

        let remote_tracking_before =
            git_stdout(repo.path(), &["branch", "-r", "--list", "*issue-1029*"]);
        assert!(
            remote_tracking_before.contains("origin/bugfix/issue-1029"),
            "expected remote-tracking ref to exist before remote deletion"
        );

        // Delete directly on remote to leave stale remote-tracking ref in the local clone.
        run_git(origin.path(), &["branch", "-D", "bugfix/issue-1029"]);

        let found = find_branch_for_issue(repo.path(), 1029).unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn test_find_branches_for_issues_finds_local_and_remote_tracking_entries() {
        let (repo, _origin) = create_repo_with_origin();
        let base = current_branch_name(repo.path());
        run_git(
            repo.path(),
            &["checkout", "-b", "feature/issue-1001", &base],
        );
        run_git(repo.path(), &["checkout", &base]);
        create_remote_issue_branch_without_local_copy(repo.path(), "bugfix/issue-1002");
        run_git(repo.path(), &["fetch", "origin"]);

        let found = find_branches_for_issues(repo.path(), &[1001, 1002, 9999]).unwrap();
        assert_eq!(
            found.get(&1001).map(String::as_str),
            Some("feature/issue-1001")
        );
        assert_eq!(
            found.get(&1002).map(String::as_str),
            Some("bugfix/issue-1002")
        );
        assert!(!found.contains_key(&9999));
    }

    #[test]
    fn test_find_branches_for_issues_ignores_stale_remote_tracking_ref() {
        let (repo, origin) = create_repo_with_origin();
        create_remote_issue_branch_without_local_copy(repo.path(), "bugfix/issue-1029");
        run_git(repo.path(), &["fetch", "origin"]);

        let remote_tracking_before =
            git_stdout(repo.path(), &["branch", "-r", "--list", "*issue-1029*"]);
        assert!(
            remote_tracking_before.contains("origin/bugfix/issue-1029"),
            "expected remote-tracking ref to exist before remote deletion"
        );

        // Delete directly on remote to leave stale remote-tracking ref in the local clone.
        run_git(origin.path(), &["branch", "-D", "bugfix/issue-1029"]);

        let found = find_branches_for_issues(repo.path(), &[1029]).unwrap();
        assert!(!found.contains_key(&1029));
    }

    #[test]
    fn test_extract_issue_number_from_branch_name() {
        assert_eq!(
            extract_issue_number_from_branch_name("feature/issue-1200"),
            Some(1200)
        );
        assert_eq!(
            extract_issue_number_from_branch_name("origin/bugfix/issue-77-extra"),
            Some(77)
        );
        assert_eq!(
            extract_issue_number_from_branch_name("feature/not-an-issue"),
            None
        );
    }

    // ==========================================================
    // Search API endpoint construction tests
    // ==========================================================

    #[test]
    fn test_issue_search_api_endpoint_all_category() {
        let endpoint = issue_search_api_endpoint("owner/repo", 1, 30, "open", IssueCategory::All);
        assert_eq!(
            endpoint,
            "search/issues?q=repo:owner/repo+is:issue+state:open+sort:updated-desc&per_page=30&page=1"
        );
    }

    #[test]
    fn test_issue_search_api_endpoint_issues_category() {
        let endpoint = issue_search_api_endpoint("owner/repo", 2, 10, "open", IssueCategory::Issues);
        assert_eq!(
            endpoint,
            "search/issues?q=repo:owner/repo+is:issue+state:open+sort:updated-desc+-label:gwt-spec&per_page=10&page=2"
        );
    }

    #[test]
    fn test_issue_search_api_endpoint_specs_category() {
        let endpoint = issue_search_api_endpoint("owner/repo", 1, 20, "closed", IssueCategory::Specs);
        assert_eq!(
            endpoint,
            "search/issues?q=repo:owner/repo+is:issue+state:closed+sort:updated-desc+label:gwt-spec&per_page=20&page=1"
        );
    }

    #[test]
    fn test_issue_search_api_endpoint_defaults_to_open_for_unknown_state() {
        let endpoint = issue_search_api_endpoint("owner/repo", 1, 10, "unknown", IssueCategory::All);
        assert!(endpoint.contains("state:open"));
    }

    // ==========================================================
    // Search API JSON parsing tests
    // ==========================================================

    #[test]
    fn test_parse_search_issues_json_basic() {
        let json = r#"{
            "total_count": 2,
            "items": [
                {
                    "number": 42,
                    "title": "Fix login bug",
                    "updated_at": "2025-01-25T10:00:00Z",
                    "state": "open",
                    "html_url": "https://github.com/user/repo/issues/42",
                    "labels": [{"name": "bug", "color": "d73a4a"}],
                    "body": "Issue body",
                    "assignees": [{"login": "octocat", "avatar_url": "https://avatars.example.com/1"}],
                    "comments": 5,
                    "milestone": {"title": "v1.0", "number": 1}
                },
                {
                    "number": 10,
                    "title": "Update docs",
                    "updated_at": "2025-01-24T08:00:00Z",
                    "state": "open",
                    "html_url": "https://github.com/user/repo/issues/10",
                    "labels": [],
                    "assignees": [],
                    "comments": 0,
                    "milestone": null
                }
            ]
        }"#;

        let result = parse_search_issues_json(json, 1, 30, true).unwrap();
        assert_eq!(result.issues.len(), 2);
        assert!(!result.has_next_page);

        let issue = &result.issues[0];
        assert_eq!(issue.number, 42);
        assert_eq!(issue.title, "Fix login bug");
        assert_eq!(issue.updated_at, "2025-01-25T10:00:00Z");
        assert_eq!(issue.state, "open");
        assert_eq!(issue.html_url, "https://github.com/user/repo/issues/42");
        assert_eq!(issue.labels.len(), 1);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.labels[0].color, "d73a4a");
        assert_eq!(issue.body.as_deref(), Some("Issue body"));
        assert_eq!(issue.assignees.len(), 1);
        assert_eq!(issue.assignees[0].login, "octocat");
        assert_eq!(
            issue.assignees[0].avatar_url,
            "https://avatars.example.com/1"
        );
        assert_eq!(issue.comments_count, 5);
        assert!(issue.milestone.is_some());
        let ms = issue.milestone.as_ref().unwrap();
        assert_eq!(ms.title, "v1.0");
        assert_eq!(ms.number, 1);

        let issue2 = &result.issues[1];
        assert_eq!(issue2.number, 10);
        assert!(issue2.milestone.is_none());
    }

    #[test]
    fn test_parse_search_issues_json_has_next_page_detection() {
        // per_page=2, but 3 items returned → has_next_page = true, only 2 issues kept
        let json = r#"{
            "total_count": 10,
            "items": [
                {"number": 1, "title": "A", "updated_at": "2025-01-03T00:00:00Z", "state": "open", "html_url": "", "labels": [], "assignees": [], "comments": 0, "milestone": null},
                {"number": 2, "title": "B", "updated_at": "2025-01-02T00:00:00Z", "state": "open", "html_url": "", "labels": [], "assignees": [], "comments": 0, "milestone": null},
                {"number": 3, "title": "C", "updated_at": "2025-01-01T00:00:00Z", "state": "open", "html_url": "", "labels": [], "assignees": [], "comments": 0, "milestone": null}
            ]
        }"#;

        let result = parse_search_issues_json(json, 1, 2, true).unwrap();
        assert!(result.has_next_page);
        assert_eq!(result.issues.len(), 2);
        assert_eq!(result.issues[0].number, 1);
        assert_eq!(result.issues[1].number, 2);
    }

    #[test]
    fn test_parse_search_issues_json_no_next_page() {
        // per_page=5, only 2 items returned → has_next_page = false
        let json = r#"{
            "total_count": 2,
            "items": [
                {"number": 1, "title": "A", "updated_at": "2025-01-02T00:00:00Z", "state": "open", "html_url": "", "labels": [], "assignees": [], "comments": 0, "milestone": null},
                {"number": 2, "title": "B", "updated_at": "2025-01-01T00:00:00Z", "state": "open", "html_url": "", "labels": [], "assignees": [], "comments": 0, "milestone": null}
            ]
        }"#;

        let result = parse_search_issues_json(json, 1, 5, true).unwrap();
        assert!(!result.has_next_page);
        assert_eq!(result.issues.len(), 2);
    }

    #[test]
    fn test_parse_search_issues_json_empty_items() {
        let json = r#"{"total_count": 0, "items": []}"#;
        let result = parse_search_issues_json(json, 1, 30, true).unwrap();
        assert!(result.issues.is_empty());
        assert!(!result.has_next_page);
    }

    #[test]
    fn test_parse_search_issues_json_invalid_json() {
        let result = parse_search_issues_json("not json", 1, 30, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_search_issues_json_missing_items_field() {
        let json = r#"{"total_count": 0}"#;
        let result = parse_search_issues_json(json, 1, 30, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("items"));
    }

    #[test]
    fn test_parse_search_issues_json_closed_state() {
        let json = r#"{
            "total_count": 1,
            "items": [
                {
                    "number": 99,
                    "title": "Closed issue",
                    "updated_at": "2025-01-01T00:00:00Z",
                    "state": "closed",
                    "html_url": "https://github.com/user/repo/issues/99",
                    "labels": [],
                    "assignees": [],
                    "comments": 0,
                    "milestone": null
                }
            ]
        }"#;

        let result = parse_search_issues_json(json, 1, 30, true).unwrap();
        assert_eq!(result.issues[0].state, "closed");
    }

    #[test]
    fn test_parse_search_issues_json_excludes_body_when_requested() {
        let json = r#"{
            "total_count": 1,
            "items": [
                {
                    "number": 1,
                    "title": "Issue",
                    "updated_at": "2025-01-01T00:00:00Z",
                    "state": "open",
                    "html_url": "https://github.com/user/repo/issues/1",
                    "labels": [],
                    "body": "Body content",
                    "assignees": [],
                    "comments": 0,
                    "milestone": null
                }
            ]
        }"#;

        let result = parse_search_issues_json(json, 1, 30, false).unwrap();
        assert_eq!(result.issues.len(), 1);
        assert!(result.issues[0].body.is_none());
    }

    #[test]
    fn test_parse_search_issues_json_respects_search_api_1000_limit() {
        let json = r#"{
            "total_count": 5000,
            "items": [
                {
                    "number": 999,
                    "title": "Issue",
                    "updated_at": "2025-01-01T00:00:00Z",
                    "state": "open",
                    "html_url": "https://github.com/user/repo/issues/999",
                    "labels": [],
                    "assignees": [],
                    "comments": 0,
                    "milestone": null
                }
            ]
        }"#;

        let page_33 = parse_search_issues_json(json, 33, 30, false).unwrap();
        assert!(page_33.has_next_page);

        let page_34 = parse_search_issues_json(json, 34, 30, false).unwrap();
        assert!(!page_34.has_next_page);
    }
}
