//! GitHub Issue tracking with file-based cache

use std::{
    fs,
    path::{Path, PathBuf},
};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Rows one live Issue list may return (`REST_MAX_PAGES_PER_READ` pages of
/// `REST_PAGE_SIZE`). A list this long may be incomplete.
pub const GITHUB_ISSUE_LIST_LIMIT: &str = "1000";

/// A GitHub Issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    /// "OPEN" | "CLOSED"
    pub state: String,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub body: Option<String>,
    pub url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Issue rows together with the completeness of the underlying REST read.
#[derive(Debug, Clone)]
pub struct IssueListing {
    pub issues: Vec<Issue>,
    /// The REST page budget was exhausted, even if filtering PRs reduced the row count.
    pub capped: bool,
}

/// File-based cache for GitHub Issues.
///
/// Stores fetched issues under `~/.gwt/cache/issues/<owner>-<repo>.json`.
pub struct IssueCache {
    cache_dir: PathBuf,
}

impl IssueCache {
    /// Create a cache instance using the default directory (`~/.gwt/cache/issues/`).
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| GwtError::Other("Cannot determine home directory".into()))?;
        let cache_dir = home.join(".gwt").join("cache").join("issues");
        fs::create_dir_all(&cache_dir).map_err(|e| GwtError::Other(e.to_string()))?;
        Ok(Self { cache_dir })
    }

    /// Create a cache instance at a custom directory (useful for testing).
    pub fn with_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let cache_dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&cache_dir).map_err(|e| GwtError::Other(e.to_string()))?;
        Ok(Self { cache_dir })
    }

    /// Read cached issues for a repository.
    pub fn read(&self, owner: &str, repo: &str) -> Result<Option<Vec<Issue>>> {
        let path = self.cache_path(owner, repo);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path).map_err(|e| GwtError::Other(e.to_string()))?;
        let issues: Vec<Issue> =
            serde_json::from_str(&data).map_err(|e| GwtError::Other(e.to_string()))?;
        Ok(Some(issues))
    }

    /// Write issues to cache.
    pub fn write(&self, owner: &str, repo: &str, issues: &[Issue]) -> Result<()> {
        let path = self.cache_path(owner, repo);
        let data =
            serde_json::to_string_pretty(issues).map_err(|e| GwtError::Other(e.to_string()))?;
        fs::write(&path, data).map_err(|e| GwtError::Other(e.to_string()))?;
        Ok(())
    }

    fn cache_path(&self, owner: &str, repo: &str) -> PathBuf {
        self.cache_dir.join(cache_filename(owner, repo))
    }
}

/// Sanitize a string for safe use in filenames by replacing unsafe characters.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Build a safe cache filename from owner and repo.
fn cache_filename(owner: &str, repo: &str) -> String {
    format!(
        "{}_{}_issues.json",
        sanitize_filename(owner),
        sanitize_filename(repo)
    )
}

/// Fetch open issues from GitHub through the paged REST list
/// (`GET /repos/{owner}/{repo}/issues?state=open`, SPEC #4093 FR-002). Costs at
/// most [`crate::gh_rest::REST_MAX_PAGES_PER_READ`] REST requests and no
/// GraphQL. Use [`fetch_issue_listing`] when completeness matters: filtering
/// pull requests can leave a capped read below [`GITHUB_ISSUE_LIST_LIMIT`] rows.
pub fn fetch_issues(owner: &str, repo: &str) -> Result<Vec<Issue>> {
    fetch_issue_listing(owner, repo).map(|listing| listing.issues)
}

/// Fetch open Issues while retaining the REST page-cap signal.
pub fn fetch_issue_listing(owner: &str, repo: &str) -> Result<IssueListing> {
    fetch_issue_listing_with(owner, repo, |path| {
        let hub = gwt_core::process_console::global();
        let output = gwt_core::process_console::spawn_logged_blocking(
            &hub,
            gwt_core::process_console::ProcessKind::Gh,
            "gh",
            &["api", path],
            gwt_core::process_console::SpawnOptions::new("gh api issues"),
        )
        .map_err(|e| e.to_string())?;
        if output.success() {
            Ok(output.stdout)
        } else {
            Err(output.stderr.trim().to_string())
        }
    })
}

/// Injectable core of [`fetch_issues`]: `fetch` runs one `gh api <path>`.
pub fn fetch_issues_with<F>(owner: &str, repo: &str, fetch: F) -> Result<Vec<Issue>>
where
    F: FnMut(&str) -> std::result::Result<String, String>,
{
    fetch_issue_listing_with(owner, repo, fetch).map(|listing| listing.issues)
}

