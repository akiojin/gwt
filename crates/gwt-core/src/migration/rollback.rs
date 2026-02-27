//! Migration rollback (SPEC-a70a1ece T813-T814)

use super::{backup::restore_backup, config::MigrationConfig, error::MigrationError};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use tracing::{debug, info, warn};

const EVACUATION_MANIFEST_FILENAME: &str = "evacuation-manifest.json";
const EVACUATION_MANIFEST_ENCODING: &str = "base64-os";

#[derive(Debug, Deserialize)]
struct EvacuationManifest {
    entries: Vec<String>,
    #[serde(default)]
    encoding: Option<String>,
}

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

    // Recover files evacuated from dirty main repo before migration failed
    recover_evacuated_main_repo_files(config)?;

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

fn recover_evacuated_main_repo_files(config: &MigrationConfig) -> Result<(), MigrationError> {
    let temp_dir = config.evacuation_temp_path();
    if !temp_dir.exists() {
        return Ok(());
    }

    debug!(
        temp = %temp_dir.display(),
        source = %config.source_root.display(),
        "Recovering evacuated main repo files"
    );

    for entry_name in collect_evacuation_entries(&temp_dir)? {
        let src_path = temp_dir.join(&entry_name);
        if !src_path.exists() {
            continue;
        }
        let dst_path = config.source_root.join(&entry_name);
        std::fs::rename(&src_path, &dst_path).map_err(|e| MigrationError::IoError {
            path: src_path.clone(),
            reason: format!(
                "Failed to recover evacuated entry from {} to {}: {}",
                src_path.display(),
                dst_path.display(),
                e
            ),
        })?;
    }

    if let Err(e) = std::fs::remove_dir_all(&temp_dir) {
        warn!(
            temp = %temp_dir.display(),
            error = %e,
            "Failed to remove evacuation temp directory after recovery"
        );
    }

    Ok(())
}

fn collect_evacuation_entries(temp_dir: &std::path::Path) -> Result<Vec<OsString>, MigrationError> {
    let manifest_path = temp_dir.join(EVACUATION_MANIFEST_FILENAME);
    if manifest_path.exists() {
        match std::fs::read_to_string(&manifest_path) {
            Ok(content) => match serde_json::from_str::<EvacuationManifest>(&content) {
                Ok(manifest) => {
                    if manifest.encoding.as_deref() == Some(EVACUATION_MANIFEST_ENCODING) {
                        let mut decoded = Vec::with_capacity(manifest.entries.len());
                        for encoded_name in manifest.entries {
                            match decode_entry_name(&encoded_name) {
                                Ok(name) => decoded.push(name),
                                Err(err) => {
                                    warn!(
                                        path = %manifest_path.display(),
                                        entry = %encoded_name,
                                        error = %err,
                                        "Failed to decode evacuation manifest during rollback, falling back to directory scan"
                                    );
                                    return collect_evacuation_entries_from_scan(temp_dir);
                                }
                            }
                        }
                        return Ok(decoded);
                    }

                    warn!(
                        path = %manifest_path.display(),
                        encoding = ?manifest.encoding,
                        "Unsupported or missing evacuation manifest encoding during rollback, falling back to directory scan"
                    );
                    return collect_evacuation_entries_from_scan(temp_dir);
                }
                Err(err) => warn!(
                    path = %manifest_path.display(),
                    error = %err,
                    "Failed to parse evacuation manifest during rollback, falling back to directory scan"
                ),
            },
            Err(err) => warn!(
                path = %manifest_path.display(),
                error = %err,
                "Failed to read evacuation manifest during rollback, falling back to directory scan"
            ),
        }
    }

    collect_evacuation_entries_from_scan(temp_dir)
}

fn collect_evacuation_entries_from_scan(
    temp_dir: &std::path::Path,
) -> Result<Vec<OsString>, MigrationError> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(temp_dir).map_err(|e| MigrationError::IoError {
        path: temp_dir.to_path_buf(),
        reason: format!("Failed to read evacuation temp directory: {}", e),
    })? {
        let entry = entry.map_err(|e| MigrationError::IoError {
            path: temp_dir.to_path_buf(),
            reason: format!("Failed to read evacuation temp entry: {}", e),
        })?;
        let name = entry.file_name();
        if name == OsStr::new(EVACUATION_MANIFEST_FILENAME) {
            continue;
        }
        entries.push(name);
    }

    Ok(entries)
}

fn decode_entry_name(encoded: &str) -> Result<OsString, String> {
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|e| format!("Invalid base64 entry name: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes))
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if bytes.len() % 2 != 0 {
            return Err("Invalid UTF-16 byte length".to_string());
        }
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        Ok(OsString::from_wide(&units))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(OsString::from(String::from_utf8_lossy(&bytes).to_string()))
    }
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

    #[test]
    fn test_recover_evacuated_main_repo_files() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        let config = MigrationConfig::new(source.clone(), source.clone(), "repo.git".to_string());

        let temp_dir = config.evacuation_temp_path();
        std::fs::create_dir_all(temp_dir.join(".svn/pristine")).unwrap();
        std::fs::write(temp_dir.join(".svn/pristine/a.svn-base"), "svn").unwrap();
        std::fs::write(temp_dir.join("notes.txt"), "note").unwrap();
        let manifest = serde_json::json!({
            "entries": [".svn", "notes.txt"]
        });
        std::fs::write(
            temp_dir.join(EVACUATION_MANIFEST_FILENAME),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        recover_evacuated_main_repo_files(&config).unwrap();

        assert!(source.join(".svn/pristine/a.svn-base").exists());
        assert!(source.join("notes.txt").exists());
        assert!(!temp_dir.exists());
    }

    #[test]
    fn test_rollback_migration_recovers_evacuated_files_without_backup() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        let config = MigrationConfig::new(source.clone(), source.clone(), "repo.git".to_string());

        let temp_dir = config.evacuation_temp_path();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("notes.txt"), "note").unwrap();

        rollback_migration(&config).unwrap();

        assert!(source.join("notes.txt").exists());
        assert!(!temp_dir.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_collect_evacuation_entries_decodes_non_utf8_manifest() {
        use std::os::unix::ffi::OsStringExt;

        let temp = TempDir::new().unwrap();
        let temp_dir = temp.path().join(".gwt-migration-temp");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let name = OsString::from_vec(vec![0x66, 0x6f, 0x80]);
        std::fs::write(temp_dir.join(&name), "x").unwrap();
        let encoded = BASE64_STANDARD.encode(vec![0x66, 0x6f, 0x80]);
        let manifest = serde_json::json!({
            "entries": [encoded],
            "encoding": EVACUATION_MANIFEST_ENCODING,
        });
        std::fs::write(
            temp_dir.join(EVACUATION_MANIFEST_FILENAME),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let entries = collect_evacuation_entries(&temp_dir).unwrap();
        assert_eq!(entries, vec![name]);
    }
}
