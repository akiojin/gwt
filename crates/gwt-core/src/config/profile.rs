//! Profile management for environment variables (gwt-spec issue)
//!
//! Manages environment profiles with automatic migration from legacy profile files.
//! - Current format: ~/.gwt/config.toml [profiles]
//! - Legacy format: ~/.gwt/profiles.toml, ~/.gwt/profiles.yaml

use super::settings::Settings;
use crate::config::migration::backup_broken_file;
use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Environment profile
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Profile name
    pub name: String,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Disabled OS environment variables
    #[serde(default)]
    pub disabled_env: Vec<String>,
    /// Description
    #[serde(default)]
    pub description: String,
    /// AI settings (optional)
    #[serde(default)]
    pub ai: Option<AISettings>,
    /// AI settings enabled flag (optional)
    #[serde(default)]
    pub ai_enabled: Option<bool>,
}

impl Profile {
    /// Create a new profile
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            env: HashMap::new(),
            disabled_env: Vec::new(),
            description: String::new(),
            ai: None,
            ai_enabled: None,
        }
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Apply profile to current environment
    pub fn apply(&self) {
        for (key, value) in &self.env {
            std::env::set_var(key, value);
        }
    }

    /// Resolve AI settings with environment fallbacks
    pub fn resolved_ai_settings(&self) -> Option<ResolvedAISettings> {
        self.ai.as_ref().map(|settings| settings.resolved())
    }

    /// Check if AI settings are enabled for this profile
    pub fn ai_enabled(&self) -> bool {
        self.ai
            .as_ref()
            .map(|settings| settings.is_enabled())
            .unwrap_or(false)
    }
}

/// AI settings for OpenAI-compatible APIs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AISettings {
    /// API endpoint
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// API key (optional for local LLMs)
    #[serde(default)]
    pub api_key: String,
    /// Model name
    #[serde(default = "default_model")]
    pub model: String,
    /// Output language ("en" | "ja" | "auto")
    #[serde(default = "default_ai_language")]
    pub language: String,
}

/// Resolved AI settings with defaults and environment fallbacks applied
#[derive(Debug, Clone)]
pub struct ResolvedAISettings {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveAISettingsSource {
    ActiveProfile,
    DefaultAI,
    None,
}

#[derive(Debug, Clone)]
pub struct ActiveAISettingsResolution {
    pub source: ActiveAISettingsSource,
    pub ai_enabled: bool,
    pub resolved: Option<ResolvedAISettings>,
}

impl AISettings {
    /// Resolve AI settings (no environment variable fallback - settings must be explicit)
    pub fn resolved(&self) -> ResolvedAISettings {
        ResolvedAISettings {
            endpoint: self.endpoint.trim().to_string(),
            api_key: self.api_key.trim().to_string(),
            model: self.model.trim().to_string(),
            language: normalize_ai_language(&self.language),
        }
    }

    /// Check if settings are enabled (endpoint/model required, API key optional)
    pub fn is_enabled(&self) -> bool {
        let endpoint = self.endpoint.trim();
        let model = self.model.trim();
        !endpoint.is_empty() && !model.is_empty()
    }
}

fn default_endpoint() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_model() -> String {
    String::new() // No default - must be selected via wizard
}

fn default_ai_language() -> String {
    "en".to_string()
}

fn default_profile_ai_settings() -> AISettings {
    AISettings {
        endpoint: default_endpoint(),
        api_key: String::new(),
        model: default_model(),
        language: default_ai_language(),
    }
}

fn default_profile() -> Profile {
    let mut profile = Profile::new("default");
    profile.ai = Some(default_profile_ai_settings());
    profile
}

fn is_default_profile_ai_placeholder(settings: &AISettings) -> bool {
    settings.endpoint.trim() == default_endpoint()
        && settings.api_key.trim().is_empty()
        && settings.model.trim().is_empty()
        && normalize_ai_language(&settings.language) == default_ai_language()
}

fn normalize_ai_language(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "ja" => "ja".to_string(),
        "auto" => "auto".to_string(),
        _ => "en".to_string(),
    }
}

