//! Worktree manager

use super::{Worktree, WorktreePath, WorktreeStatus};
use crate::error::{GwtError, Result};
use std::path::{Path, PathBuf};

/// Protected branch names that cannot be deleted
const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop", "release"];

/// Worktree manager for creating, listing, and removing worktrees
pub struct WorktreeManager {
    /// Repository root path
    repo_root: PathBuf,
}

impl WorktreeManager {
    /// Create a new worktree manager
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    /// List all worktrees
    pub fn list(&self) -> Result<Vec<Worktree>> {
        let worktrees_dir = self.repo_root.join(".worktrees");
        if !worktrees_dir.exists() {
            return Ok(Vec::new());
        }

        let mut worktrees = Vec::new();
        for entry in std::fs::read_dir(&worktrees_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                // TODO: Read branch and commit from git metadata
                worktrees.push(Worktree {
                    path: path.clone(),
                    branch: path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    commit: String::new(),
                    status: WorktreeStatus::Active,
                });
            }
        }

        Ok(worktrees)
    }

    /// Create a new worktree
    pub fn create(&self, branch_name: &str, base_branch: Option<&str>) -> Result<Worktree> {
        let path = WorktreePath::generate(&self.repo_root, branch_name);

        if path.exists() {
            return Err(GwtError::WorktreeAlreadyExists { path });
        }

        // Create worktree directory
        std::fs::create_dir_all(&path)?;

        // TODO: Actually create git worktree using gix or external git
        let _ = base_branch;

        Ok(Worktree::new(path, branch_name, ""))
    }

    /// Remove a worktree
    pub fn remove(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Err(GwtError::WorktreeNotFound {
                path: path.to_path_buf(),
            });
        }

        // TODO: Use git worktree remove command
        std::fs::remove_dir_all(path)?;

        Ok(())
    }

    /// Check if a branch is protected
    pub fn is_protected(branch_name: &str) -> bool {
        PROTECTED_BRANCHES
            .iter()
            .any(|&protected| branch_name == protected)
    }

    /// Repair a worktree path
    pub fn repair_path(&self, _path: &Path) -> Result<()> {
        // TODO: Implement repair logic
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_protected() {
        assert!(WorktreeManager::is_protected("main"));
        assert!(WorktreeManager::is_protected("master"));
        assert!(WorktreeManager::is_protected("develop"));
        assert!(!WorktreeManager::is_protected("feature/foo"));
    }
}
