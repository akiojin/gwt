//! Pre-flight checks before a migration starts (Phase 3 of the build plan).
//!
//! - disk space (project size × 2 must be available)
//! - locked worktrees
//! - write permission on the project root
//!
//! Implementations are filled in by Phase 3 tasks; only signatures live here
//! to let dependent modules compile.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ValidationError {
    InsufficientDiskSpace { required: u64, available: u64 },
    LockedWorktrees(Vec<PathBuf>),
    WritePermissionDenied(PathBuf),
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
        }
    }
}

impl std::error::Error for ValidationError {}

pub fn validate(_project_root: &Path) -> Result<(), ValidationError> {
    // Filled in by Phase 3 tasks (T-026).
    Ok(())
}
