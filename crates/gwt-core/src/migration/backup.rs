//! Migration backup and restore (SPEC-a70a1ece T804-T805, FR-202)

use super::MigrationError;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Information about a created backup
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// Backup directory path
    pub path: PathBuf,
    /// Timestamp of backup creation
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Create a backup of the current repository state (SPEC-a70a1ece T804, FR-202)
pub fn create_backup(source: &Path, backup_dir: &Path) -> Result<BackupInfo, MigrationError> {
    debug!(
        source = %source.display(),
        backup = %backup_dir.display(),
        "Creating migration backup"
    );

    // Create backup directory
    std::fs::create_dir_all(backup_dir).map_err(|e| MigrationError::BackupFailed {
        reason: format!("Failed to create backup directory: {}", e),
    })?;

    // Copy .git directory
    let git_source = source.join(".git");
    let git_backup = backup_dir.join(".git");
    if git_source.exists() {
        copy_dir_recursive(&git_source, &git_backup)?;
    }

    // Copy .worktrees directory
    let worktrees_source = source.join(".worktrees");
    let worktrees_backup = backup_dir.join(".worktrees");
    if worktrees_source.exists() {
        copy_dir_recursive(&worktrees_source, &worktrees_backup)?;
    }

    // Copy .gwt directory if exists
    let gwt_source = source.join(".gwt");
    let gwt_backup = backup_dir.join(".gwt");
    if gwt_source.exists() {
        copy_dir_recursive(&gwt_source, &gwt_backup)?;
    }

    // Save backup metadata
    let info = BackupInfo {
        path: backup_dir.to_path_buf(),
        created_at: chrono::Utc::now(),
    };

    let metadata_path = backup_dir.join("backup-info.json");
    let metadata = serde_json::json!({
        "source": source.to_string_lossy(),
        "created_at": info.created_at.to_rfc3339(),
    });
    std::fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
    .map_err(|e| MigrationError::BackupFailed {
        reason: format!("Failed to write backup metadata: {}", e),
    })?;

    info!(
        backup = %backup_dir.display(),
        "Backup created successfully"
    );

    Ok(info)
}

/// Restore repository state from backup (SPEC-a70a1ece T805, FR-210)
pub fn restore_backup(backup_dir: &Path, target: &Path) -> Result<(), MigrationError> {
    debug!(
        backup = %backup_dir.display(),
        target = %target.display(),
        "Restoring from backup"
    );

    // Verify backup exists
    if !backup_dir.exists() {
        return Err(MigrationError::RestoreFailed {
            reason: "Backup directory does not exist".to_string(),
        });
    }

    // Restore .git directory
    let git_backup = backup_dir.join(".git");
    let git_target = target.join(".git");
    if git_backup.exists() {
        if git_target.exists() {
            std::fs::remove_dir_all(&git_target).map_err(|e| MigrationError::RestoreFailed {
                reason: format!("Failed to remove existing .git: {}", e),
            })?;
        }
        copy_dir_recursive(&git_backup, &git_target)?;
    }

    // Restore .worktrees directory
    let worktrees_backup = backup_dir.join(".worktrees");
    let worktrees_target = target.join(".worktrees");
    if worktrees_backup.exists() {
        if worktrees_target.exists() {
            std::fs::remove_dir_all(&worktrees_target).map_err(|e| {
                MigrationError::RestoreFailed {
                    reason: format!("Failed to remove existing .worktrees: {}", e),
                }
            })?;
        }
        copy_dir_recursive(&worktrees_backup, &worktrees_target)?;
    }

    // Restore .gwt directory
    let gwt_backup = backup_dir.join(".gwt");
    let gwt_target = target.join(".gwt");
    if gwt_backup.exists() {
        if gwt_target.exists() {
            std::fs::remove_dir_all(&gwt_target).map_err(|e| MigrationError::RestoreFailed {
                reason: format!("Failed to remove existing .gwt: {}", e),
            })?;
        }
        copy_dir_recursive(&gwt_backup, &gwt_target)?;
    }

    info!(
        target = %target.display(),
        "Backup restored successfully"
    );

    Ok(())
}

/// Copy directory recursively, preserving permissions (FR-214)
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), MigrationError> {
    #[cfg(not(windows))]
    {
        copy_dir_recursive_with_cp_program(src, dst, "cp")
    }

    #[cfg(windows)]
    {
        copy_dir_recursive_native(src, dst)
    }
}

