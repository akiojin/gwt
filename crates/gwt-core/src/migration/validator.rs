//! Pre-flight checks before a migration starts (Phase 3 of the build plan).
//!
//! - disk space (project size × 2 must be available)
//! - locked worktrees
//! - write permission on the project root

use std::{
    fs,
    path::{Path, PathBuf},
};

use fs2::available_space;

use crate::process::hidden_command;

#[derive(Debug)]
pub enum ValidationError {
    InsufficientDiskSpace { required: u64, available: u64 },
    LockedWorktrees(Vec<PathBuf>),
    WritePermissionDenied(PathBuf),
    Io(std::io::Error),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientDiskSpace {
                required,
                available,
            } => write!(
                f,
                "insufficient disk space: required {required} bytes, available {available} bytes"
            ),
            Self::LockedWorktrees(paths) => write!(f, "locked worktrees: {paths:?}"),
            Self::WritePermissionDenied(path) => {
                write!(f, "write permission denied: {}", path.display())
            }
            Self::Io(e) => write!(f, "validator io error: {e}"),
        }
    }
}

impl std::error::Error for ValidationError {}

impl From<std::io::Error> for ValidationError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Pure decision rule used by [`check_disk_space`]: succeed when at least
/// `required` bytes are available.
pub fn evaluate_disk_space(required: u64, available: u64) -> Result<(), ValidationError> {
    if available >= required {
        Ok(())
    } else {
        Err(ValidationError::InsufficientDiskSpace {
            required,
            available,
        })
    }
}

/// Run an IO-backed disk-space check for the given `project_root`. The
/// migration writes a full backup before mutating the layout, so we require
/// `directory_size × 2` bytes of headroom.
pub fn check_disk_space(project_root: &Path) -> Result<(), ValidationError> {
    let used = directory_size(project_root)?;
    let required = used.saturating_mul(2);
    let available = available_space(project_root).map_err(ValidationError::Io)?;
    evaluate_disk_space(required, available)
}

/// Compute the total bytes consumed by `path`, walking subdirectories.
/// Symlinks are not followed.
fn directory_size(path: &Path) -> Result<u64, ValidationError> {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&current)? {
                let entry = entry?;
                stack.push(entry.path());
            }
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

/// Detect any worktrees marked as locked under the given project. Locked
/// worktrees prevent migration because we cannot move their on-disk path.
pub fn check_locked_worktrees(project_root: &Path) -> Result<(), ValidationError> {
    let output = hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map_err(ValidationError::Io)?;

    if !output.status.success() {
        // Not a git repository or `git` failed: treat as no locked worktrees.
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut locked = Vec::new();
    let mut current_path: Option<PathBuf> = None;

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            current_path = Some(PathBuf::from(rest.trim()));
        } else if line.starts_with("locked") {
            if let Some(p) = current_path.clone() {
                locked.push(p);
            }
        } else if line.is_empty() {
            current_path = None;
        }
    }

    if locked.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::LockedWorktrees(locked))
    }
}

/// Confirm the migration can write into `project_root` by creating a probe
/// file. The probe is removed before returning.
pub fn check_write_permission(project_root: &Path) -> Result<(), ValidationError> {
    let probe = project_root.join(".gwt-migration-probe");
    match fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&probe)
    {
        Ok(_) => {
            // Best-effort cleanup; ignore unlink errors so a transient probe is
            // never the reason a migration fails.
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(
            ValidationError::WritePermissionDenied(project_root.to_path_buf()),
        ),
        Err(e) => Err(ValidationError::Io(e)),
    }
}

/// Run all pre-flight checks against `project_root`. Short-circuits on the
/// first failure.
pub fn validate(project_root: &Path) -> Result<(), ValidationError> {
    check_write_permission(project_root)?;
    check_disk_space(project_root)?;
    check_locked_worktrees(project_root)?;
    Ok(())
}
