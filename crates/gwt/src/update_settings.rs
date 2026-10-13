//! App-wide automatic-update preference and one-time project migration.

use std::{fs, io};

use gwt_config::Settings;
use serde::Deserialize;

/// Reading a status projection never migrates or writes configuration.
pub fn configured() -> io::Result<Option<bool>> {
    let path = gwt_core::paths::gwt_config_path();
    if !path.try_exists()? {
        return Ok(None);
    }
    Settings::load_from_path(&path)
        .map(|settings| settings.auto_apply_updates)
        .map_err(io::Error::other)
}

pub fn enabled() -> bool {
    match configured() {
        Ok(value) => value.unwrap_or(false),
        Err(error) => {
            tracing::warn!(target: "gwt::update", %error, "automatic update settings unreadable; treating as OFF");
            false
        }
    }
}

/// None initializes from all legacy projects. Some is an explicit operator
/// value and takes precedence over migration, including explicit OFF.
pub fn set(value: Option<bool>) -> io::Result<bool> {
    let path = gwt_core::paths::gwt_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &path.with_extension("update-settings.lock"),
        "automatic update settings",
    )?;
    let enabled =
        Settings::persist_auto_apply_updates(&path, |configured| match (value, configured) {
            (Some(enabled), _) | (None, Some(enabled)) => Ok(enabled),
            (None, None) => {
                legacy_projects_enabled().map_err(|error| gwt_config::ConfigError::ParseError {
                    reason: error.to_string(),
                })
            }
        })
        .map_err(io::Error::other)?;
    Ok(enabled)
}

pub fn migrate() -> io::Result<bool> {
    if let Some(enabled) = configured()? {
        return Ok(enabled);
    }
    set(None)
}

fn legacy_projects_enabled() -> io::Result<bool> {
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct LegacyPrefs {
        autonomous_mode: bool,
        auto_apply_updates: Option<bool>,
    }
    let entries = match fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut enabled = false;
    for entry in entries {
        let path = entry?.path().join("project-state/issue-monitor.json");
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let prefs: LegacyPrefs = serde_json::from_str(&content)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        enabled |= prefs.auto_apply_updates.unwrap_or(prefs.autonomous_mode);
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    #[test]
    fn auto_apply_updates_global_setter_and_snapshot_use_scoped_home() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let path = gwt_core::paths::gwt_config_path();
        assert_eq!(path, home.path().join(".gwt/config.toml"));
        assert_eq!(super::configured().unwrap(), None);
        super::set(Some(true)).unwrap();
        assert_eq!(super::configured().unwrap(), Some(true));
        assert!(matches!(
            crate::system_settings::get_event(&path),
            crate::protocol::BackendEvent::SystemSettings {
                auto_apply_updates: true,
                ..
            }
        ));
    }
}
