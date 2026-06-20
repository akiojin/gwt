//! Failure-recovery routine triggered when any phase after `Backup` fails.
//!
//! Removes any partially-created bare or worktree directories, then restores
//! the original Normal Git layout from the backup snapshot.

use super::backup::{self, BackupError, BackupSnapshot};
use crate::process::hidden_command;

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

pub fn rollback_migration(snapshot: &BackupSnapshot) -> Result<(), RollbackError> {
    // Step 1: replay the full file-tree snapshot back into the project root.
    // This already brings back the original `.git/` directory, but we restore
    // the fetch refspec explicitly below so rollback is correct even if the
    // refspec was normalized in-place on a repo that survives the tree restore
    // (SPEC-1934 US-7 / FR-033, T-158).
    backup::restore(snapshot)?;

    // Step 2: restore the pre-normalize `remote.origin.fetch` value if one was
    // captured before the migration normalized it. Best-effort and mirrors the
    // other rollback steps: a git failure here must not mask the primary
    // restore outcome, so the write is run for effect only.
    restore_fetch_refspec(snapshot);

    Ok(())
}

/// Best-effort restore of `remote.origin.fetch` in the restored project to its
/// pre-normalize value. Does nothing when no refspec was recorded.
fn restore_fetch_refspec(snapshot: &BackupSnapshot) {
    let Some(refspec) = snapshot.pre_normalize_fetch_refspec.as_deref() else {
        return;
    };
    let _ = hidden_command("git")
        .args(["config", "remote.origin.fetch", refspec])
        .current_dir(&snapshot.project_root)
        .output();
}
