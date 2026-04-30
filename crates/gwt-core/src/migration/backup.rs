//! Full-tree backup and restore for the migration's `.gwt-migration-backup/`
//! directory (Phase 4 of the build plan).

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum BackupError {
    Io(std::io::Error),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "backup io error: {e}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<std::io::Error> for BackupError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// The directory name reserved for migration backups under `<project_root>`.
pub const BACKUP_DIR_NAME: &str = ".gwt-migration-backup";

/// Snapshot returned by [`create`]; used by rollback to find the source.
#[derive(Debug, Clone)]
pub struct BackupSnapshot {
    pub project_root: PathBuf,
    pub backup_dir: PathBuf,
}

pub fn create(_project_root: &Path) -> Result<BackupSnapshot, BackupError> {
    // Filled in by Phase 4 tasks (T-031, T-033).
    unimplemented!("backup::create — see SPEC-1934 tasks T-031/T-033")
}

pub fn restore(_snapshot: &BackupSnapshot) -> Result<(), BackupError> {
    // Filled in by Phase 4 tasks (T-035).
    unimplemented!("backup::restore — see SPEC-1934 task T-035")
}

pub fn discard(_snapshot: BackupSnapshot) -> Result<(), BackupError> {
    // Filled in by Phase 8 tasks (T-074/T-075).
    unimplemented!("backup::discard — see SPEC-1934 task T-074/T-075")
}
