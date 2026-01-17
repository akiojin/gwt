//! Repository operations

use crate::error::{GwtError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info};

/// Represents a Git repository
#[derive(Debug)]
pub struct Repository {
    /// Repository root path
    root: PathBuf,
    /// Internal gix repository handle (lazy loaded)
    gix_repo: Option<gix::Repository>,
}

impl Repository {
    /// Discover a repository from a path
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        // Try using gix first
        match gix::discover(path) {
            Ok(repo) => {
                let root = repo
                    .workdir()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| repo.git_dir().to_path_buf());
                Ok(Self {
                    root,
                    gix_repo: Some(repo),
                })
            }
            Err(_) => {
                // Fallback: Manual .git directory search
                let mut current = path.to_path_buf();
                loop {
                    if current.join(".git").exists() {
                        return Ok(Self {
                            root: current,
                            gix_repo: None,
                        });
                    }
                    if !current.pop() {
                        break;
                    }
                }

                Err(GwtError::RepositoryNotFound {
                    path: path.to_path_buf(),
                })
            }
        }
    }

    /// Open a repository at the given path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        match gix::open(path) {
            Ok(repo) => {
                let work_dir = repo.workdir().map(|p| p.to_path_buf());
                let git_dir = repo.git_dir().to_path_buf();
                let root = work_dir.clone().unwrap_or_else(|| git_dir.clone());

                tracing::debug!(
                    "Repository::open: input_path={:?}, work_dir={:?}, git_dir={:?}, resolved_root={:?}",
                    path, work_dir, git_dir, root
                );

                Ok(Self {
                    root,
                    gix_repo: Some(repo),
                })
            }
            Err(_) => Err(GwtError::RepositoryNotFound {
                path: path.to_path_buf(),
            }),
        }
    }

    /// Get the repository root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the main repository root (resolves through worktree to main repo)
    /// For worktrees, this returns the path to the main repository.
    /// For normal repos, this returns the same as root().
    pub fn main_repo_root(&self) -> PathBuf {
        // Use git rev-parse --git-common-dir to get the common git directory
        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(&self.root)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let common_dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
                // common_dir is like "/gwt/.git" - parent is the repo root
                let common_path = PathBuf::from(&common_dir);
                if common_path.is_absolute() {
                    common_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| self.root.clone())
                } else {
                    // Relative path - resolve from current root
                    self.root
                        .join(&common_path)
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| self.root.clone())
                }
            }
            _ => self.root.clone(),
        }
    }

    /// Get internal gix repository reference
    fn gix_repo(&self) -> Option<&gix::Repository> {
        self.gix_repo.as_ref()
    }

    /// Check if there are uncommitted changes (staged or unstaged)
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        // Use external git for reliability with worktrees
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "status".to_string(),
                details: e.to_string(),
            })?;

        let has_changes = !output.stdout.is_empty();

        tracing::debug!(
            "has_uncommitted_changes: path={:?}, has_changes={}, output={:?}",
            self.root,
            has_changes,
            String::from_utf8_lossy(&output.stdout)
        );

        Ok(has_changes)
    }

    /// Check if there are unpushed commits
    pub fn has_unpushed_commits(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["log", "@{u}..", "--oneline"])
            .current_dir(&self.root)
            .output();

        match output {
            Ok(o) => Ok(!o.stdout.is_empty()),
            Err(_) => Ok(false), // No upstream configured
        }
    }

    /// Get the current HEAD reference name
    pub fn head_name(&self) -> Result<Option<String>> {
        if let Some(repo) = self.gix_repo() {
            match repo.head_name() {
                Ok(Some(name)) => Ok(Some(name.shorten().to_string())),
                Ok(None) => Ok(None), // Detached HEAD
                Err(_) => self.head_name_external(),
            }
        } else {
            self.head_name_external()
        }
    }

    /// Get HEAD name using external git command
    fn head_name_external(&self) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if name == "HEAD" {
                Ok(None) // Detached HEAD
            } else {
                Ok(Some(name))
            }
        } else {
            Ok(None)
        }
    }

    /// Get the current HEAD commit SHA
    pub fn head_commit(&self) -> Result<String> {
        if let Some(repo) = self.gix_repo() {
            match repo.head_id() {
                Ok(id) => Ok(id.to_hex().to_string()),
                Err(_) => self.head_commit_external(),
            }
        } else {
            self.head_commit_external()
        }
    }

    /// Get HEAD commit using external git command
    fn head_commit_external(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "rev-parse HEAD".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Pull with fast-forward only
    pub fn pull_fast_forward(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "pull".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "pull --ff-only".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Fetch all remotes
    pub fn fetch_all(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["fetch", "--all", "--prune"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "fetch".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "fetch --all".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// List all worktrees using git worktree list
    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree list".to_string(),
                details: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(GwtError::GitOperationFailed {
                operation: "worktree list".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current: Option<WorktreeInfo> = None;

        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                if let Some(wt) = current.take() {
                    worktrees.push(wt);
                }
                current = Some(WorktreeInfo {
                    path: PathBuf::from(path),
                    head: String::new(),
                    branch: None,
                    is_bare: false,
                    is_detached: false,
                    is_locked: false,
                    is_prunable: false,
                });
            } else if let Some(ref mut wt) = current {
                if let Some(head) = line.strip_prefix("HEAD ") {
                    wt.head = head.to_string();
                } else if let Some(branch) = line.strip_prefix("branch ") {
                    // Convert refs/heads/xxx to xxx
                    wt.branch = Some(
                        branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_string(),
                    );
                } else if line == "bare" {
                    wt.is_bare = true;
                } else if line == "detached" {
                    wt.is_detached = true;
                } else if line == "locked" {
                    wt.is_locked = true;
                } else if line == "prunable" {
                    wt.is_prunable = true;
                }
            }
        }

        if let Some(wt) = current {
            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// Create a new worktree
    pub fn create_worktree(&self, path: &Path, branch: &str, new_branch: bool) -> Result<()> {
        debug!(
            category = "git",
            path = %path.display(),
            branch,
            new_branch,
            "Creating git worktree"
        );

        let mut args = vec!["worktree", "add"];

        if new_branch {
            args.push("-b");
            args.push(branch);
        }

        let path_str = path.to_string_lossy();
        args.push(&path_str);

        if !new_branch {
            args.push(branch);
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree add".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_add",
                path = %path.display(),
                branch,
                new_branch,
                "Git worktree created"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_add",
                path = %path.display(),
                branch,
                error = err_msg.as_str(),
                "Failed to create git worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree add".to_string(),
                details: err_msg,
            })
        }
    }

    /// Remove a worktree
    pub fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "git",
            path = %path.display(),
            force,
            "Removing git worktree"
        );

        let path_str = path.to_string_lossy();
        let args = if force {
            vec!["worktree", "remove", "--force", &path_str]
        } else {
            vec!["worktree", "remove", &path_str]
        };

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree remove".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_remove",
                path = %path.display(),
                force,
                "Git worktree removed"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_remove",
                path = %path.display(),
                error = err_msg.as_str(),
                "Failed to remove git worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree remove".to_string(),
                details: err_msg,
            })
        }
    }

    /// Prune stale worktree metadata
    pub fn prune_worktrees(&self) -> Result<()> {
        debug!(category = "git", "Pruning stale worktree metadata");

        let output = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree prune".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_prune",
                "Worktree metadata pruned"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_prune",
                error = err_msg.as_str(),
                "Failed to prune worktree metadata"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree prune".to_string(),
                details: err_msg,
            })
        }
    }

    /// Repair worktree administrative files
    pub fn repair_worktrees(&self) -> Result<()> {
        debug!(category = "git", "Repairing worktree administrative files");

        let output = Command::new("git")
            .args(["worktree", "repair"])
            .current_dir(&self.root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree repair".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            info!(
                category = "git",
                operation = "worktree_repair",
                "Worktree administrative files repaired"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "git",
                operation = "worktree_repair",
                error = err_msg.as_str(),
                "Failed to repair worktree administrative files"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree repair".to_string(),
                details: err_msg,
            })
        }
    }
}

