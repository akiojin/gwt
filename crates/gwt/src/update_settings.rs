//! App-wide automatic-update preference and one-time project migration.

use std::{fs, io, path::Path};

use gwt_config::Settings;
use serde::{Deserialize, Serialize};

use crate::issue_monitor::{
    issue_monitor_prefs_path_for_repo_path, load_issue_monitor_prefs, IssueMonitorPrefs,
    IssueMonitorUpdateDrain, IssueMonitorUpdateDrainReason,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub auto_apply_updates: Option<bool>,
    pub auto_apply_updates_effective: bool,
    pub update_drain: Option<IssueMonitorUpdateDrain>,
}

/// The global preference and this project's drain are independent of monitor
/// admission settings. Status reads never migrate or write either source.
pub fn status(project_root: &Path) -> io::Result<Status> {
    let auto_apply_updates = configured()?;
    let mut update_drain =
        load_issue_monitor_prefs(&issue_monitor_prefs_path_for_repo_path(project_root))?
            .update_drain;
    if let Some(drain) = update_drain.as_mut() {
        let wait = gwt_core::update::load_update_wait_observation(project_root).filter(|wait| {
            wait.version == drain.version
                && matches!((
                    chrono::DateTime::parse_from_rfc3339(&wait.observed_at),
                    chrono::DateTime::parse_from_rfc3339(&drain.since),
                ), (Ok(observed), Ok(since)) if observed >= since)
        });
        if let Some(blockers) = wait.as_ref().and_then(|wait| {
            serde_json::from_str::<Vec<crate::update_drain::UpdateBlocker>>(&wait.reason).ok()
        }) {
            drain.blocking = blockers;
        }
    }
    Ok(Status {
        auto_apply_updates,
        auto_apply_updates_effective: auto_apply_updates.unwrap_or(false),
        update_drain,
    })
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LegacyPrefs {
    autonomous_mode: bool,
    auto_apply_updates: Option<bool>,
}

/// Preserve an old drain's origin before project preferences discard the old
/// update override. Explicit legacy OFF identifies a manual-click drain even
/// when the project's autonomous mode was ON.
pub(crate) fn decode_project_prefs(content: &str) -> Result<IssueMonitorPrefs, serde_json::Error> {
    decode_stored_project_prefs(content).map(|(prefs, _)| prefs)
}

fn decode_stored_project_prefs(
    content: &str,
) -> Result<(IssueMonitorPrefs, Option<bool>), serde_json::Error> {
    // A serde flatten buffer loses JSON's numeric map-key conversion, which
    // monitor launch maps require. Keep JSON's own value deserializer instead.
    let mut value: serde_json::Value = serde_json::from_str(content)?;
    let legacy_override = value
        .get("auto_apply_updates")
        .cloned()
        .map(serde_json::from_value::<Option<bool>>)
        .transpose()?
        .flatten();
    if let Some(continuity) = value.get("last_auto_max_active_agents").cloned() {
        value["max_active_agents"] = continuity;
    }
    let mut prefs: IssueMonitorPrefs = serde_json::from_value(value)?;
    if let Some(drain) = prefs.update_drain.as_mut().filter(|drain| {
        drain.reason == IssueMonitorUpdateDrainReason::Auto && drain.automatic.is_none()
    }) {
        drain.automatic = legacy_override;
    }
    Ok((prefs, legacy_override))
}

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
    set_with_legacy_reset(value, None)
}

