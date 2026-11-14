//! PM window activity comes from its own successful operations, never prose.

use std::{io, path::Path};

use serde::{Deserialize, Serialize};

use crate::pm_registry::{self, PmPrefs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmActivity {
    Starting,
    Reconciling,
    DrivingPr,
    RegisteringIssue,
    Deciding,
    Waiting,
}

impl PmActivity {
    pub fn from_operation(operation: &str) -> Option<Self> {
        match operation {
            "issue.monitor.status" | "pm.plan.status" | "pm.plan.apply" => Some(Self::Reconciling),
            "pr.current"
            | "pr.list"
            | "pr.view"
            | "pr.checks"
            | "pr.create"
            | "pr.ready"
            | "pr.edit"
            | "pr.update_branch"
            | "pr.auto_merge.enable" => Some(Self::DrivingPr),
            "issue.create" | "issue.spec.create" => Some(Self::RegisteringIssue),
            "board.post" | "pm.approval.request" | "pm.operation.execute" => Some(Self::Deciding),
            "daemon.subscribe" | "issue.monitor.wait" => Some(Self::Waiting),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmActivityRecord {
    pub session_id: String,
    pub activity: PmActivity,
    pub updated_at_ms: u64,
}

pub(crate) fn record_operation(
    prefs_path: &Path,
    session_id: &str,
    operation: &str,
    updated_at_ms: u64,
) -> io::Result<bool> {
    let Some(activity) = PmActivity::from_operation(operation) else {
        return Ok(false);
    };
    pm_registry::record_pm_title_activity(prefs_path, session_id, activity, updated_at_ms)
}

/// An absent or previous PTY's activity always returns the PM's initial label.
pub fn for_window(
    prefs: &PmPrefs,
    session_id: Option<&str>,
    runtime_started_at_ms: Option<u64>,
) -> Option<PmActivity> {
    let session_id = session_id?;
    if !prefs
        .registration
        .as_ref()
        .is_some_and(|registration| registration.session_id == session_id)
    {
        return None;
    }
    Some(
        prefs
            .title_activity
            .as_ref()
            .filter(|record| {
                record.session_id == session_id
                    && runtime_started_at_ms.is_some_and(|started| record.updated_at_ms >= started)
            })
            .map_or(PmActivity::Starting, |record| record.activity),
    )
}

/// Best-effort display metadata cannot change the operation's result.
pub(crate) fn record_successful_operation(repo: &Path, operation: &str) {
    if PmActivity::from_operation(operation).is_none() {
        return;
    }
    let Ok(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV) else {
        return;
    };
    if session_id.is_empty() {
        return;
    }
    let direct_path = repo
        .ancestors()
        .find_map(pm_registry::pm_worktree_store_dir)
        .map(|store| store.join("project-state/pm.json"))
        .unwrap_or_else(|| pm_registry::pm_prefs_path_for_repo_path(repo));
    let path = if pm_registry::session_is_registered_pm(&direct_path, &session_id) {
        Some(direct_path)
    } else {
        pm_registry::pm_repository_key(repo).and_then(|key| {
            pm_registry::pm_registrations_for_repository(&key)
                .into_iter()
                .find(|record| record.registration.session_id == session_id)
                .map(|record| record.prefs_path)
        })
    };
    let Some(path) = path else {
        return;
    };
    let updated_at_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    match record_operation(&path, &session_id, operation, updated_at_ms) {
        Ok(true) => crate::daemon_runtime::notify_pm_status_changed(),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "could not update PM title activity"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_registry::{self, PmPrefs, PmRegistration};

    fn registration(session_id: &str) -> PmRegistration {
        PmRegistration {
            session_id: session_id.into(),
            agent_id: "codex".into(),
            worktree_path: "/tmp/pm-worktree".into(),
            created_at: None,
            consecutive_crashes: 0,
            next_not_before: None,
        }
    }

    #[test]
    fn pm_title_activity_comes_only_from_known_operations() {
        for (operation, expected) in [
            ("issue.monitor.status", PmActivity::Reconciling),
            ("pr.list", PmActivity::DrivingPr),
            ("pr.ready", PmActivity::DrivingPr),
            ("issue.create", PmActivity::RegisteringIssue),
            ("board.post", PmActivity::Deciding),
            ("daemon.subscribe", PmActivity::Waiting),
        ] {
            assert_eq!(PmActivity::from_operation(operation), Some(expected));
        }
        for text in [
            "workspace.update",
            "pm.status.set",
            "another agent failed: workspace.ensure refused",
            "pr.unrecognized",
        ] {
            assert_eq!(PmActivity::from_operation(text), None);
        }
    }

    #[test]
    fn pm_title_activity_rejects_other_agents_and_pre_restore_operations() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pm.json");
        pm_registry::save_pm_prefs(
            &path,
            &PmPrefs {
                registration: Some(registration("resident-pm")),
                ..Default::default()
            },
        )
        .unwrap();
        let initial = std::fs::read(&path).unwrap();
        assert!(!record_operation(&path, "worker", "pr.ready", 2000).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), initial);
        assert!(!record_operation(&path, "resident-pm", "workspace.update", 2000).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), initial);

        assert!(record_operation(&path, "resident-pm", "pr.ready", 2000).unwrap());
        let prefs = pm_registry::load_pm_prefs(&path).unwrap();
        assert_eq!(for_window(&prefs, Some("worker"), Some(1000)), None);
        assert_eq!(
            for_window(&prefs, Some("resident-pm"), Some(1000)),
            Some(PmActivity::DrivingPr)
        );
        assert_eq!(
            for_window(&prefs, Some("resident-pm"), Some(3000)),
            Some(PmActivity::Starting)
        );
        assert_eq!(
            for_window(&prefs, Some("resident-pm"), None),
            Some(PmActivity::Starting)
        );

        pm_registry::try_register_pm(&path, registration("successor-pm"), |_| false).unwrap();
        let prefs = pm_registry::load_pm_prefs(&path).unwrap();
        assert_eq!(
            for_window(&prefs, Some("successor-pm"), Some(1000)),
            Some(PmActivity::Starting)
        );
    }

    #[test]
    fn pm_title_activity_is_wire_only_and_never_restored_from_disk() {
        let mut window = crate::persistence::default_workspace_state()
            .windows
            .remove(0);
        window.pm_activity = Some(PmActivity::Reconciling);
        let wire = serde_json::to_value(&window).unwrap();
        assert_eq!(wire["pm_activity"], "reconciling");
        let restored: crate::PersistedWindowState = serde_json::from_value(wire).unwrap();
        assert_eq!(restored.pm_activity, None);
    }

    #[test]
    fn pm_title_activity_dispatch_records_success_only_for_the_registered_pm() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "resident-pm");
        let _runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let path = pm_registry::pm_prefs_path_for_repo_path(&repo);
        pm_registry::save_pm_prefs(
            &path,
            &PmPrefs {
                registration: Some(registration("resident-pm")),
                ..Default::default()
            },
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(repo);
        assert_eq!(
            crate::cli::dispatch_json_input(
                &mut env,
                "gwtd",
                r#"{"schema_version":1,"operation":"pr.current","params":{}}"#,
            ),
            0
        );
        assert_eq!(
            pm_registry::load_pm_prefs(&path)
                .unwrap()
                .title_activity
                .unwrap()
                .activity,
            PmActivity::DrivingPr
        );

        let recorded = std::fs::read(&path).unwrap();
        assert_ne!(
            crate::cli::dispatch_json_input(
                &mut env,
                "gwtd",
                r#"{"schema_version":1,"operation":"pr.checks","params":{"number":999}}"#,
            ),
            0
        );
        assert_eq!(std::fs::read(&path).unwrap(), recorded);

        let _worker = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "worker");
        assert_eq!(
            crate::cli::dispatch_json_input(
                &mut env,
                "gwtd",
                r#"{"schema_version":1,"operation":"pr.current","params":{}}"#,
            ),
            0
        );
        assert_eq!(std::fs::read(&path).unwrap(), recorded);
    }

    #[test]
    fn pm_title_activity_is_projected_by_both_window_producers() {
        for producer in [
            include_str!("app_runtime/mod.rs"),
            include_str!("project_runtime/owner.rs"),
        ] {
            assert!(producer.contains("pm_activity::for_window("));
        }
        assert!(include_str!("app_runtime/runtime_events.rs")
            .contains("PM_STATUS_CHANGED_SOURCE_EVENT"));
    }
}
