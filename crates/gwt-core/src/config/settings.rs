//! Settings management (SPEC-a3f4c9df)
//!
//! Manages application settings with automatic migration and path unification.
//!
//! Global config locations (priority):
//! 1. ~/.gwt/config.toml (new, preferred)
//! 2. ~/.config/gwt/config.toml (legacy, fallback)

use super::migration::{auto_migrate, ensure_config_dir, write_atomic};
use crate::error::{GwtError, Result};
use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};

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
    /// Auto install dependencies before launching agent
    pub auto_install_deps: bool,
}

impl Settings {
    /// Load settings from configuration files and environment
    pub fn load(repo_root: &Path) -> Result<Self> {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Loading settings"
        );

        auto_migrate(repo_root)?;
        let config_path = Self::find_config_file(repo_root);

        let mut figment = Figment::new().merge(Toml::string(&Self::default_toml()));

        if let Some(ref path) = config_path {
            debug!(
                category = "config",
                config_path = %path.display(),
                "Merging config file"
            );
            figment = figment.merge(Toml::file(path));
        }

        figment = figment.merge(Env::prefixed("GWT_").split("_"));

        let mut settings: Settings = figment.extract().map_err(|e| {
            error!(
                category = "config",
                error = %e,
                "Failed to parse config"
            );
            GwtError::ConfigParseError {
                reason: e.to_string(),
            }
        })?;

        if let Ok(value) = std::env::var("GWT_AGENT_AUTO_INSTALL_DEPS") {
            if let Some(parsed) = parse_env_bool(&value) {
                settings.agent.auto_install_deps = parsed;
            }
        }

        info!(
            category = "config",
            operation = "load",
            config_path = config_path.as_ref().map(|p| p.display().to_string()).as_deref(),
            debug = settings.debug,
            worktree_root = %settings.worktree_root,
            "Settings loaded"
        );

