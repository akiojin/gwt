//! Git stash operations for GitView

use super::{is_bare_repository, Repository};
use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A stash entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub file_count: usize,
}

fn find_any_worktree_path(repo_path: &Path) -> Option<PathBuf> {
    let repo = Repository::discover(repo_path).ok()?;
    let worktrees = repo.list_worktrees().ok()?;
    worktrees
        .into_iter()
        .find(|wt| !wt.is_bare && wt.path.exists())
        .map(|wt| wt.path)
}

/// Get the list of stash entries with file counts
pub fn get_stash_list(repo_path: &Path) -> Result<Vec<StashEntry>> {
    let mut exec_path = repo_path.to_path_buf();
    let mut output = crate::process::command("git")
        .args(["stash", "list", "--format=%gd%x00%gs"])
        .current_dir(&exec_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "stash list".to_string(),
            details: e.to_string(),
        })?;

    // Bare repositories require a worktree for stash operations. If we were given a bare repo,
    // retry with any existing worktree to list stashes correctly.
    if !output.status.success() && is_bare_repository(repo_path) {
        if let Some(wt_path) = find_any_worktree_path(repo_path) {
            exec_path = wt_path;
            output = crate::process::command("git")
                .args(["stash", "list", "--format=%gd%x00%gs"])
                .current_dir(&exec_path)
                .output()
                .map_err(|e| GwtError::GitOperationFailed {
                    operation: "stash list".to_string(),
                    details: e.to_string(),
                })?;
        }
    }

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
        let file_count = get_stash_file_count(&exec_path, index);

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
    let output = crate::process::command("git")
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

    fn get_current_branch_name(repo_path: &Path) -> String {
        let output = crate::process::command("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
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

    // T-STASH-003: Bare repo should list stashes via any existing worktree
    #[test]
    fn test_get_stash_list_from_bare_repo() {
        let temp = TempDir::new().unwrap();

        // Create a normal repo (source)
        let src = temp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        run_git(&src, &["init"]);
        run_git(&src, &["config", "user.email", "test@test.com"]);
        run_git(&src, &["config", "user.name", "Test User"]);
        std::fs::write(src.join("README.md"), "# Test\n").unwrap();
        run_git(&src, &["add", "."]);
        run_git(&src, &["commit", "-m", "initial commit"]);

        let base = get_current_branch_name(&src);

        // Clone as bare repo (gwt bare project style)
        let bare = temp.path().join("repo.git");
        let status = crate::process::command("git")
            .args(["clone", "--bare"])
            .arg(&src)
            .arg(&bare)
            .status()
            .unwrap();
        assert!(status.success(), "git clone --bare failed");

        // Create a worktree so stash operations are possible
        let wt = temp.path().join("wt");
        let status = crate::process::command("git")
            .args(["worktree", "add"])
            .arg(&wt)
            .arg(&base)
            .current_dir(&bare)
            .status()
            .unwrap();
        assert!(status.success(), "git worktree add failed");

        run_git(&wt, &["config", "user.email", "test@test.com"]);
        run_git(&wt, &["config", "user.name", "Test User"]);
        std::fs::write(wt.join("README.md"), "# Test\nstash change\n").unwrap();
        run_git(&wt, &["add", "README.md"]);
        run_git(&wt, &["stash", "push", "-m", "wip"]);

        let stashes = get_stash_list(&bare).unwrap();
        assert_eq!(stashes.len(), 1);
        assert!(stashes[0].message.contains("wip"));
        assert!(stashes[0].file_count >= 1);
    }
}
