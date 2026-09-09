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
    let max_count = format!("--max-count={count}");
    let format_arg = format!("--format={format}");
    let output =
        gwt_core::process::run_git_logged(&["log", &max_count, &format_arg], Some(repo_path))
            .map_err(|e| GwtError::Git(format!("log: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // Empty repo has no commits — treat as empty list
        if stderr.contains("does not have any commits") || stderr.contains("bad default revision") {
            return Ok(Vec::new());
        }
        return Err(GwtError::Git(format!("log: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_log_output(&stdout))
}

/// SPEC-3075 FR-006: the recent NON-merge commit subjects of a single branch,
/// newest first (up to `count`). This is the AI summary input for branches
/// whose tip is a merge/release commit (no informative purpose): the real work
/// is in the underlying feature commits, so `--no-merges` skips the noise. The
/// branch may be a local or `origin/<branch>` short ref. Empty when the branch
/// is unknown or has only merge commits.
pub fn branch_recent_subjects(repo_path: &Path, branch: &str, count: usize) -> Result<Vec<String>> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Ok(Vec::new());
    }
    let max_count = format!("--max-count={count}");
    let output = gwt_core::process::run_git_logged(
        &[
            "log",
            "--no-merges",
            &max_count,
            "--format=%s",
            branch,
            "--",
        ],
        Some(repo_path),
    )
    .map_err(|e| GwtError::Git(format!("log {branch}: {e}")))?;
    if !output.status.success() {
        // Unknown branch / empty history is not an error for this best-effort
        // signal — return nothing so the caller falls back to other sources.
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
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
    fn branch_recent_subjects_returns_non_merge_subjects_newest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let ok = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "--initial-branch=main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        git(&["commit", "--allow-empty", "-m", "feat: first work"]);
        git(&["commit", "--allow-empty", "-m", "fix: second work"]);

        let subjects = branch_recent_subjects(repo, "main", 5).unwrap();
        assert_eq!(subjects, vec!["fix: second work", "feat: first work"]);
        // Unknown branch is best-effort empty, not an error.
        assert!(branch_recent_subjects(repo, "no/such-branch", 5)
            .unwrap()
            .is_empty());
    }

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

    /// Set committer identity on the fixture repo; CI runners have no
    /// global git config, so `git commit` fails silently without this.
    fn set_test_identity(path: &std::path::Path) {
        for (key, value) in [("user.email", "test@example.com"), ("user.name", "Test")] {
            let output = gwt_core::process::hidden_command("git")
                .args(["config", key, value])
                .current_dir(path)
                .output()
                .unwrap();
            assert!(output.status.success(), "git config {key} failed");
        }
    }

    #[test]
    fn recent_commits_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        set_test_identity(path);
        gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-m", "first commit"])
            .current_dir(path)
            .output()
            .unwrap();
        gwt_core::process::hidden_command("git")
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
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        set_test_identity(path);
        for i in 0..5 {
            gwt_core::process::hidden_command("git")
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
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();

        let commits = recent_commits(path, 10).unwrap();
        assert!(commits.is_empty());
    }
}
