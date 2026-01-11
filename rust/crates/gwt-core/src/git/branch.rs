//! Branch operations

use crate::error::Result;

/// Represents a Git branch
#[derive(Debug, Clone)]
pub struct Branch {
    /// Branch name (e.g., "main", "feature/foo")
    pub name: String,
    /// Whether this is the current branch
    pub is_current: bool,
    /// Whether this branch has a remote tracking branch
    pub has_remote: bool,
    /// Commit SHA
    pub commit: String,
}

impl Branch {
    /// Create a new branch instance
    pub fn new(name: impl Into<String>, commit: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_current: false,
            has_remote: false,
            commit: commit.into(),
        }
    }

    /// List all branches in a repository
    pub fn list(_repo_path: &std::path::Path) -> Result<Vec<Branch>> {
        // TODO: Implement using gix
        Ok(Vec::new())
    }

    /// Get the current branch
    pub fn current(_repo_path: &std::path::Path) -> Result<Option<Branch>> {
        // TODO: Implement using gix
        Ok(None)
    }

    /// Create a new branch
    pub fn create(_repo_path: &std::path::Path, _name: &str, _base: &str) -> Result<Branch> {
        // TODO: Implement using gix
        todo!()
    }

    /// Delete a branch
    pub fn delete(_repo_path: &std::path::Path, _name: &str) -> Result<()> {
        // TODO: Implement using gix
        todo!()
    }

    /// Check divergence status from remote
    pub fn divergence_status(&self) -> DivergenceStatus {
        // TODO: Implement
        DivergenceStatus::UpToDate
    }
}

/// Branch divergence status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// Branch is up to date with remote
    UpToDate,
    /// Branch is ahead of remote
    Ahead(usize),
    /// Branch is behind remote
    Behind(usize),
    /// Branch has diverged from remote
    Diverged { ahead: usize, behind: usize },
    /// No remote tracking branch
    NoRemote,
}
