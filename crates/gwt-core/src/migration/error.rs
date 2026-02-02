//! Migration error types (SPEC-a70a1ece T704)

use std::path::PathBuf;
use thiserror::Error;

/// Migration-specific errors
#[derive(Error, Debug)]
pub enum MigrationError {
    /// Insufficient disk space
    #[error("Insufficient disk space: need {needed} bytes, have {available} bytes")]
    InsufficientDiskSpace { needed: u64, available: u64 },

    /// Locked worktree detected
    #[error("Locked worktree detected: {path}. Please unlock it first with 'git worktree unlock'")]
    LockedWorktree { path: PathBuf },

    /// Backup creation failed
    #[error("Failed to create backup: {reason}")]
    BackupFailed { reason: String },

    /// Backup restoration failed
    #[error("Failed to restore backup: {reason}")]
    RestoreFailed { reason: String },

    /// Worktree migration failed
    #[error("Failed to migrate worktree '{branch}': {reason}")]
    WorktreeMigrationFailed { branch: String, reason: String },

    /// Bare repository creation failed
    #[error("Failed to create bare repository: {reason}")]
    BareRepoCreationFailed { reason: String },

    /// Network error (retryable)
    #[error("Network error (attempt {attempt}/{max_attempts}): {reason}")]
    NetworkError {
        reason: String,
        attempt: u32,
        max_attempts: u32,
    },

    /// Rollback failed
    #[error("Rollback failed: {reason}")]
    RollbackFailed { reason: String },

    /// Validation failed
    #[error("Validation failed: {reason}")]
    ValidationFailed { reason: String },

    /// Migration was cancelled by user
    #[error("Migration cancelled by user")]
    Cancelled,

    /// Source is not a valid migration candidate
    #[error("Not a valid migration source: {reason}")]
    InvalidSource { reason: String },

    /// Git operation failed
    #[error("Git operation failed: {reason}")]
    GitError { reason: String },

    /// IO error
    #[error("IO error at {path}: {reason}")]
    IoError { path: PathBuf, reason: String },

    /// Permission error
    #[error("Permission denied: {path}")]
    PermissionDenied { path: PathBuf },
}

impl MigrationError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, MigrationError::NetworkError { .. })
    }

    /// Check if this error should trigger a rollback
    pub fn should_rollback(&self) -> bool {
        matches!(
            self,
            MigrationError::WorktreeMigrationFailed { .. }
                | MigrationError::BareRepoCreationFailed { .. }
                | MigrationError::NetworkError { .. }
                | MigrationError::GitError { .. }
                | MigrationError::IoError { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable() {
        assert!(MigrationError::NetworkError {
            reason: "timeout".to_string(),
            attempt: 1,
            max_attempts: 3
        }
        .is_retryable());

        assert!(!MigrationError::Cancelled.is_retryable());
        assert!(!MigrationError::LockedWorktree {
            path: PathBuf::from("/test")
        }
        .is_retryable());
    }

    #[test]
    fn test_should_rollback() {
        assert!(MigrationError::WorktreeMigrationFailed {
            branch: "main".to_string(),
            reason: "error".to_string()
        }
        .should_rollback());

        assert!(!MigrationError::Cancelled.should_rollback());
        assert!(!MigrationError::LockedWorktree {
            path: PathBuf::from("/test")
        }
        .should_rollback());
    }

    #[test]
    fn test_error_messages() {
        let err = MigrationError::InsufficientDiskSpace {
            needed: 1000,
            available: 500,
        };
        assert!(err.to_string().contains("1000"));
        assert!(err.to_string().contains("500"));
    }
}
