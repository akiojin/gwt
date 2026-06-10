//! Top-level application settings backed by `~/.gwt/config.toml`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::{
    agent_config::AgentConfig,
    ai_settings::AISettings,
    atomic::write_atomic,
    board_config::BoardConfig,
    error::{ConfigError, Result},
    profile::ProfilesConfig,
    usage_config::UsageConfig,
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
    /// Global AI provider defaults (SPEC-1933 FR-006). The active profile may
    /// override individual fields via [`crate::profile::Profile::ai_settings`].
    /// Currently the only reader is [`AISettings::effective_language`] for
    /// narrative output language resolution (SPEC-1933 FR-009 / FR-010).
    pub ai: AISettings,
    /// Board provider selection (SPEC-2959). Defaults to `local`.
    pub board: BoardConfig,
    /// Provider usage display configuration (SPEC-2970).
    pub usage: UsageConfig,
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
            ai: AISettings::default(),
            board: BoardConfig::default(),
            usage: UsageConfig::default(),
        }
    }
}

impl Settings {
    /// Build the global config file path for a known home directory.
    pub fn global_config_path_for_home(home: &Path) -> PathBuf {
        home.join(".gwt").join("config.toml")
    }

    /// Return the global config file path: `~/.gwt/config.toml`.
    pub fn global_config_path() -> Option<PathBuf> {
        resolve_home_dir(
            std::env::var_os("HOME"),
            std::env::var_os("USERPROFILE"),
            dirs::home_dir(),
        )
        .map(|home| Self::global_config_path_for_home(&home))
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
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut settings = Self::load()?;
        mutate(&mut settings)?;
        settings.save_global()
    }
}

fn resolve_home_dir(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    non_empty_os(home)
        .or_else(|| non_empty_os(userprofile))
        .map(PathBuf::from)
        .or(fallback)
}

fn non_empty_os(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn default_settings_are_sane() {
        let s = Settings::default();
        assert_eq!(s.default_base_branch, "main");
        assert!(s.protected_branches.contains(&"main".to_string()));
        assert!(!s.debug);
        assert!(!s.profiling);
    }

    #[test]
    fn legacy_config_without_usage_section_defaults() {
        // A config written before SPEC-2970 has no [usage] table; Codex
        // (local-only) defaults on while Claude account usage stays opt-in
        // and defaults off (FR-009/FR-013 consent model).
        let s: Settings =
            toml::from_str("default_base_branch = \"main\"\ndebug = false\n").unwrap();
        assert!(s.usage.codex_enabled);
        assert!(!s.usage.claude_account_enabled);
    }

    #[test]
    fn global_config_path_for_home_uses_canonical_layout() {
        let home = PathBuf::from("home-dir");

        assert_eq!(
            Settings::global_config_path_for_home(&home),
            home.join(".gwt").join("config.toml")
        );
    }

    #[test]
    fn global_config_path_prefers_env_home_for_test_isolation() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path().join("other"));

        assert_eq!(
            Settings::global_config_path(),
            Some(temp.path().join(".gwt").join("config.toml"))
        );
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
