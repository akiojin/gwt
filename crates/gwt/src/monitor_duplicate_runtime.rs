//! Monitor implementation runtime reconciliation, independent of GUI dispatch.

use crate::cli::execution_state::{
    diagnose_owner, monitor_runtime_uncertainty_affects_issue, stop_monitor_duplicate,
    ExecutionControlStatus, ExecutionOwnerKey, ExecutionOwnerKind, MonitorDuplicateRuntimeProof,
    MonitorDuplicateStopOutcome,
};
use gwt_terminal::PtyHandle;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

/// Captured with the PTY registration, never resolved later by a reused window id.
#[derive(Clone)]
pub struct MonitorRuntimeRegistration {
    pub window_id: String,
    pub session_id: String,
    pub issue_number: u64,
    pub worktree_path: PathBuf,
    pub project_root: PathBuf,
    pub sessions_dir: PathBuf,
    pub incarnation: u64,
    pub review_dispatch: bool,
}

#[derive(Clone)]
pub struct RegisteredMonitorRuntime {
    pub registration: MonitorRuntimeRegistration,
    pub handle: Arc<PtyHandle>,
}

/// The host owns the handles; this worker only retains exact captured references.
/// No client connection, canvas snapshot or GUI event dispatch is needed.
pub fn spawn(
    open_projects: Arc<RwLock<BTreeMap<PathBuf, PathBuf>>>,
    snapshot: impl Fn() -> Option<Vec<RegisteredMonitorRuntime>> + Send + 'static,
) {
    let result = std::thread::Builder::new()
        .name("gwt-monitor-runtime".into())
        .spawn(move || {
            let mut retained = BTreeMap::new();
            let mut projects = BTreeMap::<PathBuf, PathBuf>::new();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                // Remember projects independently of PTYs. In particular, a new
                // host must purge old occupancy before it can admit its first PTY.
                if let Ok(open) = open_projects.read() {
                    projects.extend(
                        open.iter()
                            .map(|(root, sessions)| (root.clone(), sessions.clone())),
                    );
                }
                let Some(current) = snapshot() else { continue };
                for entry in current {
                    let registration = &entry.registration;
                    projects.insert(
                        registration.project_root.clone(),
                        registration.sessions_dir.clone(),
                    );
                    retained.insert(
                        (registration.window_id.clone(), registration.incarnation),
                        entry,
                    );
                }
                // Removal from the GUI registry is not exit evidence. Keep counting
                // a detached child until its captured PTY confirms it has exited.
                retained.retain(|_, entry| !matches!(entry.handle.try_wait(), Ok(Some(_))));
                reconcile_projects(&projects, &retained.values().cloned().collect::<Vec<_>>());
            }
        });
    if let Err(error) = result {
        tracing::error!(target: "gwt.monitor.runtime", %error, "failed to start Monitor runtime reconciliation");
    }
}

