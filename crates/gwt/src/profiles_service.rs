//! Profile window service backed by `gwt-config`.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use gwt_config::{Profile, Settings};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEnvVarView {
    pub key: String,
    pub value: String,
    pub source: ProfileEnvVarSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileEnvVarSource {
    Os,
    Profile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub name: String,
    pub description: String,
    pub active: bool,
    pub env_vars: Vec<ProfileEnvVarView>,
    pub disabled_env: Vec<String>,
    pub merged_env: Vec<ProfileEnvVarView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSnapshot {
    pub active: String,
    pub selected: String,
    pub profiles: Vec<ProfileView>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileServiceError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("duplicate profile: {0}")]
    Duplicate(String),
    #[error("protected profile: {0}")]
    Protected(String),
}

fn load_settings(path: &Path) -> Result<Settings, ProfileServiceError> {
    if path.exists() {
        Settings::load_from_path(path).map_err(|error| {
            ProfileServiceError::Storage(format!("failed to load {}: {error}", path.display()))
        })
    } else {
        Ok(Settings::default())
    }
}

fn save_settings(path: &Path, settings: &Settings) -> Result<(), ProfileServiceError> {
    settings.save(path).map_err(|error| {
        ProfileServiceError::Storage(format!("failed to save {}: {error}", path.display()))
    })
}

fn map_profile_error(message: String) -> ProfileServiceError {
    if message.contains("default profile cannot") {
        ProfileServiceError::Protected(message)
    } else if message.contains("already exists") {
        ProfileServiceError::Duplicate(message)
    } else if message.contains("not found") {
        ProfileServiceError::NotFound(message)
    } else {
        ProfileServiceError::InvalidInput(message)
    }
}

fn validate_profile_name(name: &str) -> Result<&str, ProfileServiceError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProfileServiceError::InvalidInput(
            "profile name cannot be empty".to_string(),
        ));
    }
    Ok(trimmed)
}

fn sorted_env_vars(profile: &Profile) -> Vec<ProfileEnvVarView> {
    profile
        .env_vars
        .iter()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(key, value)| ProfileEnvVarView {
            key: key.clone(),
            value: value.clone(),
            source: ProfileEnvVarSource::Profile,
        })
        .collect()
}

fn sorted_disabled_env(profile: &Profile) -> Vec<String> {
    let mut disabled = profile.disabled_env.clone();
    disabled.sort();
    disabled.dedup();
    disabled
}

fn merged_env(profile: &Profile, base_env: &[(String, String)]) -> Vec<ProfileEnvVarView> {
    profile
        .merged_env_pairs(base_env.iter().cloned())
        .into_iter()
        .map(|(key, value)| {
            let source = if profile.env_vars.contains_key(&key) {
                ProfileEnvVarSource::Profile
            } else {
                ProfileEnvVarSource::Os
            };
            ProfileEnvVarView { key, value, source }
        })
        .collect()
}

fn snapshot_from_settings_with_env<I>(
    settings: &Settings,
    selected_profile: Option<&str>,
    base_env: I,
) -> ProfileSnapshot
where
    I: IntoIterator<Item = (String, String)>,
{
    let base_env = base_env.into_iter().collect::<Vec<_>>();
    let active = settings
        .profiles
        .active_profile()
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "default".to_string());
    let selected = selected_profile
        .filter(|name| settings.profiles.get(name).is_some())
        .unwrap_or(active.as_str())
        .to_string();
    let mut profiles = settings
        .profiles
        .profiles
        .iter()
        .map(|profile| ProfileView {
            name: profile.name.clone(),
            description: profile.description.clone(),
            active: profile.name == active,
            env_vars: sorted_env_vars(profile),
            disabled_env: sorted_disabled_env(profile),
            merged_env: merged_env(profile, &base_env),
        })
        .collect::<Vec<_>>();
    profiles.sort_by(
        |left, right| match (left.name.as_str(), right.name.as_str()) {
            ("default", "default") => std::cmp::Ordering::Equal,
            ("default", _) => std::cmp::Ordering::Less,
            (_, "default") => std::cmp::Ordering::Greater,
            _ => left.name.cmp(&right.name),
        },
    );

    ProfileSnapshot {
        active,
        selected,
        profiles,
    }
}

fn snapshot_from_settings(settings: &Settings, selected_profile: Option<&str>) -> ProfileSnapshot {
    snapshot_from_settings_with_env(settings, selected_profile, std::env::vars())
}

pub fn load_profile_snapshot(
    config_path: &Path,
    selected_profile: Option<&str>,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let mut settings = load_settings(config_path)?;
    let had_default = settings.profiles.get("default").is_some();
    let active_before = settings.profiles.active.clone();
    let resolution = settings.profiles.normalize_active_profile();
    if config_path.exists()
        && (!had_default || resolution.fallback || active_before != settings.profiles.active)
    {
        save_settings(config_path, &settings)?;
    }
    Ok(snapshot_from_settings(&settings, selected_profile))
}

fn mutate_profile_settings<F>(
    config_path: &Path,
    selected_profile: Option<&str>,
    mutate: F,
) -> Result<ProfileSnapshot, ProfileServiceError>
where
    F: FnOnce(&mut Settings) -> Result<(), ProfileServiceError>,
{
    let mut settings = load_settings(config_path)?;
    settings.profiles.normalize_active_profile();
    mutate(&mut settings)?;
    settings.profiles.normalize_active_profile();
    save_settings(config_path, &settings)?;
    Ok(snapshot_from_settings(&settings, selected_profile))
}

