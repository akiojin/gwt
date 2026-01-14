//! Settings management

use super::migration::auto_migrate;
use crate::error::{GwtError, Result};
use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Log directory path
    pub log_dir: Option<PathBuf>,
    /// Log retention days
    pub log_retention_days: u32,
    /// Web server settings
    pub web: WebSettings,
    /// Agent settings
    pub agent: AgentSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            protected_branches: vec![
                "main".to_string(),
                "master".to_string(),
                "develop".to_string(),
            ],
            default_base_branch: "main".to_string(),
            worktree_root: ".worktrees".to_string(),
            debug: false,
            log_dir: None,
            log_retention_days: 7,
            web: WebSettings::default(),
            agent: AgentSettings::default(),
        }
    }
}

/// Web server settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSettings {
    /// Server port
    pub port: u16,
    /// Bind address
    pub address: String,
    /// Enable CORS
    pub cors: bool,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            port: 8080,
            address: "127.0.0.1".to_string(),
            cors: true,
        }
    }
}

/// Agent settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Default agent to use
    pub default_agent: Option<String>,
    /// Claude Code path
    pub claude_path: Option<PathBuf>,
    /// Codex CLI path
    pub codex_path: Option<PathBuf>,
    /// Gemini CLI path
    pub gemini_path: Option<PathBuf>,
}

impl Settings {
    /// Load settings from configuration files and environment
    pub fn load(repo_root: &Path) -> Result<Self> {
        auto_migrate(repo_root)?;
        let config_path = Self::find_config_file(repo_root);

        let mut figment = Figment::new().merge(Toml::string(&Self::default_toml()));

        if let Some(path) = config_path {
            figment = figment.merge(Toml::file(path));
        }

        figment = figment.merge(Env::prefixed("GWT_").split("_"));

        figment.extract().map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })
    }

    /// Get default TOML configuration
    fn default_toml() -> String {
        toml::to_string_pretty(&Self::default()).unwrap_or_default()
    }

    /// Find the configuration file
    pub fn find_config_file(repo_root: &Path) -> Option<PathBuf> {
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

    /// Get global config directory
    pub fn global_config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gwt").map(|dirs| dirs.config_dir().to_path_buf())
    }

    /// Get log directory path
    pub fn log_dir(&self, repo_root: &Path) -> PathBuf {
        if let Some(ref log_dir) = self.log_dir {
            if log_dir.is_absolute() {
                return log_dir.clone();
            }
            return repo_root.join(log_dir);
        }

        // Default: ~/.gwt/logs/{workspace_name}
        if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
            let workspace_name = repo_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".to_string());
            return home.join(".gwt").join("logs").join(workspace_name);
        }

        repo_root.join(".gwt").join("logs")
    }

    /// Save settings to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Create default config file
    pub fn create_default(path: &Path) -> Result<Self> {
        let settings = Self::default();
        settings.save(path)?;
        Ok(settings)
    }

    /// Check if a branch is protected
    pub fn is_branch_protected(&self, branch: &str) -> bool {
        self.protected_branches.iter().any(|p| p == branch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert!(!settings.protected_branches.is_empty());
        assert!(settings.protected_branches.contains(&"main".to_string()));
        assert!(!settings.debug);
        assert_eq!(settings.web.port, 8080);
    }

    #[test]
    fn test_load_auto_migrates_json() {
        let temp = TempDir::new().unwrap();
        let json_path = temp.path().join(".gwt.json");
        let toml_path = temp.path().join(".gwt.toml");

        std::fs::write(
            &json_path,
            r#"{"default_base_branch":"develop","worktree_root":".worktrees"}"#,
        )
        .unwrap();

        let settings = Settings::load(temp.path()).unwrap();
        assert!(toml_path.exists());
        assert_eq!(settings.default_base_branch, "develop");
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join(".gwt.toml");

        let settings = Settings {
            protected_branches: vec!["main".to_string(), "release".to_string()],
            debug: true,
            web: WebSettings {
                port: 9090,
                ..Default::default()
            },
            ..Default::default()
        };

        settings.save(&config_path).unwrap();

        let loaded = Settings::load(temp.path()).unwrap();
        assert!(loaded.protected_branches.contains(&"main".to_string()));
        assert!(loaded.protected_branches.contains(&"release".to_string()));
        assert!(loaded.debug);
        assert_eq!(loaded.web.port, 9090);
    }

    #[test]
    fn test_is_branch_protected() {
        let settings = Settings::default();
        assert!(settings.is_branch_protected("main"));
        assert!(settings.is_branch_protected("master"));
        assert!(settings.is_branch_protected("develop"));
        assert!(!settings.is_branch_protected("feature/foo"));
    }

    #[test]
    fn test_env_override() {
        let temp = TempDir::new().unwrap();

        // Set environment variable
        std::env::set_var("GWT_DEBUG", "true");

        let settings = Settings::load(temp.path()).unwrap();

        // Clean up
        std::env::remove_var("GWT_DEBUG");

        assert!(settings.debug);
    }
}
