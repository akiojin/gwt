//! Domain types for the Normal-to-Bare+Worktree migration workflow.

use std::path::PathBuf;

/// Caller-tunable options for a migration run.
#[derive(Debug, Clone, Default)]
pub struct MigrationOptions {
    /// When true, validate and report what would happen but do not mutate state.
    pub dry_run: bool,
    /// When true, keep the migration backup directory after a successful run.
    pub keep_backup_on_success: bool,
    /// Override the branch name used for the main worktree directory.
    pub branch_override: Option<String>,
}

/// One worktree slated for migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMigration {
    pub path: PathBuf,
    pub branch: String,
    pub is_main_repo: bool,
    pub is_dirty: bool,
    pub is_locked: bool,
}

/// Plan computed before execution: the set of worktrees to move and the
/// resulting layout root.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub project_root: PathBuf,
    pub bare_repo_name: String,
    pub remote_url: Option<String>,
    pub worktrees: Vec<WorktreeMigration>,
}

/// Phase markers used for state-machine progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationPhase {
    Confirm,
    Validate,
    Backup,
    Bareify,
    Worktrees,
    Submodules,
    Tracking,
    Cleanup,
    Done,
    Error,
    RolledBack,
}

impl MigrationPhase {
    /// Stable, lower-case, dash-free identifier used in WebSocket events.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Validate => "validate",
            Self::Backup => "backup",
            Self::Bareify => "bareify",
            Self::Worktrees => "worktrees",
            Self::Submodules => "submodules",
            Self::Tracking => "tracking",
            Self::Cleanup => "cleanup",
            Self::Done => "done",
            Self::Error => "error",
            Self::RolledBack => "rolled_back",
        }
    }
}

impl std::fmt::Display for MigrationPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Successful outcome reported back to the caller.
#[derive(Debug, Clone)]
pub struct MigrationOutcome {
    pub branch_worktree_path: PathBuf,
    pub bare_repo_path: PathBuf,
    pub migrated_worktrees: Vec<PathBuf>,
}

/// Recovery state attached to a [`MigrationError`] so the UI can decide
/// what to offer the user (Retry / Restore / Quit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryState {
    /// Nothing was changed on disk yet; safe to retry.
    Untouched,
    /// A partial mutation was made and successfully rolled back.
    RolledBack,
    /// Rollback could not fully restore the original layout.
    Partial,
}

/// Failure shape returned by [`super::executor::execute_migration`].
#[derive(Debug)]
pub struct MigrationError {
    pub phase: MigrationPhase,
    pub message: String,
    pub recovery: RecoveryState,
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "migration failed at phase {} (recovery: {:?}): {}",
            self.phase, self.recovery, self.message
        )
    }
}

impl std::error::Error for MigrationError {}
