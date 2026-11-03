//! Atomic file write utilities.

use std::{io::Write, path::Path};

#[cfg(unix)]
use tracing::warn;

use crate::error::{ConfigError, Result};

/// Write content to a file atomically via temp file + rename.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    crate::home::assert_test_path(path);
    if let Some(parent) = path.parent() {
        create_dir_all_durable(parent)?;
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

    // Never follow a preexisting sibling symlink, even in temporary storage.
    let mut temp_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|e| ConfigError::WriteError {
            reason: format!("failed to create temp file: {e}"),
        })?;
    temp_file
        .write_all(content.as_bytes())
        .map_err(|e| ConfigError::WriteError {
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

    temp_file.sync_all().map_err(|e| ConfigError::WriteError {
        reason: format!("failed to sync temp file before rename: {e}"),
    })?;
    drop(temp_file);

    std::fs::rename(&temp_path, path).map_err(|e| ConfigError::WriteError {
        reason: format!("failed to rename temp file: {e}"),
    })?;

    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        sync_directory(parent).map_err(|e| ConfigError::WriteError {
            reason: format!("file renamed but parent sync failed; inspect current settings before retrying: {e}"),
        })?;
    }

    Ok(())
}

fn create_dir_all_durable(path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut existing = path;
    while !existing.as_os_str().is_empty() && !existing.is_dir() {
        missing.push(existing.to_path_buf());
        existing = existing.parent().unwrap_or_else(|| Path::new(""));
    }
    // A failed previous attempt may have created this directory before its
    // link was synced. Recheck just this boundary before creating descendants.
    let existing = if existing.as_os_str().is_empty() {
        Path::new(".")
    } else {
        existing
    };
    sync_directory_and_parent(existing)?;
    for directory in missing.iter().rev() {
        match std::fs::create_dir(directory) {
            Ok(()) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(error) => return Err(error.into()),
        }
        sync_directory_and_parent(directory)?;
    }
    Ok(())
}

fn sync_directory_and_parent(path: &Path) -> Result<()> {
    sync_directory(path)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        sync_directory(parent)?;
    }
    Ok(())
}

fn sync_directory(_path: &Path) -> Result<()> {
    #[cfg(test)]
    if tests::directory_sync_fails(_path) {
        return Err(std::io::Error::other("injected directory sync failure").into());
    }
    #[cfg(unix)]
    std::fs::File::open(_path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static FAIL_DIRECTORY_SYNC: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
    }

    struct DirectorySyncFailure(Option<std::path::PathBuf>);

    impl DirectorySyncFailure {
        fn at(path: &Path) -> Self {
            Self(FAIL_DIRECTORY_SYNC.with(|fault| fault.replace(Some(path.to_path_buf()))))
        }
    }

    impl Drop for DirectorySyncFailure {
        fn drop(&mut self) {
            FAIL_DIRECTORY_SYNC.with(|fault| fault.replace(self.0.take()));
        }
    }

    pub(super) fn directory_sync_fails(path: &Path) -> bool {
        FAIL_DIRECTORY_SYNC.with(|fault| fault.borrow().as_deref() == Some(path))
    }

    #[test]
    fn newly_created_ancestor_sync_failure_prevents_publication_and_retries() {
        let directory = tempfile::tempdir().unwrap();
        let ancestor = directory.path().join("project-state");
        let path = ancestor.join("release-policy").join("request.json");
        let failure = DirectorySyncFailure::at(&ancestor);
        assert!(
            write_atomic(&path, "pending").is_err(),
            "ancestor sync failure must be reported before publication"
        );
        assert!(
            !path.exists(),
            "an unsynced directory chain cannot contain a published receipt"
        );
        assert!(
            write_atomic(&path, "pending").is_err(),
            "retry must recheck the failed directory link"
        );
        assert!(!path.exists());
        drop(failure);
        write_atomic(&path, "pending").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "pending");
    }

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
