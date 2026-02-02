//! Configuration format migration utilities (SPEC-a3f4c9df)
//!
//! Provides utilities for migrating configuration files between formats:
//! - JSON → TOML
//! - YAML → TOML
//!
//! Migration strategy:
//! 1. New format (TOML) is always preferred for reading if it exists
//! 2. Old format is read as fallback
//! 3. Writes always use new format (TOML)
//! 4. Old files are never auto-deleted (use `gwt config cleanup`)

use crate::error::{GwtError, Result};
use std::path::Path;
use tracing::{debug, error, info, warn};

/// Migrate JSON configuration to TOML
pub fn migrate_json_to_toml(json_path: &Path, toml_path: &Path) -> Result<()> {
    debug!(
        category = "config",
        json_path = %json_path.display(),
        toml_path = %toml_path.display(),
        "Starting JSON to TOML migration"
    );

    // Read JSON file
    let json_content = std::fs::read_to_string(json_path)?;

    // Parse JSON
    let json_value: serde_json::Value = serde_json::from_str(&json_content).map_err(|e| {
        error!(
            category = "config",
            json_path = %json_path.display(),
            error = %e,
            "Failed to parse JSON config"
        );
        GwtError::MigrationFailed {
            reason: format!("Failed to parse JSON: {}", e),
        }
    })?;

    // Convert to TOML
    let toml_content = toml::to_string_pretty(&json_value).map_err(|e| {
        error!(
            category = "config",
            error = %e,
            "Failed to convert JSON to TOML"
        );
        GwtError::MigrationFailed {
            reason: format!("Failed to convert to TOML: {}", e),
        }
    })?;

    // Write TOML file
    std::fs::write(toml_path, &toml_content)?;

    info!(
        category = "config",
        operation = "migration",
        json_path = %json_path.display(),
        toml_path = %toml_path.display(),
        "Migration completed successfully"
    );

    Ok(())
}

/// Migrate YAML configuration to TOML (SPEC-a3f4c9df FR-001)
pub fn migrate_yaml_to_toml(yaml_path: &Path, toml_path: &Path) -> Result<()> {
    debug!(
        category = "config",
        yaml_path = %yaml_path.display(),
        toml_path = %toml_path.display(),
        "Starting YAML to TOML migration"
    );

    // Read YAML file
    let yaml_content = std::fs::read_to_string(yaml_path)?;

    // Parse YAML
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_content).map_err(|e| {
        error!(
            category = "config",
            yaml_path = %yaml_path.display(),
            error = %e,
            "Failed to parse YAML config"
        );
        GwtError::MigrationFailed {
            reason: format!("Failed to parse YAML: {}", e),
        }
    })?;

    // Convert YAML value to JSON value first (for TOML compatibility)
    let json_str = serde_json::to_string(&yaml_value).map_err(|e| GwtError::MigrationFailed {
        reason: format!("Failed to convert YAML to JSON: {}", e),
    })?;

    let json_value: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| GwtError::MigrationFailed {
            reason: format!("Failed to parse intermediate JSON: {}", e),
        })?;

    // Convert to TOML
    let toml_content = toml::to_string_pretty(&json_value).map_err(|e| {
        error!(
            category = "config",
            error = %e,
            "Failed to convert to TOML"
        );
        GwtError::MigrationFailed {
            reason: format!("Failed to convert to TOML: {}", e),
        }
    })?;

    // Write TOML file atomically
    write_atomic(toml_path, &toml_content)?;

    info!(
        category = "config",
        operation = "migration",
        yaml_path = %yaml_path.display(),
        toml_path = %toml_path.display(),
        "YAML to TOML migration completed successfully"
    );

    Ok(())
}

/// Write file atomically using temp file + rename pattern (SPEC-a3f4c9df FR-008)
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Create temp file path
    let temp_path = path.with_extension("tmp");

    // Write to temp file
    std::fs::write(&temp_path, content).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write temp file: {}", e),
    })?;

    // Set private permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&temp_path, perms) {
            warn!(
                category = "config",
                path = %temp_path.display(),
                error = %e,
                "Failed to set file permissions"
            );
        }
    }

    // Atomic rename
    std::fs::rename(&temp_path, path).map_err(|e| {
        // Clean up temp file on failure
        let _ = std::fs::remove_file(&temp_path);
        GwtError::ConfigWriteError {
            reason: format!("Failed to rename temp file: {}", e),
        }
    })?;

    debug!(
        category = "config",
        path = %path.display(),
        "File written atomically"
    );

    Ok(())
}

