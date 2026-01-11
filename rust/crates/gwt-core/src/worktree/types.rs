//! Worktree type definitions

use std::path::PathBuf;

/// Represents a Git worktree
#[derive(Debug, Clone)]
pub struct Worktree {
    /// Worktree path
    pub path: PathBuf,
    /// Branch name (None if detached HEAD or bare)
    pub branch: Option<String>,
    /// HEAD commit SHA
    pub commit: String,
    /// Worktree status
    pub status: WorktreeStatus,
    /// Is this the main worktree (bare repository)
    pub is_main: bool,
    /// Has uncommitted changes
    pub has_changes: bool,
    /// Has unpushed commits
    pub has_unpushed: bool,
}

impl Worktree {
    /// Create a new worktree instance
    pub fn new(
        path: impl Into<PathBuf>,
        branch: impl Into<String>,
        commit: impl Into<String>,
    ) -> Self {
        let branch_str = branch.into();
        Self {
            path: path.into(),
            branch: if branch_str.is_empty() {
                None
            } else {
                Some(branch_str)
            },
            commit: commit.into(),
            status: WorktreeStatus::Active,
            is_main: false,
            has_changes: false,
            has_unpushed: false,
        }
    }

    /// Create from git worktree info
    pub fn from_git_info(info: &crate::git::WorktreeInfo) -> Self {
        let status = if info.is_prunable {
            WorktreeStatus::Prunable
        } else if info.is_locked {
            WorktreeStatus::Locked
        } else if !info.path.exists() {
            WorktreeStatus::Missing
        } else {
            WorktreeStatus::Active
        };

        Self {
            path: info.path.clone(),
            branch: info.branch.clone(),
            commit: info.head.clone(),
            status,
            is_main: info.is_bare,
            has_changes: false,
            has_unpushed: false,
        }
    }

    /// Get display name (branch name or commit)
    pub fn display_name(&self) -> String {
        self.branch
            .clone()
            .unwrap_or_else(|| format!("({})", &self.commit[..7.min(self.commit.len())]))
    }

    /// Check if this worktree is active
    pub fn is_active(&self) -> bool {
        self.status == WorktreeStatus::Active
    }

    /// Check if this worktree needs attention
    pub fn needs_attention(&self) -> bool {
        matches!(
            self.status,
            WorktreeStatus::Prunable | WorktreeStatus::Missing
        )
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

impl std::fmt::Display for WorktreeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Locked => write!(f, "locked"),
            Self::Prunable => write!(f, "prunable"),
            Self::Missing => write!(f, "missing"),
        }
    }
}

/// Cleanup candidate for orphaned worktrees
#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    /// Worktree path
    pub path: PathBuf,
    /// Branch name if available
    pub branch: Option<String>,
    /// Reason for cleanup
    pub reason: CleanupReason,
}

impl CleanupCandidate {
    /// Detect orphaned worktrees in a repository
    pub fn detect(repo_path: &std::path::Path) -> Vec<CleanupCandidate> {
        use crate::git::Repository;

        let mut candidates = Vec::new();

        let repo = match Repository::discover(repo_path) {
            Ok(r) => r,
            Err(_) => return candidates,
        };

        let worktrees = match repo.list_worktrees() {
            Ok(w) => w,
            Err(_) => return candidates,
        };

        for wt in worktrees {
            if wt.is_bare {
                continue;
            }

            let reason = if wt.is_prunable {
                CleanupReason::Orphaned
            } else if !wt.path.exists() {
                CleanupReason::PathMissing
            } else {
                continue;
            };

            candidates.push(CleanupCandidate {
                path: wt.path,
                branch: wt.branch,
                reason,
            });
        }

        candidates
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

impl std::fmt::Display for CleanupReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathMissing => write!(f, "path missing"),
            Self::BranchDeleted => write!(f, "branch deleted"),
            Self::Orphaned => write!(f, "orphaned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_display_name() {
        let wt = Worktree::new("/path", "feature/test", "abc123");
        assert_eq!(wt.display_name(), "feature/test");

        let wt_detached = Worktree::new("/path", "", "abc123456789");
        assert_eq!(wt_detached.display_name(), "(abc1234)");
    }

    #[test]
    fn test_worktree_status_display() {
        assert_eq!(WorktreeStatus::Active.to_string(), "active");
        assert_eq!(WorktreeStatus::Locked.to_string(), "locked");
        assert_eq!(WorktreeStatus::Prunable.to_string(), "prunable");
        assert_eq!(WorktreeStatus::Missing.to_string(), "missing");
    }
}