        Ok(settings)
    }

    /// Get default TOML configuration
    fn default_toml() -> String {
        toml::to_string_pretty(&Self::default()).unwrap_or_default()
    }

    /// Find the configuration file (SPEC-a3f4c9df FR-013)
    ///
    /// Priority (highest to lowest):
    /// 1. .gwt.toml (local, highest priority)
    /// 2. .gwt/config.toml (local)
    /// 3. ~/.gwt/config.toml (global, new location)
    /// 4. ~/.config/gwt/config.toml (global, legacy fallback)
    pub fn find_config_file(repo_root: &Path) -> Option<PathBuf> {
        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "Searching for config file"
        );

        // Local config candidates
        let local_candidates = [
            repo_root.join(".gwt.toml"),
            repo_root.join(".gwt/config.toml"),
        ];

        for path in local_candidates {
            if path.exists() {
                debug!(
                    category = "config",
                    config_path = %path.display(),
                    "Found local config file"
                );
                return Some(path);
            }
        }

        // Check new global config location (~/.gwt/config.toml)
        if let Some(new_global) = Self::new_global_config_path() {
            if new_global.exists() {
                debug!(
                    category = "config",
                    config_path = %new_global.display(),
                    "Found global config file (new location)"
                );
                return Some(new_global);
            }
        }

        // Check legacy global config (~/.config/gwt/config.toml)
        if let Some(legacy_global) = Self::legacy_global_config_path() {
            if legacy_global.exists() {
                debug!(
                    category = "config",
                    config_path = %legacy_global.display(),
                    "Found global config file (legacy location)"
                );
                return Some(legacy_global);
            }
        }

        debug!(
            category = "config",
            repo_root = %repo_root.display(),
            "No config file found, using defaults"
        );
        None
    }

    /// Get the new global config path (~/.gwt/config.toml)
    pub fn new_global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".gwt").join("config.toml"))
    }

    /// Get the legacy global config path (~/.config/gwt/config.toml)
    pub fn legacy_global_config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gwt")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Get global config directory (deprecated - use new_global_config_dir)
    #[deprecated(note = "Use new_global_config_dir() for new code")]
    pub fn global_config_dir() -> Option<PathBuf> {
        Self::legacy_global_config_dir()
    }

    /// Get the new global config directory (~/.gwt/)
    pub fn new_global_config_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".gwt"))
    }

    /// Get the legacy global config directory (~/.config/gwt/)
    pub fn legacy_global_config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gwt").map(|dirs| dirs.config_dir().to_path_buf())
    }

    /// Check if migration from legacy to new global config path is needed
    pub fn needs_global_path_migration() -> bool {
        let new_path = Self::new_global_config_path();
        let legacy_path = Self::legacy_global_config_path();

        match (new_path, legacy_path) {
            (Some(new), Some(legacy)) => legacy.exists() && !new.exists(),
            _ => false,
        }
    }

    /// Migrate global config from legacy to new path if needed
    pub fn migrate_global_path_if_needed() -> Result<bool> {
        if !Self::needs_global_path_migration() {
            return Ok(false);
        }

        info!(
            category = "config",
            operation = "migration",
            "Migrating global config from legacy to new path"
        );

        let legacy_path =
            Self::legacy_global_config_path().ok_or_else(|| GwtError::ConfigParseError {
                reason: "Could not determine legacy config path".to_string(),
            })?;

        let new_path =
            Self::new_global_config_path().ok_or_else(|| GwtError::ConfigParseError {
                reason: "Could not determine new config path".to_string(),
            })?;

        // Read from legacy
        let content = std::fs::read_to_string(&legacy_path)?;

        // Write to new location
        if let Some(parent) = new_path.parent() {
            ensure_config_dir(parent)?;
        }
        write_atomic(&new_path, &content)?;

        info!(
            category = "config",
            operation = "migration",
            legacy_path = %legacy_path.display(),
            new_path = %new_path.display(),
            "Global config path migration completed"
        );

        Ok(true)
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

    /// Save settings to file (SPEC-a3f4c9df FR-008)
    ///
    /// Uses atomic write (temp file + rename) for data safety.
    pub fn save(&self, path: &Path) -> Result<()> {
        debug!(
            category = "config",
            path = %path.display(),
            "Saving settings"
        );

        let content = toml::to_string_pretty(self).map_err(|e| {
            error!(
                category = "config",
                path = %path.display(),
                error = %e,
                "Failed to serialize settings"
            );
            GwtError::ConfigWriteError {
                reason: e.to_string(),
            }
        })?;

        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)?;
        }

        write_atomic(path, &content)?;

        info!(
            category = "config",
            operation = "save",
            path = %path.display(),
            "Settings saved"
        );
        Ok(())
    }

    /// Save settings to the new global config path (~/.gwt/config.toml)
    pub fn save_global(&self) -> Result<()> {
        let path = Self::new_global_config_path().ok_or_else(|| GwtError::ConfigWriteError {
            reason: "Could not determine global config path".to_string(),
        })?;
        self.save(&path)
    }

    /// Create default config file
    pub fn create_default(path: &Path) -> Result<Self> {
        debug!(
            category = "config",
            path = %path.display(),
            "Creating default config"
        );

        let settings = Self::default();
        settings.save(path)?;

        info!(
            category = "config",
            operation = "create_default",
            path = %path.display(),
            "Default config created"
        );
        Ok(settings)
    }

    /// Check if a branch is protected
    pub fn is_branch_protected(&self, branch: &str) -> bool {
        self.protected_branches.iter().any(|p| p == branch)
    }
}

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
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

    #[test]
    fn test_env_override_auto_install_deps() {
        let temp = TempDir::new().unwrap();

        std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", "true");
        let settings = Settings::load(temp.path()).unwrap();
        std::env::remove_var("GWT_AGENT_AUTO_INSTALL_DEPS");

        assert!(settings.agent.auto_install_deps);
    }

    #[test]
    fn test_new_global_config_path() {
        let path = Settings::new_global_config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".gwt"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_legacy_global_config_path() {
        let path = Settings::legacy_global_config_path();
        // This may be None on some systems without XDG_CONFIG_HOME
        if let Some(path) = path {
            assert!(path.to_string_lossy().contains("gwt"));
            assert!(path.to_string_lossy().ends_with("config.toml"));
        }
    }

    #[test]
    fn test_new_global_config_priority() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        // Create new global config
        let new_gwt = temp.path().join(".gwt");
        std::fs::create_dir_all(&new_gwt).unwrap();
        std::fs::write(
            new_gwt.join("config.toml"),
            r#"
debug = true
default_base_branch = "new-global"
"#,
        )
        .unwrap();

        // Create a repo without local config
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let settings = Settings::load(&repo).unwrap();
        assert!(settings.debug);
        assert_eq!(settings.default_base_branch, "new-global");

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_legacy_global_config_fallback() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join(".config"));

        // Only create legacy global config (no new global)
        let legacy_config = temp.path().join(".config").join("gwt");
        std::fs::create_dir_all(&legacy_config).unwrap();
        std::fs::write(
            legacy_config.join("config.toml"),
            r#"
debug = true
default_base_branch = "legacy-global"
"#,
        )
        .unwrap();

        // Create a repo without local config
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let settings = Settings::load(&repo).unwrap();
        assert!(settings.debug);
        assert_eq!(settings.default_base_branch, "legacy-global");

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match prev_xdg {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn test_needs_global_path_migration() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join(".config"));

        // No files - no migration needed
        assert!(!Settings::needs_global_path_migration());

        // Create legacy config only
        let legacy_config = temp.path().join(".config").join("gwt");
        std::fs::create_dir_all(&legacy_config).unwrap();
        std::fs::write(legacy_config.join("config.toml"), "debug = true").unwrap();
        assert!(Settings::needs_global_path_migration());

        // Create new config - no longer needs migration
        let new_gwt = temp.path().join(".gwt");
        std::fs::create_dir_all(&new_gwt).unwrap();
        std::fs::write(new_gwt.join("config.toml"), "debug = false").unwrap();
        assert!(!Settings::needs_global_path_migration());

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match prev_xdg {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn test_save_global() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        let settings = Settings {
            debug: true,
            default_base_branch: "save-global-test".to_string(),
            ..Default::default()
        };

        settings.save_global().unwrap();

        // Should be saved to new location
        let new_path = temp.path().join(".gwt").join("config.toml");
        assert!(new_path.exists());

        let content = std::fs::read_to_string(&new_path).unwrap();
        assert!(content.contains("debug = true"));
        assert!(content.contains("save-global-test"));

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
}
