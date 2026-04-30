//! Failure-recovery routine triggered when any phase after `Backup` fails.
//!
//! Removes any partially-created bare or worktree directories, then restores
//! the original Normal Git layout from the backup snapshot.

use super::backup::{self, BackupError, BackupSnapshot};

#[derive(Debug)]
pub enum RollbackError {
    Backup(BackupError),
}

impl std::fmt::Display for RollbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backup(e) => write!(f, "rollback failed: {e}"),
        }
    }
}

impl std::error::Error for RollbackError {}

impl From<BackupError> for RollbackError {
    fn from(value: BackupError) -> Self {
        Self::Backup(value)
    }
}

pub fn rollback_migration(_snapshot: &BackupSnapshot) -> Result<(), RollbackError> {
    // Filled in by Phase 9 tasks (T-081, T-083, T-085).
    backup::restore(_snapshot)?;
    Ok(())
}
