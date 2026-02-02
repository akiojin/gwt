//! Migration rollback (SPEC-a70a1ece T813-T814)

use super::{backup::restore_backup, config::MigrationConfig, error::MigrationError};
use tracing::{debug, info, warn};

/// Rollback migration on failure (SPEC-a70a1ece T813, FR-210)
pub fn rollback_migration(config: &MigrationConfig) -> Result<(), MigrationError> {
    info!("Rolling back migration...");

    // Remove partially created bare repository
    let bare_path = config.bare_repo_path();
    if bare_path.exists() {
        debug!(bare = %bare_path.display(), "Removing bare repository");
        if let Err(e) = std::fs::remove_dir_all(&bare_path) {
            warn!("Failed to remove bare repository: {}", e);
        }
    }

    // Remove partially created worktrees
    cleanup_migrated_worktrees(config)?;

    // Restore backup
    let backup_path = config.backup_path();
    if backup_path.exists() {
        restore_backup(&backup_path, &config.source_root)?;

        // Remove backup directory after successful restore
        if let Err(e) = std::fs::remove_dir_all(&backup_path) {
            warn!("Failed to remove backup directory: {}", e);
        }
    }

    info!("Rollback completed");
    Ok(())
}

/// Cleanup worktrees that were created during migration
fn cleanup_migrated_worktrees(config: &MigrationConfig) -> Result<(), MigrationError> {
    // List all directories in target root that are worktrees
    if !config.target_root.exists() {
        return Ok(());
    }

    let entries = match std::fs::read_dir(&config.target_root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Skip the bare repo
        if path == config.bare_repo_path() {
            continue;
        }

        // Check if this is a git worktree
        let git_dir = path.join(".git");
        if git_dir.exists() {
            // Read .git file to check if it's a worktree
            if let Ok(content) = std::fs::read_to_string(&git_dir) {
                if content.starts_with("gitdir:") {
                    debug!(path = %path.display(), "Removing migrated worktree");
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        warn!("Failed to remove worktree {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Retry a network operation with exponential backoff (SPEC-a70a1ece T814, FR-226)
#[allow(dead_code)]
pub fn retry_with_backoff<T, F>(mut operation: F, max_attempts: u32) -> Result<T, MigrationError>
where
    F: FnMut() -> Result<T, MigrationError>,
{
    let mut attempt = 1;
    loop {
        match operation() {
            Ok(result) => return Ok(result),
            Err(e) if e.is_retryable() && attempt < max_attempts => {
                let backoff_ms = (2u64.pow(attempt - 1)) * 1000;
                warn!(
                    "Attempt {}/{} failed, retrying in {}ms: {}",
                    attempt, max_attempts, backoff_ms, e
                );
                std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
                attempt += 1;
            }
            Err(e) => {
                return Err(MigrationError::NetworkError {
                    reason: e.to_string(),
                    attempt,
                    max_attempts,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_retry_with_backoff_success() {
        let mut count = 0;
        let result = retry_with_backoff(
            || {
                count += 1;
                Ok::<_, MigrationError>(count)
            },
            3,
        );
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_retry_with_backoff_eventual_success() {
        let mut count = 0;
        let result = retry_with_backoff(
            || {
                count += 1;
                if count < 2 {
                    Err(MigrationError::NetworkError {
                        reason: "test".to_string(),
                        attempt: count,
                        max_attempts: 3,
                    })
                } else {
                    Ok(count)
                }
            },
            3,
        );
        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn test_cleanup_empty_target() {
        let temp = TempDir::new().unwrap();
        let config = MigrationConfig::new(
            temp.path().to_path_buf(),
            temp.path().join("target"),
            "repo.git".to_string(),
        );

        // Should not fail on non-existent target
        let result = cleanup_migrated_worktrees(&config);
        assert!(result.is_ok());
    }
}
