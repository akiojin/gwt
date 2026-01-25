//! GitHub Issue operations (SPEC-e4798383)
//!
//! Provides Issue information using GitHub CLI (gh) for branch creation from issues.

use std::path::Path;
use std::process::Command;

/// GitHub Issue information
#[derive(Debug, Clone)]
pub struct GitHubIssue {
    /// Issue number
    pub number: u64,
    /// Issue title
    pub title: String,
    /// Issue updatedAt timestamp (ISO-8601)
    pub updated_at: String,
}

impl GitHubIssue {
    /// Create a new GitHubIssue
    pub fn new(number: u64, title: String, updated_at: String) -> Self {
        Self {
            number,
            title,
            updated_at,
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
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Fetch open issues from GitHub using gh CLI
/// Returns issues sorted by updated_at descending (most recently updated first)
/// Limited to 50 issues per FR-005a
pub fn fetch_open_issues(repo_path: &Path) -> Result<Vec<GitHubIssue>, String> {
    let output = Command::new("gh")
        .args([
            "issue",
            "list",
            "--state",
            "open",
            "--json",
            "number,title,updatedAt",
            "--limit",
            "50",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh CLI: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_gh_issues_json(&stdout)
}

/// Parse gh issue list JSON output
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
            Some(GitHubIssue::new(number, title, updated_at))
        })
        .collect();

    // Sort by updated_at descending (FR-006)
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(result)
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

    let output = Command::new("git")
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
    // FR-016b: Omit --checkout flag (default: no checkout) since worktree will handle checkout
    let output = Command::new("gh")
        .args([
            "issue",
            "develop",
            &issue_number.to_string(),
            "--name",
            branch_name,
        ])
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
}
