//! Machine-wide agent admission preference and one-time project migration.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::issue_monitor::{
    IssueMonitorMaxActiveChange, IssueMonitorMaxActiveChangeSource, IssueMonitorMaxActiveMode,
    IssueMonitorPrefs,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub mode: IssueMonitorMaxActiveMode,
    pub manual_limit: Option<usize>,
    pub last_change: Option<IssueMonitorMaxActiveChange>,
}

#[derive(Serialize, Deserialize)]
struct AppliedControl {
    change: IssueMonitorMaxActiveChange,
    previous_policy: Policy,
}

#[derive(Serialize, Deserialize)]
struct StoredPolicy {
    #[serde(flatten)]
    policy: Policy,
    // Actual control identities survive project receipt/ACK failures. These
    // rare operator writes cannot be identified by wall-clock timestamps.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    applied_controls: BTreeMap<String, AppliedControl>,
}

pub fn path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-settings.json")
}

/// Read the authoritative preference without creating files or migrating.
pub fn load() -> io::Result<Option<Policy>> {
    load_stored().map(|stored| stored.map(|stored| stored.policy))
}

fn load_stored() -> io::Result<Option<StoredPolicy>> {
    let content = match fs::read_to_string(path()) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let stored: StoredPolicy = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match (stored.policy.mode, stored.policy.manual_limit) {
        (IssueMonitorMaxActiveMode::Auto, None)
        | (IssueMonitorMaxActiveMode::Manual, Some(1..)) => Ok(Some(stored)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid machine agent policy",
        )),
    }
}

/// Initialize once under the same cross-process lock used by explicit changes.
pub fn ensure_migrated() -> io::Result<Policy> {
    if let Some(policy) = load()? {
        return Ok(policy);
    }
    let _lock = lock()?;
    if let Some(policy) = load()? {
        return Ok(policy);
    }
    let policy = legacy_policy()?;
    save(&StoredPolicy {
        applied_controls: BTreeMap::new(),
        policy: policy.clone(),
    })?;
    Ok(policy)
}

pub fn set(
    max_active_agents_override: Option<usize>,
    source: IssueMonitorMaxActiveChangeSource,
    changed_at: &str,
) -> io::Result<Policy> {
    set_with_control_id(
        max_active_agents_override,
        source,
        changed_at,
        &uuid::Uuid::new_v4().to_string(),
    )
}

/// Apply a durable control once, even when a project receipt must be retried.
pub fn set_with_control_id(
    max_active_agents_override: Option<usize>,
    source: IssueMonitorMaxActiveChangeSource,
    changed_at: &str,
    control_id: &str,
) -> io::Result<Policy> {
    let change = IssueMonitorMaxActiveChange {
        changed_at: changed_at.to_string(),
        source,
        max_active_agents_override,
    };
    validate_change(&change)?;
    let _lock = lock()?;
    let stored = load_stored()?.unwrap_or(StoredPolicy {
        policy: Policy {
            mode: IssueMonitorMaxActiveMode::Auto,
            manual_limit: None,
            last_change: None,
        },
        applied_controls: BTreeMap::new(),
    });
    apply_unlocked(stored, &change, control_id)
}

/// Check the observed project and machine configuration at the machine write
/// boundary. A replay uses its original machine inputs while retaining today's
/// machine policy; project inputs must still match that original observation.
pub(crate) fn checked_update(
    prefs: &IssueMonitorPrefs,
    expected_token: Option<&str>,
    change: Option<&IssueMonitorMaxActiveChange>,
    control_id: &str,
) -> io::Result<Policy> {
    if let Some(change) = change {
        validate_change(change)?;
    }
    ensure_migrated()?;
    let _lock = lock()?;
    let stored = load_stored()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "machine agent settings disappeared",
        )
    })?;
    let mut observed = prefs.clone();
    let comparison_policy = if let Some(change) = change {
        if let Some(applied) = stored.applied_controls.get(control_id) {
            validate_replay(applied, change)?;
            &applied.previous_policy
        } else {
            &stored.policy
        }
    } else {
        &stored.policy
    };
    crate::issue_monitor::apply_machine_policy_to_prefs(&mut observed, comparison_policy);
    crate::issue_monitor::check_expected_issue_monitor_config_token(&observed, expected_token)?;
    match change {
        Some(change) => apply_unlocked(stored, change, control_id),
        None => Ok(stored.policy),
    }
}