#[cfg(not(windows))]
fn copy_dir_recursive_with_cp_program(
    src: &Path,
    dst: &Path,
    cp_program: &str,
) -> Result<(), MigrationError> {
    match copy_dir_with_program(src, dst, cp_program) {
        Ok(()) => Ok(()),
        Err(reason) => {
            warn!(
                source = %src.display(),
                target = %dst.display(),
                cp_program = cp_program,
                %reason,
                "cp -a failed or unavailable, falling back to native recursive copy"
            );
            copy_dir_recursive_native(src, dst)
        }
    }
}

#[cfg(not(windows))]
fn copy_dir_with_program(src: &Path, dst: &Path, program: &str) -> Result<(), String> {
    let output = crate::process::command(program)
        .args(["-a", "--"])
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("Failed to spawn {} for {}: {}", program, src.display(), e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("cp failed: {}", stderr.trim()))
}

fn copy_dir_recursive_native(src: &Path, dst: &Path) -> Result<(), MigrationError> {
    std::fs::create_dir_all(dst).map_err(|e| MigrationError::BackupFailed {
        reason: format!("Failed to create directory {}: {}", dst.display(), e),
    })?;

    copy_permissions(src, dst);

    for entry in std::fs::read_dir(src).map_err(|e| MigrationError::BackupFailed {
        reason: format!("Failed to read directory {}: {}", src.display(), e),
    })? {
        let entry = entry.map_err(|e| MigrationError::BackupFailed {
            reason: format!("Failed to read directory entry in {}: {}", src.display(), e),
        })?;

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| MigrationError::BackupFailed {
                reason: format!("Failed to read file type for {}: {}", src_path.display(), e),
            })?;

        if file_type.is_dir() {
            copy_dir_recursive_native(&src_path, &dst_path)?;
            continue;
        }

        if file_type.is_file() {
            copy_file_with_permissions(&src_path, &dst_path)?;
            continue;
        }

        if file_type.is_symlink() {
            // Fallback path: dereference symlink target to keep backup portable on Windows.
            let meta = std::fs::metadata(&src_path).map_err(|e| MigrationError::BackupFailed {
                reason: format!(
                    "Failed to read symlink target metadata for {}: {}",
                    src_path.display(),
                    e
                ),
            })?;

            if meta.is_dir() {
                copy_dir_recursive_native(&src_path, &dst_path)?;
            } else {
                copy_file_with_permissions(&src_path, &dst_path)?;
            }

            continue;
        }
    }

    Ok(())
}

fn copy_file_with_permissions(src: &Path, dst: &Path) -> Result<(), MigrationError> {
    std::fs::copy(src, dst).map_err(|e| MigrationError::BackupFailed {
        reason: format!("Failed to copy {}: {}", src.display(), e),
    })?;

    copy_permissions(src, dst);
    Ok(())
}

fn copy_permissions(src: &Path, dst: &Path) {
    if let Ok(meta) = std::fs::metadata(src) {
        let _ = std::fs::set_permissions(dst, meta.permissions());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_and_restore_backup() {
        let source = TempDir::new().unwrap();
        let backup = TempDir::new().unwrap();

        // Create some test files
        std::fs::create_dir_all(source.path().join(".git")).unwrap();
        std::fs::write(source.path().join(".git/config"), "test config").unwrap();

        // Create backup
        let backup_path = backup.path().join("backup");
        let info = create_backup(source.path(), &backup_path).unwrap();
        assert!(info.path.exists());

        // Verify backup contents
        assert!(backup_path.join(".git/config").exists());

        // Modify source
        std::fs::write(source.path().join(".git/config"), "modified").unwrap();

        // Restore backup
        restore_backup(&backup_path, source.path()).unwrap();

        // Verify restoration
        let content = std::fs::read_to_string(source.path().join(".git/config")).unwrap();
        assert_eq!(content, "test config");
    }

    #[test]
    #[cfg(not(windows))]
    fn test_copy_dir_recursive_falls_back_when_cp_program_is_missing() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");

        std::fs::create_dir_all(source.join(".git")).unwrap();
        std::fs::write(source.join(".git/config"), "test config").unwrap();

        let result =
            copy_dir_recursive_with_cp_program(&source, &target, "definitely-missing-cp-binary");
        assert!(
            result.is_ok(),
            "copy_dir_recursive should fall back to native copy when cp is missing: {:?}",
            result
        );
        assert!(target.join(".git/config").exists());
    }
}