/// Backup a broken config file (SPEC-a3f4c9df FR-009)
pub fn backup_broken_file(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let broken_path = path.with_extension("broken");
    std::fs::rename(path, &broken_path).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to backup broken file: {}", e),
    })?;

    warn!(
        category = "config",
        original = %path.display(),
        backup = %broken_path.display(),
        "Broken config file backed up"
    );

    Ok(())
}

/// Ensure directory exists with proper permissions (SPEC-a3f4c9df FR-010)
pub fn ensure_config_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            if let Err(e) = std::fs::set_permissions(dir, perms) {
                warn!(
                    category = "config",
                    path = %dir.display(),
                    error = %e,
                    "Failed to set directory permissions"
                );
            }
        }

        debug!(
            category = "config",
            path = %dir.display(),
            "Created config directory"
        );
    }

    Ok(())
}

/// Get list of old files that can be cleaned up (SPEC-a3f4c9df FR-011)
pub fn get_cleanup_candidates() -> Vec<CleanupCandidate> {
    let mut candidates = Vec::new();
    let home = dirs::home_dir();

    if let Some(home) = &home {
        let gwt_dir = home.join(".gwt");

        // profiles.yaml -> profiles.toml
        let yaml_path = gwt_dir.join("profiles.yaml");
        let toml_path = gwt_dir.join("profiles.toml");
        if yaml_path.exists() && toml_path.exists() {
            candidates.push(CleanupCandidate {
                old_path: yaml_path,
                new_path: toml_path,
                format_change: "YAML → TOML".to_string(),
            });
        }

        // tools.json -> tools.toml
        let json_path = gwt_dir.join("tools.json");
        let toml_path = gwt_dir.join("tools.toml");
        if json_path.exists() && toml_path.exists() {
            candidates.push(CleanupCandidate {
                old_path: json_path,
                new_path: toml_path,
                format_change: "JSON → TOML".to_string(),
            });
        }

        // agent-history.json -> agent-history.toml
        let json_path = gwt_dir.join("agent-history.json");
        let toml_path = gwt_dir.join("agent-history.toml");
        if json_path.exists() && toml_path.exists() {
            candidates.push(CleanupCandidate {
                old_path: json_path,
                new_path: toml_path,
                format_change: "JSON → TOML".to_string(),
            });
        }

        // ~/.config/gwt/ -> ~/.gwt/
        let old_config_dir = home.join(".config").join("gwt");
        if old_config_dir.exists() {
            let old_config = old_config_dir.join("config.toml");
            let new_config = gwt_dir.join("config.toml");
            if old_config.exists() && new_config.exists() {
                candidates.push(CleanupCandidate {
                    old_path: old_config,
                    new_path: new_config,
                    format_change: "path change".to_string(),
                });
            }
        }
    }

    candidates
}

/// Cleanup candidate representing an old file that can be removed
#[derive(Debug, Clone)]
pub struct CleanupCandidate {
    /// Path to the old file
    pub old_path: std::path::PathBuf,
    /// Path to the new file (for verification)
    pub new_path: std::path::PathBuf,
    /// Description of the format change
    pub format_change: String,
}

impl CleanupCandidate {
    /// Remove the old file
    pub fn cleanup(&self) -> Result<()> {
        if !self.new_path.exists() {
            return Err(GwtError::MigrationFailed {
                reason: format!(
                    "Cannot cleanup {} because {} does not exist",
                    self.old_path.display(),
                    self.new_path.display()
                ),
            });
        }

        std::fs::remove_file(&self.old_path)?;
        info!(
            category = "config",
            operation = "cleanup",
            old_path = %self.old_path.display(),
            "Removed old config file"
        );

        Ok(())
    }
}

/// Check if migration is needed
pub fn needs_migration(repo_root: &Path) -> bool {
    let json_path = repo_root.join(".gwt.json");
    let toml_path = repo_root.join(".gwt.toml");

    let needs = json_path.exists() && !toml_path.exists();
    debug!(
        category = "config",
        repo_root = %repo_root.display(),
        json_exists = json_path.exists(),
        toml_exists = toml_path.exists(),
        needs_migration = needs,
        "Checked migration status"
    );
    needs
}

