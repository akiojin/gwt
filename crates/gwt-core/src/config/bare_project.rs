//! Bare project configuration (SPEC-a70a1ece US5, SPEC-a3f4c9df)
//!
//! Manages configuration for bare repository based projects with automatic
//! migration from JSON to TOML format.
//!
//! File locations:
//! - New format: .gwt/project.toml
//! - Legacy format: .gwt/project.json

use crate::config::migration::{ensure_config_dir, write_atomic};
use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// New TOML configuration file name
const CONFIG_FILE_NAME_TOML: &str = "project.toml";

/// Legacy JSON configuration file name
const CONFIG_FILE_NAME_JSON: &str = "project.json";

/// Configuration directory name
const CONFIG_DIR_NAME: &str = ".gwt";

/// Bare project configuration (SPEC-a70a1ece T501)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BareProjectConfig {
    /// Bare repository name (e.g., "repo.git")
    pub bare_repo_name: String,
    /// Remote URL (for reference)
    pub remote_url: Option<String>,
    /// Worktree location strategy
    pub location: String,
    /// Created timestamp
    pub created_at: String,
}

impl BareProjectConfig {
    /// Create a new bare project configuration
    pub fn new(bare_repo_name: impl Into<String>) -> Self {
        Self {
            bare_repo_name: bare_repo_name.into(),
            remote_url: None,
            location: "sibling".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create with remote URL
    pub fn with_remote(bare_repo_name: impl Into<String>, remote_url: impl Into<String>) -> Self {
        Self {
            bare_repo_name: bare_repo_name.into(),
            remote_url: Some(remote_url.into()),
            location: "sibling".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Get the config directory path for a project root
    /// SPEC-a70a1ece T504: .gwt/ is placed at project root (bare's parent)
    pub fn config_dir(project_root: &Path) -> PathBuf {
        project_root.join(CONFIG_DIR_NAME)
    }

    /// Get the TOML config file path (new format)
    pub fn toml_config_path(project_root: &Path) -> PathBuf {
        Self::config_dir(project_root).join(CONFIG_FILE_NAME_TOML)
    }

    /// Get the JSON config file path (legacy format)
    pub fn json_config_path(project_root: &Path) -> PathBuf {
        Self::config_dir(project_root).join(CONFIG_FILE_NAME_JSON)
    }

    /// Get the config file path for a project root (deprecated)
    #[deprecated(note = "Use toml_config_path() for new code")]
    pub fn config_path(project_root: &Path) -> PathBuf {
        Self::json_config_path(project_root)
    }

    /// Load configuration from a project root with format auto-detection (SPEC-a3f4c9df FR-005)
    ///
    /// Priority: TOML > JSON
    pub fn load(project_root: &Path) -> Result<Option<Self>> {
        // Try TOML first (new format)
        let toml_path = Self::toml_config_path(project_root);
        if toml_path.exists() {
            debug!(
                category = "config",
                path = %toml_path.display(),
                "Loading bare project config from TOML"
            );
            match Self::load_from_toml(&toml_path) {
                Ok(config) => return Ok(Some(config)),
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %toml_path.display(),
                        error = %e,
                        "Failed to load TOML bare project config, trying JSON fallback"
                    );
                }
            }
        }

        // Try JSON fallback (legacy format)
        let json_path = Self::json_config_path(project_root);
        if json_path.exists() {
            debug!(
                category = "config",
                path = %json_path.display(),
                "Loading bare project config from JSON (legacy)"
            );
            match Self::load_from_json(&json_path) {
                Ok(config) => {
                    // Auto-migrate: save as TOML for next time (SPEC-a3f4c9df)
                    if let Err(e) = config.save(project_root) {
                        warn!(
                            category = "config",
                            error = %e,
                            "Failed to auto-migrate project.json to TOML"
                        );
                    } else {
                        info!(
                            category = "config",
                            operation = "auto_migrate",
                            "Auto-migrated project.json to project.toml"
                        );
                    }
                    return Ok(Some(config));
                }
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %json_path.display(),
                        error = %e,
                        "Failed to load JSON bare project config"
                    );
                }
            }
        }

        Ok(None)
    }

