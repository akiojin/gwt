use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use gwt_config::{Profile, Settings};

use crate::protocol::{ProfileEntryView, ProfileEnvEntryView, ProfileSnapshotView};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileServiceError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("duplicate entry: {0}")]
    Duplicate(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("profile not found: {0}")]
    NotFound(String),
}

impl From<gwt_config::ConfigError> for ProfileServiceError {
    fn from(value: gwt_config::ConfigError) -> Self {
        Self::Storage(value.to_string())
    }
}

pub fn config_path() -> Result<PathBuf, ProfileServiceError> {
    Settings::global_config_path().ok_or_else(|| {
        ProfileServiceError::Storage(
            "unable to resolve home directory (`~/.gwt/config.toml`); set HOME/USERPROFILE before managing profiles"
                .to_string(),
        )
    })
}

pub fn load_snapshot(
    selected_profile: Option<&str>,
) -> Result<ProfileSnapshotView, ProfileServiceError> {
    let path = config_path()?;
    load_snapshot_at(&path, selected_profile)
}

pub fn load_snapshot_at(
    path: &Path,
    selected_profile: Option<&str>,
) -> Result<ProfileSnapshotView, ProfileServiceError> {
    let mut settings = load_settings_or_default(path)?;
    snapshot_from_settings(&mut settings, selected_profile, std::env::vars())
}

pub fn create_profile(name: &str) -> Result<(), ProfileServiceError> {
    let path = config_path()?;
    create_profile_at(&path, name)
}

pub fn create_profile_at(path: &Path, name: &str) -> Result<(), ProfileServiceError> {
    let mut settings = load_settings_or_default(path)?;
    let name = require_non_empty("profile name", name)?;
    if settings
        .profiles
        .profiles
        .iter()
        .any(|profile| profile.name == name)
    {
        return Err(ProfileServiceError::Duplicate(name.to_string()));
    }
    settings
        .profiles
        .add(Profile::new(name))
        .map_err(ProfileServiceError::InvalidInput)?;
    settings.save(path).map_err(ProfileServiceError::from)
}

pub fn switch_active_profile(name: &str) -> Result<(), ProfileServiceError> {
    let path = config_path()?;
    switch_active_profile_at(&path, name)
}

pub fn switch_active_profile_at(path: &Path, name: &str) -> Result<(), ProfileServiceError> {
    let mut settings = load_settings_or_default(path)?;
    let name = require_non_empty("profile name", name)?;
    if settings
        .profiles
        .profiles
        .iter()
        .all(|profile| profile.name != name)
    {
        return Err(ProfileServiceError::NotFound(name.to_string()));
    }
    settings
        .profiles
        .switch(name)
        .map_err(ProfileServiceError::InvalidInput)?;
    settings.save(path).map_err(ProfileServiceError::from)
}

pub fn save_profile(
    current_name: &str,
    name: &str,
    description: &str,
    env_vars: &[ProfileEnvEntryView],
    disabled_env: &[String],
) -> Result<(), ProfileServiceError> {
    let path = config_path()?;
    save_profile_at(
        &path,
        current_name,
        name,
        description,
        env_vars,
        disabled_env,
    )
}

pub fn save_profile_at(
    path: &Path,
    current_name: &str,
    name: &str,
    description: &str,
    env_vars: &[ProfileEnvEntryView],
    disabled_env: &[String],
) -> Result<(), ProfileServiceError> {
    let mut settings = load_settings_or_default(path)?;
    let current_name = require_non_empty("profile name", current_name)?;
    if settings
        .profiles
        .profiles
        .iter()
        .all(|profile| profile.name != current_name)
    {
        return Err(ProfileServiceError::NotFound(current_name.to_string()));
    }

    let name = require_non_empty("profile name", name)?;
    if current_name == "default" && name != "default" {
        return Err(ProfileServiceError::InvalidInput(
            "default profile cannot be renamed".to_string(),
        ));
    }
    if current_name != name
        && settings
            .profiles
            .profiles
            .iter()
            .any(|profile| profile.name == name)
    {
        return Err(ProfileServiceError::Duplicate(name.to_string()));
    }

    let env_vars = normalize_env_vars(env_vars)?;
    let disabled_env = normalize_disabled_env(disabled_env)?;

    settings
        .profiles
        .update_profile(current_name, name, description.trim())
        .map_err(ProfileServiceError::InvalidInput)?;

    let Some(profile) = settings
        .profiles
        .profiles
        .iter_mut()
        .find(|profile| profile.name == name)
    else {
        return Err(ProfileServiceError::NotFound(name.to_string()));
    };
    profile.env_vars = env_vars;
    profile.disabled_env = disabled_env;

    settings.save(path).map_err(ProfileServiceError::from)
}

pub fn delete_profile(name: &str) -> Result<(), ProfileServiceError> {
    let path = config_path()?;
    delete_profile_at(&path, name)
}

