//! Atomic file write utilities.

use std::path::Path;

use tracing::warn;

use crate::error::{ConfigError, Result};

/// Write content to a file atomically via temp file + rename.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temp_path = path.with_extension("tmp");

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