    /// Load configuration from TOML file
    fn load_from_toml(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        let config: Self = toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to parse TOML bare project config: {}", e),
        })?;

        Ok(config)
    }

    /// Load configuration from JSON file (legacy)
    fn load_from_json(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        let config: Self =
            serde_json::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to parse JSON bare project config: {}", e),
            })?;

        Ok(config)
    }

    /// Save configuration to a project root in TOML format (SPEC-a3f4c9df FR-006)
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let config_dir = Self::config_dir(project_root);
        ensure_config_dir(&config_dir)?;

        let config_path = Self::toml_config_path(project_root);
        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to serialize bare project config: {}", e),
        })?;

        write_atomic(&config_path, &content)?;

        info!(
            category = "config",
            path = %config_path.display(),
            "Saved bare project config (TOML)"
        );

        Ok(())
    }

    /// Check if migration from JSON to TOML is needed
    pub fn needs_migration(project_root: &Path) -> bool {
        let toml_path = Self::toml_config_path(project_root);
        let json_path = Self::json_config_path(project_root);
        json_path.exists() && !toml_path.exists()
    }

    /// Migrate from JSON to TOML if needed
    pub fn migrate_if_needed(project_root: &Path) -> Result<bool> {
        if !Self::needs_migration(project_root) {
            return Ok(false);
        }

        info!(
            category = "config",
            operation = "migration",
            project_root = %project_root.display(),
            "Migrating bare project config from JSON to TOML"
        );

        if let Some(config) = Self::load(project_root)? {
            config.save(project_root)?;
            info!(
                category = "config",
                operation = "migration",
                "Bare project config migration completed"
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Extract bare repository name from URL (SPEC-a70a1ece T505)
    ///
    /// Examples:
    /// - `https://github.com/user/repo.git` -> `repo.git`
    /// - `git@github.com:user/repo.git` -> `repo.git`
    /// - `https://github.com/user/repo` -> `repo.git`
    pub fn derive_bare_repo_name(url: &str) -> String {
        let url = url.trim_end_matches('/');

        // Extract the last path segment
        let name = url
            .rsplit('/')
            .next()
            .or_else(|| url.rsplit(':').next())
            .unwrap_or("repo");

        // Add .git suffix if not present
        if name.ends_with(".git") {
            name.to_string()
        } else {
            format!("{}.git", name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_config() {
        let config = BareProjectConfig::new("repo.git");
        assert_eq!(config.bare_repo_name, "repo.git");
        assert_eq!(config.location, "sibling");
        assert!(config.remote_url.is_none());
    }

    #[test]
    fn test_with_remote() {
        let config = BareProjectConfig::with_remote("repo.git", "https://github.com/user/repo.git");
        assert_eq!(config.bare_repo_name, "repo.git");
        assert_eq!(
            config.remote_url,
            Some("https://github.com/user/repo.git".to_string())
        );
    }

    #[test]
    fn test_save_and_load_toml() {
        let temp = TempDir::new().unwrap();
        let config = BareProjectConfig::with_remote("test.git", "https://example.com/test.git");

        config.save(temp.path()).unwrap();

        // Should save as TOML
        let toml_path = BareProjectConfig::toml_config_path(temp.path());
        assert!(toml_path.exists());

        // Verify TOML content
        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("bare_repo_name = \"test.git\""));
        assert!(content.contains("remote_url = \"https://example.com/test.git\""));

        let loaded = BareProjectConfig::load(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.bare_repo_name, "test.git");
        assert_eq!(
            loaded.remote_url,
            Some("https://example.com/test.git".to_string())
        );
    }

    #[test]
    fn test_load_json_fallback() {
        let temp = TempDir::new().unwrap();
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create JSON file manually
        let json_path = gwt_dir.join("project.json");
        std::fs::write(
            &json_path,
            r#"{
                "bare_repo_name": "legacy.git",
                "remote_url": "https://example.com/legacy.git",
                "location": "sibling",
                "created_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();

        let loaded = BareProjectConfig::load(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.bare_repo_name, "legacy.git");
    }

    #[test]
    fn test_toml_priority_over_json() {
        let temp = TempDir::new().unwrap();
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create both JSON and TOML
        let json_path = gwt_dir.join("project.json");
        std::fs::write(
            &json_path,
            r#"{
                "bare_repo_name": "json.git",
                "location": "sibling",
                "created_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();

        let toml_path = gwt_dir.join("project.toml");
        std::fs::write(
            &toml_path,
            r#"
bare_repo_name = "toml.git"
location = "sibling"
created_at = "2026-01-01T00:00:00Z"
"#,
        )
        .unwrap();

        // TOML should be loaded
        let loaded = BareProjectConfig::load(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.bare_repo_name, "toml.git");
    }

    #[test]
    fn test_load_missing() {
        let temp = TempDir::new().unwrap();
        let result = BareProjectConfig::load(temp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_needs_migration() {
        let temp = TempDir::new().unwrap();

        // No files - no migration needed
        assert!(!BareProjectConfig::needs_migration(temp.path()));

        // Create JSON only
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();
        std::fs::write(
            gwt_dir.join("project.json"),
            r#"{"bare_repo_name":"test.git","location":"sibling","created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert!(BareProjectConfig::needs_migration(temp.path()));

        // Create TOML - no longer needs migration
        std::fs::write(
            gwt_dir.join("project.toml"),
            "bare_repo_name = \"test.git\"\nlocation = \"sibling\"\ncreated_at = \"2026-01-01T00:00:00Z\"",
        )
        .unwrap();
        assert!(!BareProjectConfig::needs_migration(temp.path()));
    }

    #[test]
    fn test_migrate_if_needed() {
        let temp = TempDir::new().unwrap();
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create JSON file
        std::fs::write(
            gwt_dir.join("project.json"),
            r#"{
                "bare_repo_name": "migrate.git",
                "remote_url": "https://example.com/migrate.git",
                "location": "sibling",
                "created_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();

        // Migrate
        let migrated = BareProjectConfig::migrate_if_needed(temp.path()).unwrap();
        assert!(migrated);

        // TOML should now exist
        let toml_path = gwt_dir.join("project.toml");
        assert!(toml_path.exists());

        // Load should work
        let loaded = BareProjectConfig::load(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.bare_repo_name, "migrate.git");

        // Second migration should be no-op
        let migrated_again = BareProjectConfig::migrate_if_needed(temp.path()).unwrap();
        assert!(!migrated_again);
    }

    #[test]
    fn test_derive_bare_repo_name_https_with_git() {
        assert_eq!(
            BareProjectConfig::derive_bare_repo_name("https://github.com/user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_derive_bare_repo_name_https_without_git() {
        assert_eq!(
            BareProjectConfig::derive_bare_repo_name("https://github.com/user/repo"),
            "repo.git"
        );
    }

    #[test]
    fn test_derive_bare_repo_name_ssh() {
        assert_eq!(
            BareProjectConfig::derive_bare_repo_name("git@github.com:user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_config_dir_path() {
        let path = PathBuf::from("/project");
        assert_eq!(
            BareProjectConfig::config_dir(&path),
            PathBuf::from("/project/.gwt")
        );
    }
}