fn validate_change(change: &IssueMonitorMaxActiveChange) -> io::Result<()> {
    if change.max_active_agents_override == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "manual limit must be positive",
        ));
    }
    Ok(())
}

fn validate_replay(
    applied: &AppliedControl,
    change: &IssueMonitorMaxActiveChange,
) -> io::Result<()> {
    if applied.change != *change {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "machine agent control ID was already used for a different change",
        ));
    }
    Ok(())
}

fn apply_unlocked(
    mut stored: StoredPolicy,
    change: &IssueMonitorMaxActiveChange,
    control_id: &str,
) -> io::Result<Policy> {
    if let Some(applied) = stored.applied_controls.get(control_id) {
        validate_replay(applied, change)?;
        // A prior rename may have succeeded before its directory sync failed.
        // Make the current policy durable before acknowledging that retry.
        save(&stored)?;
        return Ok(stored.policy);
    }
    stored.applied_controls.insert(
        control_id.to_string(),
        AppliedControl {
            change: change.clone(),
            previous_policy: stored.policy.clone(),
        },
    );
    stored.policy = Policy {
        mode: if change.max_active_agents_override.is_some() {
            IssueMonitorMaxActiveMode::Manual
        } else {
            IssueMonitorMaxActiveMode::Auto
        },
        manual_limit: change.max_active_agents_override,
        last_change: Some(change.clone()),
    };
    save(&stored)?;
    Ok(stored.policy)
}

/// Capture legacy preferences before canonical project saves remove their keys.
pub(crate) fn migration_before_project_save(project_path: &Path) -> io::Result<()> {
    if project_path
        .strip_prefix(gwt_core::paths::gwt_projects_dir())
        .is_ok_and(|relative| {
            relative.components().count() == 3
                && relative.ends_with("project-state/issue-monitor.json")
        })
    {
        ensure_migrated()?;
        let _lock = lock()?;
        confirm_durable()?;
    }
    Ok(())
}

fn confirm_durable() -> io::Result<()> {
    let path = path();
    // A previous migration may have renamed successfully but failed its parent
    // sync. Confirm the current file under the policy lock before legacy keys
    // disappear, without changing its bytes or modification timestamp.
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?
        .sync_all()?;
    #[cfg(test)]
    if std::env::var_os("GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE")
        .is_some_and(|target| Path::new(&target) == path)
    {
        let fail_once = path.with_extension("parent-sync-fail-once");
        if fail_once.exists() {
            fs::remove_file(fail_once)?;
            return Err(io::Error::other(
                "injected machine agent settings parent sync failure",
            ));
        }
    }
    #[cfg(unix)]
    fs::File::open(path.parent().expect("machine settings parent"))?.sync_all()?;
    // Windows cannot open a directory with std::fs::File; the file itself is
    // synced above, matching the existing durable writer's compatibility path.
    Ok(())
}

fn lock() -> io::Result<gwt_core::operation_deadline::NamedFileLock> {
    let path = path();
    let lock_path = path.with_extension("lock");
    gwt_config::home::assert_test_path(&path);
    gwt_config::home::assert_test_path(&lock_path);
    fs::create_dir_all(path.parent().expect("machine settings parent"))?;
    gwt_core::operation_deadline::NamedFileLock::acquire(&lock_path, "machine agent settings")
}