/// Auto-migrate if needed
pub fn auto_migrate(repo_root: &Path) -> Result<bool> {
    if needs_migration(repo_root) {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Auto-migration triggered"
        );
        let json_path = repo_root.join(".gwt.json");
        let toml_path = repo_root.join(".gwt.toml");
        migrate_json_to_toml(&json_path, &toml_path)?;
        info!(
            category = "config",
            operation = "auto_migrate",
            repo_root = %repo_root.display(),
            "Auto-migration completed"
        );
        Ok(true)
    } else {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Auto-migration skipped (not needed)"
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_migrate_json_to_toml() {
        let temp = TempDir::new().unwrap();
        let json_path = temp.path().join("config.json");
        let toml_path = temp.path().join("config.toml");

        std::fs::write(
            &json_path,
            r#"{"protected_branches": ["main"], "debug": true}"#,
        )
        .unwrap();

        migrate_json_to_toml(&json_path, &toml_path).unwrap();

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("protected_branches"));
        assert!(content.contains("debug = true"));
    }

    #[test]
    fn test_migrate_yaml_to_toml() {
        let temp = TempDir::new().unwrap();
        let yaml_path = temp.path().join("config.yaml");
        let toml_path = temp.path().join("config.toml");

        std::fs::write(
            &yaml_path,
            r#"
version: 1
active: default
profiles:
  default:
    name: default
    env:
      FOO: bar
"#,
        )
        .unwrap();

        migrate_yaml_to_toml(&yaml_path, &toml_path).unwrap();

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("version = 1"));
        assert!(content.contains("active = \"default\""));
    }

    #[test]
    fn test_write_atomic() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.toml");

        write_atomic(&path, "key = \"value\"").unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "key = \"value\"");

        // Temp file should not exist
        assert!(!temp.path().join("test.tmp").exists());
    }

    #[test]
    fn test_backup_broken_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("broken.toml");
        let broken_path = temp.path().join("broken.broken");

        std::fs::write(&path, "invalid content").unwrap();

        backup_broken_file(&path).unwrap();

        assert!(!path.exists());
        assert!(broken_path.exists());
    }

    #[test]
    fn test_ensure_config_dir() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("config").join("nested");

        ensure_config_dir(&dir).unwrap();

        assert!(dir.exists());
        assert!(dir.is_dir());
    }

    #[test]
    fn test_needs_migration() {
        let temp = TempDir::new().unwrap();

        // No files - no migration needed
        assert!(!needs_migration(temp.path()));

        // JSON only - migration needed
        std::fs::write(temp.path().join(".gwt.json"), "{}").unwrap();
        assert!(needs_migration(temp.path()));

        // Both files - no migration needed
        std::fs::write(temp.path().join(".gwt.toml"), "").unwrap();
        assert!(!needs_migration(temp.path()));
    }

    #[test]
    fn test_cleanup_candidate() {
        let temp = TempDir::new().unwrap();
        let old_path = temp.path().join("old.yaml");
        let new_path = temp.path().join("new.toml");

        std::fs::write(&old_path, "old content").unwrap();
        std::fs::write(&new_path, "new content").unwrap();

        let candidate = CleanupCandidate {
            old_path: old_path.clone(),
            new_path: new_path.clone(),
            format_change: "YAML → TOML".to_string(),
        };

        candidate.cleanup().unwrap();

        assert!(!old_path.exists());
        assert!(new_path.exists());
    }

    #[test]
    fn test_cleanup_candidate_fails_without_new_file() {
        let temp = TempDir::new().unwrap();
        let old_path = temp.path().join("old.yaml");
        let new_path = temp.path().join("new.toml");

        std::fs::write(&old_path, "old content").unwrap();
        // new_path does not exist

        let candidate = CleanupCandidate {
            old_path: old_path.clone(),
            new_path,
            format_change: "YAML → TOML".to_string(),
        };

        let result = candidate.cleanup();
        assert!(result.is_err());
        assert!(old_path.exists()); // Old file should not be deleted
    }
}
