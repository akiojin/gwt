//! Top-level application settings backed by `~/.gwt/config.toml`.

use std::{
    ffi::OsString,
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Deserializer, Serialize};
use toml_edit::{value, DocumentMut, Item, Table};
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

fn deserialize_optional_nonzero_port<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<NonZeroU16>, D::Error>
where
    D: Deserializer<'de>,
{
    let port = Option::<u16>::deserialize(deserializer)?;
    Ok(port.and_then(NonZeroU16::new))
}

/// Embedded browser server settings persisted under `[server]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Last successfully bound implicit port. Zero normalizes to absent.
    #[serde(default, deserialize_with = "deserialize_optional_nonzero_port")]
    pub embedded_port: Option<NonZeroU16>,
}

fn resolve_config_home_dir(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty())
        .or_else(|| userprofile.filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .or(fallback)
}

fn resolve_existing_settings_target(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::canonicalize(path).map_err(|error| ConfigError::WriteError {
                reason: format!(
                    "failed to resolve settings symlink {}: {error}",
                    path.display()
                ),
            })
        }
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(ConfigError::WriteError {
            reason: format!(
                "failed to inspect settings path {}: {error}",
                path.display()
            ),
        }),
    }
}

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
    /// Embedded browser server configuration (SPEC-3287).
    pub server: ServerConfig,
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
            server: ServerConfig::default(),
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
        resolve_config_home_dir(
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

    /// Atomically merge the selected embedded-server port into a config file.
    ///
    /// Unlike a full typed-settings rewrite, this preserves comments, unknown
    /// future keys, and the file's minimal shape. The typed parse still runs
    /// first so malformed or incompatible settings remain fatal.
    pub fn persist_embedded_port(path: &Path, port: NonZeroU16) -> Result<()> {
        let _guard = UPDATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let target_path = resolve_existing_settings_target(path)?;
        let content = match std::fs::read_to_string(&target_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ConfigError::ParseError {
                    reason: error.to_string(),
                });
            }
        };

        toml::from_str::<Self>(&content).map_err(|error| ConfigError::ParseError {
            reason: error.to_string(),
        })?;
        let mut document =
            content
                .parse::<DocumentMut>()
                .map_err(|error| ConfigError::ParseError {
                    reason: error.to_string(),
                })?;
        let existing_decor = document
            .as_table()
            .get("server")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get("embedded_port"))
            .and_then(|item| item.as_value())
            .map(|value| value.decor().clone());
        if document.as_table().get("server").is_none() {
            document["server"] = Item::Table(Table::new());
        }
        document["server"]["embedded_port"] = value(i64::from(port.get()));
        if let (Some(existing_decor), Some(updated)) = (
            existing_decor,
            document["server"]["embedded_port"].as_value_mut(),
        ) {
            *updated.decor_mut() = existing_decor;
        }

        write_atomic(&target_path, &document.to_string())?;
        info!(path = %path.display(), port = port.get(), "Embedded server port saved");
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
    fn config_home_resolution_prefers_env_over_dirs_fallback() {
        assert_eq!(
            resolve_config_home_dir(
                Some("home-env".into()),
                Some("userprofile-env".into()),
                Some(PathBuf::from("dirs-home")),
            ),
            Some(PathBuf::from("home-env"))
        );
        assert_eq!(
            resolve_config_home_dir(
                Some("".into()),
                Some("userprofile-env".into()),
                Some(PathBuf::from("dirs-home")),
            ),
            Some(PathBuf::from("userprofile-env"))
        );
        assert_eq!(
            resolve_config_home_dir(
                Some("".into()),
                Some("".into()),
                Some(PathBuf::from("dirs-home"))
            ),
            Some(PathBuf::from("dirs-home"))
        );
        assert_eq!(resolve_config_home_dir(None, None, None), None);
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
    fn persist_embedded_port_is_serialized_with_global_updates() {
        use std::{
            sync::mpsc::{self, RecvTimeoutError},
            thread,
            time::Duration,
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "debug = false\n").expect("write config fixture");

        let updater_path = path.clone();
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let updater = thread::spawn(move || {
            let _guard = UPDATE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut settings = Settings::load_from_path(&updater_path).expect("load old snapshot");
            loaded_tx.send(()).expect("signal old snapshot loaded");
            release_rx.recv().expect("release global updater");
            settings.debug = true;
            settings.save(&updater_path).expect("save global update");
        });
        loaded_rx.recv().expect("wait for old snapshot load");

        let (ready_tx, ready_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let persistence_path = path.clone();
        let worker = thread::spawn(move || {
            ready_tx.send(()).expect("signal worker ready");
            start_rx.recv().expect("receive worker start");
            let result = Settings::persist_embedded_port(
                &persistence_path,
                NonZeroU16::new(45000).expect("non-zero fixture port"),
            );
            done_tx.send(result).expect("send persistence result");
        });

        ready_rx.recv().expect("wait for worker readiness");
        start_tx.send(()).expect("start persistence worker");
        let before_release = done_rx.recv_timeout(Duration::from_millis(250));
        release_tx.send(()).expect("release global updater");
        updater.join().expect("join global updater");

        let (completed_while_update_locked, result) = match before_release {
            Err(RecvTimeoutError::Timeout) => done_rx
                .recv_timeout(Duration::from_secs(2))
                .map(|result| (false, result))
                .expect("persistence must finish after releasing the update lock"),
            Ok(result) => (true, result),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("persistence worker disconnected before returning a result")
            }
        };

        result.expect("persist embedded port after lock release");
        worker.join().expect("join persistence worker");
        assert!(
            !completed_while_update_locked,
            "port persistence must wait for the shared settings update lock"
        );

        let settings = Settings::load_from_path(&path).expect("load merged settings");
        assert!(settings.debug, "the concurrent global update must survive");
        assert_eq!(
            settings.server.embedded_port,
            NonZeroU16::new(45000),
            "the persisted port must survive the concurrent global update"
        );
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
