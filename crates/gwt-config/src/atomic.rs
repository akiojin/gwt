//! Atomic file write utilities.

use std::path::Path;

use tracing::warn;

use crate::error::{ConfigError, Result};

/// Write content to a file atomically via temp file + rename.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Use PID + timestamp to avoid collisions when multiple processes
    // write to different targets in the same directory concurrently.
    let suffix = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let temp_name = format!(
        ".{}.tmp.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        suffix
    );
    let temp_path = path.with_file_name(temp_name);

    std::fs::write(&temp_path, content).map_err(|e| ConfigError::WriteError {
        reason: format!("failed to write temp file: {e}"),
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&temp_path, perms) {
            warn!(
                path = %temp_path.display(),
                error = %e,
                "Failed to set temp file permissions"
            );
        }
    }

    std::fs::rename(&temp_path, path).map_err(|e| ConfigError::WriteError {
        reason: format!("failed to rename temp file: {e}"),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        write_atomic(&path, "key = \"value\"").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "key = \"value\"");
    }

    #[test]
    fn write_atomic_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("config.toml");
        write_atomic(&path, "data").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "data");
    }

    #[test]
    fn write_atomic_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overwrite.toml");
        write_atomic(&path, "first").unwrap();
        write_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perms.toml");
        write_atomic(&path, "secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
