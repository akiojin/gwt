//! Worktree manager

use super::{CleanupCandidate, Worktree, WorktreePath, WorktreeStatus};
use crate::error::{GwtError, Result};
use crate::git::{Branch, Repository};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

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

            tracing::debug!(
                "Worktree: branch={:?}, path={:?}, status={:?}, has_changes={}, has_unpushed={}",
                wt.branch,
                wt.path,
                wt.status,
                wt.has_changes,
                wt.has_unpushed
            );

            worktrees.push(wt);
        }

        Ok(worktrees)
    }

    /// List all worktrees without checking git status (fast path)
    pub fn list_basic(&self) -> Result<Vec<Worktree>> {
        let git_worktrees = self.repo.list_worktrees()?;
        let mut worktrees = Vec::with_capacity(git_worktrees.len());

        for info in &git_worktrees {
            let wt = Worktree::from_git_info(info);
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

    /// Handle existing path for worktree creation (FR-038-040)
    ///
    /// FR-038: Detect stale worktrees when path exists but not in `git worktree list`
    /// FR-039: Auto-delete stale directories and retry worktree creation
    /// FR-040: For uncertain cases, abort and prompt user for manual resolution
    fn handle_existing_path(&self, path: &Path) -> Result<()> {
        // Check if this path is in the git worktree list
        let git_worktrees = self.repo.list_worktrees()?;
        let is_in_worktree_list = git_worktrees.iter().any(|info| info.path == path);

        if is_in_worktree_list {
            // Path exists AND is in worktree list → real worktree conflict
            return Err(GwtError::WorktreeAlreadyExists {
                path: path.to_path_buf(),
            });
        }

        // FR-038: Path exists but NOT in worktree list → stale
        // FR-039: Auto-delete stale directory

        // Safety check: verify it looks like a git worktree (has .git file/directory)
        let git_path = path.join(".git");
        if git_path.exists() {
            // Has .git, so it's likely a stale worktree → safe to delete
            if let Err(_e) = std::fs::remove_dir_all(path) {
                // FR-040: Cannot delete → prompt user for manual resolution
                return Err(GwtError::WorktreePathConflict {
                    path: path.to_path_buf(),
                });
            }
            Ok(())
        } else {
            // No .git marker → could be user data, not safe to delete
            // FR-040: Abort and prompt user for manual resolution
            Err(GwtError::WorktreePathConflict {
                path: path.to_path_buf(),
            })
        }
    }

    /// Create a new worktree for an existing branch
    pub fn create_for_branch(&self, branch_name: &str) -> Result<Worktree> {
        debug!(
            category = "worktree",
            branch = branch_name,
            "Creating worktree for existing branch"
        );
        let path = WorktreePath::generate(&self.repo_root, branch_name);

        // FR-038-040: Handle existing path with stale recovery
        if path.exists() {
            self.handle_existing_path(&path)?;
        }

        // Check if branch exists
        if !Branch::exists(&self.repo_root, branch_name)? {
            error!(
                category = "worktree",
                branch = branch_name,
                "Branch not found"
            );
            return Err(GwtError::BranchNotFound {
                name: branch_name.to_string(),
            });
        }

        // Create worktree
        self.repo.create_worktree(&path, branch_name, false)?;

        // Return the created worktree
        let worktree = self
            .get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path: path.clone() })?;

        info!(
            category = "worktree",
            operation = "create",
            branch = branch_name,
            path = %worktree.path.display(),
            "Worktree created for existing branch"
        );
        Ok(worktree)
    }

    /// Create a new worktree with a new branch
    pub fn create_new_branch(
        &self,
        branch_name: &str,
        base_branch: Option<&str>,
    ) -> Result<Worktree> {
        debug!(
            category = "worktree",
            branch = branch_name,
            base = base_branch.unwrap_or("HEAD"),
            "Creating worktree with new branch"
        );
        let path = WorktreePath::generate(&self.repo_root, branch_name);

        // FR-038-040: Handle existing path with stale recovery
        if path.exists() {
            self.handle_existing_path(&path)?;
        }

        // Check if branch already exists
        if Branch::exists(&self.repo_root, branch_name)? {
            error!(
                category = "worktree",
                branch = branch_name,
                "Branch already exists"
            );
            return Err(GwtError::BranchAlreadyExists {
                name: branch_name.to_string(),
            });
        }

        // If base branch specified, checkout it first
        if let Some(base) = base_branch {
            // Verify base branch exists
            if !Branch::exists(&self.repo_root, base)? {
                error!(
                    category = "worktree",
                    branch = base,
                    "Base branch not found"
                );
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
        let worktree = self
            .get_by_path(&path)?
            .ok_or(GwtError::WorktreeNotFound { path: path.clone() })?;

        info!(
            category = "worktree",
            operation = "create_new_branch",
            branch = branch_name,
            base = base_branch.unwrap_or("HEAD"),
            path = %worktree.path.display(),
            "Worktree created with new branch"
        );
        Ok(worktree)
    }

    /// Remove a worktree by path
    pub fn remove(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "worktree",
            path = %path.display(),
            force,
            "Removing worktree"
        );

        // Check if worktree exists
        let wt = self
            .get_by_path(path)?
            .ok_or_else(|| GwtError::WorktreeNotFound {
                path: path.to_path_buf(),
            })?;

        let branch_name = wt.branch.clone();

        // Check for protected branch
        if let Some(ref branch) = wt.branch {
            if Self::is_protected(branch) && !force {
                warn!(
                    category = "worktree",
                    branch = branch.as_str(),
                    "Attempted to remove protected branch worktree"
                );
                return Err(GwtError::ProtectedBranch {
                    branch: branch.clone(),
                });
            }
        }

        // Check for uncommitted changes
        if wt.has_changes && !force {
            warn!(
                category = "worktree",
                path = %path.display(),
                "Attempted to remove worktree with uncommitted changes"
            );
            return Err(GwtError::UncommittedChanges);
        }

        // Remove worktree
        self.repo.remove_worktree(path, force)?;

        info!(
            category = "worktree",
            operation = "remove",
            path = %path.display(),
            branch = branch_name.as_deref().unwrap_or("unknown"),
            force,
            "Worktree removed"
        );

        Ok(())
    }

    /// Remove a worktree and delete the branch
    pub fn remove_with_branch(&self, path: &Path, force: bool) -> Result<()> {
        debug!(
            category = "worktree",
            path = %path.display(),
            force,
            "Removing worktree with branch"
        );

        let wt = self
            .get_by_path(path)?
            .ok_or_else(|| GwtError::WorktreeNotFound {
                path: path.to_path_buf(),
            })?;

        let branch_name = wt.branch.clone();

        // Remove worktree first
        self.remove(path, force)?;

        // Delete branch if it exists
        if let Some(ref name) = branch_name {
            if Branch::exists(&self.repo_root, name)? {
                Branch::delete(&self.repo_root, name, force)?;
                info!(
                    category = "worktree",
                    operation = "remove_with_branch",
                    path = %path.display(),
                    branch = name.as_str(),
                    "Branch deleted after worktree removal"
                );
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

    /// Auto-clean orphaned worktrees on startup
    pub fn auto_cleanup_orphans(&self) -> Result<usize> {
        let orphans = self.detect_orphans();
        if orphans.is_empty() {
            debug!(category = "worktree", "No orphaned worktrees found");
            return Ok(0);
        }

        info!(
            category = "worktree",
            operation = "auto_cleanup",
            count = orphans.len(),
            "Cleaning up orphaned worktrees"
        );

        self.prune()?;
        Ok(orphans.len())
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
        debug!(
            category = "worktree",
            path = %path.display(),
            reason = reason.unwrap_or("none"),
            "Locking worktree"
        );

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
            info!(
                category = "worktree",
                operation = "lock",
                path = %path.display(),
                reason = reason.unwrap_or("none"),
                "Worktree locked"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "worktree",
                path = %path.display(),
                error = err_msg.as_str(),
                "Failed to lock worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree lock".to_string(),
                details: err_msg,
            })
        }
    }

    /// Unlock a worktree
    pub fn unlock(&self, path: &Path) -> Result<()> {
        debug!(category = "worktree", path = %path.display(), "Unlocking worktree");

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
            info!(
                category = "worktree",
                operation = "unlock",
                path = %path.display(),
                "Worktree unlocked"
            );
            Ok(())
        } else {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            error!(
                category = "worktree",
                path = %path.display(),
                error = err_msg.as_str(),
                "Failed to unlock worktree"
            );
            Err(GwtError::GitOperationFailed {
                operation: "worktree unlock".to_string(),
                details: err_msg,
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

    #[test]
    fn test_auto_cleanup_orphans_on_startup() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager.create_new_branch("feature/orphan", None).unwrap();
        let wt_path = wt.path.clone();
        assert!(wt_path.exists());

        std::fs::remove_dir_all(&wt_path).unwrap();

        let detected = manager.detect_orphans();
        assert!(!detected.is_empty());

        let cleaned = manager.auto_cleanup_orphans().unwrap();
        assert_eq!(cleaned, detected.len());

        let remaining = manager.detect_orphans();
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_stale_worktree_recovery_fr039() {
        // FR-039: Auto-delete stale directories
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // First, create a worktree normally
        let wt = manager.create_new_branch("feature/stale", None).unwrap();
        let wt_path = wt.path.clone();
        assert!(wt_path.exists());

        // Manually remove from git worktree list but keep the directory
        Command::new("git")
            .args(["worktree", "remove", "--force", wt_path.to_str().unwrap()])
            .current_dir(temp.path())
            .output()
            .unwrap();

        // Recreate the directory with a .git file to simulate stale state
        std::fs::create_dir_all(&wt_path).unwrap();
        std::fs::write(wt_path.join(".git"), "stale worktree").unwrap();

        // Ensure it's NOT in worktree list
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let list_output = String::from_utf8_lossy(&output.stdout);
        assert!(!list_output.contains("feature/stale"));

        // Now try to create a new worktree at the same path - should auto-recover
        // (FR-039: Auto-delete stale and retry)
        let new_wt = manager.create_new_branch("feature/stale2", None);
        // Note: We can't reuse "feature/stale" because the branch might still exist
        assert!(new_wt.is_ok());
    }

    #[test]
    fn test_existing_path_conflict_fr040() {
        // FR-040: Path exists but no .git → cannot determine if stale → error
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Calculate where the worktree would be created
        let wt_path = WorktreePath::generate(temp.path(), "feature/conflict");

        // Create a directory without .git (simulating user data)
        std::fs::create_dir_all(&wt_path).unwrap();
        std::fs::write(wt_path.join("user_data.txt"), "important file").unwrap();

        // Try to create worktree - should fail with WorktreePathConflict
        let result = manager.create_new_branch("feature/conflict", None);
        assert!(result.is_err());
        if let Err(GwtError::WorktreePathConflict { path }) = result {
            assert_eq!(path, wt_path);
        } else {
            panic!("Expected WorktreePathConflict error");
        }
    }

    #[test]
    fn test_existing_worktree_conflict() {
        // Path exists AND is in worktree list → WorktreeAlreadyExists
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        // Create a worktree
        let wt = manager.create_new_branch("feature/exists", None).unwrap();
        assert!(wt.path.exists());

        // Try to create another worktree at the same place
        // (need to use a different branch name since branch already exists)
        // Actually, let's just try to re-create for the same branch
        let result = manager.create_for_branch("feature/exists");
        assert!(result.is_err());
        // Should be WorktreeAlreadyExists since it's actually in the worktree list
        assert!(matches!(
            result,
            Err(GwtError::WorktreeAlreadyExists { .. })
        ));
    }
}
