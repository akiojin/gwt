//! Worktree type definitions

use std::path::PathBuf;

/// Represents a Git worktree
#[derive(Debug, Clone)]
pub struct Worktree {
    /// Worktree path
    pub path: PathBuf,
    /// Branch name
    pub branch: String,
    /// Commit SHA
    pub commit: String,
    /// Worktree status
    pub status: WorktreeStatus,
}

impl Worktree {
    /// Create a new worktree instance
    pub fn new(
        path: impl Into<PathBuf>,
        branch: impl Into<String>,
        commit: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            branch: branch.into(),
            commit: commit.into(),
            status: WorktreeStatus::Active,
        }
    }
}

/// Worktree status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    /// Active and healthy
    Active,
    /// Locked by this or another process
    Locked,
    /// Prunable (orphaned)
    Prunable,
    /// Path is missing
    Missing,
}

/// Cleanup candidate for orphaned worktrees
#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    /// Worktree path
    pub path: PathBuf,
    /// Reason for cleanup
    pub reason: CleanupReason,
}

impl CleanupCandidate {
    /// Detect orphaned worktrees
    pub fn detect(_repo_path: &std::path::Path) -> Vec<CleanupCandidate> {
        // TODO: Implement detection logic
        Vec::new()
    }
}

/// Reason for worktree cleanup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupReason {
    /// Path no longer exists
    PathMissing,
    /// Branch was deleted
    BranchDeleted,
    /// Orphaned (no git metadata)
    Orphaned,
}
