//! Top-level application settings backed by `~/.gwt/config.toml`.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::{
    agent_config::AgentConfig,
    atomic::write_atomic,
    error::{ConfigError, Result},
    profile::ProfilesConfig,
    voice_config::VoiceConfig,
};

static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Top-level application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Branches that cannot be deleted.
    pub protected_branches: Vec<String>,
    /// Default base branch for new worktrees.
    pub default_base_branch: String,
    /// Worktree root directory override.
    pub worktree_root: Option<PathBuf>,
    /// Enable debug logging.
    pub debug: bool,
    /// Enable performance profiling.
    pub profiling: bool,
    /// Profile management.
    pub profiles: ProfilesConfig,
    /// Voice input configuration.
    pub voice: VoiceConfig,
    /// Agent configuration.
    pub agent: AgentConfig,
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
            worktree_root: None,
            debug: false,
            profiling: false,
            profiles: ProfilesConfig::default(),
            voice: VoiceConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

impl Settings {
    /// Return the global config file path: `~/.gwt/config.toml`.
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".gwt").join("config.toml"))
    }

    /// Load settings from `~/.gwt/config.toml`, falling back to defaults.
    pub fn load() -> Result<Self> {
        let path = match Self::global_config_path() {
            Some(p) if p.exists() => p,
            _ => {
                debug!("No global config found, using defaults");
                return Ok(Self::default());
            }
        };

        Self::load_from_path(&path)
    }

    /// Load settings from an explicit path.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            error!(path = %path.display(), error = %e, "Failed to read config");
            ConfigError::ParseError {
                reason: e.to_string(),
            }
        })?;

        toml::from_str(&content).map_err(|e| {
            error!(path = %path.display(), error = %e, "Failed to parse config");
            ConfigError::ParseError {
                reason: e.to_string(),
            }
        })
    }

    /// Save settings to the given path using atomic write.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| ConfigError::WriteError {
            reason: format!("failed to serialize settings: {e}"),
        })?;

        write_atomic(path, &content)?;

        info!(path = %path.display(), "Settings saved");
        Ok(())
    }

    /// Save settings to the global config path.
    pub fn save_global(&self) -> Result<()> {
        let path = Self::global_config_path().ok_or(ConfigError::NoConfigPath)?;
        self.save(&path)
    }

    /// Load, mutate, and save the global config atomically.
    pub fn update_global<F>(mutate: F) -> Result<()>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        let _guard = UPDATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let mut settings = Self::load()?;
        mutate(&mut settings)?;
        settings.save_global()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_sane() {
        let s = Settings::default();
        assert_eq!(s.default_base_branch, "main");
        assert!(s.protected_branches.contains(&"main".to_string()));
        assert!(!s.debug);
        assert!(!s.profiling);
    }

    #[test]
    fn roundtrip_toml() {
        let s = Settings {
            debug: true,
            worktree_root: Some(PathBuf::from("/tmp/wt")),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let loaded: Settings = toml::from_str(&toml_str).unwrap();
        assert!(loaded.debug);
        assert_eq!(loaded.worktree_root, Some(PathBuf::from("/tmp/wt")));
    }

    #[test]
    fn save_and_load_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let s = Settings {
            debug: true,
            default_base_branch: "develop".to_string(),
            ..Default::default()
        };
        s.save(&path).unwrap();

        let loaded = Settings::load_from_path(&path).unwrap();
        assert!(loaded.debug);
        assert_eq!(loaded.default_base_branch, "develop");
    }

    #[test]
    fn load_from_missing_file_returns_error() {
        let result = Settings::load_from_path(Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dir").join("config.toml");

        let s = Settings::default();
        s.save(&path).unwrap();

        assert!(path.exists());
        let loaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(loaded.default_base_branch, "main");
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let toml_str = r#"
debug = true
"#;
        let loaded: Settings = toml::from_str(toml_str).unwrap();
        assert!(loaded.debug);
        assert_eq!(loaded.default_base_branch, "main");
        assert!(loaded.protected_branches.contains(&"main".to_string()));
    }

    #[test]
    fn missing_voice_section_uses_voice_defaults() {
        let toml_str = r#"
debug = true
"#;
        let loaded: Settings = toml::from_str(toml_str).unwrap();
        assert!(loaded.voice.model_path.is_none());
        assert_eq!(loaded.voice.hotkey, "Ctrl+G,v");
        assert_eq!(loaded.voice.input_device, "system_default");
        assert_eq!(loaded.voice.language, "auto");
        assert!(!loaded.voice.enabled);
    }
}
