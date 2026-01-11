//! Git backend abstraction for gix/external command switching

#![allow(dead_code)] // Backend implementations for future use

use crate::error::Result;
use std::path::Path;

/// Git backend trait for abstracting git operations
pub trait GitBackend: Send + Sync {
    /// Check if the backend is available
    fn is_available(&self) -> bool;

    /// Discover a repository from a path
    fn discover(&self, path: &Path) -> Result<Box<dyn GitRepository>>;
}

/// Git repository abstraction
pub trait GitRepository: Send + Sync {
    /// Get the repository root path
    fn root(&self) -> &Path;

    /// Check if the repository has uncommitted changes
    fn has_uncommitted_changes(&self) -> Result<bool>;

    /// Check if the repository has unpushed commits
    fn has_unpushed_commits(&self) -> Result<bool>;
}

/// Gix-based git backend
pub struct GixBackend;

impl GixBackend {
    /// Create a new gix backend
    pub fn new() -> Self {
        Self
    }
}

impl Default for GixBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// External git command backend (fallback)
pub struct ExternalGitBackend;

impl ExternalGitBackend {
    /// Create a new external git backend
    pub fn new() -> Self {
        Self
    }

    /// Check if git command is available
    pub fn check_git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok()
    }
}

impl Default for ExternalGitBackend {
    fn default() -> Self {
        Self::new()
    }
}