fn save(stored: &StoredPolicy) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(stored).map_err(io::Error::other)?;
    crate::issue_monitor::durable_atomic_write(&path(), &bytes)
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LegacyPrefs {
    max_active_agents: Option<usize>,
    max_active_agents_mode: Option<IssueMonitorMaxActiveMode>,
    max_active_last_change: Option<serde_json::Value>,
}

fn legacy_policy() -> io::Result<Policy> {
    let entries = match fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut selected = None;
    for entry in entries.into_iter().flatten() {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join("project-state/issue-monitor.json");
        let Some((legacy, modified)) = legacy_project(&path)? else {
            continue;
        };
        if legacy.max_active_agents_mode.is_none() && legacy.max_active_agents.is_none() {
            continue;
        }
        let mode = legacy
            .max_active_agents_mode
            .unwrap_or(IssueMonitorMaxActiveMode::Manual);
        let changed_at = legacy
            .max_active_last_change
            .as_ref()
            .and_then(|change| change.get("changed_at")?.as_str())
            .and_then(|changed_at| chrono::DateTime::parse_from_rfc3339(changed_at).ok())
            .map(|time| time.with_timezone(&chrono::Utc))
            .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from(modified));
        let rank = (mode == IssueMonitorMaxActiveMode::Auto, changed_at, path);
        if selected.as_ref().is_none_or(|(current, _)| rank > *current) {
            selected = Some((
                rank,
                Policy {
                    mode,
                    manual_limit: (mode == IssueMonitorMaxActiveMode::Manual)
                        .then_some(legacy.max_active_agents.unwrap_or(1).max(1)),
                    last_change: legacy
                        .max_active_last_change
                        .and_then(|change| serde_json::from_value(change).ok()),
                },
            ));
        }
    }
    Ok(selected.map(|(_, policy)| policy).unwrap_or(Policy {
        mode: IssueMonitorMaxActiveMode::Auto,
        manual_limit: None,
        last_change: None,
    }))
}

fn legacy_project(path: &Path) -> io::Result<Option<(LegacyPrefs, SystemTime)>> {
    let committed = path.with_extension("committed.json");
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return read_legacy(&committed),
        Err(error) => return Err(error),
    };
    match serde_json::from_str(&content) {
        Ok(prefs) => Ok(Some((prefs, fs::metadata(path)?.modified()?))),
        Err(error) => read_legacy(&committed)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, error))
            .map(Some),
    }
}

