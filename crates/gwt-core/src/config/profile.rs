//! Profile management for environment variables (SPEC-a3f4c9df)
//!
//! Manages environment profiles with automatic migration from YAML to TOML.
//! - New format: ~/.gwt/profiles.toml (TOML)
//! - Legacy format: ~/.gwt/profiles.yaml (YAML)

use crate::config::migration::{backup_broken_file, ensure_config_dir, write_atomic};
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
}

/// Resolved AI settings with defaults and environment fallbacks applied
#[derive(Debug, Clone)]
pub struct ResolvedAISettings {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

impl AISettings {
    /// Resolve AI settings (no environment variable fallback - settings must be explicit)
    pub fn resolved(&self) -> ResolvedAISettings {
        ResolvedAISettings {
            endpoint: self.endpoint.trim().to_string(),
            api_key: self.api_key.trim().to_string(),
            model: self.model.trim().to_string(),
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
    #[serde(default)]
    pub default_ai: Option<AISettings>,
    /// Profiles map
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}

impl ProfilesConfig {
    /// New TOML profiles config file path (~/.gwt/profiles.toml)
    pub fn toml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("profiles.toml")
    }

    /// Legacy YAML profiles config file path (~/.gwt/profiles.yaml)
    pub fn yaml_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("profiles.yaml")
    }

    /// Get the preferred config file path (TOML if exists, else YAML)
    /// For backward compatibility, returns YAML path if only YAML exists
    #[deprecated(note = "Use toml_path() for new code")]
    pub fn path() -> PathBuf {
        let toml = Self::toml_path();
        if toml.exists() {
            return toml;
        }
        let yaml = Self::yaml_path();
        if yaml.exists() {
            return yaml;
        }
        // Default to TOML for new files
        toml
    }

    /// Load profiles from disk with automatic format detection (SPEC-a3f4c9df FR-005)
    ///
    /// Priority:
    /// 1. profiles.toml (new format)
    /// 2. profiles.yaml (legacy format)
    /// 3. Default profile
    pub fn load() -> Result<Self> {
        let toml_path = Self::toml_path();
        let yaml_path = Self::yaml_path();

        // Try TOML first (new format takes priority)
        if toml_path.exists() {
            debug!(
                category = "config",
                path = %toml_path.display(),
                "Loading profiles from TOML"
            );
            match Self::load_toml(&toml_path) {
                Ok(mut config) => {
                    config.ensure_defaults();
                    return Ok(config);
                }
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %toml_path.display(),
                        error = %e,
                        "Failed to load TOML profiles, trying YAML fallback"
                    );
                    // Backup broken TOML file
                    let _ = backup_broken_file(&toml_path);
                }
            }
        }

        // Try YAML fallback (legacy format)
        if yaml_path.exists() {
            debug!(
                category = "config",
                path = %yaml_path.display(),
                "Loading profiles from YAML (legacy)"
            );
            match Self::load_yaml(&yaml_path) {
                Ok(mut config) => {
                    config.ensure_defaults();
                    // Auto-migrate: save as TOML for next time (SPEC-a3f4c9df)
                    if let Err(e) = config.save() {
                        warn!(
                            category = "config",
                            error = %e,
                            "Failed to auto-migrate profiles to TOML"
                        );
                    } else {
                        info!(
                            category = "config",
                            operation = "auto_migrate",
                            "Auto-migrated profiles.yaml to profiles.toml"
                        );
                    }
                    return Ok(config);
                }
                Err(e) => {
                    warn!(
                        category = "config",
                        path = %yaml_path.display(),
                        error = %e,
                        "Failed to load YAML profiles"
                    );
                    // Backup broken YAML file
                    let _ = backup_broken_file(&yaml_path);
                }
            }
        }

        // Return default
        debug!(
            category = "config",
            "No profiles config found, using default"
        );
        Ok(Self::default_with_profile())
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

