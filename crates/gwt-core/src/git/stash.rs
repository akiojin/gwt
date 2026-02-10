//! Git stash operations for GitView

use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// A stash entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub file_count: usize,
}

/// Get the list of stash entries with file counts
pub fn get_stash_list(repo_path: &Path) -> Result<Vec<StashEntry>> {
    let output = Command::new("git")
        .args(["stash", "list", "--format=%gd%x00%gs"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "stash list".to_string(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "stash list".to_string(),
            details: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, '\0').collect();
        if parts.len() < 2 {
            continue;
        }

        let gd = parts[0]; // e.g., "stash@{0}"
        let message = parts[1].to_string();

        // Parse index from "stash@{N}"
        let index = gd
            .strip_prefix("stash@{")
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Get file count for this stash entry
        let file_count = get_stash_file_count(repo_path, index);

        entries.push(StashEntry {
            index,
            message,
            file_count,
        });
    }

    Ok(entries)
}

/// Get the number of files changed in a stash entry
fn get_stash_file_count(repo_path: &Path, index: usize) -> usize {
    let stash_ref = format!("stash@{{{}}}", index);
    let output = Command::new("git")
        .args(["stash", "show", &stash_ref, "--name-only"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .count(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = Command::new("git")
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

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test User"]);
        std::fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial commit"]);
        temp
    }

    // T-STASH-001: Basic stash retrieval
    #[test]
    fn test_get_stash_list_with_entries() {
        let temp = create_test_repo();

        // Create a stash entry
        std::fs::write(temp.path().join("stash1.txt"), "stash content 1\n").unwrap();
        run_git(temp.path(), &["add", "stash1.txt"]);
        run_git(temp.path(), &["stash", "push", "-m", "first stash"]);

        // Create another stash entry
        std::fs::write(temp.path().join("stash2.txt"), "stash content 2\n").unwrap();
        std::fs::write(temp.path().join("stash3.txt"), "stash content 3\n").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["stash", "push", "-m", "second stash"]);

        let stashes = get_stash_list(temp.path()).unwrap();
        assert_eq!(stashes.len(), 2);

        // Most recent stash is index 0
        assert_eq!(stashes[0].index, 0);
        assert!(stashes[0].message.contains("second stash"));
        assert_eq!(stashes[0].file_count, 2);

        assert_eq!(stashes[1].index, 1);
        assert!(stashes[1].message.contains("first stash"));
        assert_eq!(stashes[1].file_count, 1);
    }

    // T-STASH-002: Empty stash
    #[test]
    fn test_get_stash_list_empty() {
        let temp = create_test_repo();

        let stashes = get_stash_list(temp.path()).unwrap();
        assert!(stashes.is_empty());
    }
}
