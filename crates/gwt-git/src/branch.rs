//! Branch information and tracking

use std::path::Path;

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Information about a Git branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Branch name (e.g. "main", "origin/main").
    pub name: String,
    /// Whether this is a local branch.
    pub is_local: bool,
    /// Whether this is a remote-tracking branch.
    pub is_remote: bool,
    /// Whether this branch is currently checked out (HEAD).
    pub is_head: bool,
    /// Upstream tracking branch name (e.g. "origin/main").
    pub upstream: Option<String>,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
    /// ISO 8601 date of the last commit on this branch.
    pub last_commit_date: Option<String>,
}

/// List branches with full tracking info for the repo at `repo_path`.
pub fn list_branches(repo_path: &Path) -> Result<Vec<Branch>> {
    let format = "%(refname:short)\t%(HEAD)\t%(upstream:short)\t%(upstream:track)\t%(creatordate:iso8601)";
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            &format!("--format={format}"),
            "refs/heads/",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("for-each-ref: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("for-each-ref: {stderr}")));
    }

    let mut branches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(b) = parse_branch_line(line) {
            branches.push(b);
        }
    }

    // Also list remote branches
    let remote_output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)\t%(creatordate:iso8601)",
            "refs/remotes/",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("for-each-ref remotes: {e}")))?;

    if remote_output.status.success() {
        for line in String::from_utf8_lossy(&remote_output.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() || parts[0].is_empty() {
                continue;
            }
            let name = parts[0].to_string();
            // Skip HEAD pointer
            if name.ends_with("/HEAD") {
                continue;
            }
            let date = parts.get(1).map(|s| s.trim().to_string());
            branches.push(Branch {
                name,
                is_local: false,
                is_remote: true,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: date,
            });
        }
    }

    Ok(branches)
}

/// Parse ahead/behind from the tracking info string like "[ahead 3, behind 2]".
fn parse_ahead_behind(track: &str) -> (u32, u32) {
    let mut ahead = 0u32;
    let mut behind = 0u32;

    if track.contains("ahead") {
        if let Some(n) = track
            .split("ahead ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
        {
            ahead = n;
        }
    }
    if track.contains("behind") {
        if let Some(n) = track
            .split("behind ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
        {
            behind = n;
        }
    }

    (ahead, behind)
}

fn parse_branch_line(line: &str) -> Option<Branch> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }

    let name = parts[0].to_string();
    let is_head = parts.get(1).is_some_and(|s| s.trim() == "*");
    let upstream = parts
        .get(2)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let track = parts.get(3).unwrap_or(&"");
    let (ahead, behind) = parse_ahead_behind(track);
    let last_commit_date = parts
        .get(4)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string());

    Some(Branch {
        name,
        is_local: true,
        is_remote: false,
        is_head,
        upstream,
        ahead,
        behind,
        last_commit_date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ahead_behind_both() {
        assert_eq!(parse_ahead_behind("[ahead 3, behind 2]"), (3, 2));
    }

    #[test]
    fn parse_ahead_behind_ahead_only() {
        assert_eq!(parse_ahead_behind("[ahead 5]"), (5, 0));
    }

    #[test]
    fn parse_ahead_behind_behind_only() {
        assert_eq!(parse_ahead_behind("[behind 1]"), (0, 1));
    }

    #[test]
    fn parse_ahead_behind_empty() {
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn parse_branch_line_full() {
        let line = "main\t*\torigin/main\t[ahead 1]\t2025-01-01 00:00:00 +0000";
        let b = parse_branch_line(line).unwrap();
        assert_eq!(b.name, "main");
        assert!(b.is_head);
        assert_eq!(b.upstream.as_deref(), Some("origin/main"));
        assert_eq!(b.ahead, 1);
        assert_eq!(b.behind, 0);
        assert!(b.last_commit_date.is_some());
    }

    #[test]
    fn parse_branch_line_minimal() {
        let line = "feature\t \t\t\t";
        let b = parse_branch_line(line).unwrap();
        assert_eq!(b.name, "feature");
        assert!(!b.is_head);
        assert!(b.upstream.is_none());
    }

    #[test]
    fn parse_branch_line_empty() {
        assert!(parse_branch_line("").is_none());
    }

    #[test]
    fn list_branches_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let branches = list_branches(path).unwrap();
        assert!(!branches.is_empty());
        assert!(branches.iter().any(|b| b.is_local));
    }
}
