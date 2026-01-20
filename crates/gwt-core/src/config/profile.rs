//! Profile management for environment variables

use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// Resolve AI settings with defaults and environment fallbacks
    pub fn resolved(&self) -> ResolvedAISettings {
        let endpoint = resolve_endpoint(&self.endpoint);
        let api_key = resolve_api_key(&self.api_key);
        let model = resolve_model(&self.model);
        ResolvedAISettings {
            endpoint,
            api_key,
            model,
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
    "gpt-4o-mini".to_string()
}

fn resolve_endpoint(value: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(env_value) = std::env::var("OPENAI_API_BASE") {
        let env_trimmed = env_value.trim();
        if !env_trimmed.is_empty() {
            return env_trimmed.to_string();
        }
    }
    default_endpoint()
}

fn resolve_model(value: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(env_value) = std::env::var("OPENAI_MODEL") {
        let env_trimmed = env_value.trim();
        if !env_trimmed.is_empty() {
            return env_trimmed.to_string();
        }
    }
    default_model()
}

fn resolve_api_key(value: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    std::env::var("OPENAI_API_KEY").unwrap_or_default()
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
    /// Profiles config file path (~/.gwt/profiles.yaml)
    pub fn path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gwt").join("profiles.yaml")
    }

    /// Load profiles from disk, falling back to default
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default_with_profile());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut config: ProfilesConfig =
            serde_yaml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
                reason: e.to_string(),
            })?;
        config.ensure_defaults();
        Ok(config)
    }

    /// Save profiles to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yaml::to_string(self).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;
        std::fs::write(&path, content)?;
        set_private_permissions(&path);
        Ok(())
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

fn set_private_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn test_profiles_config_roundtrip() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());
        let path = ProfilesConfig::path();

        let mut config = ProfilesConfig::default();
        config
            .profiles
            .insert("dev".to_string(), Profile::new("dev"));
        config.active = Some("dev".to_string());
        config.save().unwrap();

        assert!(path.exists());
        let loaded = ProfilesConfig::load().unwrap();
        assert_eq!(loaded.active.as_deref(), Some("dev"));
        assert!(loaded.profiles.contains_key("dev"));

        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_ai_settings_resolved_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENAI_API_BASE");
        std::env::remove_var("OPENAI_MODEL");

        let settings = AISettings::default();
        let resolved = settings.resolved();
        assert_eq!(resolved.endpoint, "https://api.openai.com/v1");
        assert_eq!(resolved.model, "gpt-4o-mini");
        assert_eq!(resolved.api_key, "");
    }

    #[test]
    fn test_ai_settings_env_fallbacks() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prev_key = std::env::var_os("OPENAI_API_KEY");
        let prev_base = std::env::var_os("OPENAI_API_BASE");
        let prev_model = std::env::var_os("OPENAI_MODEL");

        std::env::set_var("OPENAI_API_KEY", "env-key");
        std::env::set_var("OPENAI_API_BASE", "http://localhost:11434/v1");
        std::env::set_var("OPENAI_MODEL", "llama3.2");

        let settings = AISettings {
            endpoint: "".to_string(),
            api_key: "".to_string(),
            model: "".to_string(),
        };
        let resolved = settings.resolved();
        assert_eq!(resolved.endpoint, "http://localhost:11434/v1");
        assert_eq!(resolved.model, "llama3.2");
        assert_eq!(resolved.api_key, "env-key");

        match prev_key {
            Some(value) => std::env::set_var("OPENAI_API_KEY", value),
            None => std::env::remove_var("OPENAI_API_KEY"),
        }
        match prev_base {
            Some(value) => std::env::set_var("OPENAI_API_BASE", value),
            None => std::env::remove_var("OPENAI_API_BASE"),
        }
        match prev_model {
            Some(value) => std::env::set_var("OPENAI_MODEL", value),
            None => std::env::remove_var("OPENAI_MODEL"),
        }
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
