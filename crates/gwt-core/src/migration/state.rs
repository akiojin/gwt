//! Migration state machine (SPEC-a70a1ece T703)

use serde::{Deserialize, Serialize};

/// Migration state (SPEC-a70a1ece FR-224)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MigrationState {
    /// Initial state - waiting for user confirmation
    #[default]
    Pending,
    /// Validating prerequisites (disk space, locked worktrees, etc.)
    Validating,
    /// Creating backup of current state
    BackingUp,
    /// Creating the bare repository
    CreatingBareRepo,
    /// Migrating worktrees one by one
    MigratingWorktrees {
        /// Current worktree index (0-based)
        current: usize,
        /// Total number of worktrees
        total: usize,
    },
    /// Cleaning up old .worktrees/ directory
    CleaningUp,
    /// Migration completed successfully
    Completed,
    /// Migration failed, rolling back
    RollingBack,
    /// Migration was cancelled by user
    Cancelled,
    /// Migration failed with error
    Failed,
}

impl MigrationState {
    /// Get a human-readable description of the current state
    pub fn description(&self) -> String {
        match self {
            MigrationState::Pending => "Waiting for confirmation...".to_string(),
            MigrationState::Validating => "Validating prerequisites...".to_string(),
            MigrationState::BackingUp => "Creating backup...".to_string(),
            MigrationState::CreatingBareRepo => "Creating bare repository...".to_string(),
            MigrationState::MigratingWorktrees { current, total } => {
                format!("Migrating worktrees ({}/{})...", current + 1, total)
            }
            MigrationState::CleaningUp => "Cleaning up...".to_string(),
            MigrationState::Completed => "Migration completed!".to_string(),
            MigrationState::RollingBack => "Rolling back changes...".to_string(),
            MigrationState::Cancelled => "Migration cancelled.".to_string(),
            MigrationState::Failed => "Migration failed.".to_string(),
        }
    }

    /// Check if this is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            MigrationState::Completed | MigrationState::Cancelled | MigrationState::Failed
        )
    }

    /// Check if migration is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            MigrationState::Validating
                | MigrationState::BackingUp
                | MigrationState::CreatingBareRepo
                | MigrationState::MigratingWorktrees { .. }
                | MigrationState::CleaningUp
                | MigrationState::RollingBack
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = MigrationState::default();
        assert_eq!(state, MigrationState::Pending);
    }

    #[test]
    fn test_is_terminal() {
        assert!(MigrationState::Completed.is_terminal());
        assert!(MigrationState::Cancelled.is_terminal());
        assert!(MigrationState::Failed.is_terminal());
        assert!(!MigrationState::Pending.is_terminal());
        assert!(!MigrationState::BackingUp.is_terminal());
    }

    #[test]
    fn test_is_in_progress() {
        assert!(!MigrationState::Pending.is_in_progress());
        assert!(MigrationState::Validating.is_in_progress());
        assert!(MigrationState::BackingUp.is_in_progress());
        assert!(MigrationState::MigratingWorktrees {
            current: 0,
            total: 3
        }
        .is_in_progress());
        assert!(!MigrationState::Completed.is_in_progress());
    }

    #[test]
    fn test_description() {
        assert_eq!(
            MigrationState::MigratingWorktrees {
                current: 1,
                total: 5
            }
            .description(),
            "Migrating worktrees (2/5)..."
        );
    }
}