pub fn delete_profile_at(path: &Path, name: &str) -> Result<(), ProfileServiceError> {
    let mut settings = load_settings_or_default(path)?;
    let name = require_non_empty("profile name", name)?;
    if name == "default" {
        return Err(ProfileServiceError::InvalidInput(
            "default profile cannot be deleted".to_string(),
        ));
    }
    if settings
        .profiles
        .profiles
        .iter()
        .all(|profile| profile.name != name)
    {
        return Err(ProfileServiceError::NotFound(name.to_string()));
    }
    settings
        .profiles
        .delete_profile(name)
        .map_err(ProfileServiceError::InvalidInput)?;
    settings.save(path).map_err(ProfileServiceError::from)
}

fn load_settings_or_default(path: &Path) -> Result<Settings, ProfileServiceError> {
    if path.exists() {
        Settings::load_from_path(path).map_err(ProfileServiceError::from)
    } else {
        Ok(Settings::default())
    }
}

fn require_non_empty<'a>(field: &str, value: &'a str) -> Result<&'a str, ProfileServiceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(ProfileServiceError::InvalidInput(format!(
            "{field} cannot be empty"
        )))
    } else {
        Ok(trimmed)
    }
}

fn normalize_env_vars(
    env_vars: &[ProfileEnvEntryView],
) -> Result<std::collections::HashMap<String, String>, ProfileServiceError> {
    let mut map = std::collections::HashMap::new();
    for entry in env_vars {
        let key = require_non_empty("environment variable key", &entry.key)?;
        if map.contains_key(key) {
            return Err(ProfileServiceError::Duplicate(key.to_string()));
        }
        map.insert(key.to_string(), entry.value.clone());
    }
    Ok(map)
}

fn normalize_disabled_env(disabled_env: &[String]) -> Result<Vec<String>, ProfileServiceError> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for entry in disabled_env {
        let key = require_non_empty("disabled environment variable key", entry)?;
        if !seen.insert(key.to_string()) {
            return Err(ProfileServiceError::Duplicate(key.to_string()));
        }
        values.push(key.to_string());
    }
    values.sort();
    Ok(values)
}

fn snapshot_from_settings<I>(
    settings: &mut Settings,
    selected_profile: Option<&str>,
    base_env: I,
) -> Result<ProfileSnapshotView, ProfileServiceError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let active_profile = settings.profiles.normalize_active_profile().name;
    let selected_profile = selected_profile
        .filter(|name| settings.profiles.get(name).is_some())
        .unwrap_or(active_profile.as_str())
        .to_string();

    let Some(selected) = settings.profiles.get(&selected_profile) else {
        return Err(ProfileServiceError::NotFound(selected_profile));
    };

    let profiles = settings
        .profiles
        .profiles
        .iter()
        .map(|profile| ProfileEntryView {
            name: profile.name.clone(),
            description: profile.description.clone(),
            env_vars: sorted_env_entries(&profile.env_vars),
            disabled_env: sorted_disabled_env(&profile.disabled_env),
            is_default: profile.name == "default",
            is_active: profile.name == active_profile,
        })
        .collect();

    let merged_preview = selected
        .merged_env_pairs(base_env)
        .into_iter()
        .map(mask_preview_entry)
        .collect();

    Ok(ProfileSnapshotView {
        active_profile,
        selected_profile: selected.name.clone(),
        profiles,
        merged_preview,
    })
}

fn sorted_env_entries(
    env_vars: &std::collections::HashMap<String, String>,
) -> Vec<ProfileEnvEntryView> {
    env_vars
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(key, value)| ProfileEnvEntryView { key, value })
        .collect()
}

fn sorted_disabled_env(disabled_env: &[String]) -> Vec<String> {
    let mut values = disabled_env.to_vec();
    values.sort();
    values
}

fn mask_preview_entry((key, value): (String, String)) -> ProfileEnvEntryView {
    let value = if is_sensitive_env_key(&key) {
        "<redacted>".to_string()
    } else {
        value
    };
    ProfileEnvEntryView { key, value }
}

