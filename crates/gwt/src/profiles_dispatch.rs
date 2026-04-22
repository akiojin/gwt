//! WebSocket dispatch helpers for Profile window requests.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use gwt_config::Settings;

use crate::{
    profiles_service::{
        add_disabled_env, add_profile, delete_disabled_env, delete_env_var, delete_profile,
        load_profile_snapshot, set_env_var, switch_profile, update_disabled_env, update_env_var,
        update_profile, ProfileServiceError,
    },
    protocol::{BackendEvent, ProfileErrorCode},
};

static CONFIG_PATH_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

pub fn set_config_path_override_for_tests(path: PathBuf) {
    let _ = CONFIG_PATH_OVERRIDE.set(path);
}

pub fn config_path() -> Result<PathBuf, ProfileServiceError> {
    if let Some(path) = CONFIG_PATH_OVERRIDE.get() {
        return Ok(path.clone());
    }
    Settings::global_config_path().ok_or_else(|| {
        ProfileServiceError::Storage(
            "unable to resolve home directory (`~/.gwt/config.toml`); \
             set HOME/USERPROFILE before managing profiles"
                .to_string(),
        )
    })
}

fn with_config_path<F>(id: String, f: F) -> BackendEvent
where
    F: FnOnce(&Path, String) -> BackendEvent,
{
    match config_path() {
        Ok(path) => f(&path, id),
        Err(err) => error_to_event(id, err),
    }
}

pub fn error_to_event(id: String, err: ProfileServiceError) -> BackendEvent {
    use ProfileServiceError as E;
    let code = match &err {
        E::Storage(_) => ProfileErrorCode::Storage,
        E::InvalidInput(_) => ProfileErrorCode::InvalidInput,
        E::NotFound(_) => ProfileErrorCode::NotFound,
        E::Duplicate(_) => ProfileErrorCode::Duplicate,
        E::Protected(_) => ProfileErrorCode::Protected,
    };
    BackendEvent::ProfileError {
        id,
        code,
        message: err.to_string(),
    }
}

fn snapshot_result(
    id: String,
    result: Result<crate::profiles_service::ProfileSnapshot, ProfileServiceError>,
) -> BackendEvent {
    match result {
        Ok(snapshot) => BackendEvent::ProfileSnapshot { id, snapshot },
        Err(err) => error_to_event(id, err),
    }
}

pub fn list_event(id: String, selected_profile: Option<String>) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, load_profile_snapshot(path, selected_profile.as_deref()))
    })
}

pub fn switch_event(id: String, profile_name: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, switch_profile(path, &profile_name))
    })
}

pub fn add_profile_event(id: String, name: String, description: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, add_profile(path, &name, &description))
    })
}

pub fn update_profile_event(
    id: String,
    current_name: String,
    name: String,
    description: String,
) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, update_profile(path, &current_name, &name, &description))
    })
}

pub fn delete_profile_event(id: String, profile_name: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, delete_profile(path, &profile_name))
    })
}

pub fn set_env_var_event(
    id: String,
    profile_name: String,
    key: String,
    value: String,
) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, set_env_var(path, &profile_name, &key, &value))
    })
}

pub fn update_env_var_event(
    id: String,
    profile_name: String,
    current_key: String,
    key: String,
    value: String,
) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(
            id,
            update_env_var(path, &profile_name, &current_key, &key, &value),
        )
    })
}

pub fn delete_env_var_event(id: String, profile_name: String, key: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, delete_env_var(path, &profile_name, &key))
    })
}

pub fn add_disabled_env_event(id: String, profile_name: String, key: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, add_disabled_env(path, &profile_name, &key))
    })
}

pub fn update_disabled_env_event(
    id: String,
    profile_name: String,
    current_key: String,
    key: String,
) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(
            id,
            update_disabled_env(path, &profile_name, &current_key, &key),
        )
    })
}

pub fn delete_disabled_env_event(id: String, profile_name: String, key: String) -> BackendEvent {
    with_config_path(id, |path, id| {
        snapshot_result(id, delete_disabled_env(path, &profile_name, &key))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_to_event_preserves_code_per_variant() {
        let cases = [
            (
                ProfileServiceError::Storage("x".into()),
                ProfileErrorCode::Storage,
            ),
            (
                ProfileServiceError::InvalidInput("x".into()),
                ProfileErrorCode::InvalidInput,
            ),
            (
                ProfileServiceError::NotFound("x".into()),
                ProfileErrorCode::NotFound,
            ),
            (
                ProfileServiceError::Duplicate("x".into()),
                ProfileErrorCode::Duplicate,
            ),
            (
                ProfileServiceError::Protected("x".into()),
                ProfileErrorCode::Protected,
            ),
        ];
        for (err, expected) in cases {
            match error_to_event("profile-1".to_string(), err) {
                BackendEvent::ProfileError { id, code, .. } => {
                    assert_eq!(id, "profile-1");
                    assert_eq!(code, expected);
                }
                other => panic!("expected ProfileError, got {other:?}"),
            }
        }
    }
}