/// Profiles configuration stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesConfig {
    /// Schema version
    #[serde(default)]
    pub version: u8,
    /// Active profile name
    #[serde(default)]
    pub active: Option<String>,
    /// Default AI settings (profile fallback)
    #[serde(default, skip_serializing)]
    pub default_ai: Option<AISettings>,
    /// Profiles map
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

impl ProfilesConfig {
    /// Legacy TOML profiles config file path (~/.gwt/profiles.toml)
    ///
    /// Profiles are now persisted in `~/.gwt/config.toml` under `[profiles]`.
    /// This path is retained for one-time migration compatibility.
    pub fn toml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("profiles.toml")
    }

    /// Legacy YAML profiles config file path (~/.gwt/profiles.yaml)
    pub fn yaml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("profiles.yaml")
    }

    /// Load profiles from global settings with automatic legacy migration.
    ///
    /// Priority:
    /// 1. `~/.gwt/config.toml` `[profiles]` section
    /// 2. legacy `~/.gwt/profiles.toml`
    /// 3. legacy `~/.gwt/profiles.yaml`
    /// 4. Default profile
    pub fn load() -> Result<Self> {
        // Migrate legacy profiles onto the raw on-disk config so temporary env
        // overrides are not serialized into config.toml.
        let mut settings = Settings::load_global_raw()?;
        let has_profiles_in_global = Self::global_config_has_profiles_section();

        if !has_profiles_in_global {
            if let Some(mut legacy) = Self::load_legacy_fallback()? {
                legacy.normalize_loaded();
                settings.profiles = legacy.clone();
                settings.save_global()?;
                info!(
                    category = "config",
                    path = %Settings::global_config_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string()),
                    "Migrated legacy profiles into global config"
                );
                return Ok(legacy);
            }
        }

        let mut profiles = settings.profiles;
        profiles.normalize_loaded();
        Ok(profiles)
    }

    /// Load profiles from TOML file
    fn load_toml(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: ProfilesConfig =
            toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to parse TOML: {}", e),
            })?;
        Ok(config)
    }

    /// Load profiles from YAML file
    fn load_yaml(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: ProfilesConfig =
            serde_yaml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: format!("Failed to parse YAML: {}", e),
            })?;
        Ok(config)
    }

    fn global_config_path() -> Option<PathBuf> {
        Settings::global_config_path().filter(|p| p.exists())
    }

    fn global_config_has_profiles_section() -> bool {
        let Some(path) = Self::global_config_path() else {
            return false;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return false;
        };
        let Ok(value) = content.parse::<toml::Value>() else {
            return false;
        };
        value
            .as_table()
            .is_some_and(|table| table.contains_key("profiles"))
    }

    fn load_legacy_fallback() -> Result<Option<Self>> {
        let toml_path = Self::toml_path();
        if toml_path.exists() {
            debug!(
                category = "config",
                path = %toml_path.display(),
                "Loading legacy profiles from TOML"
            );
            match Self::load_toml(&toml_path) {
                Ok(config) => return Ok(Some(config)),
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %toml_path.display(),
                        error = %e,
                        "Failed to load legacy TOML profiles, trying YAML fallback"
                    );
                    let _ = backup_broken_file(&toml_path);
                }
            }
        }

        let yaml_path = Self::yaml_path();
        if yaml_path.exists() {
            debug!(
                category = "config",
                path = %yaml_path.display(),
                "Loading legacy profiles from YAML"
            );
            match Self::load_yaml(&yaml_path) {
                Ok(config) => return Ok(Some(config)),
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %yaml_path.display(),
                        error = %e,
                        "Failed to load legacy YAML profiles"
                    );
                    let _ = backup_broken_file(&yaml_path);
                }
            }
        }
        Ok(None)
    }

    /// Save profiles to disk in TOML format (gwt-spec issue FR-006)
    ///
    /// Persists profiles under `[profiles]` in `~/.gwt/config.toml`.
    /// Uses settings save path handling (atomic write).
    pub fn save(&self) -> Result<()> {
        let mut normalized = self.clone();
        normalized.normalize_for_save()?;

        // Persist profiles onto the raw on-disk config so temporary env
        // overrides are not serialized into config.toml.
        let mut settings = Settings::load_global_raw()?;
        settings.profiles = normalized;
        settings.save_global()?;

        let path = Settings::global_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        info!(category = "config", path = %path, "Saved profiles in config.toml");
        Ok(())
    }

    /// Check if migration from YAML to TOML is needed
    pub fn needs_migration() -> bool {
        let has_profiles_in_global = Self::global_config_has_profiles_section();
        let has_global_config = Settings::global_config_path().is_some_and(|p| p.exists());
        let yaml_path = Self::yaml_path();
        yaml_path.exists() && !has_profiles_in_global && !has_global_config
    }

    /// Migrate from YAML to TOML if needed
    pub fn migrate_if_needed() -> Result<bool> {
        if !Self::needs_migration() {
            return Ok(false);
        }

        info!(
            category = "config",
            operation = "migration",
            "Migrating legacy profiles into config.toml"
        );

        let config = Self::load()?;
        config.save()?;

        info!(
            category = "config",
            operation = "migration",
            "Profiles migration completed"
        );
        Ok(true)
    }

    /// Get active profile
    pub fn active_profile(&self) -> Option<&Profile> {
        self.active
            .as_ref()
            .and_then(|name| self.profiles.get(name))
    }

    /// Resolve active AI settings and feature flags.
    ///
    /// Rules:
    /// - Active profile `ai` is the single source of truth.
    /// - `default_ai` is legacy input only and must not be used as runtime fallback.
    pub fn resolve_active_ai_settings(&self) -> ActiveAISettingsResolution {
        if let Some(profile) = self.active_profile() {
            if let Some(settings) = profile.ai.as_ref() {
                let ai_enabled = settings.is_enabled();
                return ActiveAISettingsResolution {
                    source: ActiveAISettingsSource::ActiveProfile,
                    ai_enabled,
                    resolved: ai_enabled.then(|| settings.resolved()),
                };
            }
        }

        ActiveAISettingsResolution {
            source: ActiveAISettingsSource::None,
            ai_enabled: false,
            resolved: None,
        }
    }

    /// Set active profile
    pub fn set_active(&mut self, name: Option<String>) {
        self.active = name;
    }

    /// Ensure defaults and absorb legacy fields.
    pub(crate) fn ensure_defaults(&mut self) {
        if self.version == 0 {
            self.version = 1;
        }

        if !self.profiles.contains_key("default") {
            self.profiles
                .insert("default".to_string(), default_profile());
        }

        if let Some(default) = self.profiles.get_mut("default") {
            if default.ai.is_none() {
                default.ai = Some(
                    self.default_ai
                        .clone()
                        .unwrap_or_else(default_profile_ai_settings),
                );
            } else if let (Some(profile_ai), Some(default_ai)) =
                (default.ai.as_ref(), self.default_ai.as_ref())
            {
                if is_default_profile_ai_placeholder(profile_ai) {
                    default.ai = Some(default_ai.clone());
                }
            }
        }

        if self
            .active
            .as_ref()
            .is_none_or(|name| !self.profiles.contains_key(name))
        {
            self.active = Some("default".to_string());
        }

        // Legacy compatibility: after migration, runtime must not read/write default_ai.
        self.default_ai = None;
    }

    pub(crate) fn normalize_loaded(&mut self) {
        self.ensure_defaults();
    }

    fn normalize_for_save(&mut self) -> Result<()> {
        self.normalize_profile_keys_to_lowercase()?;
        self.ensure_defaults();
        Ok(())
    }

    fn normalize_profile_keys_to_lowercase(&mut self) -> Result<()> {
        let mut normalized_profiles = HashMap::new();
        for (raw_key, mut profile) in std::mem::take(&mut self.profiles) {
            let normalized_key = raw_key.trim().to_lowercase();
            if normalized_key.is_empty() {
                return Err(GwtError::ConfigWriteError {
                    reason: "Profile name must not be empty".to_string(),
                });
            }
            if normalized_profiles.contains_key(&normalized_key) {
                return Err(GwtError::ConfigWriteError {
                    reason: format!(
                        "Profile name collision after lowercase normalization: {}",
                        normalized_key
                    ),
                });
            }
            profile.name = normalized_key.clone();
            normalized_profiles.insert(normalized_key, profile);
        }
        self.profiles = normalized_profiles;

        self.active = self
            .active
            .as_ref()
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty());
        Ok(())
    }

    fn default_with_profile() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), default_profile());
        Self {
            version: 1,
            active: Some("default".to_string()),
            default_ai: None,
            profiles,
        }
    }
}

