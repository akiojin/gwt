//! Repository operations

use crate::error::{GwtError, Result};
use std::path::{Path, PathBuf};

/// Represents a Git repository
#[derive(Debug)]
pub struct Repository {
    /// Repository root path
    root: PathBuf,
}

impl Repository {
    /// Discover a repository from a path
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        // Try to find .git directory by walking up
        let mut current = path.to_path_buf();
        loop {
            if current.join(".git").exists() {
                return Ok(Self { root: current });
            }
            if !current.pop() {
                break;
            }
        }

        Err(GwtError::RepositoryNotFound {
            path: path.to_path_buf(),
        })
    }

    /// Get the repository root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Check if there are uncommitted changes
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        // TODO: Implement using gix
        Ok(false)
    }

    /// Check if there are unpushed commits
    pub fn has_unpushed_commits(&self) -> Result<bool> {
        // TODO: Implement using gix
        Ok(false)
    }

    /// Pull with fast-forward only
    pub fn pull_fast_forward(&self) -> Result<()> {
        // TODO: Implement using gix or external git
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_discover_not_found() {
        let temp = TempDir::new().unwrap();
        let result = Repository::discover(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_discover_found() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        let repo = Repository::discover(temp.path()).unwrap();
        assert_eq!(repo.root(), temp.path());
    }
}