/// Information about a worktree from git worktree list
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Worktree path
    pub path: PathBuf,
    /// HEAD commit SHA
    pub head: String,
    /// Branch name (None if detached or bare)
    pub branch: Option<String>,
    /// Is this a bare repository
    pub is_bare: bool,
    /// Is HEAD detached
    pub is_detached: bool,
    /// Is worktree locked
    pub is_locked: bool,
    /// Is worktree prunable
    pub is_prunable: bool,
}

/// Get the main repository root from any path (resolves through worktree to main repo)
/// This is a standalone function that doesn't require a Repository instance.
/// For worktrees, this returns the path to the main repository.
/// For normal repos or non-repo paths, this returns the original path.
pub fn get_main_repo_root(path: &Path) -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let common_dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let common_path = PathBuf::from(&common_dir);
            if common_path.is_absolute() {
                common_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.to_path_buf())
            } else {
                // Relative path - resolve from current path
                let resolved = path.join(&common_path);
                resolved
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.to_path_buf())
            }
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, Repository) {
        let temp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let repo = Repository::discover(temp.path()).unwrap();
        (temp, repo)
    }

    #[test]
    fn test_discover_not_found() {
        let temp = TempDir::new().unwrap();
        let result = Repository::discover(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_discover_found() {
        let (temp, repo) = create_test_repo();
        assert_eq!(repo.root(), temp.path());
    }

    #[test]
    fn test_has_uncommitted_changes_clean() {
        let (_temp, repo) = create_test_repo();
        // Empty repo, no changes
        let result = repo.has_uncommitted_changes().unwrap();
        assert!(!result);
    }

    #[test]
    fn test_has_uncommitted_changes_dirty() {
        let (temp, repo) = create_test_repo();
        // Create an untracked file
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        let result = repo.has_uncommitted_changes().unwrap();
        assert!(result);
    }

    #[test]
    fn test_head_name_initial() {
        let (_temp, repo) = create_test_repo();
        // Git 2.28+ defaults to main, older versions use master
        let name = repo.head_name().unwrap();
        // Initial repo might not have a valid HEAD yet
        assert!(
            name.is_none() || name.as_deref() == Some("main") || name.as_deref() == Some("master")
        );
    }

    #[test]
    fn test_list_worktrees() {
        let (temp, repo) = create_test_repo();
        // Create initial commit
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        let worktrees = repo.list_worktrees().unwrap();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, temp.path());
    }
}
