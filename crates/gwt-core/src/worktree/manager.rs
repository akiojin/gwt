//! Worktree manager

use super::{CleanupCandidate, Worktree, WorktreePath, WorktreeStatus};
use crate::error::{GwtError, Result};
use crate::git::{Branch, Repository};
use std::path::{Path, PathBuf};

/// Protected branch names that cannot be deleted
const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop", "release"];

/// Worktree manager for creating, listing, and removing worktrees
pub struct WorktreeManager {
    /// Repository root path
    repo_root: PathBuf,
    /// Git repository handle
    repo: Repository,
}

impl WorktreeManager {
    /// Create a new worktree manager
    pub fn new(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let repo = Repository::discover(&repo_root)?;
        Ok(Self { repo_root, repo })
    }

    /// Get the repository root path
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// List all worktrees
    pub fn list(&self) -> Result<Vec<Worktree>> {
        let git_worktrees = self.repo.list_worktrees()?;
        let mut worktrees = Vec::with_capacity(git_worktrees.len());

        for info in &git_worktrees {
            let mut wt = Worktree::from_git_info(info);

            // Check for changes if worktree is active
            if wt.status == WorktreeStatus::Active {
                if let Ok(wt_repo) = Repository::open(&wt.path) {
                    wt.has_changes = wt_repo.has_uncommitted_changes().unwrap_or(false);
                    wt.has_unpushed = wt_repo.has_unpushed_commits().unwrap_or(false);
                }
            }

            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// Get a specific worktree by branch name
    pub fn get_by_branch(&self, branch_name: &str) -> Result<Option<Worktree>> {
        let worktrees = self.list()?;
        Ok(worktrees
            .into_iter()
            .find(|wt| wt.branch.as_deref() == Some(branch_name)))
    }

    /// Get a specific worktree by path
    pub fn get_by_path(&self, path: &Path) -> Result<Option<Worktree>> {
        let worktrees = self.list()?;
        Ok(worktrees.into_iter().find(|wt| wt.path == path))
    }

    /// Create a new worktree for an existing branch
    pub fn create_for_branch(&self, branch_name: &str) -> Result<Worktree> {
        let path = WorktreePath::generate(&self.repo_root, branch_name);

        if path.exists() {
            return Err(GwtError::WorktreeAlreadyExists { path });
        }

        // Check if branch exists
        if !Branch::exists(&self.repo_root, branch_name)? {
            return Err(GwtError::BranchNotFound {
                name: branch_name.to_string(),
            });
        }

        // Create worktree
        self.repo.create_worktree(&path, branch_name, false)?;

        // Return the created worktree
        self.get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path })
    }

