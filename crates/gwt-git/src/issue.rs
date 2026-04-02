//! GitHub Issue tracking with file-based cache

use std::{
    fs,
    path::{Path, PathBuf},
};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

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
        let home = dirs::home_dir().ok_or_else(|| {
            GwtError::Other("Cannot determine home directory".into())
        })?;
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
        self.cache_dir.join(format!("{owner}-{repo}.json"))
    }
}

/// Fetch open issues from GitHub via `gh issue list --json`.
pub fn fetch_issues(owner: &str, repo: &str) -> Result<Vec<Issue>> {
    let repo_slug = format!("{owner}/{repo}");
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "list",
            "--repo",
            &repo_slug,
            "--state",
            "open",
            "--json",
            "number,title,state,labels,assignees,body,url",
            "--limit",
            "100",
        ])
        .output()
        .map_err(|e| GwtError::Git(format!("gh issue list: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("gh issue list: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_gh_issues_json(&stdout)
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

        issues.push(Issue {
            number,
            title,
            state,
            labels,
            assignee,
            body,
            url,
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
                "url": "https://github.com/owner/repo/issues/42"
            },
            {
                "number": 43,
                "title": "Add feature",
                "state": "OPEN",
                "labels": [],
                "assignees": [],
                "body": null,
                "url": "https://github.com/owner/repo/issues/43"
            }
        ]"#;

        let issues = parse_gh_issues_json(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 42);
        assert_eq!(issues[0].title, "Fix bug");
        assert_eq!(issues[0].labels, vec!["bug"]);
        assert_eq!(issues[0].assignee.as_deref(), Some("alice"));
        assert_eq!(issues[1].number, 43);
        assert!(issues[1].assignee.is_none());
        assert!(issues[1].body.is_none());
    }

    #[test]
    fn parse_gh_issues_json_empty() {
        let issues = parse_gh_issues_json("[]").unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn parse_gh_issues_json_invalid() {
        let result = parse_gh_issues_json("not json");
        assert!(result.is_err());
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
        }];
        cache.write("owner", "repo", &issues).unwrap();

        let loaded = cache.read("owner", "repo").unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].number, 1);
        assert_eq!(loaded[0].title, "Test");
    }
}