fn read_legacy(path: &Path) -> io::Result<Option<(LegacyPrefs, SystemTime)>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let prefs = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some((prefs, fs::metadata(path)?.modified()?)))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    use serde_json::json;

    use super::*;

    fn legacy(project: &str, value: serde_json::Value) -> PathBuf {
        let path = gwt_core::paths::gwt_projects_dir()
            .join(project)
            .join("project-state/issue-monitor.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, value.to_string()).unwrap();
        path
    }

    #[test]
    fn migration_prefers_any_auto_including_committed_partial_preferences() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        assert_eq!(load().unwrap(), None);
        assert!(!home.path().join("machine-state").exists());
        legacy(
            "manual",
            json!({
                "max_active_agents": 8,
                "max_active_last_change": {
                    "changed_at": "2099-01-01T00:00:00Z",
                    "source": "gui", "max_active_agents_override": 8
                }
            }),
        );
        let auto = legacy("auto", json!({"max_active_agents_mode": "auto"}));
        fs::rename(&auto, auto.with_extension("committed.json")).unwrap();
        fs::write(&auto, "{").unwrap();

        let policy = ensure_migrated().unwrap();
        assert_eq!(policy.mode, IssueMonitorMaxActiveMode::Auto);
        assert_eq!(policy.manual_limit, None);
        assert_eq!(policy.last_change, None);
        assert_eq!(load().unwrap(), Some(policy));
        assert_eq!(fs::read_to_string(auto).unwrap(), "{");
    }

    #[test]
    fn manual_migration_orders_parsed_timestamps_metadata_fallback_and_path_ties() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        for (project, limit, changed_at, modified_hour) in [
            ("a", 2, "2025-01-01T10:00:00+09:00", 9),
            ("b", 4, "2025-01-01T05:00:00Z", 9),
            ("c", 6, "invalid", 4),
            ("d", 7, "2025-01-01T05:00:00Z", 0),
        ] {
            let change = if project == "d" {
                json!({"changed_at": changed_at})
            } else {
                json!({
                    "changed_at": changed_at, "source": "json_operation",
                    "max_active_agents_override": limit
                })
            };
            let path = legacy(
                project,
                json!({
                    "max_active_agents": limit,
                    "max_active_agents_mode": "manual",
                    "max_active_last_change": change
                }),
            );
            let start = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .timestamp() as u64;
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(
                    SystemTime::UNIX_EPOCH + Duration::from_secs(start + modified_hour * 3600),
                ))
                .unwrap();
        }

        let policy = ensure_migrated().unwrap();
        assert_eq!(policy.mode, IssueMonitorMaxActiveMode::Manual);
        assert_eq!(policy.manual_limit, Some(7));
        assert_eq!(policy.last_change, None);
    }

    #[test]
    fn established_policy_ignores_projects_and_records_explicit_shared_changes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        migration_before_project_save(&home.path().join("custom/issue-monitor.json")).unwrap();
        assert_eq!(load().unwrap(), None);
        let first = legacy("first", json!({"max_active_agents": 2}));
        fs::create_dir_all(path().parent().unwrap()).unwrap();
        let _fault = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            path(),
        );
        let fail_once = path().with_extension("parent-sync-fail-once");
        fs::write(&fail_once, "fail").unwrap();
        let error = migration_before_project_save(&first).unwrap_err();
        assert!(error.to_string().contains("parent sync failure"));
        fs::write(&fail_once, "fail").unwrap();
        let error = migration_before_project_save(&first).unwrap_err();
        assert!(error.to_string().contains("parent sync failure"));
        migration_before_project_save(&first).unwrap();
        assert_eq!(load().unwrap().unwrap().manual_limit, Some(2));
        fs::write(&first, "invalid legacy").unwrap();

        let changed = set(
            Some(4),
            IssueMonitorMaxActiveChangeSource::Gui,
            "verbatim timestamp",
        )
        .unwrap();
        let second = legacy("second", json!({"max_active_agents_mode": "auto"}));
        migration_before_project_save(&second).unwrap();
        assert_eq!(ensure_migrated().unwrap(), changed);
        assert_eq!(load().unwrap(), Some(changed));
        assert_eq!(
            load().unwrap().unwrap().last_change,
            Some(IssueMonitorMaxActiveChange {
                changed_at: "verbatim timestamp".to_string(),
                source: IssueMonitorMaxActiveChangeSource::Gui,
                max_active_agents_override: Some(4),
            })
        );
        let auto = set(
            None,
            IssueMonitorMaxActiveChangeSource::JsonOperation,
            "later",
        )
        .unwrap();
        assert_eq!(auto.mode, IssueMonitorMaxActiveMode::Auto);
        assert_eq!(auto.manual_limit, None);
        assert_eq!(auto.last_change.unwrap().max_active_agents_override, None);
        assert_eq!(
            set(Some(0), IssueMonitorMaxActiveChangeSource::Gui, "bad")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        fs::write(path(), "{}").unwrap();
        assert_eq!(load().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            ensure_migrated().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn replay_of_an_applied_change_preserves_a_later_machine_setting() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = IssueMonitorMaxActiveChangeSource::JsonOperation;
        set_with_control_id(Some(5), source, "same-second", "a").unwrap();
        let _fault = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            path(),
        );
        let fail_once = path().with_extension("parent-sync-fail-once");
        fs::write(&fail_once, "fail").unwrap();
        let error = set_with_control_id(Some(3), source, "same-second", "b").unwrap_err();
        assert!(error.to_string().contains("parent sync failure"));
        let later = set_with_control_id(Some(5), source, "same-second", "c").unwrap();
        assert_eq!(later.manual_limit, Some(5));

        fs::write(&fail_once, "fail").unwrap();
        let error = set_with_control_id(Some(3), source, "same-second", "b").unwrap_err();
        assert!(error.to_string().contains("parent sync failure"));
        let replay = set_with_control_id(Some(3), source, "same-second", "b").unwrap();

        assert_eq!(replay, later);
        assert_eq!(load().unwrap(), Some(later));
        let error = set_with_control_id(Some(9), source, "same-second", "b").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn checked_update_rejects_stale_machine_tokens_and_accepts_latest_observation() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let prefs = IssueMonitorPrefs::default();
        let mut observed = prefs.clone();
        crate::issue_monitor::apply_machine_policy_to_prefs(
            &mut observed,
            &ensure_migrated().unwrap(),
        );
        let stale_token = observed.config_token();
        let current = set_with_control_id(
            Some(3),
            IssueMonitorMaxActiveChangeSource::Gui,
            "other-project",
            "other",
        )
        .unwrap();
        let change = IssueMonitorMaxActiveChange {
            changed_at: "requested".to_string(),
            source: IssueMonitorMaxActiveChangeSource::JsonOperation,
            max_active_agents_override: Some(7),
        };
        assert_eq!(
            checked_update(&prefs, Some(&stale_token), Some(&change), "request")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            checked_update(&prefs, Some(&stale_token), None, "read")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(load().unwrap(), Some(current.clone()));

        crate::issue_monitor::apply_machine_policy_to_prefs(&mut observed, &current);
        let latest_token = observed.config_token();
        let bytes = fs::read(path()).unwrap();
        assert_eq!(
            checked_update(&prefs, Some(&latest_token), None, "read").unwrap(),
            current
        );
        assert_eq!(fs::read(path()).unwrap(), bytes);
        let updated =
            checked_update(&prefs, Some(&latest_token), Some(&change), "request").unwrap();
        assert_eq!(updated.manual_limit, Some(7));
        assert_eq!(updated.last_change, Some(change));
    }

    #[test]
    fn checked_retry_validates_original_project_and_intent_without_reverting_machine() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let prefs = IssueMonitorPrefs::default();
        let mut observed = prefs.clone();
        crate::issue_monitor::apply_machine_policy_to_prefs(
            &mut observed,
            &ensure_migrated().unwrap(),
        );
        let original_token = observed.config_token();
        let change = IssueMonitorMaxActiveChange {
            changed_at: "first".to_string(),
            source: IssueMonitorMaxActiveChangeSource::JsonOperation,
            max_active_agents_override: Some(7),
        };
        let _fault = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            path(),
        );
        fs::write(path().with_extension("parent-sync-fail-once"), "fail").unwrap();
        assert!(
            checked_update(&prefs, Some(&original_token), Some(&change), "a")
                .unwrap_err()
                .to_string()
                .contains("parent sync failure")
        );
        let later = set_with_control_id(
            Some(3),
            IssueMonitorMaxActiveChangeSource::Gui,
            "later",
            "b",
        )
        .unwrap();
        assert_eq!(
            checked_update(&prefs, Some(&original_token), Some(&change), "a").unwrap(),
            later
        );

        let mut changed_project = prefs.clone();
        changed_project.enabled = !prefs.enabled;
        assert_eq!(
            checked_update(&changed_project, Some(&original_token), Some(&change), "a")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let divergent = IssueMonitorMaxActiveChange {
            source: IssueMonitorMaxActiveChangeSource::Gui,
            ..change.clone()
        };
        assert_eq!(
            checked_update(&prefs, Some(&original_token), Some(&divergent), "a")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            checked_update(&prefs, Some(&original_token), None, "a")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(load().unwrap(), Some(later));
    }
}
