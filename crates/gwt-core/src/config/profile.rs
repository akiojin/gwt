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
    /// Description
    #[serde(default)]
    pub description: String,
}

impl Profile {
    /// Create a new profile
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            env: HashMap::new(),
            description: String::new(),
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
            *self = Self::default_with_profile();
            return;
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
}