pub fn switch_profile(
    config_path: &Path,
    profile_name: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .switch(&profile_name)
            .map_err(map_profile_error)
    })
}

pub fn add_profile(
    config_path: &Path,
    name: &str,
    description: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let name = validate_profile_name(name)?.to_string();
    mutate_profile_settings(config_path, Some(&name), |settings| {
        let mut profile = Profile::new(name.clone());
        profile.description = description.to_string();
        settings.profiles.add(profile).map_err(map_profile_error)
    })
}

pub fn update_profile(
    config_path: &Path,
    current_name: &str,
    name: &str,
    description: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let current_name = validate_profile_name(current_name)?.to_string();
    let name = validate_profile_name(name)?.to_string();
    mutate_profile_settings(config_path, Some(&name), |settings| {
        settings
            .profiles
            .update_profile(&current_name, &name, description)
            .map_err(map_profile_error)
    })
}

pub fn delete_profile(
    config_path: &Path,
    profile_name: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some("default"), |settings| {
        settings
            .profiles
            .delete_profile(&profile_name)
            .map(|_| ())
            .map_err(map_profile_error)
    })
}

pub fn set_env_var(
    config_path: &Path,
    profile_name: &str,
    key: &str,
    value: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .set_env_var(&profile_name, key, value)
            .map_err(map_profile_error)
    })
}

pub fn update_env_var(
    config_path: &Path,
    profile_name: &str,
    current_key: &str,
    key: &str,
    value: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .update_env_var(&profile_name, current_key, key, value)
            .map_err(map_profile_error)
    })
}

pub fn delete_env_var(
    config_path: &Path,
    profile_name: &str,
    key: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .remove_env_var(&profile_name, key)
            .map_err(map_profile_error)
    })
}

pub fn add_disabled_env(
    config_path: &Path,
    profile_name: &str,
    key: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .add_disabled_env(&profile_name, key)
            .map_err(map_profile_error)
    })
}

pub fn update_disabled_env(
    config_path: &Path,
    profile_name: &str,
    current_key: &str,
    key: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .update_disabled_env(&profile_name, current_key, key)
            .map_err(map_profile_error)
    })
}

pub fn delete_disabled_env(
    config_path: &Path,
    profile_name: &str,
    key: &str,
) -> Result<ProfileSnapshot, ProfileServiceError> {
    let profile_name = validate_profile_name(profile_name)?.to_string();
    mutate_profile_settings(config_path, Some(&profile_name), |settings| {
        settings
            .profiles
            .remove_disabled_env(&profile_name, key)
            .map_err(map_profile_error)
    })
}

#[cfg(test)]
mod tests {
    use gwt_config::{Profile, Settings};

    use super::*;

    fn config_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("config.toml")
    }

    fn profile<'a>(snapshot: &'a ProfileSnapshot, name: &str) -> &'a ProfileView {
        snapshot
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .expect("profile present")
    }

    #[test]
    fn add_profile_persists_and_returns_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(&dir);

        let snapshot = add_profile(&path, "dev", "Development").expect("add profile");

        assert_eq!(snapshot.active, "default");
        assert_eq!(snapshot.selected, "dev");
        assert_eq!(profile(&snapshot, "dev").description, "Development");

        let stored = Settings::load_from_path(&path).expect("stored config");
        assert!(stored.profiles.get("dev").is_some());
    }

    #[test]
    fn switch_profile_persists_active_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(&dir);
        let mut settings = Settings::default();
        settings.profiles.add(Profile::new("dev")).unwrap();
        settings.save(&path).unwrap();

        let snapshot = switch_profile(&path, "dev").expect("switch profile");

        assert_eq!(snapshot.active, "dev");
        assert!(profile(&snapshot, "dev").active);
        let stored = Settings::load_from_path(&path).expect("stored config");
        assert_eq!(stored.profiles.active.as_deref(), Some("dev"));
    }

    #[test]
    fn default_profile_rename_and_delete_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(&dir);
        Settings::default().save(&path).unwrap();

        assert!(matches!(
            update_profile(&path, "default", "renamed", "Nope"),
            Err(ProfileServiceError::Protected(_))
        ));
        assert!(matches!(
            delete_profile(&path, "default"),
            Err(ProfileServiceError::Protected(_))
        ));
    }

    #[test]
    fn env_and_disabled_env_edits_update_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(&dir);
        let mut settings = Settings::default();
        settings.profiles.add(Profile::new("dev")).unwrap();
        settings.save(&path).unwrap();

        set_env_var(&path, "dev", "API_URL", "https://example.test").expect("set env");
        add_disabled_env(&path, "dev", "PATH").expect("disable env");
        let snapshot = load_profile_snapshot(&path, Some("dev")).expect("snapshot");
        let dev = profile(&snapshot, "dev");

        assert!(dev.env_vars.iter().any(|entry| {
            entry.key == "API_URL"
                && entry.value == "https://example.test"
                && entry.source == ProfileEnvVarSource::Profile
        }));
        assert!(dev.disabled_env.iter().any(|entry| entry == "PATH"));
        assert!(dev.merged_env.iter().any(|entry| {
            entry.key == "API_URL"
                && entry.value == "https://example.test"
                && entry.source == ProfileEnvVarSource::Profile
        }));
        assert!(!dev.merged_env.iter().any(|entry| entry.key == "PATH"));
    }
}
