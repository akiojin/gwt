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
        let output = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::Git(format!("worktree list: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(format!("worktree list: {stderr}")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_porcelain_output(&stdout))
    }

    /// Create a new worktree at `path` for `branch`.
    pub fn create(&self, branch: &str, path: &Path) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["worktree", "add", path.to_str().unwrap_or(""), branch])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::Git(format!("worktree add: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Create a new worktree at `path`, creating `new_branch` from `base_branch`.
    pub fn create_from_base(&self, base_branch: &str, new_branch: &str, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(GwtError::Git(format!(
                "worktree path already exists: {}",
                path.display()
            )));
        }

        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                new_branch,
                path.to_str().unwrap_or(""),
                base_branch,
            ])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::Git(format!("worktree add -b: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }

    /// Remove the worktree at `path`.
    pub fn remove(&self, path: &Path) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["worktree", "remove", path.to_str().unwrap_or("")])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| GwtError::Git(format!("worktree remove: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(stderr));
        }

        Ok(())
    }
}

/// Derive a sibling worktree path from the repo root and branch name.
pub fn sibling_worktree_path(repo_path: &Path, branch: &str) -> PathBuf {
    let repo_name = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repo");
    let suffix = slug_branch_for_path(branch);
    let dir_name = if suffix.is_empty() {
        repo_name.to_string()
    } else {
        format!("{repo_name}-{suffix}")
    };

    repo_path.parent().unwrap_or(repo_path).join(dir_name)
}

fn slug_branch_for_path(branch: &str) -> String {
    let mut out = String::with_capacity(branch.len());
    let mut prev_dash = false;

    for ch in branch.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if matches!(ch, '-' | '_') {
            ch
        } else {
            '-'
        };

        if mapped == '-' {
            if !prev_dash {
                out.push(mapped);
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }

    out.trim_matches('-').to_string()
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
            branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        } else if matches_annotation(line, "locked") {
            locked = true;
        } else if matches_annotation(line, "prunable") {
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

fn matches_annotation(line: &str, key: &str) -> bool {
    line == key
        || line
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(path: &Path) {
        let output = std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("git config user.email");
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("git config user.name");
        assert!(name.status.success(), "git config user.name failed");
    }

    fn git_commit_allow_empty(path: &Path, message: &str) {
        let output = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(path)
            .output()
            .expect("git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_checkout_new_branch(path: &Path, branch: &str) {
        let output = std::process::Command::new("git")
            .args(["checkout", "-b", branch])
            .current_dir(path)
            .output()
            .expect("git checkout -b");
        assert!(
            output.status.success(),
            "git checkout -b failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

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
    fn parse_porcelain_reasoned_annotations() {
        let output = "\
worktree /repo
branch refs/heads/main
locked because maintenance is running

worktree /repo/wt-1
branch refs/heads/feature
prunable gitdir file points to non-existent location
";
        let entries = parse_porcelain_output(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(entries[0].locked);
        assert!(!entries[0].prunable);
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
        assert!(!entries[1].locked);
        assert!(entries[1].prunable);
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
    fn sibling_worktree_path_uses_repo_name_and_slugged_branch() {
        let repo_path = Path::new("/tmp/my-repo");
        let worktree = sibling_worktree_path(repo_path, "feature/banner");
        assert_eq!(worktree, PathBuf::from("/tmp/my-repo-feature-banner"));
    }

    #[test]
    fn create_from_base_creates_new_branch_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        init_git_repo(tmp.path());
        git_commit_allow_empty(tmp.path(), "initial commit");
        git_checkout_new_branch(tmp.path(), "develop");

        let manager = WorktreeManager::new(tmp.path());
        let worktree_path = sibling_worktree_path(tmp.path(), "feature/materialized");

        manager
            .create_from_base("develop", "feature/materialized", &worktree_path)
            .unwrap();

        assert!(worktree_path.exists());
        let branch_output = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&worktree_path)
            .output()
            .expect("git branch --show-current");
        assert!(branch_output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&branch_output.stdout).trim(),
            "feature/materialized"
        );
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