impl Default for ProfilesConfig {
    fn default() -> Self {
        Self::default_with_profile()
    }
}

// set_private_permissions removed - now handled by write_atomic in migration.rs

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ai_settings(model: &str) -> AISettings {
        AISettings {
            endpoint: default_endpoint(),
            api_key: String::new(),
            model: model.to_string(),
            language: "en".to_string(),
        }
    }

    #[test]
    fn test_profile_builder() {
        let profile = Profile::new("test")
            .with_env("FOO", "bar")
            .with_env("BAZ", "qux");

        assert_eq!(profile.name, "test");
        assert_eq!(profile.env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(profile.env.get("BAZ"), Some(&"qux".to_string()));
    }

    #[test]
    fn test_profiles_config_roundtrip_config_toml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("dev".to_string(), Profile::new("dev"));
        config.active = Some("dev".to_string());
        config.save().unwrap();

        // Should save into global config TOML
        let config_path = Settings::global_config_path().unwrap();
        assert!(config_path.exists());
        assert!(config_path.to_string_lossy().ends_with("config.toml"));

        // Content should be TOML format
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("[profiles]"));
        assert!(content.contains("active = \"dev\""));

        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("dev"));
        assert!(loaded.profiles.contains_key("dev"));
    }

    #[test]
    fn test_load_yaml_fallback() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Create YAML file manually
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();
        let yaml_path = gwt_dir.join("profiles.yaml");
        std::fs::write(
            &yaml_path,
            r#"
version: 1
active: legacy
profiles:
  legacy:
    name: legacy
    env:
      KEY: value
"#,
        )
        .unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("legacy"));
        assert!(loaded.profiles.contains_key("legacy"));
    }

    #[test]
    fn test_toml_priority_over_yaml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create both YAML and TOML
        let yaml_path = gwt_dir.join("profiles.yaml");
        std::fs::write(
            &yaml_path,
            r#"
version: 1
active: yaml-profile
profiles:
  yaml-profile:
    name: yaml-profile
"#,
        )
        .unwrap();

        // Create TOML with proper profile structure
        let toml_path = gwt_dir.join("profiles.toml");
        std::fs::write(
            &toml_path,
            r#"version = 1
active = "toml-profile"

[profiles.toml-profile]
name = "toml-profile"
description = ""

[profiles.toml-profile.env]
"#,
        )
        .unwrap();

        // TOML should be loaded (priority)
        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("toml-profile"));
    }

    #[test]
    fn test_needs_migration() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // No files - no migration needed
        assert!(!ProfilesConfig::needs_migration());

        // Create YAML only
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();
        std::fs::write(gwt_dir.join("profiles.yaml"), "version: 1").unwrap();
        assert!(ProfilesConfig::needs_migration());

        // Create config with profiles section - no longer needs migration
        std::fs::write(
            gwt_dir.join("config.toml"),
            r#"[profiles]
version = 1
active = "default"
"#,
        )
        .unwrap();
        assert!(!ProfilesConfig::needs_migration());
    }

    #[test]
    fn test_migrate_if_needed() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create YAML file
        std::fs::write(
            gwt_dir.join("profiles.yaml"),
            r#"
version: 1
active: migrate-me
profiles:
  migrate-me:
    name: migrate-me
    env:
      MIGRATED: "true"
"#,
        )
        .unwrap();

        // Migrate
        let migrated = ProfilesConfig::migrate_if_needed().unwrap();
        assert!(migrated);

        // config.toml should now exist
        let config_path = gwt_dir.join("config.toml");
        assert!(config_path.exists());

        // Load should work
        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("migrate-me"));

        // Second migration should be no-op
        let migrated_again = ProfilesConfig::migrate_if_needed().unwrap();
        assert!(!migrated_again);
    }

    #[test]
    fn test_save_does_not_persist_env_only_overrides() {
        let _lock = crate::config::HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let prev_debug = std::env::var_os("GWT_DEBUG");
        let prev_docker_force_host = std::env::var_os("GWT_DOCKER_FORCE_HOST");
        let prev_auto_install = std::env::var_os("GWT_AGENT_AUTO_INSTALL_DEPS");

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();
        std::fs::write(
            gwt_dir.join("config.toml"),
            r#"
debug = false

[agent]
auto_install_deps = false

[docker]
force_host = false
"#,
        )
        .unwrap();

        std::env::set_var("GWT_DEBUG", "true");
        std::env::set_var("GWT_DOCKER_FORCE_HOST", "1");
        std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", "true");

        let mut config = ProfilesConfig::default();
        config.profiles.insert(
            "dev".to_string(),
            Profile::new("dev").with_env("DEV_KEY", "dev-value"),
        );
        config.active = Some("dev".to_string());
        config.save().unwrap();

        match prev_debug {
            Some(value) => std::env::set_var("GWT_DEBUG", value),
            None => std::env::remove_var("GWT_DEBUG"),
        }
        match prev_docker_force_host {
            Some(value) => std::env::set_var("GWT_DOCKER_FORCE_HOST", value),
            None => std::env::remove_var("GWT_DOCKER_FORCE_HOST"),
        }
        match prev_auto_install {
            Some(value) => std::env::set_var("GWT_AGENT_AUTO_INSTALL_DEPS", value),
            None => std::env::remove_var("GWT_AGENT_AUTO_INSTALL_DEPS"),
        }

        let saved = Settings::load_global_raw().unwrap();
        assert!(!saved.debug);
        assert!(!saved.docker.force_host);
        assert!(!saved.agent.auto_install_deps);
        assert_eq!(saved.profiles.active.as_deref(), Some("dev"));
        assert_eq!(
            saved
                .profiles
                .profiles
                .get("dev")
                .and_then(|profile| profile.env.get("DEV_KEY"))
                .map(String::as_str),
            Some("dev-value")
        );
    }

    #[test]
    fn test_ai_settings_resolved_defaults() {
        // AISettings::default() uses #[derive(Default)], so all fields are empty
        // The serde default functions (default_endpoint, default_model) are only used during deserialization
        let settings = AISettings::default();
        let resolved = settings.resolved();
        assert_eq!(resolved.endpoint, "");
        assert_eq!(resolved.model, "");
        assert_eq!(resolved.api_key, "");
        assert_eq!(resolved.language, "en");
    }

    #[test]
    fn test_ai_settings_serde_defaults() {
        // When deserializing YAML with missing fields, serde defaults are applied
        let yaml = "{}";
        let settings: AISettings = serde_yaml::from_str(yaml).unwrap();
        let resolved = settings.resolved();
        assert_eq!(resolved.endpoint, "https://api.openai.com/v1"); // serde default
        assert_eq!(resolved.model, ""); // No default model
        assert_eq!(resolved.api_key, "");
        assert_eq!(settings.language, "en");
        assert_eq!(resolved.language, "en");
    }

    #[test]
    fn test_ai_settings_language_normalizes_known_values() {
        let settings = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            language: "JA".to_string(),
        };
        assert_eq!(settings.resolved().language, "ja");

        let settings = AISettings {
            language: " auto ".to_string(),
            ..settings
        };
        assert_eq!(settings.resolved().language, "auto");

        let settings = AISettings {
            language: "fr".to_string(),
            ..settings
        };
        assert_eq!(settings.resolved().language, "en");
    }

    #[test]
    fn test_ai_settings_no_env_fallback() {
        // Environment variables are NOT used as fallback (settings must be explicit)
        let settings = AISettings {
            endpoint: "".to_string(),
            api_key: "".to_string(),
            model: "".to_string(),
            language: "en".to_string(),
        };
        let resolved = settings.resolved();
        // Should return empty strings, not environment variable values
        assert_eq!(resolved.endpoint, "");
        assert_eq!(resolved.model, "");
        assert_eq!(resolved.api_key, "");
    }

    #[test]
    fn test_ai_settings_enabled_local_without_key() {
        let settings = AISettings {
            endpoint: "http://localhost:11434/v1".to_string(),
            api_key: "".to_string(),
            model: "llama3.2".to_string(),
            language: "en".to_string(),
        };
        assert!(settings.is_enabled());
    }

    #[test]
    fn test_ai_settings_enabled_without_key() {
        let settings = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            language: "en".to_string(),
        };
        assert!(settings.is_enabled());
    }

    #[test]
    fn test_ai_settings_requires_endpoint_and_model() {
        let missing_endpoint = AISettings {
            endpoint: "".to_string(),
            api_key: "key".to_string(),
            model: "gpt-4o-mini".to_string(),
            language: "en".to_string(),
        };
        assert!(!missing_endpoint.is_enabled());

        let missing_model = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "key".to_string(),
            model: "".to_string(),
            language: "en".to_string(),
        };
        assert!(!missing_model.is_enabled());
    }

    #[test]
    fn test_profile_ai_enabled_requires_settings() {
        let profile = Profile::new("dev");
        assert!(!profile.ai_enabled());
    }

    #[test]
    fn test_profile_ai_disabled_flag_is_ignored() {
        let mut profile = Profile::new("dev");
        profile.ai = Some(ai_settings("gpt-5.2"));
        profile.ai_enabled = Some(false);
        assert!(profile.ai_enabled());
        assert!(profile.resolved_ai_settings().is_some());
    }

    #[test]
    fn resolve_active_ai_settings_prefers_active_profile_when_enabled() {
        let mut profiles = HashMap::new();
        let mut dev = Profile::new("dev");
        dev.ai = Some(ai_settings("gpt-5.2"));
        profiles.insert("dev".to_string(), dev);

        let config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: Some(ai_settings("gpt-5.1")),
            profiles,
        };

        let resolved = config.resolve_active_ai_settings();
        assert_eq!(resolved.source, ActiveAISettingsSource::ActiveProfile);
        assert!(resolved.ai_enabled);

        assert_eq!(resolved.resolved.unwrap().model, "gpt-5.2");
    }

    #[test]
    fn resolve_active_ai_settings_does_not_fallback_to_default_when_profile_ai_is_disabled() {
        let mut profiles = HashMap::new();
        let mut dev = Profile::new("dev");
        // Explicit AI config exists but is disabled (empty model).
        dev.ai = Some(ai_settings(""));
        profiles.insert("dev".to_string(), dev);

        let config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: Some(ai_settings("gpt-5.1")),
            profiles,
        };

        let resolved = config.resolve_active_ai_settings();
        assert_eq!(resolved.source, ActiveAISettingsSource::ActiveProfile);
        assert!(!resolved.ai_enabled);

        assert!(resolved.resolved.is_none());
    }

    #[test]
    fn resolve_active_ai_settings_ignores_ai_enabled_flag_false() {
        let mut profiles = HashMap::new();
        let mut dev = Profile::new("dev");
        dev.ai = Some(ai_settings("gpt-5.2"));
        dev.ai_enabled = Some(false);
        profiles.insert("dev".to_string(), dev);

        let config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: Some(ai_settings("gpt-5.1")),
            profiles,
        };

        let resolved = config.resolve_active_ai_settings();
        assert_eq!(resolved.source, ActiveAISettingsSource::ActiveProfile);
        assert!(resolved.ai_enabled);

        assert_eq!(resolved.resolved.unwrap().model, "gpt-5.2");
    }

    #[test]
    fn resolve_active_ai_settings_returns_none_when_profile_has_no_ai_config() {
        let mut profiles = HashMap::new();
        profiles.insert("dev".to_string(), Profile::new("dev"));

        let config = ProfilesConfig {
            version: 1,
            active: Some("dev".to_string()),
            default_ai: Some(ai_settings("gpt-5.1")),
            profiles,
        };

        let resolved = config.resolve_active_ai_settings();
        assert_eq!(resolved.source, ActiveAISettingsSource::None);
        assert!(!resolved.ai_enabled);

        assert!(resolved.resolved.is_none());
    }

    #[test]
    fn resolve_active_ai_settings_returns_none_source_when_no_ai_is_configured() {
        let config = ProfilesConfig {
            version: 1,
            active: None,
            default_ai: None,
            profiles: HashMap::new(),
        };

        let resolved = config.resolve_active_ai_settings();
        assert_eq!(resolved.source, ActiveAISettingsSource::None);
        assert!(!resolved.ai_enabled);

        assert!(resolved.resolved.is_none());
    }

    #[test]
    fn save_and_load_inserts_default_profile_when_missing() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        config.profiles.remove("default");
        config
            .profiles
            .insert("dev".to_string(), Profile::new("dev"));
        config.active = Some("dev".to_string());
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        assert!(loaded.profiles.contains_key("default"));
        assert_eq!(loaded.active.as_deref(), Some("dev"));
    }

    #[test]
    fn save_and_load_fills_default_profile_ai_when_missing() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        let default = config.profiles.get_mut("default").unwrap();
        default.ai = None;
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        let default = loaded.profiles.get("default").unwrap();
        assert!(default.ai.is_some());
    }

    #[test]
    fn save_and_load_keeps_default_profile_api_key_optional() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        let default = config.profiles.get_mut("default").unwrap();
        default.ai = None;
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        let default = loaded.profiles.get("default").unwrap();
        let ai = default.ai.as_ref().unwrap();
        assert_eq!(ai.api_key, "");
    }

    #[test]
    fn save_and_load_keeps_default_profile_api_key_value() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        let default = config.profiles.get_mut("default").unwrap();
        default.ai = Some(AISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "sk-default-persisted".to_string(),
            model: String::new(),
            language: "ja".to_string(),
        });
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        let default = loaded.profiles.get("default").unwrap();
        let ai = default.ai.as_ref().unwrap();
        assert_eq!(ai.api_key, "sk-default-persisted");
        assert_eq!(ai.endpoint, "https://api.openai.com/v1");
        assert_eq!(ai.language, "ja");
    }

    #[test]
    fn save_and_load_keeps_default_ai_only_configuration_effective() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let config = ProfilesConfig {
            default_ai: Some(AISettings {
                endpoint: "https://api.openai.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-4o-mini".to_string(),
                language: "en".to_string(),
            }),
            ..ProfilesConfig::default()
        };
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        let resolved = loaded.resolve_active_ai_settings();
        assert!(resolved.ai_enabled);
        assert_eq!(resolved.resolved.unwrap().model, "gpt-4o-mini");
        assert!(loaded.default_ai.is_none());

        let config_path = Settings::global_config_path().unwrap();
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(!saved.contains("default_ai"));
    }

    #[test]
    fn save_and_load_normalizes_profile_keys_to_lowercase() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("Test".to_string(), Profile::new("Test"));
        config.active = Some("Test".to_string());
        config.save().unwrap();

        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("test"));
        assert!(loaded.profiles.contains_key("test"));
        assert_eq!(
            loaded
                .profiles
                .get("test")
                .map(|profile| profile.name.as_str()),
            Some("test")
        );
    }

    #[test]
    fn save_rejects_profile_name_collision_after_lowercase_normalization() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("Test".to_string(), Profile::new("Test"));
        config
            .profiles
            .insert("test".to_string(), Profile::new("test"));

        let err = config.save().unwrap_err();
        assert!(matches!(
            err,
            crate::error::GwtError::ConfigWriteError { .. }
        ));
        assert!(err.to_string().contains("collision"));
    }
}