// Project roots can be separate worktrees of one repository. Their prefs and
// host census key are shared, so publish one complete census per prefs path.
fn reconcile_projects(projects: &BTreeMap<PathBuf, PathBuf>, entries: &[RegisteredMonitorRuntime]) {
    let mut repositories = BTreeMap::new();
    for (root, sessions) in projects {
        repositories
            .entry(crate::issue_monitor_prefs_path_for_repo_path(root))
            .or_insert((root, sessions));
    }
    for (prefs_path, (project, sessions)) in repositories {
        let entries = entries
            .iter()
            .filter(|entry| {
                crate::issue_monitor_prefs_path_for_repo_path(&entry.registration.project_root)
                    == prefs_path
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Err(error) = reconcile_project(project, sessions, &entries) {
            tracing::warn!(target: "gwt.monitor.runtime", project = %project.display(), %error, "runtime reconciliation failed; retaining unknown authority");
        }
    }
}

fn counts(entries: &[RegisteredMonitorRuntime]) -> BTreeMap<u64, usize> {
    let mut counts = BTreeMap::new();
    for entry in entries
        .iter()
        .filter(|entry| !entry.registration.review_dispatch)
    {
        if !matches!(entry.handle.try_wait(), Ok(Some(_))) {
            *counts.entry(entry.registration.issue_number).or_default() += 1;
        }
    }
    counts
}

fn publish_counts(
    project_root: &Path,
    entries: &[RegisteredMonitorRuntime],
) -> io::Result<BTreeMap<u64, usize>> {
    let host_pid = std::process::id();
    let host_started_at = crate::process::host_process_start_time(host_pid)
        .ok_or_else(|| io::Error::other("Monitor host process identity unavailable"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(
        &crate::issue_monitor_prefs_path_for_repo_path(project_root),
        |prefs| {
            let dead_hosts = prefs
                .monitor_runtime_counts
                .values()
                .filter(|snapshot| {
                    crate::process::host_process_start_time(snapshot.host_pid)
                        .is_some_and(|started| started != snapshot.host_started_at)
                        || !crate::process::is_host_process_alive(snapshot.host_pid)
                })
                .map(|snapshot| (snapshot.host_pid, snapshot.host_started_at))
                .collect::<Vec<_>>();
            let mut monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            for (pid, started) in dead_hosts {
                monitor.record_monitor_runtime_counts(pid, started, BTreeMap::new(), &now);
            }
            let windows = entries
                .iter()
                .filter(|entry| {
                    !entry.registration.review_dispatch
                        && !matches!(entry.handle.try_wait(), Ok(Some(_)))
                })
                .map(|entry| {
                    (
                        entry.registration.window_id.clone(),
                        entry.registration.issue_number,
                    )
                })
                .collect();
            monitor.record_monitor_runtime_windows(
                host_pid,
                host_started_at,
                counts(entries),
                windows,
                &now,
            );
            *prefs = monitor.prefs();
            Ok(())
        },
    )?;
    let mut complete_counts = BTreeMap::new();
    for host in prefs.monitor_runtime_counts.values() {
        for (&issue, &count) in &host.counts {
            *complete_counts.entry(issue).or_default() += count;
        }
    }
    Ok(complete_counts)
}

fn park(project_root: &Path, issue_number: u64, reason: &str) -> io::Result<()> {
    crate::try_mutate_issue_monitor_prefs(
        &crate::issue_monitor_prefs_path_for_repo_path(project_root),
        |prefs| {
            let mut monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            monitor.escalate_to_needs_human(
                issue_number,
                crate::issue_monitor::NeedsHumanKind::UserChoiceRequired,
                format!(
                    "duplicate Monitor runtimes: {reason}; no automatic stop (Issue #4466 AC-5)"
                ),
            );
            *prefs = monitor.prefs();
            Ok(())
        },
    )?;
    Ok(())
}

fn runtime_proof(entry: &RegisteredMonitorRuntime) -> io::Result<MonitorDuplicateRuntimeProof> {
    let registration = &entry.registration;
    let session = gwt_agent::Session::load(
        &registration
            .sessions_dir
            .join(format!("{}.toml", registration.session_id)),
    )?;
    let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path_for_pid(
        &registration.sessions_dir,
        std::process::id(),
        &registration.session_id,
    ))?;
    if session.id != registration.session_id
        || session.linked_issue_number != Some(registration.issue_number)
        || dunce::canonicalize(&session.worktree_path)?
            != dunce::canonicalize(&registration.worktree_path)?
        || runtime.runtime_incarnation != Some(registration.incarnation)
        || runtime.child_pid.is_none()
        || runtime.child_pid != entry.handle.process_id()
    {
        return Err(io::Error::other(
            "captured PTY Session/runtime identity changed",
        ));
    }
    Ok(MonitorDuplicateRuntimeProof {
        session,
        host_pid: std::process::id(),
        runtime,
    })
}

fn reconcile_project(
    project_root: &Path,
    sessions_dir: &Path,
    entries: &[RegisteredMonitorRuntime],
) -> io::Result<()> {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + Duration::from_secs(5),
    );
    let complete_counts = publish_counts(project_root, entries)?;
    let inventory = crate::session_inventory::observe_sessions(project_root, sessions_dir);
    let mut groups = BTreeMap::<u64, Vec<&RegisteredMonitorRuntime>>::new();
    for entry in entries.iter().filter(|entry| {
        !entry.registration.review_dispatch && !matches!(entry.handle.try_wait(), Ok(Some(_)))
    }) {
        groups
            .entry(entry.registration.issue_number)
            .or_default()
            .push(entry);
    }
    for (issue_number, pair) in groups {
        // A review process is an independent dispatch, not a duplicate implementation.
        let live_implementations = inventory
            .sessions
            .iter()
            .filter(|observed| {
                observed.issue_number == Some(issue_number)
                    && !entries.iter().any(|entry| {
                        entry.registration.review_dispatch
                            && entry.registration.session_id == observed.session_id
                    })
            })
            .count();
        if pair.len() < 2
            && live_implementations < 2
            && complete_counts.get(&issue_number).copied().unwrap_or(0) < 2
        {
            continue;
        }
        let uncertain = inventory.uncertainties.iter().any(|uncertainty| {
            monitor_runtime_uncertainty_affects_issue(uncertainty, sessions_dir, issue_number)
        });
        if pair.len() != 2 || live_implementations != 2 || uncertain {
            park(
                project_root,
                issue_number,
                "the complete exact pair could not be established",
            )?;
            continue;
        }
        let proofs = match pair
            .iter()
            .map(|entry| runtime_proof(entry))
            .collect::<io::Result<Vec<_>>>()
        {
            Ok(proofs) => proofs,
            Err(error) => {
                park(project_root, issue_number, &error.to_string())?;
                continue;
            }
        };
        let kind = if proofs.iter().any(|proof| {
            proof
                .session
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.owner_kind == "spec")
        }) {
            ExecutionOwnerKind::Spec
        } else {
            ExecutionOwnerKind::Issue
        };
        let diagnosis = diagnose_owner(
            project_root,
            ExecutionOwnerKey {
                kind,
                number: issue_number,
            },
        );
        let holder = if diagnosis.ecr_status == Some(ExecutionControlStatus::Active)
            && diagnosis.holder_runtime.as_deref() == Some("live")
        {
            pair.iter().position(|entry| {
                Some(entry.registration.session_id.as_str())
                    == diagnosis.holder_session_id.as_deref()
            })
        } else {
            None
        };
        let Some(holder) = holder else {
            park(
                project_root,
                issue_number,
                "no unique live ECR holder could be identified",
            )?;
            continue;
        };
        let target = 1 - holder;
        let result = stop_monitor_duplicate(
            project_root,
            issue_number,
            sessions_dir,
            &proofs,
            &pair[target].registration.session_id,
            || {
                // Repair the projection while authority is leased, before kill can
                // emit a late WindowClosed/agent_failed for the discarded binding.
                let (_, rebound) = crate::try_mutate_issue_monitor_prefs(
                    &crate::issue_monitor_prefs_path_for_repo_path(project_root),
                    |prefs| {
                        let mut monitor = crate::IssueMonitorState::with_prefs(
                            crate::IssueMonitorConfig::default(),
                            prefs.clone(),
                        );
                        let rebound = monitor.bind_duplicate_runtime_holder(
                            issue_number,
                            &pair[target].registration.window_id,
                            &pair[holder].registration.window_id,
                        );
                        if rebound {
                            *prefs = monitor.prefs();
                        }
                        Ok(rebound)
                    },
                )?;
                if !rebound {
                    return Err(io::Error::other(
                        "Monitor binding changed before exact stop",
                    ));
                }
                crate::session_finalizer::kill_and_reap(
                    &pair[target].handle,
                    &pair[target].registration.window_id,
                )
            },
        );
        match result {
            Ok(MonitorDuplicateStopOutcome::Stopped) => {
                tracing::info!(target: "gwt.monitor.runtime", issue_number, session_id = %pair[target].registration.session_id, holder_session_id = %pair[holder].registration.session_id, "revoked exact nonholder Monitor runtime")
            }
            Ok(MonitorDuplicateStopOutcome::Refused { reason }) => {
                park(project_root, issue_number, &reason)?
            }
            Err(error) => park(project_root, issue_number, &error.to_string())?,
        }
    }
    publish_counts(project_root, entries).map(|_| ())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    struct PtyGuard(Arc<PtyHandle>);
    impl Drop for PtyGuard {
        fn drop(&mut self) {
            let _ = crate::session_finalizer::kill_and_reap(&self.0, "test-cleanup");
        }
    }

    fn runtime(
        project: &Path,
        sessions: &Path,
        session: &gwt_agent::Session,
        incarnation: u64,
    ) -> (RegisteredMonitorRuntime, PtyGuard) {
        let id = &session.id;
        let handle = Arc::new(
            PtyHandle::spawn(gwt_terminal::pty::SpawnConfig {
                command: "sleep".into(),
                args: vec!["60".into()],
                cols: 80,
                rows: 24,
                env: Default::default(),
                remove_env: vec![],
                cwd: Some(project.into()),
            })
            .unwrap(),
        );
        let mut sidecar = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
        sidecar.execution_identity =
            gwt_agent::SessionExecutionIdentity::from_session(session).unwrap();
        sidecar.runtime_incarnation = Some(incarnation);
        sidecar.host_started_at = crate::process::host_process_start_time(std::process::id());
        sidecar.child_pid = handle.process_id();
        sidecar.child_started_at = sidecar
            .child_pid
            .and_then(crate::process::host_process_start_time);
        sidecar
            .save(&gwt_agent::runtime_state_path_for_pid(
                sessions,
                std::process::id(),
                id,
            ))
            .unwrap();
        let entry = RegisteredMonitorRuntime {
            registration: MonitorRuntimeRegistration {
                window_id: format!("tab::{id}"),
                session_id: id.clone(),
                issue_number: 4466,
                worktree_path: project.into(),
                project_root: project.into(),
                sessions_dir: sessions.into(),
                incarnation,
                review_dispatch: false,
            },
            handle: handle.clone(),
        };
        (entry, PtyGuard(handle))
    }

    #[test]
    fn two_unbound_live_ptys_are_counted_and_parked_without_gui_dispatch() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let pair = ["first", "second"].map(|id| {
            let mut session = gwt_agent::Session::new(
                project.path(),
                "work/issue-4466",
                gwt_agent::AgentId::Codex,
            );
            session.id = id.into();
            session.linked_issue_number = Some(4466);
            session.launch_route = gwt_agent::LaunchRoute::Autonomous;
            session.project_state_root = Some(project.path().into());
            session.save(&sessions).unwrap();
            session
        });
        let (first, _first_guard) = runtime(project.path(), &sessions, &pair[0], 1);
        let (second, _second_guard) = runtime(project.path(), &sessions, &pair[1], 2);
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default()).unwrap();
        // No GUI event loop or pane.list exists in this fixture.
        reconcile_project(project.path(), &sessions, &[first.clone(), second.clone()]).unwrap();
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(project.path()),
        )
        .unwrap();
        let monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        assert_eq!(monitor.active_count(), 2, "both real PTYs consume slots");
        assert!(
            monitor.agent_status().needs_human.contains(&4466),
            "neither holds ECR authority"
        );
        assert!(first.handle.try_wait().unwrap().is_none());
        assert!(second.handle.try_wait().unwrap().is_none());
    }

    #[test]
    fn uncaptured_live_peer_is_parked_without_stopping_the_local_runtime() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let pair =
            crate::cli::execution_state::seed_monitor_pair_for_test(project.path(), &sessions);
        let (local, _local_guard) = runtime(project.path(), &sessions, &pair[0], 1);
        let (peer, _peer_guard) = runtime(project.path(), &sessions, &pair[1], 2);
        // The worker has a captured handle for only one member, as with two hosts.
        reconcile_project(project.path(), &sessions, std::slice::from_ref(&local)).unwrap();
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(project.path()),
        )
        .unwrap();
        let monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        assert!(monitor.agent_status().needs_human.contains(&4466));
        assert!(local.handle.try_wait().unwrap().is_none());
        assert!(peer.handle.try_wait().unwrap().is_none());
    }

    #[test]
    fn registered_project_without_ptys_releases_a_dead_hosts_slots() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut old_host = gwt_core::process::hidden_command("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let pid = old_host.id();
        let started = crate::process::host_process_start_time(pid).unwrap();
        old_host.kill().unwrap();
        old_host.wait().unwrap();
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        monitor.record_monitor_runtime_counts(
            pid,
            started,
            BTreeMap::from([(4466, 2)]),
            "2026-01-01T00:00:00Z",
        );
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
        let projects = BTreeMap::from([(project.path().to_path_buf(), sessions)]);
        reconcile_projects(&projects, &[]);
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
        let monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        assert_eq!(
            monitor.active_count(),
            0,
            "no new PTY is needed to release dead-host occupancy"
        );
    }

    #[test]
    fn nonholder_pty_exits_and_holder_survives_without_gui_dispatch() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let pair =
            crate::cli::execution_state::seed_monitor_pair_for_test(project.path(), &sessions);
        let (holder, _holder_guard) = runtime(project.path(), &sessions, &pair[0], 1);
        let (duplicate, _duplicate_guard) = runtime(project.path(), &sessions, &pair[1], 2);
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        // A late duplicate ACK took the projection; it did not take ECR authority.
        monitor.complete_active_launch(4466, &holder.registration.window_id);
        monitor.complete_active_launch(4466, &duplicate.registration.window_id);
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
        let holder_before = std::fs::read(sessions.join("holder.toml")).unwrap();
        reconcile_project(
            project.path(),
            &sessions,
            &[duplicate.clone(), holder.clone()],
        )
        .unwrap();
        assert!(
            duplicate.handle.try_wait().unwrap().is_some(),
            "the captured nonholder PTY must exit"
        );
        assert!(
            holder.handle.try_wait().unwrap().is_none(),
            "current ECR holder must remain live"
        );
        assert_eq!(
            std::fs::read(sessions.join("holder.toml")).unwrap(),
            holder_before
        );
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::load_issue_monitor_prefs(&prefs_path).unwrap(),
        );
        assert_eq!(monitor.active_count(), 1);
        assert_eq!(
            monitor.launched_window_id(4466).as_deref(),
            Some(holder.registration.window_id.as_str())
        );
        monitor.record_agent_window_failed(&duplicate.registration.window_id, "late PTY exit");
        assert_eq!(
            monitor.active_count(),
            1,
            "late exit cannot release the holder"
        );
    }
}