    /// Create a new worktree with a new branch
    pub fn create_new_branch(
        &self,
        branch_name: &str,
        base_branch: Option<&str>,
    ) -> Result<Worktree> {
        let path = WorktreePath::generate(&self.repo_root, branch_name);

        if path.exists() {
            return Err(GwtError::WorktreeAlreadyExists { path });
        }

        // Check if branch already exists
        if Branch::exists(&self.repo_root, branch_name)? {
            return Err(GwtError::BranchAlreadyExists {
                name: branch_name.to_string(),
            });
        }

        // If base branch specified, checkout it first
        if let Some(base) = base_branch {
            // Verify base branch exists
            if !Branch::exists(&self.repo_root, base)? {
                return Err(GwtError::BranchNotFound {
                    name: base.to_string(),
                });
            }
        }

        // Create worktree with new branch
        self.repo.create_worktree(&path, branch_name, true)?;

        // If base branch specified, reset to it
        if let Some(base) = base_branch {
            let wt_repo = Repository::open(&path)?;
            std::process::Command::new("git")
                .args(["reset", "--hard", base])
                .current_dir(&path)
                .output()
                .map_err(|e| GwtError::WorktreeCreateFailed {
                    reason: e.to_string(),
                })?;
            drop(wt_repo);
        }

        // Return the created worktree
        self.get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path })
    }

    /// Remove a worktree by path
    pub fn remove(&self, path: &Path, force: bool) -> Result<()> {
        // Check if worktree exists
        let wt = self.get_by_path(path)?.ok_or_else(|| GwtError::WorktreeNotFound {
            path: path.to_path_buf(),
        })?;

        // Check for protected branch
        if let Some(ref branch) = wt.branch {
            if Self::is_protected(branch) && !force {
                return Err(GwtError::ProtectedBranch {
                    branch: branch.clone(),
                });
            }
        }

        // Check for uncommitted changes
        if wt.has_changes && !force {
            return Err(GwtError::UncommittedChanges);
        }

        // Remove worktree
        self.repo.remove_worktree(path, force)?;

        Ok(())
    }

    /// Remove a worktree and delete the branch
    pub fn remove_with_branch(&self, path: &Path, force: bool) -> Result<()> {
        let wt = self.get_by_path(path)?.ok_or_else(|| GwtError::WorktreeNotFound {
            path: path.to_path_buf(),
        })?;

        let branch_name = wt.branch.clone();

        // Remove worktree first
        self.remove(path, force)?;

        // Delete branch if it exists
        if let Some(name) = branch_name {
            if Branch::exists(&self.repo_root, &name)? {
                Branch::delete(&self.repo_root, &name, force)?;
            }
        }

        Ok(())
    }

    /// Check if a branch is protected
    pub fn is_protected(branch_name: &str) -> bool {
        PROTECTED_BRANCHES.contains(&branch_name)
    }

    /// Detect orphaned worktrees
    pub fn detect_orphans(&self) -> Vec<CleanupCandidate> {
        CleanupCandidate::detect(&self.repo_root)
    }

    /// Prune orphaned worktree metadata
    pub fn prune(&self) -> Result<()> {
        self.repo.prune_worktrees()
    }

    /// Repair worktree administrative files
    pub fn repair(&self) -> Result<()> {
        self.repo.repair_worktrees()
    }

    /// Repair a specific worktree path
    pub fn repair_path(&self, _path: &Path) -> Result<()> {
        // Repair all worktrees (git doesn't support per-path repair)
        self.repo.repair_worktrees()
    }

    /// Lock a worktree
    pub fn lock(&self, path: &Path, reason: Option<&str>) -> Result<()> {
        let path_str = path.to_string_lossy();
        let mut args = vec!["worktree", "lock", &path_str];
        if let Some(r) = reason {
            args.push("--reason");
            args.push(r);
        }

        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree lock".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "worktree lock".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Unlock a worktree
    pub fn unlock(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        let output = std::process::Command::new("git")
            .args(["worktree", "unlock", &path_str])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "worktree unlock".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GwtError::GitOperationFailed {
                operation: "worktree unlock".to_string(),
                details: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    /// Get count of active worktrees (excluding main)
    pub fn active_count(&self) -> Result<usize> {
        let worktrees = self.list()?;
        Ok(worktrees
            .iter()
            .filter(|wt| !wt.is_main && wt.is_active())
            .count())
    }

    /// Get worktrees needing attention
    pub fn needing_attention(&self) -> Result<Vec<Worktree>> {
        let worktrees = self.list()?;
        Ok(worktrees
            .into_iter()
            .filter(|wt| wt.needs_attention())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
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
        temp
    }

    #[test]
    fn test_is_protected() {
        assert!(WorktreeManager::is_protected("main"));
        assert!(WorktreeManager::is_protected("master"));
        assert!(WorktreeManager::is_protected("develop"));
        assert!(!WorktreeManager::is_protected("feature/foo"));
    }

    #[test]
    fn test_list_worktrees() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();
        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_create_new_branch_worktree() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/test", None).unwrap();
        assert_eq!(wt.branch, Some("feature/test".to_string()));
        assert!(wt.path.exists());

        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 2);
    }

    #[test]
    fn test_create_for_existing_branch() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Create a branch first
        Branch::create(temp.path(), "feature/existing", "HEAD").unwrap();

        let wt = manager.create_for_branch("feature/existing").unwrap();
        assert_eq!(wt.branch, Some("feature/existing".to_string()));
        assert!(wt.path.exists());
    }

    #[test]
    fn test_remove_worktree() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/remove", None).unwrap();
        let path = wt.path.clone();

        manager.remove(&path, false).unwrap();

        let worktrees = manager.list().unwrap();
        assert_eq!(worktrees.len(), 1);
    }

    #[test]
    fn test_active_count() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Main worktree exists - it counts as 1 because is_main is false for regular repos
        // (is_main/is_bare is only true for bare repositories)
        let initial_count = manager.active_count().unwrap();

        manager.create_new_branch("feature/count", None).unwrap();
        let count = manager.active_count().unwrap();
        assert_eq!(count, initial_count + 1);
    }
}