/// Injectable core of [`fetch_issue_listing`]: `fetch` runs one `gh api <path>`.
pub fn fetch_issue_listing_with<F>(owner: &str, repo: &str, fetch: F) -> Result<IssueListing>
where
    F: FnMut(&str) -> std::result::Result<String, String>,
{
    let endpoint = format!("repos/{owner}/{repo}/issues?state=open&sort=updated&direction=desc");
    let pages = crate::gh_rest::read_pages_with(&endpoint, fetch)
        .map_err(|e| GwtError::Git(format!("gh api issues: {e}")))?;
    Ok(IssueListing {
        issues: crate::gh_rest::parse_issue_rows(&pages.rows)
            .into_iter()
            .map(Issue::from)
            .collect(),
        capped: pages.capped,
    })
}

impl From<crate::gh_rest::RestIssueRow> for Issue {
    fn from(row: crate::gh_rest::RestIssueRow) -> Self {
        Self {
            number: row.number,
            title: row.title,
            state: row.state,
            labels: row.labels,
            assignee: row.assignee,
            body: row.body,
            url: row.url,
            updated_at: row.updated_at,
        }
    }
}

/// Fetch the comment bodies of one Issue via `gh issue view --json comments`
/// (Issue #3917 AC-2: delegation records may live in Issue comments).
pub fn fetch_issue_comment_bodies(owner: &str, repo: &str, number: u64) -> Result<Vec<String>> {
    let repo_slug = format!("{owner}/{repo}");
    let number = number.to_string();
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "issue",
            "view",
            number.as_str(),
            "--repo",
            repo_slug.as_str(),
            "--json",
            "comments",
        ],
        gwt_core::process_console::SpawnOptions::new("gh issue view comments"),
    )
    .map_err(|e| GwtError::Git(format!("gh issue view comments: {e}")))?;
    if !output.success() {
        return Err(GwtError::Git(format!(
            "gh issue view comments: {}",
            output.stderr
        )));
    }
    parse_gh_issue_comment_bodies(&output.stdout)
}

/// Repository roles whose comment may carry a delegation record (Issue #3917
/// AC-2). A settlement can close an Issue with unchecked criteria on the
/// strength of such a comment, so a drive-by commenter must not be able to
/// author one. Anything else — including a missing association — fails closed.
const TRUSTED_COMMENT_AUTHOR_ASSOCIATIONS: &[&str] = &["OWNER", "MEMBER", "COLLABORATOR"];

/// Whether `association` (GitHub's `authorAssociation`) is a repository role
/// gwt trusts to record decisions about the Issue.
fn comment_author_is_trusted(association: Option<&str>) -> bool {
    association.is_some_and(|association| {
        TRUSTED_COMMENT_AUTHOR_ASSOCIATIONS
            .iter()
            .any(|trusted| association.eq_ignore_ascii_case(trusted))
    })
}