    /// Save profiles to disk in TOML format (SPEC-a3f4c9df FR-006)
    ///
    /// Always saves to profiles.toml regardless of source format.
    /// Uses atomic write (temp file + rename) for data safety.
    pub fn save(&self) -> Result<()> {
        let path = Self::toml_path();

        // Ensure ~/.gwt directory exists
        if let Some(parent) = path.parent() {
            ensure_config_dir(parent)?;
        }

        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to serialize to TOML: {}", e),
        })?;

        write_atomic(&path, &content)?;

        info!(
            category = "config",
            path = %path.display(),
            "Saved profiles config (TOML)"
        );
        Ok(())
    }

    /// Check if migration from YAML to TOML is needed
    pub fn needs_migration() -> bool {
        let toml_path = Self::toml_path();
        let yaml_path = Self::yaml_path();
        yaml_path.exists() && !toml_path.exists()
    }

    /// Migrate from YAML to TOML if needed
    pub fn migrate_if_needed() -> Result<bool> {
        if !Self::needs_migration() {
            return Ok(false);
        }

        info!(
            category = "config",
            operation = "migration",
            "Migrating profiles from YAML to TOML"
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

    /// Set active profile
    pub fn set_active(&mut self, name: Option<String>) {
        self.active = name;
    }

    /// Ensure default profile exists
    fn ensure_defaults(&mut self) {
        if self.profiles.is_empty() {
            self.profiles
                .insert("default".to_string(), Profile::new("default"));
            if self.active.is_none() {
                self.active = Some("default".to_string());
            }
            if self.version == 0 {
                self.version = 1;
            }
        }
        if self.active.is_none() && self.profiles.contains_key("default") {
            self.active = Some("default".to_string());
        }
    }

    fn default_with_profile() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), Profile::new("default"));
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
    fn test_profiles_config_roundtrip_toml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("dev".to_string(), Profile::new("dev"));
        config.active = Some("dev".to_string());
        config.save().unwrap();

        // Should save as TOML
        let toml_path = ProfilesConfig::toml_path();
        assert!(toml_path.exists());
        assert!(toml_path.to_string_lossy().ends_with("profiles.toml"));

        // Content should be TOML format
        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("active = \"dev\""));

        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("dev"));
        assert!(loaded.profiles.contains_key("dev"));

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_load_yaml_fallback() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_toml_priority_over_yaml() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_needs_migration() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        // No files - no migration needed
        assert!(!ProfilesConfig::needs_migration());

        // Create YAML only
        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();
        std::fs::write(gwt_dir.join("profiles.yaml"), "version: 1").unwrap();
        assert!(ProfilesConfig::needs_migration());

        // Create TOML - no longer needs migration
        std::fs::write(gwt_dir.join("profiles.toml"), "version = 1").unwrap();
        assert!(!ProfilesConfig::needs_migration());

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_migrate_if_needed() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

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

        // TOML should now exist
        let toml_path = gwt_dir.join("profiles.toml");
        assert!(toml_path.exists());

        // Load should work
        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("migrate-me"));

        // Second migration should be no-op
        let migrated_again = ProfilesConfig::migrate_if_needed().unwrap();
        assert!(!migrated_again);

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
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
    }

    #[test]
    fn test_ai_settings_no_env_fallback() {
        // Environment variables are NOT used as fallback (settings must be explicit)
        let settings = AISettings {
            endpoint: "".to_string(),
            api_key: "".to_string(),
            model: "".to_string(),
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
        };
        assert!(settings.is_enabled());
    }

    #[test]
    fn test_ai_settings_enabled_without_key() {
        let settings = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
        };
        assert!(settings.is_enabled());
    }

    #[test]
    fn test_ai_settings_requires_endpoint_and_model() {
        let missing_endpoint = AISettings {
            endpoint: "".to_string(),
            api_key: "key".to_string(),
            model: "gpt-4o-mini".to_string(),
        };
        assert!(!missing_endpoint.is_enabled());

        let missing_model = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "key".to_string(),
            model: "".to_string(),
        };
        assert!(!missing_model.is_enabled());
    }

    #[test]
    fn test_profile_ai_enabled_requires_settings() {
        let profile = Profile::new("dev");
        assert!(!profile.ai_enabled());
    }
}