fn is_sensitive_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.starts_with("AWS_")
        || [
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASS",
            "API_KEY",
            "APIKEY",
            "CREDENTIAL",
            "PRIVATE_KEY",
            "ACCESS_KEY",
        ]
        .iter()
        .any(|marker| key.contains(marker))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn config_path(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("config.toml")
    }

    #[test]
    fn load_snapshot_at_missing_file_returns_default_profile_snapshot() {
        let temp = tempdir().expect("tempdir");
        let snapshot = load_snapshot_at(&config_path(&temp), None).expect("default snapshot");

        assert_eq!(snapshot.active_profile, "default");
        assert_eq!(snapshot.selected_profile, "default");
        assert_eq!(snapshot.profiles.len(), 1);
        assert!(snapshot.profiles[0].is_default);
        assert!(snapshot.profiles[0].is_active);
    }

    #[test]
    fn create_switch_and_save_profile_persists_metadata_and_preview_contract() {
        let temp = tempdir().expect("tempdir");
        let path = config_path(&temp);

        create_profile_at(&path, "dev").expect("create profile");
        switch_active_profile_at(&path, "dev").expect("switch active profile");
        save_profile_at(
            &path,
            "dev",
            "dev",
            "Repo development",
            &[
                ProfileEnvEntryView {
                    key: "API_KEY".to_string(),
                    value: "override".to_string(),
                },
                ProfileEnvEntryView {
                    key: "CI".to_string(),
                    value: "0".to_string(),
                },
            ],
            &["SECRET".to_string()],
        )
        .expect("save profile");

        let mut settings = load_settings_or_default(&path).expect("load settings");
        let snapshot = snapshot_from_settings(
            &mut settings,
            Some("dev"),
            [
                ("SECRET".to_string(), "hidden".to_string()),
                ("PATH".to_string(), "/bin".to_string()),
            ],
        )
        .expect("snapshot");

        assert_eq!(snapshot.active_profile, "dev");
        assert_eq!(snapshot.selected_profile, "dev");
        assert!(snapshot.profiles.iter().any(|profile| {
            profile.name == "dev"
                && profile.description == "Repo development"
                && profile.disabled_env == vec!["SECRET".to_string()]
                && profile.env_vars
                    == vec![
                        ProfileEnvEntryView {
                            key: "API_KEY".to_string(),
                            value: "override".to_string(),
                        },
                        ProfileEnvEntryView {
                            key: "CI".to_string(),
                            value: "0".to_string(),
                        },
                    ]
        }));
        assert!(snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "API_KEY" && entry.value == "<redacted>"));
        assert!(!snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "SECRET"));
        assert!(snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "PATH" && entry.value == "/bin"));
    }

    #[test]
    fn save_profile_at_rejects_duplicate_env_keys() {
        let temp = tempdir().expect("tempdir");
        let path = config_path(&temp);
        create_profile_at(&path, "dev").expect("create profile");

        let error = save_profile_at(
            &path,
            "dev",
            "dev",
            "",
            &[
                ProfileEnvEntryView {
                    key: "API_KEY".to_string(),
                    value: "first".to_string(),
                },
                ProfileEnvEntryView {
                    key: "API_KEY".to_string(),
                    value: "second".to_string(),
                },
            ],
            &[],
        )
        .expect_err("duplicate env key should fail");

        assert_eq!(error, ProfileServiceError::Duplicate("API_KEY".to_string()));
    }

    #[test]
    fn delete_profile_at_rejects_default_and_resets_active_to_default() {
        let temp = tempdir().expect("tempdir");
        let path = config_path(&temp);
        create_profile_at(&path, "dev").expect("create profile");
        switch_active_profile_at(&path, "dev").expect("switch active");

        let error =
            delete_profile_at(&path, "default").expect_err("default profile delete should fail");
        assert_eq!(
            error,
            ProfileServiceError::InvalidInput("default profile cannot be deleted".to_string())
        );

        delete_profile_at(&path, "dev").expect("delete profile");
        let snapshot = load_snapshot_at(&path, Some("dev")).expect("load snapshot");
        assert_eq!(snapshot.active_profile, "default");
        assert_eq!(snapshot.selected_profile, "default");
    }

    #[test]
    fn save_profile_at_renames_active_profile() {
        let temp = tempdir().expect("tempdir");
        let path = config_path(&temp);
        create_profile_at(&path, "dev").expect("create profile");
        switch_active_profile_at(&path, "dev").expect("switch active");

        save_profile_at(&path, "dev", "review", "Review profile", &[], &[])
            .expect("rename profile");

        let snapshot = load_snapshot_at(&path, Some("review")).expect("load renamed snapshot");
        assert_eq!(snapshot.active_profile, "review");
        assert_eq!(snapshot.selected_profile, "review");
        assert!(snapshot
            .profiles
            .iter()
            .any(|profile| profile.name == "review" && profile.is_active));
    }

    #[test]
    fn snapshot_from_settings_masks_sensitive_merged_preview_values() {
        let mut settings = Settings::default();
        let snapshot = snapshot_from_settings(
            &mut settings,
            Some("default"),
            [
                ("GITHUB_TOKEN".to_string(), "ghp_secret".to_string()),
                (
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                    "aws-secret".to_string(),
                ),
                ("PATH".to_string(), "/usr/bin".to_string()),
            ],
        )
        .expect("snapshot");

        assert!(snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "GITHUB_TOKEN" && entry.value == "<redacted>"));
        assert!(snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "AWS_SECRET_ACCESS_KEY" && entry.value == "<redacted>"));
        assert!(snapshot
            .merged_preview
            .iter()
            .any(|entry| entry.key == "PATH" && entry.value == "/usr/bin"));
    }
}