fn set_with_legacy_reset(
    value: Option<bool>,
    untrusted_project: Option<&Path>,
) -> io::Result<bool> {
    let path = gwt_core::paths::gwt_config_path();
    let lock_path = path.with_extension("update-settings.lock");
    gwt_config::home::assert_test_path(&path);
    gwt_config::home::assert_test_path(&lock_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &lock_path,
        "automatic update settings",
    )?;
    let enabled =
        Settings::persist_auto_apply_updates(&path, |configured| match (value, configured) {
            (Some(enabled), _) | (None, Some(enabled)) => Ok(enabled),
            (None, None) => legacy_projects_enabled(untrusted_project).map_err(|error| {
                gwt_config::ConfigError::ParseError {
                    reason: error.to_string(),
                }
            }),
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

/// A daemon can save project preferences before the application starts.
/// Migrate while the old bytes still exist so that save cannot erase an
/// explicit override or change the autonomous-mode fallback first.
pub(crate) fn migrate_before_project_save(
    path: &Path,
    recovery_without_floor: bool,
) -> io::Result<()> {
    // Custom snapshots are not part of the legacy managed-project corpus.
    if !path
        .strip_prefix(gwt_core::paths::gwt_projects_dir())
        .is_ok_and(|relative| {
            relative.components().count() == 3
                && relative.ends_with("project-state/issue-monitor.json")
        })
    {
        return Ok(());
    }
    if (path.try_exists()? || path.with_extension("committed.json").try_exists()?)
        && configured()?.is_none()
    {
        if recovery_without_floor {
            // Exclude only this untrusted project, rather than infer ON from
            // its replacement baseline. Trusted legacy ON elsewhere survives.
            set_with_legacy_reset(None, Some(path))?;
        } else {
            migrate()?;
        }
    }
    Ok(())
}

fn legacy_projects_enabled(untrusted_project: Option<&Path>) -> io::Result<bool> {
    let entries = match fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut enabled = false;
    for entry in entries {
        let path = entry?.path().join("project-state/issue-monitor.json");
        if untrusted_project == Some(path.as_path()) {
            continue;
        }
        if let Some(prefs) = legacy_project_prefs(&path)? {
            enabled |= prefs.auto_apply_updates.unwrap_or(prefs.autonomous_mode);
        }
    }
    Ok(enabled)
}

fn legacy_project_prefs(path: &Path) -> io::Result<Option<LegacyPrefs>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return committed_legacy_project_prefs(path);
        }
        Err(error) => return Err(error),
    };
    match serde_json::from_str(&content) {
        Ok(prefs) => Ok(Some(prefs)),
        Err(error) if error.is_syntax() || error.is_eof() => committed_legacy_project_prefs(path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, error))
            .map(Some),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

fn committed_legacy_project_prefs(path: &Path) -> io::Result<Option<LegacyPrefs>> {
    let content = match fs::read_to_string(path.with_extension("committed.json")) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (prefs, auto_apply_updates) = decode_stored_project_prefs(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(LegacyPrefs {
        autonomous_mode: prefs.autonomous_mode,
        auto_apply_updates,
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn auto_apply_updates_migrates_legacy_on_once_and_preserves_explicit_global_off() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let projects = gwt_core::paths::gwt_projects_dir();
        for (name, prefs) in [
            ("attended", serde_json::json!({"auto_apply_updates": false})),
            ("autonomous", serde_json::json!({"autonomous_mode": true})),
        ] {
            let dir = projects.join(name).join("project-state");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("issue-monitor.json"), prefs.to_string()).unwrap();
        }
        let config_path = gwt_core::paths::gwt_config_path();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "# preserved comment\n[future]\nvalue = 7\n").unwrap();

        assert!(
            super::migrate().unwrap(),
            "any legacy ON migrates to global ON"
        );
        let config = fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("# preserved comment") && config.contains("[future]"));
        assert_eq!(super::configured().unwrap(), Some(true));
        super::set(Some(false)).unwrap();
        assert!(
            !super::migrate().unwrap(),
            "legacy ON cannot re-enable explicit OFF"
        );
        assert_eq!(super::configured().unwrap(), Some(false));
    }

    #[test]
    fn project_save_migrates_legacy_preference_before_removing_it() {
        for (legacy_override, migrated) in [(Some(false), false), (None, true)] {
            let home = tempfile::tempdir().unwrap();
            let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let project_root = home.path().join("repo");
            fs::create_dir_all(&project_root).unwrap();
            let path = crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&project_root);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut stored = serde_json::to_value(crate::issue_monitor::IssueMonitorPrefs {
                autonomous_mode: true,
                ..Default::default()
            })
            .unwrap();
            if let Some(value) = legacy_override {
                stored["auto_apply_updates"] = serde_json::json!(value);
            }
            fs::write(&path, stored.to_string()).unwrap();
            let mut prefs = crate::issue_monitor::load_issue_monitor_prefs(&path).unwrap();
            assert_eq!(super::configured().unwrap(), None, "loads stay read-only");
            let custom_path = home.path().join("custom-monitor.json");
            fs::write(&custom_path, stored.to_string()).unwrap();
            crate::issue_monitor::save_issue_monitor_prefs(&custom_path, &prefs).unwrap();
            assert_eq!(
                super::configured().unwrap(),
                None,
                "custom saves stay project-local"
            );
            prefs.enabled = true;
            if legacy_override.is_none() {
                prefs.autonomous_mode = false;
            }
            crate::issue_monitor::save_issue_monitor_prefs(&path, &prefs).unwrap();

            assert_eq!(super::configured().unwrap(), Some(migrated));
            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert!(saved.get("auto_apply_updates").is_none());
            assert!(saved["enabled"].as_bool().unwrap());
            assert_eq!(super::migrate().unwrap(), migrated);

            super::set(Some(!migrated)).unwrap();
            fs::write(&path, stored.to_string()).unwrap();
            crate::issue_monitor::save_issue_monitor_prefs(&path, &prefs).unwrap();
            assert_eq!(super::configured().unwrap(), Some(!migrated));
        }
    }

    #[test]
    fn status_reads_global_preference_and_project_drain_without_writes() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project_root = home.path().join("repo");
        fs::create_dir_all(&project_root).unwrap();
        let prefs_path =
            crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&project_root);
        let mut monitor = crate::issue_monitor::IssueMonitorState::new(Default::default());
        monitor.set_update_drain_with_origin(
            crate::issue_monitor::IssueMonitorUpdateDrainReason::Auto,
            "9.110.0",
            "2026-10-05T19:10:00Z",
            Some(true),
        );
        crate::issue_monitor::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
        let before = fs::read(&prefs_path).unwrap();
        gwt_core::update::log_update_event(
            "pending_waiting",
            &[
                ("project_root", project_root.to_str().unwrap()),
                ("version", "9.110.0"),
                ("observed_at", "2026-10-05T19:11:00Z"),
                (
                    "reason",
                    r#"[{"kind":"pending_acquire_claim","issue_number":4271}]"#,
                ),
            ],
        );

        let status = super::status(&project_root).unwrap();
        assert_eq!(status.auto_apply_updates, None);
        assert!(!status.auto_apply_updates_effective);
        let mut expected_drain = monitor.update_drain().unwrap().clone();
        expected_drain.blocking =
            vec![crate::update_drain::UpdateBlocker::PendingAcquireClaim { issue_number: 4271 }];
        assert_eq!(status.update_drain, Some(expected_drain));
        assert!(!gwt_core::paths::gwt_config_path().exists());
        assert_eq!(fs::read(&prefs_path).unwrap(), before);

        super::set(Some(true)).unwrap();
        let status = super::status(&project_root).unwrap();
        assert_eq!(status.auto_apply_updates, Some(true));
        assert!(status.auto_apply_updates_effective);
        super::set(Some(false)).unwrap();
        let status = super::status(&project_root).unwrap();
        assert_eq!(status.auto_apply_updates, Some(false));
        assert!(!status.auto_apply_updates_effective);
    }

    #[cfg(unix)]
    #[test]
    fn escaping_update_settings_paths_are_rejected_before_lock_creation() {
        let outside = std::path::Path::new("/__gwt_protected_test_home__/update-settings.toml");
        assert!(!outside.parent().unwrap().exists());
        for filename in ["config.toml", "config.update-settings.lock"] {
            let home = tempfile::tempdir().unwrap();
            let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let directory = home.path().join(".gwt");
            std::fs::create_dir_all(&directory).unwrap();
            std::os::unix::fs::symlink(outside, directory.join(filename)).unwrap();
            assert!(std::panic::catch_unwind(|| super::set(Some(true))).is_err());
            if filename == "config.toml" {
                assert!(!directory.join("config.update-settings.lock").exists());
            } else {
                assert!(!directory.join("config.toml").exists());
            }
        }
    }

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
