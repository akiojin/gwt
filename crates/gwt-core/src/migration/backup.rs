//! Full-tree backup and restore for the migration's `.gwt-migration-backup/`
//! directory (Phase 4 of the build plan).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::Utc;

#[derive(Debug)]
pub enum BackupError {
    Io(io::Error),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "backup io error: {e}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<io::Error> for BackupError {
    fn from(value: io::Error) -> Self {
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

/// Create a full snapshot of `project_root` into
/// `<project_root>/.gwt-migration-backup/`. Any pre-existing backup directory
/// is renamed with a UTC timestamp suffix so the previous attempt is not
/// silently overwritten.
pub fn create(project_root: &Path) -> Result<BackupSnapshot, BackupError> {
    let backup_dir = project_root.join(BACKUP_DIR_NAME);

    if backup_dir.exists() {
        let stamped = project_root.join(format!(
            "{BACKUP_DIR_NAME}-{}",
            Utc::now().format("%Y%m%dT%H%M%S")
        ));
        fs::rename(&backup_dir, &stamped)?;
    }

    fs::create_dir_all(&backup_dir)?;
    copy_dir_contents(project_root, &backup_dir, &[BACKUP_DIR_NAME])?;

    Ok(BackupSnapshot {
        project_root: project_root.to_path_buf(),
        backup_dir,
    })
}

/// Replay a snapshot back into `snapshot.project_root`. Files added since the
/// snapshot are removed (excluding the backup directory itself) and the
/// backup contents are copied back into place.
pub fn restore(snapshot: &BackupSnapshot) -> Result<(), BackupError> {
    let project_root = &snapshot.project_root;
    let backup_dir = &snapshot.backup_dir;
    let backup_name = backup_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(BACKUP_DIR_NAME)
        .to_string();

    // Step 1: remove every entry in project_root except the backup directory.
    if let Ok(entries) = fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy() == backup_name {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || metadata.is_file() {
                fs::remove_file(&path)?;
            } else if metadata.is_dir() {
                fs::remove_dir_all(&path)?;
            }
        }
    }

    // Step 2: copy backup contents back to project root.
    copy_dir_contents(backup_dir, project_root, &[])?;

    Ok(())
}

/// Delete a backup snapshot (called from the Cleanup phase on success).
pub fn discard(snapshot: BackupSnapshot) -> Result<(), BackupError> {
    if snapshot.backup_dir.exists() {
        fs::remove_dir_all(&snapshot.backup_dir)?;
    }
    Ok(())
}

/// Recursively copy the contents of `src` into `dst`, skipping any top-level
/// entry whose file name matches `excluded`.
fn copy_dir_contents(src: &Path, dst: &Path, excluded: &[&str]) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if excluded.iter().any(|e| **e == *name_str) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let metadata = fs::symlink_metadata(&from)?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            // Skip symlinks for now: backup semantics for symlinks are
            // undefined in the spec and copying targets is potentially
            // dangerous on a Normal Git repo (e.g. `.git` worktree markers).
            continue;
        }
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
