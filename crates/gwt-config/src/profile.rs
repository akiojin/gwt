//! Profile management for environment variables.

use std::collections::{BTreeMap, HashMap};

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

    /// Build a sorted effective environment preview by removing disabled OS
    /// variables and then overlaying profile-owned environment variables.
    pub fn merged_env_pairs<I>(&self, base_env: I) -> Vec<(String, String)>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut merged: BTreeMap<String, String> = base_env.into_iter().collect();
        for key in &self.disabled_env {
            merged.remove(key);
        }
        for (key, value) in &self.env_vars {
            merged.insert(key.clone(), value.clone());
        }
        merged.into_iter().collect()
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

/// Outcome of deleting a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDeleteOutcome {
    /// Whether deleting the profile switched the active selection back to
    /// `default`.
    pub active_switched_to_default: bool,
}

fn default_profile() -> Profile {
    Profile {
        name: "default".to_string(),
        description: "Default profile".to_string(),
        ..Default::default()
    }
}

impl Default for ProfilesConfig {
    fn default() -> Self {
        Self {
            profiles: vec![default_profile()],
            active: Some("default".to_string()),
        }
    }
}

impl ProfilesConfig {
    fn ensure_default_profile(&mut self) {
        if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == "default")
        {
            if profile.description.is_empty() {
                profile.description = "Default profile".to_string();
            }
            return;
        }
        self.profiles.push(default_profile());
    }

    fn profile_mut(&mut self, name: &str) -> Option<&mut Profile> {
        self.profiles
            .iter_mut()
            .find(|profile| profile.name == name)
    }

    /// Resolve the active profile, falling back to `default` when the current
    /// active name is missing or invalid. Ensures the default profile exists.
    pub fn normalize_active_profile(&mut self) -> ActiveProfileResolution {
        self.ensure_default_profile();

        if let Some(profile) = self.active_profile() {
            return ActiveProfileResolution {
                name: profile.name.clone(),
                fallback: false,
            };
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

    /// Update a profile's metadata.
    pub fn update_profile(
        &mut self,
        current_name: &str,
        new_name: &str,
        new_description: &str,
    ) -> Result<(), String> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err("profile name cannot be empty".to_string());
        }
        if current_name == "default" && new_name != "default" {
            return Err("default profile cannot be renamed".to_string());
        }
        if current_name != new_name && self.profiles.iter().any(|p| p.name == new_name) {
            return Err(format!("profile '{}' already exists", new_name));
        }

        let active_matches = self.active.as_deref() == Some(current_name);
        let profile = self
            .profile_mut(current_name)
            .ok_or_else(|| format!("profile '{}' not found", current_name))?;
        profile.name = new_name.to_string();
        profile.description = new_description.to_string();
        if active_matches {
            self.active = Some(new_name.to_string());
        }
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

    /// Delete a profile while preserving the permanent `default` contract.
    pub fn delete_profile(&mut self, name: &str) -> Result<ProfileDeleteOutcome, String> {
        if name == "default" {
            return Err("default profile cannot be deleted".to_string());
        }

        let idx = self
            .profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| format!("profile '{}' not found", name))?;
        let active_switched_to_default = self.active.as_deref() == Some(name);
        self.profiles.remove(idx);
        self.ensure_default_profile();
        if active_switched_to_default {
            self.active = Some("default".to_string());
        }
        Ok(ProfileDeleteOutcome {
            active_switched_to_default,
        })
    }

    /// Create or replace an environment variable inside a profile.
    pub fn set_env_var(
        &mut self,
        profile_name: &str,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("environment variable key cannot be empty".to_string());
        }
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        profile.env_vars.insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// Update an environment variable, allowing the key itself to change.
    pub fn update_env_var(
        &mut self,
        profile_name: &str,
        current_key: &str,
        new_key: &str,
        new_value: &str,
    ) -> Result<(), String> {
        let new_key = new_key.trim();
        if new_key.is_empty() {
            return Err("environment variable key cannot be empty".to_string());
        }
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        if current_key != new_key && profile.env_vars.contains_key(new_key) {
            return Err(format!("environment variable '{}' already exists", new_key));
        }
        if current_key != new_key {
            profile.env_vars.remove(current_key);
        }
        profile
            .env_vars
            .insert(new_key.to_string(), new_value.to_string());
        Ok(())
    }

    /// Remove an environment variable from a profile.
    pub fn remove_env_var(&mut self, profile_name: &str, key: &str) -> Result<(), String> {
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        profile.env_vars.remove(key);
        Ok(())
    }

    /// Add an OS environment variable to the disabled list.
    pub fn add_disabled_env(&mut self, profile_name: &str, key: &str) -> Result<(), String> {
        let key = key.trim();
        if key.is_empty() {
            return Err("disabled environment variable key cannot be empty".to_string());
        }
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        if !profile.disabled_env.iter().any(|item| item == key) {
            profile.disabled_env.push(key.to_string());
            profile.disabled_env.sort();
        }
        Ok(())
    }

    /// Update a disabled OS environment variable entry.
    pub fn update_disabled_env(
        &mut self,
        profile_name: &str,
        current_key: &str,
        new_key: &str,
    ) -> Result<(), String> {
        let new_key = new_key.trim();
        if new_key.is_empty() {
            return Err("disabled environment variable key cannot be empty".to_string());
        }
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        if current_key != new_key && profile.disabled_env.iter().any(|item| item == new_key) {
            return Err(format!(
                "disabled environment variable '{}' already exists",
                new_key
            ));
        }
        if let Some(item) = profile
            .disabled_env
            .iter_mut()
            .find(|item| item.as_str() == current_key)
        {
            *item = new_key.to_string();
        } else {
            profile.disabled_env.push(new_key.to_string());
        }
        profile.disabled_env.sort();
        Ok(())
    }

    /// Remove a disabled OS environment variable entry.
    pub fn remove_disabled_env(&mut self, profile_name: &str, key: &str) -> Result<(), String> {
        let profile = self
            .profile_mut(profile_name)
            .ok_or_else(|| format!("profile '{}' not found", profile_name))?;
        profile.disabled_env.retain(|item| item != key);
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
    fn delete_profile_rejects_default_and_switches_active_to_default() {
        let mut c = ProfilesConfig::default();
        c.add(Profile::new("dev")).unwrap();
        c.switch("dev").unwrap();

        assert!(c.delete_profile("default").is_err());

        let outcome = c.delete_profile("dev").unwrap();
        assert!(outcome.active_switched_to_default);
        assert!(c.get("dev").is_none());
        assert_eq!(c.active.as_deref(), Some("default"));
    }

    #[test]
    fn update_env_var_rewrites_key_and_value() {
        let mut c = ProfilesConfig::default();
        c.add(Profile::new("dev").with_env("OLD_KEY", "old"))
            .unwrap();

        c.update_env_var("dev", "OLD_KEY", "NEW_KEY", "new")
            .unwrap();

        let profile = c.get("dev").unwrap();
        assert!(!profile.env_vars.contains_key("OLD_KEY"));
        assert_eq!(profile.env_vars.get("NEW_KEY"), Some(&"new".to_string()));
    }

    #[test]
    fn merged_env_pairs_remove_disabled_and_override_values() {
        let mut profile = Profile::new("dev").with_env("API_KEY", "override");
        profile.disabled_env = vec!["SECRET".to_string()];

        let merged = profile.merged_env_pairs([
            ("PATH".to_string(), "/bin".to_string()),
            ("SECRET".to_string(), "hidden".to_string()),
            ("API_KEY".to_string(), "base".to_string()),
        ]);

        assert_eq!(
            merged,
            vec![
                ("API_KEY".to_string(), "override".to_string()),
                ("PATH".to_string(), "/bin".to_string()),
            ]
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
