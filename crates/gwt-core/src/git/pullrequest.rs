//! Pull Request operations (FR-016)
//!
//! Provides PR information using GitHub CLI (gh) for branch-to-PR title mapping.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

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
}

/// Check if GitHub CLI (gh) is available
fn is_gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
    let output = Command::new("gh")
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

fn is_newer_pr(candidate: &PullRequest, current: &PullRequest) -> bool {
    match (&candidate.updated_at, &current.updated_at) {
        (Some(candidate_ts), Some(current_ts)) => candidate_ts > current_ts,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => candidate.state == "OPEN" && current.state != "OPEN",
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
}
