//! Bare project configuration (SPEC-a70a1ece US5)
//!
//! Manages configuration for bare repository based projects.
//! The config file is stored in .gwt/ directory at the project root (bare repo's parent).

use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configuration file name
const CONFIG_FILE_NAME: &str = "project.json";

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

    /// Get the config file path for a project root
    pub fn config_path(project_root: &Path) -> PathBuf {
        Self::config_dir(project_root).join(CONFIG_FILE_NAME)
    }

    /// Load configuration from a project root (SPEC-a70a1ece T502)
    pub fn load(project_root: &Path) -> Result<Option<Self>> {
        let config_path = Self::config_path(project_root);
        if !config_path.exists() {
            return Ok(None);
        }

        let content =
            std::fs::read_to_string(&config_path).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to read {}: {}", config_path.display(), e),
            })?;

        let config: Self =
            serde_json::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to parse bare project config: {}", e),
            })?;

        Ok(Some(config))
    }

    /// Save configuration to a project root (SPEC-a70a1ece T502)
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let config_dir = Self::config_dir(project_root);
        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir).map_err(|e| GwtError::ConfigWriteError {
                reason: format!("Failed to create {}: {}", config_dir.display(), e),
            })?;
        }

        let config_path = Self::config_path(project_root);
        let content =
            serde_json::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
                reason: format!("Failed to serialize bare project config: {}", e),
            })?;

        std::fs::write(&config_path, content).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to write {}: {}", config_path.display(), e),
        })?;

        Ok(())
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
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let config = BareProjectConfig::with_remote("test.git", "https://example.com/test.git");

        config.save(temp.path()).unwrap();
        assert!(BareProjectConfig::config_path(temp.path()).exists());

        let loaded = BareProjectConfig::load(temp.path()).unwrap().unwrap();
        assert_eq!(loaded.bare_repo_name, "test.git");
        assert_eq!(
            loaded.remote_url,
            Some("https://example.com/test.git".to_string())
        );
    }

    #[test]
    fn test_load_missing() {
        let temp = TempDir::new().unwrap();
        let result = BareProjectConfig::load(temp.path()).unwrap();
        assert!(result.is_none());
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
