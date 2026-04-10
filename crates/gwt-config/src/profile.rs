//! Profile management for environment variables.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ai_settings::AISettings;

/// An environment profile with optional AI settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Profile name.
    #[serde(default)]
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Environment variables to set when this profile is active.
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    /// OS environment variables to suppress when this profile is active.
    #[serde(default)]
    pub disabled_env: Vec<String>,
    /// AI provider settings (optional).
    #[serde(default)]
    pub ai_settings: Option<AISettings>,
}

impl Profile {
    /// Create a new profile with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Builder: add an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Builder: set AI settings.
    pub fn with_ai(mut self, ai: AISettings) -> Self {
        self.ai_settings = Some(ai);
        self
    }
}

/// Container for all profiles and the active selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesConfig {
    /// All known profiles keyed by name.
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Currently active profile name.
    #[serde(default)]
    pub active: Option<String>,
}

/// Resolved active profile metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProfileResolution {
    /// Active profile name after applying fallback rules.
    pub name: String,
    /// Whether the configured active profile had to fall back.
    pub fallback: bool,
}

impl Default for ProfilesConfig {
    fn default() -> Self {
        Self {
            profiles: vec![Profile::new("default")],
            active: Some("default".to_string()),
        }
    }
}

impl ProfilesConfig {
    /// Resolve the active profile, falling back to `default` when the current
    /// active name is missing or invalid. Ensures the default profile exists.
    pub fn normalize_active_profile(&mut self) -> ActiveProfileResolution {
        if let Some(profile) = self.active_profile() {
            return ActiveProfileResolution {
                name: profile.name.clone(),
                fallback: false,
            };
        }

        if !self
            .profiles
            .iter()
            .any(|profile| profile.name == "default")
        {
            self.profiles.push(Profile::new("default"));
        }
        self.active = Some("default".to_string());

        ActiveProfileResolution {
            name: "default".to_string(),
            fallback: true,
        }
    }

    /// Get a reference to the active profile.
    pub fn active_profile(&self) -> Option<&Profile> {
        self.active
            .as_ref()
            .and_then(|name| self.profiles.iter().find(|p| p.name == *name))
    }

    /// Get a mutable reference to the active profile.
    pub fn active_profile_mut(&mut self) -> Option<&mut Profile> {
        let name = self.active.clone();
        name.and_then(move |n| self.profiles.iter_mut().find(|p| p.name == n))
    }

    /// Find a profile by name.
    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// Add a profile. Returns an error string if a profile with the same name exists.
    pub fn add(&mut self, profile: Profile) -> Result<(), String> {
        if self.profiles.iter().any(|p| p.name == profile.name) {
            return Err(format!("profile '{}' already exists", profile.name));
        }
        self.profiles.push(profile);
        Ok(())
    }

    /// Remove a profile by name. Returns the removed profile if found.
    pub fn remove(&mut self, name: &str) -> Option<Profile> {
        if let Some(idx) = self.profiles.iter().position(|p| p.name == name) {
            let removed = self.profiles.remove(idx);
            if self.active.as_deref() == Some(name) {
                self.active = None;
            }
            Some(removed)
        } else {
            None
        }
    }

    /// Switch the active profile. Returns an error string if the profile is not found.
    pub fn switch(&mut self, name: &str) -> Result<(), String> {
        if !self.profiles.iter().any(|p| p.name == name) {
            return Err(format!("profile '{}' not found", name));
        }
        self.active = Some(name.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_profile_has_name() {
        let p = Profile::new("dev");
        assert_eq!(p.name, "dev");
        assert!(p.env_vars.is_empty());
        assert!(p.ai_settings.is_none());
    }

    #[test]
    fn builder_methods() {
        let p = Profile::new("dev")
            .with_env("FOO", "bar")
            .with_ai(AISettings::default());
        assert_eq!(p.env_vars.get("FOO"), Some(&"bar".to_string()));
        assert!(p.ai_settings.is_some());
    }

    #[test]
    fn default_profiles_config_has_default_profile() {
        let c = ProfilesConfig::default();
        assert_eq!(c.active.as_deref(), Some("default"));
        assert!(c.active_profile().is_some());
    }

    #[test]
    fn add_and_get_profile() {
        let mut c = ProfilesConfig::default();
        c.add(Profile::new("dev")).unwrap();
        assert!(c.get("dev").is_some());
    }

    #[test]
    fn add_duplicate_fails() {
        let mut c = ProfilesConfig::default();
        assert!(c.add(Profile::new("default")).is_err());
    }

    #[test]
    fn remove_profile() {
        let mut c = ProfilesConfig::default();
        c.add(Profile::new("dev")).unwrap();
        c.switch("dev").unwrap();
        let removed = c.remove("dev");
        assert!(removed.is_some());
        assert_eq!(c.active, None);
        assert!(c.get("dev").is_none());
    }

    #[test]
    fn switch_to_nonexistent_fails() {
        let mut c = ProfilesConfig::default();
        assert!(c.switch("nonexistent").is_err());
    }

    #[test]
    fn switch_active_profile() {
        let mut c = ProfilesConfig::default();
        c.add(Profile::new("dev")).unwrap();
        c.switch("dev").unwrap();
        assert_eq!(c.active_profile().unwrap().name, "dev");
    }

    #[test]
    fn normalize_active_profile_falls_back_to_default_when_active_is_missing() {
        let mut c = ProfilesConfig {
            profiles: vec![Profile::new("default"), Profile::new("dev")],
            active: Some("missing".to_string()),
        };

        let resolved = c.normalize_active_profile();

        assert_eq!(
            resolved,
            ActiveProfileResolution {
                name: "default".to_string(),
                fallback: true,
            }
        );
        assert_eq!(c.active.as_deref(), Some("default"));
        assert_eq!(
            c.active_profile().map(|profile| profile.name.as_str()),
            Some("default")
        );
    }

    #[test]
    fn roundtrip_toml() {
        let mut c = ProfilesConfig::default();
        let p = Profile::new("dev").with_env("KEY", "val");
        c.add(p).unwrap();
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let loaded: ProfilesConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.profiles.len(), 2);
        assert!(loaded.get("dev").is_some());
        assert_eq!(
            loaded.get("dev").unwrap().env_vars.get("KEY"),
            Some(&"val".to_string())
        );
    }
}
