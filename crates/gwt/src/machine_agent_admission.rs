//! Serialize machine-wide Monitor admission through durable child publication.

use std::{fs, path::Path, time::Duration};

use gwt_core::operation_deadline::{NamedFileLock, ScopedOperationDeadline};

use crate::{
    issue_monitor::IssueMonitorMaxActiveMode,
    session_inventory::{MachineSessionInventory, MachineSessionRole},
};

/// Call from a blocking worker. The callback must publish the child's durable
/// runtime identity before returning success; no reservation survives this call.
pub fn with_admission<T>(
    sessions_dir: &Path,
    spawn: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    with_inventory(
        || crate::session_inventory::observe_machine_sessions(sessions_dir),
        spawn,
    )
}

fn with_inventory<T>(
    observe: impl FnOnce() -> MachineSessionInventory,
    spawn: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _deadline = ScopedOperationDeadline::enter(
        gwt_core::operation_deadline::now() + Duration::from_secs(120),
    );
    let path = gwt_core::paths::gwt_home().join("machine-state/agent-admission.lock");
    gwt_config::home::assert_test_path(&path);
    fs::create_dir_all(path.parent().expect("machine admission parent"))
        .map_err(|error| format!("Machine agent admission lock is unavailable: {error}"))?;
    let _guard = NamedFileLock::acquire(&path, "machine agent admission")
        .map_err(|error| format!("Machine agent admission is busy or unavailable: {error}; retry after the current launch completes"))?;
    // A prepared GUI launch may have observed an older preference. Read the
    // authoritative setting only after obtaining the machine-wide spawn fence.
    let policy = crate::machine_agent_settings::ensure_migrated()
        .map_err(|error| format!("Machine agent admission cannot read its policy: {error}; repair machine agent settings before retrying"))?;
    if policy.mode == IssueMonitorMaxActiveMode::Manual {
        let limit = policy
            .manual_limit
            .ok_or("Machine agent Manual limit is unavailable")?;
        let inventory = observe();
        if !inventory.uncertainties.is_empty() {
            let reasons = inventory
                .uncertainties
                .iter()
                .map(|uncertainty| uncertainty.reason.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("Machine agent Manual admission cannot verify live workers ({reasons}). Wait for a fresh live observation or switch to Auto."));
        }
        let workers = inventory
            .sessions
            .iter()
            .filter(|session| session.role != MachineSessionRole::Pm)
            .count();
        if workers >= limit {
            return Err(format!("Machine agent Manual limit reached: {workers}/{limit} live non-PM workers across projects. Wait for a worker to finish, raise the Manual limit, or switch to Auto."));
        }
    }
    spawn()
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier,
        },
    };

    use gwt_core::{operation_deadline::NamedFileLock, test_support::ScopedGwtHome};

    use super::*;
    use crate::{
        issue_monitor::IssueMonitorMaxActiveChangeSource,
        session_inventory::{
            MachineSessionObservation, MachineSessionRole, SessionObservation,
            SessionObservationUncertainty,
        },
    };

    fn worker(project: &str, role: MachineSessionRole) -> MachineSessionObservation {
        MachineSessionObservation {
            session: SessionObservation {
                session_id: project.into(),
                issue_number: None,
                execution_binding_missing: false,
                agent_id: "unknown".into(),
                worktree_path: PathBuf::from(project),
                worktree_exists: true,
                host_pid: 1,
                child_pid: 2,
                child_started_at: 3,
                started_at: String::new(),
                launch_origin: Default::default(),
                restore_source_session_id: None,
            },
            project_root: PathBuf::from(project),
            repo_hash: None,
            role,
            monitor_owned: false,
            resident_memory_bytes: Some(1),
        }
    }

    fn manual(limit: usize) {
        crate::machine_agent_settings::set(
            Some(limit),
            IssueMonitorMaxActiveChangeSource::Gui,
            "now",
        )
        .unwrap();
    }

    fn assert_admission_locked() {
        let path = gwt_core::paths::gwt_home().join("machine-state/agent-admission.lock");
        assert_eq!(
            NamedFileLock::try_acquire(&path, "probe")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock,
            "admission must retain the shared OS lock through observation and spawn"
        );
    }

    #[test]
    fn projects_compete_for_the_final_manual_slot_under_the_shared_file_lock() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        manual(1);
        let workers = AtomicUsize::new(0);
        let start = Arc::new(Barrier::new(3));
        let results = std::thread::scope(|scope| {
            let mut threads = Vec::new();
            for project in ["project-a", "project-b"] {
                let start = Arc::clone(&start);
                let home = home.path();
                let workers = &workers;
                threads.push(scope.spawn(move || {
                    let _home = ScopedGwtHome::set(home);
                    start.wait();
                    with_inventory(
                        || {
                            assert_admission_locked();
                            MachineSessionInventory {
                                sessions: (0..workers.load(Ordering::SeqCst))
                                    .map(|_| {
                                        worker("other-project", MachineSessionRole::Implementation)
                                    })
                                    .collect(),
                                ..Default::default()
                            }
                        },
                        || {
                            assert_admission_locked();
                            workers.fetch_add(1, Ordering::SeqCst);
                            Ok(project)
                        },
                    )
                }));
            }
            start.wait();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(workers.load(Ordering::SeqCst), 1);
        let refusal = results.into_iter().find_map(Result::err).unwrap();
        assert!(refusal.contains("1/1"), "{refusal}");
    }

    #[test]
    fn manual_counts_all_non_pm_workers_and_failed_spawn_releases_the_slot() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let inventory = || MachineSessionInventory {
            sessions: vec![
                worker("pm", MachineSessionRole::Pm),
                worker("review", MachineSessionRole::Review),
                worker("unknown", MachineSessionRole::Implementation),
            ],
            ..Default::default()
        };
        manual(3);
        assert_eq!(
            with_inventory(inventory, || {
                assert_admission_locked();
                Err::<(), _>("spawn failed".into())
            }),
            Err("spawn failed".into())
        );
        drop(
            NamedFileLock::try_acquire(
                &gwt_core::paths::gwt_home().join("machine-state/agent-admission.lock"),
                "failed spawn recovery",
            )
            .expect("a failed callback releases the shared lock"),
        );
        assert_eq!(
            with_inventory(inventory, || Ok("recovered")).unwrap(),
            "recovered"
        );
        manual(2);
        let refusal = with_inventory(inventory, || -> Result<(), String> {
            panic!("full cap must refuse before spawn")
        })
        .unwrap_err();
        assert!(refusal.contains("2/2"), "{refusal}");
    }

    #[test]
    fn uncertain_manual_observation_refuses_but_auto_preserves_existing_admission() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        manual(4);
        let refusal = with_inventory(
            || MachineSessionInventory {
                uncertainties: vec![SessionObservationUncertainty {
                    runtime_path: PathBuf::from("unknown.json"),
                    session_id: None,
                    reason: "child_identity_missing".into(),
                }],
                ..Default::default()
            },
            || -> Result<(), String> { panic!("unknown liveness must refuse before spawn") },
        )
        .unwrap_err();
        assert!(
            refusal.contains("child_identity_missing") && refusal.contains("Auto"),
            "{refusal}"
        );
        crate::machine_agent_settings::set(None, IssueMonitorMaxActiveChangeSource::Gui, "now")
            .unwrap();
        assert_eq!(
            with_inventory(
                || panic!("Auto keeps measured admission upstream"),
                || Ok("allowed")
            )
            .unwrap(),
            "allowed"
        );
    }
}