/// Parse `gh issue view --json comments` into the comment bodies in order,
/// keeping only comments written by a trusted repository role.
pub fn parse_gh_issue_comment_bodies(json: &str) -> Result<Vec<String>> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(e.to_string()))?;
    Ok(value
        .get("comments")
        .and_then(serde_json::Value::as_array)
        .map(|comments| {
            comments
                .iter()
                .filter(|comment| {
                    comment_author_is_trusted(
                        comment
                            .get("authorAssociation")
                            .and_then(serde_json::Value::as_str),
                    )
                })
                .filter_map(|comment| comment.get("body").and_then(serde_json::Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default())
}

/// Parse the JSON output from `gh issue list --json`.
pub fn parse_gh_issues_json(json: &str) -> Result<Vec<Issue>> {
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(e.to_string()))?;

    let mut issues = Vec::new();
    for v in raw {
        let number = v["number"].as_u64().unwrap_or(0);
        let title = v["title"].as_str().unwrap_or("").to_string();
        let state = v["state"].as_str().unwrap_or("OPEN").to_string();
        let labels = v["labels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let assignee = v["assignees"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|a| a["login"].as_str())
            .map(String::from);
        let body = v["body"].as_str().map(String::from);
        let url = v["url"].as_str().unwrap_or("").to_string();
        let updated_at = v["updatedAt"].as_str().map(String::from);

        issues.push(Issue {
            number,
            title,
            state,
            labels,
            assignee,
            body,
            url,
            updated_at,
        });
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gh_issues_json_valid() {
        let json = r#"[
            {
                "number": 42,
                "title": "Fix bug",
                "state": "OPEN",
                "labels": [{"name": "bug"}],
                "assignees": [{"login": "alice"}],
                "body": "Description",
                "url": "https://github.com/owner/repo/issues/42",
                "updatedAt": "2026-08-05T10:00:00Z"
            },
            {
                "number": 43,
                "title": "Add feature",
                "state": "OPEN",
                "labels": [],
                "assignees": [],
                "body": null,
                "url": "https://github.com/owner/repo/issues/43",
                "updatedAt": "2026-08-05T10:01:00Z"
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 42);
        assert_eq!(issues[0].title, "Fix bug");
        assert_eq!(issues[0].labels, vec!["bug"]);
        assert_eq!(issues[0].assignee.as_deref(), Some("alice"));
        assert_eq!(
            issues[0].updated_at.as_deref(),
            Some("2026-08-05T10:00:00Z")
        );
        assert_eq!(issues[1].number, 43);
        assert!(issues[1].assignee.is_none());
        assert!(issues[1].body.is_none());
        assert_eq!(
            issues[1].updated_at.as_deref(),
            Some("2026-08-05T10:01:00Z")
        );
    }

    #[test]
    fn parse_gh_issue_comment_bodies_reads_comments_array() {
        // Issue #3917 AC-2: delegation records may live in Issue comments.
        let json = r#"{"comments":[{"authorAssociation":"OWNER","body":"first"},{"authorAssociation":"collaborator","body":"残 AC は別 Issue に委譲 (#77)"},{"authorAssociation":"MEMBER","author":{"login":"x"}}]}"#;
        let bodies = parse_gh_issue_comment_bodies(json).unwrap();
        assert_eq!(bodies, vec!["first", "残 AC は別 Issue に委譲 (#77)"]);
        assert!(parse_gh_issue_comment_bodies("{}").unwrap().is_empty());
        assert!(parse_gh_issue_comment_bodies("not json").is_err());
    }

    #[test]
    fn parse_gh_issue_comment_bodies_drops_untrusted_authors() {
        // A delegation record can close an Issue whose criteria are unchecked,
        // so only a repository role may author one. Everything else — an
        // outside contributor, a first-time commenter, a missing association —
        // fails closed.
        let json = r#"{"comments":[
            {"authorAssociation":"NONE","body":"残 AC は別 Issue に委譲 (#77)"},
            {"authorAssociation":"CONTRIBUTOR","body":"残 AC は別 Issue に委譲 (#78)"},
            {"authorAssociation":"FIRST_TIME_CONTRIBUTOR","body":"残 AC は別 Issue に委譲 (#79)"},
            {"body":"残 AC は別 Issue に委譲 (#80)"},
            {"authorAssociation":"OWNER","body":"owner note"}
        ]}"#;
        assert_eq!(
            parse_gh_issue_comment_bodies(json).unwrap(),
            vec!["owner note"]
        );
    }

    #[test]
    fn parse_gh_issues_json_empty() {
        let issues = parse_gh_issues_json("[]").unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn parse_gh_issues_json_accepts_missing_updated_at() {
        let issues =
            parse_gh_issues_json(r#"[{"number":42,"title":"Legacy payload","state":"OPEN"}]"#)
                .expect("parse legacy payload");

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 42);
        assert!(issues[0].updated_at.is_none());
    }

    #[test]
    fn parse_gh_issues_json_invalid() {
        let result = parse_gh_issues_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn issue_list_limit_is_high_enough_for_large_repositories() {
        assert_eq!(GITHUB_ISSUE_LIST_LIMIT, "1000");
        assert_eq!(
            GITHUB_ISSUE_LIST_LIMIT.parse::<usize>().unwrap(),
            crate::gh_rest::REST_MAX_PAGES_PER_READ * crate::gh_rest::REST_PAGE_SIZE,
            "the incomplete-list signal must match the REST page budget"
        );
    }

    /// SPEC #4093 AC-3: the Issue Monitor's candidate list is a REST read
    /// (`gh api repos/{owner}/{repo}/issues`), never `gh issue list` (GraphQL),
    /// and one read spends at most the page budget.
    #[test]
    fn fetch_issues_reads_the_rest_issue_list_page_by_page() {
        let mut calls = Vec::new();
        let issues = fetch_issues_with("acme", "widgets", |path| {
            calls.push(path.to_string());
            Ok(if calls.len() == 1 {
                r#"[{"number":42,"title":"Fix bug","state":"open","labels":[{"name":"bug"}],"assignees":[{"login":"alice"}],"body":"b","html_url":"https://github.com/acme/widgets/issues/42","updated_at":"2026-09-01T00:00:00Z"},{"number":43,"title":"PR row","state":"open","pull_request":{"url":"x"}}]"#.to_string()
            } else {
                "[]".to_string()
            })
        })
        .unwrap();
        assert_eq!(
            calls,
            ["repos/acme/widgets/issues?state=open&sort=updated&direction=desc&per_page=100&page=1"]
        );
        assert_eq!(issues.len(), 1, "pull requests are not issues");
        assert_eq!(issues[0].number, 42);
        assert_eq!(issues[0].state, "OPEN");
        assert_eq!(issues[0].assignee.as_deref(), Some("alice"));
        assert_eq!(issues[0].url, "https://github.com/acme/widgets/issues/42");
        assert_eq!(
            issues[0].updated_at.as_deref(),
            Some("2026-09-01T00:00:00Z")
        );

        let failure = fetch_issues_with("acme", "widgets", |_| Err("HTTP 502".to_string()))
            .unwrap_err()
            .to_string();
        assert!(failure.contains("gh api issues"), "{failure}");
        assert!(failure.contains("HTTP 502"), "{failure}");
    }

    #[test]
    fn fetch_issue_listing_preserves_cap_after_filtering_pull_requests() {
        let mut requests = 0;
        let listing = fetch_issue_listing_with("acme", "widgets", |_| {
            let offset = requests * crate::gh_rest::REST_PAGE_SIZE;
            requests += 1;
            let rows = (0..crate::gh_rest::REST_PAGE_SIZE)
                .map(|index| {
                    let mut row = serde_json::json!({"number": offset + index + 1});
                    if index == crate::gh_rest::REST_PAGE_SIZE - 1 {
                        row["pull_request"] = serde_json::json!({"url": "https://example.test/pr"});
                    }
                    row
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_string(&rows).unwrap())
        })
        .unwrap();

        assert_eq!(requests, crate::gh_rest::REST_MAX_PAGES_PER_READ);
        assert_eq!(listing.issues.len(), 990);
        assert!(
            listing.capped,
            "filtering PRs must not erase the REST page cap"
        );
    }

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = IssueCache::with_dir(tmp.path()).unwrap();

        // Initially empty
        assert!(cache.read("owner", "repo").unwrap().is_none());

        // Write and read back
        let issues = vec![Issue {
            number: 1,
            title: "Test".into(),
            state: "OPEN".into(),
            labels: vec!["bug".into()],
            assignee: Some("alice".into()),
            body: Some("body".into()),
            url: "https://example.com".into(),
            updated_at: None,
        }];
        cache.write("owner", "repo", &issues).unwrap();

        let loaded = cache.read("owner", "repo").unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].number, 1);
        assert_eq!(loaded[0].title, "Test");
    }

    #[test]
    fn sanitize_filename_replaces_unsafe_chars() {
        assert_eq!(sanitize_filename("normal-name_1"), "normal-name_1");
        // "../etc" -> '.', '.', '/' are all non-alphanumeric -> "___etc"
        assert_eq!(sanitize_filename("../etc"), "___etc");
        // "../../passwd" -> '.', '.', '/', '.', '.', '/' -> "______passwd"
        assert_eq!(sanitize_filename("../../passwd"), "______passwd");
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn cache_filename_produces_safe_names() {
        assert_eq!(cache_filename("owner", "repo"), "owner_repo_issues.json");
        assert_eq!(
            cache_filename("../etc", "../../passwd"),
            "___etc_______passwd_issues.json"
        );
        assert_eq!(
            cache_filename("foo/bar", "baz\\qux"),
            "foo_bar_baz_qux_issues.json"
        );
    }

    #[test]
    fn cache_path_traversal_is_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = IssueCache::with_dir(tmp.path()).unwrap();

        let issues = vec![Issue {
            number: 1,
            title: "Test".into(),
            state: "OPEN".into(),
            labels: vec![],
            assignee: None,
            body: None,
            url: "https://example.com".into(),
            updated_at: None,
        }];

        // Malicious owner/repo should not escape cache directory
        cache.write("../etc", "../../passwd", &issues).unwrap();

        // File should be inside the cache dir, not outside
        let filename = cache_filename("../etc", "../../passwd");
        let expected = tmp.path().join(&filename);
        assert!(
            expected.exists(),
            "cache file should exist at: {}",
            expected.display()
        );

        // Verify it is indeed within the cache directory
        assert!(expected.starts_with(tmp.path()));

        // Should be readable back
        let loaded = cache.read("../etc", "../../passwd").unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
    }
}
