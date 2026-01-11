//! Settings management

use crate::error::{GwtError, Result};
use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    /// Protected branches that cannot be deleted
    pub protected_branches: Vec<String>,
    /// Default base branch for new worktrees
    pub default_base_branch: String,
    /// Worktree root directory (relative to repo root)
    pub worktree_root: String,
    /// Enable debug logging
    pub debug: bool,
}

impl Settings {
    /// Load settings from configuration files and environment
    pub fn load(repo_root: &Path) -> Result<Self> {
        let config_path = Self::find_config_file(repo_root);

        let figment = Figment::new()
            .merge(Toml::file(config_path.unwrap_or_default()))
            .merge(Env::prefixed("GWT_"));

        figment.extract().map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })
    }

    /// Find the configuration file
    fn find_config_file(repo_root: &Path) -> Option<PathBuf> {
        // Priority: .gwt.toml > .gwt/config.toml > ~/.config/gwt/config.toml
        let candidates = [
            repo_root.join(".gwt.toml"),
            repo_root.join(".gwt/config.toml"),
        ];

        for path in candidates {
            if path.exists() {
                return Some(path);
            }
        }

        // Check global config
        if let Some(config_dir) = directories::ProjectDirs::from("", "", "gwt") {
            let global_config = config_dir.config_dir().join("config.toml");
            if global_config.exists() {
                return Some(global_config);
            }
        }

        None
    }

    /// Save settings to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;

        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert!(settings.protected_branches.is_empty());
        assert!(!settings.debug);
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join(".gwt.toml");

        let settings = Settings {
            protected_branches: vec!["main".to_string()],
            default_base_branch: "develop".to_string(),
            worktree_root: ".worktrees".to_string(),
            debug: true,
        };

        settings.save(&config_path).unwrap();

        let loaded = Settings::load(temp.path()).unwrap();
        assert_eq!(loaded.protected_branches, vec!["main"]);
        assert!(loaded.debug);
    }
}
