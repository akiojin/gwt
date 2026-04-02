//! Git worktree management

use std::path::{Path, PathBuf};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Information about a single worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Filesystem path of the worktree.
    pub path: PathBuf,
    /// Branch checked out in this worktree.
    pub branch: Option<String>,
    /// Whether the worktree is locked.
    pub locked: bool,
    /// Whether the worktree is prunable (orphaned).
    pub prunable: bool,
}

/// Manages Git worktrees for a repository.
pub struct WorktreeManager {
    repo_path: PathBuf,
}

impl WorktreeManager {
    /// Create a new manager for the repository at `repo_path`.
    pub fn new(repo_path: impl AsRef<Path>) -> Self {
        Self {
            repo_path: repo_path.as_ref().to_path_buf(),
        }
    }

    /// List all worktrees for this repository.
    pub fn list(&self) -> Result<Vec<WorktreeInfo>> {
        let output = gwt_core::process::git_command()
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree list".into(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::GitOperationFailed {
                operation: "worktree list".into(),
                details: stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_porcelain_output(&stdout))
    }

    /// Create a new worktree at `path` for `branch`.
    pub fn create(&self, branch: &str, path: &Path) -> Result<()> {
        let output = gwt_core::process::git_command()
            .args([
                "worktree",
                "add",
                path.to_str().unwrap_or(""),
                branch,
            ])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::WorktreeCreateFailed {
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::WorktreeCreateFailed { reason: stderr });
        }

        Ok(())
    }

    /// Remove the worktree at `path`.
    pub fn remove(&self, path: &Path) -> Result<()> {
        let output = gwt_core::process::git_command()
            .args([
                "worktree",
                "remove",
                path.to_str().unwrap_or(""),
            ])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::WorktreeRemoveFailed {
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::WorktreeRemoveFailed { reason: stderr });
        }

        Ok(())
    }
}

/// Parse `git worktree list --porcelain` output into `WorktreeInfo` entries.
fn parse_porcelain_output(output: &str) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut locked = false;
    let mut prunable = false;

    for line in output.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            // Flush previous entry
            if let Some(prev_path) = path.take() {
                worktrees.push(WorktreeInfo {
                    path: prev_path,
                    branch: branch.take(),
                    locked,
                    prunable,
                });
                locked = false;
                prunable = false;
            }
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            // Strip refs/heads/ prefix
            branch = Some(
                b.strip_prefix("refs/heads/")
                    .unwrap_or(b)
                    .to_string(),
            );
        } else if line == "locked" {
            locked = true;
        } else if line == "prunable" {
            prunable = true;
        }
    }

    // Flush last entry
    if let Some(p) = path {
        worktrees.push(WorktreeInfo {
            path: p,
            branch,
            locked,
            prunable,
        });
    }

    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_single_entry() {
        let output = "worktree /home/user/repo\nbranch refs/heads/main\nHEAD abc1234\n\n";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/home/user/repo"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].locked);
        assert!(!entries[0].prunable);
    }

    #[test]
    fn parse_porcelain_multiple_entries() {
        let output = "\
worktree /repo
branch refs/heads/main

worktree /repo/wt-1
branch refs/heads/feature
locked

worktree /repo/wt-2
branch refs/heads/fix
prunable
";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].locked);
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
        assert!(entries[1].locked);
        assert_eq!(entries[2].branch.as_deref(), Some("fix"));
        assert!(entries[2].prunable);
    }

    #[test]
    fn parse_porcelain_empty() {
        let entries = parse_porcelain_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_porcelain_detached_head() {
        let output = "worktree /repo\nHEAD abc1234\ndetached\n\n";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn list_worktrees_in_test_repo() {
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

        let mgr = WorktreeManager::new(path);
        let wts = mgr.list().unwrap();
        // At minimum the main worktree
        assert!(!wts.is_empty());
    }
}
