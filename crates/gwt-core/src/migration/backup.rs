//! Migration backup and restore (SPEC-a70a1ece T804-T805, FR-202)

use super::MigrationError;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

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
    // Use cp -a to preserve permissions, timestamps, and symlinks
    let output = crate::process::command("cp")
        .args(["-a", "--"])
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| MigrationError::BackupFailed {
            reason: format!("Failed to copy {}: {}", src.display(), e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MigrationError::BackupFailed {
            reason: format!("cp failed: {}", stderr),
        });
    }

    Ok(())
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
}
