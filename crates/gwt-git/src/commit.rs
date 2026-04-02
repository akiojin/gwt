//! Git commit log queries

use std::path::Path;

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// A single commit entry from the git log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitEntry {
    /// Abbreviated commit hash.
    pub hash: String,
    /// First line of the commit message.
    pub subject: String,
    /// Author name.
    pub author: String,
    /// ISO 8601 commit timestamp.
    pub timestamp: String,
}

/// Fetch recent commits from the repository.
///
/// Returns up to `count` commits from HEAD.
pub fn recent_commits(repo_path: &Path, count: usize) -> Result<Vec<CommitEntry>> {
    let format = "%h\t%s\t%an\t%aI";
    let output = gwt_core::process::git_command()
        .args([
            "log",
            &format!("--max-count={count}"),
            &format!("--format={format}"),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "log".into(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // Empty repo has no commits — treat as empty list
        if stderr.contains("does not have any commits") || stderr.contains("bad default revision") {
            return Ok(Vec::new());
        }
        return Err(GwtError::GitOperationFailed {
            operation: "log".into(),
            details: stderr,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_log_output(&stdout))
}

/// Parse tab-separated git log output.
pub fn parse_log_output(output: &str) -> Vec<CommitEntry> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(CommitEntry {
                hash: parts[0].to_string(),
                subject: parts[1].to_string(),
                author: parts[2].to_string(),
                timestamp: parts[3].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_output_valid() {
        let output = "abc1234\tfeat: add feature\tAlice\t2025-01-01T00:00:00+00:00\n\
                       def5678\tfix: bug\tBob\t2025-01-02T00:00:00+00:00\n";
        let entries = parse_log_output(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].hash, "abc1234");
        assert_eq!(entries[0].subject, "feat: add feature");
        assert_eq!(entries[0].author, "Alice");
        assert_eq!(entries[1].hash, "def5678");
        assert_eq!(entries[1].subject, "fix: bug");
    }

    #[test]
    fn parse_log_output_with_tabs_in_subject() {
        let output = "abc1234\tsubject with\ttabs\tAlice\t2025-01-01T00:00:00+00:00\n";
        let entries = parse_log_output(output);
        assert_eq!(entries.len(), 1);
        // splitn(4, '\t') means tabs after the 4th split are part of the last field
        assert_eq!(entries[0].hash, "abc1234");
    }

    #[test]
    fn parse_log_output_empty() {
        let entries = parse_log_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_log_output_malformed_line() {
        let output = "short line\n";
        let entries = parse_log_output(output);
        assert!(entries.is_empty());
    }

    #[test]
    fn recent_commits_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "first commit"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "second commit"])
            .current_dir(path)
            .output()
            .unwrap();

        let commits = recent_commits(path, 10).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "second commit");
        assert_eq!(commits[1].subject, "first commit");
    }

    #[test]
    fn recent_commits_respects_count() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        for i in 0..5 {
            std::process::Command::new("git")
                .args(["commit", "--allow-empty", "-m", &format!("commit {i}")])
                .current_dir(path)
                .output()
                .unwrap();
        }

        let commits = recent_commits(path, 3).unwrap();
        assert_eq!(commits.len(), 3);
    }

    #[test]
    fn recent_commits_empty_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();

        let commits = recent_commits(path, 10).unwrap();
        assert!(commits.is_empty());
    }
}
