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
    sync::Arc,
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

/// Capture a live holder's complete runtime identity before revoking its launch.
/// No OS process is signalled here; the owning host uses its retained PTY later.
#[doc(hidden)]
pub fn capture_stop_request(
    project_root: &Path,
    issue_number: u64,
    window_id: &str,
    expected_session_id: Option<&str>,
    requested_at: &str,
) -> io::Result<Option<crate::issue_monitor::IssueMonitorRuntimeStopRequest>> {
    let Some(hold) =
        crate::cli::execution_state::owner_generation_hold_for_project(project_root, issue_number)?
    else {
        return Ok(None);
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session = match gwt_agent::Session::load(
        &sessions_dir.join(format!("{}.toml", hold.holder_session_id)),
    ) {
        Ok(session) => session,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(execution_identity) =
        gwt_agent::SessionExecutionIdentity::from_session(&session).map_err(io::Error::other)?
    else {
        return Ok(None);
    };
    let binding = &execution_identity.execution_binding;
    let kind = match binding.owner_kind.as_str() {
        "issue" => ExecutionOwnerKind::Issue,
        "spec" => ExecutionOwnerKind::Spec,
        _ => return Err(io::Error::other("holder execution owner kind is invalid")),
    };
    if session.linked_issue_number != Some(issue_number)
        || binding.owner_number != issue_number
        || binding.identity.generation_id != hold.generation_id
        || expected_session_id.is_some_and(|expected| expected != session.id)
        || !crate::cli::execution_state::session_binding_authorizes_current_lifecycle_descendant(
            &session.worktree_path,
            ExecutionOwnerKey {
                kind,
                number: issue_number,
            },
            &session.id,
            &binding.identity,
        )?
    {
        return Err(io::Error::other(
            "current holder execution binding changed before stop",
        ));
    }
    let inventory = crate::session_inventory::observe_session(&session, &sessions_dir);
    let mut candidates = inventory.sessions.iter();
    let Some(observed) = candidates.next() else {
        if inventory.uncertainties.is_empty() {
            return Ok(None);
        }
        return Err(io::Error::other(
            "live holder runtime identity is uncertain",
        ));
    };
    if candidates.next().is_some() || !inventory.uncertainties.is_empty() {
        return Err(io::Error::other(
            "live holder runtime identity is ambiguous",
        ));
    }
    let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path_for_pid(
        &sessions_dir,
        observed.host_pid,
        &session.id,
    ))?;
    let host_started_at = runtime
        .host_started_at
        .filter(|start| Some(*start) == crate::process::host_process_start_time(observed.host_pid));
    let runtime_incarnation = runtime
        .runtime_incarnation
        .filter(|incarnation| *incarnation != 0);
    if runtime.execution_identity.as_ref() != Some(&execution_identity)
        || runtime.child_pid != Some(observed.child_pid)
        || runtime.child_started_at != Some(observed.child_started_at)
    {
        return Err(io::Error::other(
            "live holder runtime identity changed before stop",
        ));
    }
    let (Some(host_started_at), Some(runtime_incarnation)) = (host_started_at, runtime_incarnation)
    else {
        return Err(io::Error::other(
            "live holder runtime lacks exact process identity",
        ));
    };
    Ok(Some(crate::issue_monitor::IssueMonitorRuntimeStopRequest {
        issue_number,
        window_id: window_id.into(),
        session_id: session.id,
        execution_identity,
        host_pid: observed.host_pid,
        host_started_at,
        runtime_incarnation,
        child_pid: observed.child_pid,
        child_started_at: observed.child_started_at,
        requested_at: requested_at.into(),
    }))
}

/// The host owns the handles; this worker only retains exact captured references.
/// No client connection, canvas snapshot or GUI event dispatch is needed.
pub struct MonitorRuntimeReconciler {
    stop: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for MonitorRuntimeReconciler {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::error!(target: "gwt.monitor.runtime", "Monitor runtime reconciliation worker failed during owner drain");
            }
        }
    }
}

pub fn spawn(
    project_root: PathBuf,
    sessions_dir: PathBuf,
    snapshot: impl Fn() -> Option<Vec<RegisteredMonitorRuntime>> + Send + 'static,
) -> io::Result<MonitorRuntimeReconciler> {
    let (stop, stopped) = std::sync::mpsc::channel();
    #[cfg(any(test, feature = "test-gh-guard"))]
    let home = gwt_core::test_support::gwt_home_override();
    let worker = std::thread::Builder::new()
        .name("gwt-monitor-runtime".into())
        .spawn(move || {
            #[cfg(any(test, feature = "test-gh-guard"))]
            let _home = home
                .as_ref()
                .map(gwt_core::test_support::ScopedGwtHome::set);
            let mut retained = BTreeMap::new();
            let projects = BTreeMap::from([(project_root, sessions_dir)]);
            loop {
                let draining = match stopped.recv_timeout(Duration::from_secs(5)) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
                };
                if let Some(current) = snapshot() {
                    for entry in current {
                        let registration = &entry.registration;
                        retained.insert(
                            (registration.window_id.clone(), registration.incarnation),
                            entry,
                        );
                    }
                    reconcile_retained_projects(&projects, &mut retained);
                }
                if draining {
                    break;
                }
            }
        })?;
    Ok(MonitorRuntimeReconciler {
        stop,
        worker: Some(worker),
    })
}

fn reconcile_retained_projects(
    projects: &BTreeMap<PathBuf, PathBuf>,
    retained: &mut BTreeMap<(String, u64), RegisteredMonitorRuntime>,
) {
    reconcile_projects(projects, &retained.values().cloned().collect::<Vec<_>>());
    // A reaped leader can still own descendants. Process requests before
    // pruning, and release each captured group/Job before discarding its handle.
    retained.retain(|_, entry| {
        if !matches!(entry.handle.try_wait(), Ok(Some(_))) {
            return true;
        }
        match crate::session_finalizer::kill_and_reap(
            &entry.handle,
            &entry.registration.window_id,
        ) {
            Ok(true) => false,
            result => {
                tracing::warn!(target: "gwt.monitor.runtime", window_id = %entry.registration.window_id, ?result, "retaining captured PTY until tree cleanup completes");
                true
            }
        }
    });
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
    let mut censuses = BTreeMap::new();
    for (prefs_path, (project, sessions)) in repositories {
        let entries = entries
            .iter()
            .filter(|entry| {
                crate::issue_monitor_prefs_path_for_repo_path(&entry.registration.project_root)
                    == prefs_path
            })
            .cloned()
            .collect::<Vec<_>>();
        let inventory = censuses
            .entry(sessions)
            .or_insert_with(|| crate::session_inventory::ProjectSessionCensus::capture(sessions))
            .observe(project);
        if let Err(error) = reconcile_project(project, sessions, &entries, &inventory) {
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
        if !entry.handle.tree_exit_confirmed() {
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
                    (!snapshot.counts.is_empty() || !snapshot.windows.is_empty())
                        && (crate::process::host_process_start_time(snapshot.host_pid)
                            .is_some_and(|started| started != snapshot.host_started_at)
                            || !crate::process::is_host_process_alive(snapshot.host_pid))
                })
                .map(|snapshot| (snapshot.host_pid, snapshot.host_started_at))
                .collect::<Vec<_>>();
            let counts = counts(entries);
            let windows = entries
                .iter()
                .filter(|entry| {
                    !entry.registration.review_dispatch && !entry.handle.tree_exit_confirmed()
                })
                .map(|entry| {
                    (
                        entry.registration.window_id.clone(),
                        entry.registration.issue_number,
                    )
                })
                .collect();
            let unchanged = prefs
                .monitor_runtime_counts
                .get(&format!("{host_pid}:{host_started_at}"))
                .is_some_and(|previous| previous.counts == counts && previous.windows == windows);
            // observed_at is an ordering fence, not a heartbeat. Preserve it
            // until membership changes; claim deadlines remain separate updates.
            if dead_hosts.is_empty() && unchanged {
                return Ok(());
            }
            let mut monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            for (pid, started) in dead_hosts {
                monitor.record_monitor_runtime_counts(pid, started, BTreeMap::new(), &now);
            }
            if !unchanged {
                monitor.record_monitor_runtime_windows(
                    host_pid,
                    host_started_at,
                    counts,
                    windows,
                    &now,
                );
            }
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

#[doc(hidden)]
pub fn stop_requested_runtimes(
    project_root: &Path,
    entries: &[RegisteredMonitorRuntime],
) -> io::Result<bool> {
    let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
        project_root,
    ))?;
    let mut refresh_inventory = false;
    for entry in entries
        .iter()
        .filter(|entry| !entry.registration.review_dispatch)
    {
        let registration = &entry.registration;
        for request in prefs
            .monitor_runtime_stop_requests
            .iter()
            .filter(|request| {
                request.issue_number == registration.issue_number
                    && request.session_id == registration.session_id
                    && request.host_pid == std::process::id()
                    && request.runtime_incarnation == registration.incarnation
            })
        {
            let proof = match runtime_proof(entry) {
                Ok(proof) => proof,
                Err(error) => {
                    tracing::warn!(target: "gwt.monitor.runtime", session_id = %registration.session_id, %error, "retaining runtime whose exact stop proof is unavailable");
                    continue;
                }
            };
            if !request.matches_runtime(&proof.session.id, proof.host_pid, &proof.runtime) {
                continue;
            }
            // A terminal sidecar proves only that the leader ended. Recheck
            // the full captured identity under the owner/Session lease before
            // cleaning its tree, including when the leader was already reaped.
            let result = crate::cli::execution_state::with_current_runtime_cleanup_session_execution_identity_lease(
                &registration.sessions_dir,
                &request.execution_identity,
                || {
                    let proof = runtime_proof(entry)?;
                    if !request.matches_runtime(&proof.session.id, proof.host_pid, &proof.runtime) {
                        return Ok(false);
                    }
                    // A failed teardown or persistence can still change liveness.
                    refresh_inventory = true;
                    if !crate::session_finalizer::kill_and_reap(&entry.handle, &registration.window_id)? {
                        return Err(io::Error::other("captured runtime tree did not finish teardown"));
                    }
                    if !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                        &registration.sessions_dir,
                        &request.execution_identity,
                        request.runtime_incarnation,
                        gwt_agent::AgentStatus::Stopped,
                    )? || !gwt_agent::persist_session_restore_window_on_startup_if_execution_identity_matches_under_lease(
                        &registration.sessions_dir,
                        &request.execution_identity,
                        false,
                    )? {
                        return Err(io::Error::other("captured runtime Session changed before stop persistence"));
                    }
                    Ok(true)
                },
            ).and_then(|result| result.transpose());
            match result {
                Ok(Some(true)) => {
                    // This receipt is written only after the exact captured
                    // process tree was reaped and its terminal Session saved.
                    crate::try_mutate_issue_monitor_prefs(
                        &crate::issue_monitor_prefs_path_for_repo_path(project_root),
                        |prefs| {
                            for intent in prefs.agent_switch_intents.values_mut() {
                                if matches!(
                                    intent.stage,
                                    crate::issue_monitor::AgentSwitchStage::StoppingCurrent
                                        | crate::issue_monitor::AgentSwitchStage::Unknown
                                ) && intent.runtime_stop.as_ref() == Some(request)
                                    && intent.checkpoint.is_some()
                                {
                                    intent.runtime_exit_confirmed = true;
                                }
                            }
                            Ok(())
                        },
                    )?;
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(target: "gwt.monitor.runtime", session_id = %registration.session_id, window_id = %registration.window_id, %error, "runtime stop did not complete; retaining captured runtime and continuing reconciliation");
                    break;
                }
            }
        }
    }
    Ok(refresh_inventory)
}

fn reconcile_project(
    project_root: &Path,
    sessions_dir: &Path,
    entries: &[RegisteredMonitorRuntime],
    inventory: &crate::session_inventory::SessionInventory,
) -> io::Result<()> {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + Duration::from_secs(5),
    );
    let refreshed_inventory = stop_requested_runtimes(project_root, entries)?.then(|| {
        crate::session_inventory::ProjectSessionCensus::capture(sessions_dir).observe(project_root)
    });
    let inventory = refreshed_inventory.as_ref().unwrap_or(inventory);
    let complete_counts = publish_counts(project_root, entries)?;
    let mut groups = BTreeMap::<u64, Vec<&RegisteredMonitorRuntime>>::new();
    for entry in entries
        .iter()
        .filter(|entry| !entry.registration.review_dispatch && !entry.handle.tree_exit_confirmed())
    {
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

#[cfg(test)]
mod prefs_io_tests {
    use super::*;

    #[test]
    fn prefs_cache_five_second_io_counts() {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        let pid = std::process::id();
        let started = crate::process::host_process_start_time(pid).unwrap();
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        monitor.record_monitor_runtime_counts(
            pid,
            started,
            BTreeMap::new(),
            "2000-01-01T00:00:00Z",
        );
        let mut fixture = serde_json::to_string_pretty(&monitor.prefs()).unwrap();
        fixture.extend(std::iter::repeat_n(' ', 524 * 1024 - fixture.len()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &fixture).unwrap();
        let before = crate::issue_monitor::prefs_io_counts_for_test();
        // One five-second cadence: two census publications, capacity, claim renewal,
        // live-window lookup, and eight window observations on a convergence tick.
        for _ in 0..2 {
            publish_counts(project.path(), &[]).unwrap();
        }
        for _ in 0..11 {
            crate::load_issue_monitor_prefs(&path).unwrap();
        }
        let after = crate::issue_monitor::prefs_io_counts_for_test();
        let measured = (after.0 - before.0, after.1 - before.1);
        println!("Issue #5499: 536576-byte fixture, modeled 5s, 2 publications + 11 reads: parses={}, canonical writes={} (each write also saves a committed sidecar)", measured.0, measured.1);
        assert_eq!(measured, (1, 0));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), fixture);

        let before = crate::issue_monitor::prefs_io_counts_for_test();
        for _ in 0..2 {
            publish_counts(project.path(), &[]).unwrap();
        }
        for _ in 0..11 {
            crate::load_issue_monitor_prefs(&path).unwrap();
        }
        assert_eq!(
            crate::issue_monitor::prefs_io_counts_for_test(),
            before,
            "warm cadence performs no parses or writes"
        );

        // Membership changes still commit, including one durable dead-host tombstone.
        monitor.record_monitor_runtime_windows(
            pid,
            started,
            BTreeMap::from([(42, 1)]),
            BTreeMap::from([("pane".into(), 42)]),
            "2000-01-01T00:00:01Z",
        );
        monitor.record_monitor_runtime_counts(
            u32::MAX,
            1,
            BTreeMap::from([(43, 1)]),
            "2000-01-01T00:00:01Z",
        );
        crate::save_issue_monitor_prefs(&path, &monitor.prefs()).unwrap();
        let before = crate::issue_monitor::prefs_io_counts_for_test().1;
        assert!(publish_counts(project.path(), &[]).unwrap().is_empty());
        assert_eq!(
            crate::issue_monitor::prefs_io_counts_for_test().1 - before,
            1
        );
        let prefs = crate::load_issue_monitor_prefs(&path).unwrap();
        assert!(prefs
            .monitor_runtime_counts
            .values()
            .all(|host| host.counts.is_empty() && host.windows.is_empty()));
        let committed = std::fs::read(&path).unwrap();
        publish_counts(project.path(), &[]).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            committed,
            "dead-host tombstones are not heartbeats"
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn reconcile_project(
        project: &Path,
        sessions: &Path,
        entries: &[RegisteredMonitorRuntime],
    ) -> io::Result<()> {
        let inventory =
            crate::session_inventory::ProjectSessionCensus::capture(sessions).observe(project);
        super::reconcile_project(project, sessions, entries, &inventory)
    }

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

#[cfg(test)]
mod stop_tests {
    use super::*;
    use crate::cli::execution_state::{self, ExecutionOwnerKey, ExecutionOwnerKind};
    use gwt_agent::{AgentStatus, Session, SessionExecutionBinding, SessionExecutionIdentity};
    use gwt_core::test_support::ScopedEnvVar;

    struct StopFixture {
        project: tempfile::TempDir,
        sessions: PathBuf,
        owner: ExecutionOwnerKey,
        session: Session,
        runtime: gwt_agent::SessionRuntimeState,
        entry: RegisteredMonitorRuntime,
        child_pid: u32,
        root_exit_file: PathBuf,
    }

    fn is_process_alive(pid: u32) -> bool {
        #[cfg(windows)]
        {
            crate::process::host_process_liveness(pid)
                .expect("native child liveness must be observable")
        }
        #[cfg(unix)]
        {
            let mut system = sysinfo::System::new().expect("initialize test process observation");
            let pid = sysinfo::Pid::from_u32(pid);
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::nothing(),
            );
            system
                .process(pid)
                .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie)
        }
    }

    fn with_stop_fixture(test: impl FnOnce(&mut StopFixture)) {
        let _env = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let project = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(project.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: 4168,
        };
        execution_state::materialize_at_launch(
            project.path(),
            owner.kind,
            owner.number,
            "holder",
            "gwt-execute",
            false,
        )
        .unwrap();
        execution_state::ensure_generation_ledger(
            project.path(),
            owner,
            execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let binding = execution_state::current_execution_binding(project.path(), owner)
            .unwrap()
            .unwrap();
        let mut session = Session::new(project.path(), "main", gwt_agent::AgentId::Codex);
        session.id = "holder".into();
        session.project_state_root = Some(project.path().into());
        session.linked_issue_number = Some(4168);
        session.execution_binding = Some(SessionExecutionBinding {
            schema_version: SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().unwrap(),
            owner_kind: "issue".into(),
            owner_number: 4168,
            identity: binding,
            capability_generation: 1,
        });
        session.status = AgentStatus::Running;
        session.restore_window_on_startup = true;
        session.save(&sessions).unwrap();
        let child_file = project.path().join("child.pid");
        let root_exit_file = project.path().join("exit-root");
        let (command, args) = if cfg!(windows) {
            ("powershell", vec!["-NoProfile", "-NonInteractive", "-Command", "$child = Start-Process ping -ArgumentList '-n','61','127.0.0.1' -WindowStyle Hidden -PassThru; Set-Content -LiteralPath $env:GWT_TEST_CHILD_PID -Value $child.Id; while (-not (Test-Path -LiteralPath $env:GWT_TEST_ROOT_EXIT)) { Start-Sleep -Milliseconds 100 }"])
        } else {
            (
                "/bin/sh",
                vec![
                    "-c",
                    "trap '' HUP; sleep 60 & printf '%s' \"$!\" > \"$GWT_TEST_CHILD_PID\"; while [ ! -f \"$GWT_TEST_ROOT_EXIT\" ]; do sleep 0.1; done",
                ],
            )
        };
        let handle = Arc::new(
            PtyHandle::spawn(gwt_terminal::pty::SpawnConfig {
                command: command.into(),
                args: args.into_iter().map(str::to_owned).collect(),
                cols: 80,
                rows: 24,
                env: std::collections::HashMap::from([
                    (
                        "GWT_TEST_CHILD_PID".into(),
                        child_file.display().to_string(),
                    ),
                    (
                        "GWT_TEST_ROOT_EXIT".into(),
                        root_exit_file.display().to_string(),
                    ),
                ]),
                remove_env: vec![],
                cwd: Some(project.path().into()),
            })
            .unwrap(),
        );
        #[cfg(windows)]
        handle.write_input(b"\x1b[1;1R").unwrap();
        struct Cleanup(Arc<PtyHandle>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = crate::session_finalizer::kill_and_reap(&self.0, "test-cleanup");
            }
        }
        let _cleanup = Cleanup(handle.clone());
        // test-hygiene: allow-wall-clock-deadline Bound the external OS child's PID publication; no virtual-clock equivalent exists.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let child_pid = loop {
            if let Some(pid) = std::fs::read_to_string(&child_file)
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok())
            {
                break pid;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline Observe actual child startup rather than asserting scheduler timing.
                std::time::Instant::now() < deadline,
                "child PID not published"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut runtime = gwt_agent::SessionRuntimeState::new(AgentStatus::Running);
        runtime.execution_identity = SessionExecutionIdentity::from_session(&session).unwrap();
        runtime.runtime_incarnation = Some(7);
        runtime.host_started_at = crate::process::host_process_start_time(std::process::id());
        runtime.child_pid = handle.process_id();
        runtime.child_started_at = runtime
            .child_pid
            .and_then(crate::process::host_process_start_time);
        runtime
            .save(&gwt_agent::runtime_state_path(&sessions, &session.id))
            .unwrap();
        let entry = RegisteredMonitorRuntime {
            registration: MonitorRuntimeRegistration {
                window_id: "closed-tab::holder".into(),
                session_id: session.id.clone(),
                issue_number: 4168,
                worktree_path: project.path().into(),
                project_root: project.path().into(),
                sessions_dir: sessions.clone(),
                incarnation: 7,
                review_dispatch: false,
            },
            handle,
        };
        test(&mut StopFixture {
            project,
            sessions,
            owner,
            session,
            runtime,
            entry,
            child_pid,
            root_exit_file,
        });
    }

    #[test]
    fn retained_runtime_stop_reaps_tree_and_preserves_active_execution() {
        with_stop_fixture(|fixture| {
            let StopFixture {
                project,
                sessions,
                owner,
                session,
                runtime,
                entry,
                child_pid,
                root_exit_file,
            } = fixture;
            let owner = *owner;
            let path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
            // A live launch can lose its canvas/ACK binding before an operator stop.
            let initial = crate::IssueMonitorPrefs {
                launching_issues: vec![crate::issue_monitor::IssueMonitorLaunchingIssue {
                    issue_number: 4168,
                    claimed_at: None,
                }],
                ..Default::default()
            };
            crate::save_issue_monitor_prefs(&path, &initial).unwrap();
            let mut stale =
                crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);
            let before =
                execution_state::load_owner_generation_ledger(project.path(), owner).unwrap();
            let mut env = crate::cli::TestEnv::new(project.path().into());
            let stop = || {
                crate::cli::CliCommand::Issue(crate::cli::IssueCommand::MonitorStop {
                    project_root: Some(project.path().into()),
                    number: 4168,
                    reason: "stop the detached runtime".into(),
                    remove_from_queue: true,
                    claim_id: None,
                    delivery_id: None,
                    window_id: None,
                })
            };
            let (code, output) = crate::cli::run_collect(&mut env, stop()).unwrap();
            assert_eq!(code, 0, "{output}");
            let output: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
            assert_eq!(output["runtime_stop"], "requested");
            assert_eq!(
                output["runtime_target_source"],
                "unique_current_execution_holder"
            );
            let prefs = crate::load_issue_monitor_prefs(&path).unwrap();
            assert_eq!(prefs.monitor_runtime_stop_requests.len(), 1);
            let request = prefs.monitor_runtime_stop_requests[0].clone();
            let switch_operation = "a283fc81-50bc-47d2-b65f-55a515815231";
            // A requested stop is deliberately not an exit acknowledgement.
            let switch_intent = serde_json::from_value(serde_json::json!({
                "request": {"operation_id":switch_operation,"member_id":"member","expected_roster_revision":1,"target_issue_number":4169},
                "source": {"assignment":{"reservation_id":"reservation","session_id":request.session_id,"issue_number":4168,"host_id":crate::process::current_hostname(),"host_pid":request.host_pid,"host_started_at":request.host_started_at},
                    "window_id":request.window_id,"claim_id":"claim","confirmation":{"window_id":request.window_id,"claim_id":"claim","confirmed_at":"2026-10-25T00:00:00Z"}},
                "stage":"stopping_current", "runtime_stop":request,
                "checkpoint":{"verified_head":"1111111111111111111111111111111111111111","work_id":"work","board_entry_id":"board"}
            })).unwrap();
            let mut prefs = prefs;
            prefs
                .agent_switch_intents
                .insert(switch_operation.into(), switch_intent);
            crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
            assert!(
                request.window_id.is_empty(),
                "window identity is diagnostic only"
            );
            stale.rebase_daemon_driver_prefs(&prefs);
            crate::save_issue_monitor_prefs(&path, &stale.prefs()).unwrap();
            assert_eq!(
                crate::load_issue_monitor_prefs(&path)
                    .unwrap()
                    .monitor_runtime_stop_requests,
                vec![request.clone()]
            );
            let reconcile = |entry: &RegisteredMonitorRuntime| {
                let inventory = crate::session_inventory::ProjectSessionCensus::capture(sessions)
                    .observe(project.path());
                super::reconcile_project(
                    project.path(),
                    sessions,
                    std::slice::from_ref(entry),
                    &inventory,
                )
                .unwrap();
            };
            // An old request never targets a different registration incarnation.
            let mut successor = entry.clone();
            successor.registration.incarnation = 8;
            reconcile(&successor);
            assert!(entry.handle.try_wait().unwrap().is_none());
            for mismatch in ["child", "host", "identity", "incarnation"] {
                let mut changed = request.clone();
                match mismatch {
                    "child" => changed.child_started_at += 1,
                    "host" => changed.host_started_at += 1,
                    "identity" => {
                        changed
                            .execution_identity
                            .execution_binding
                            .capability_generation += 1
                    }
                    _ => changed.runtime_incarnation += 1,
                }
                let mut prefs = crate::load_issue_monitor_prefs(&path).unwrap();
                prefs.monitor_runtime_stop_requests = vec![changed];
                crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
                reconcile(entry);
                assert!(
                    entry.handle.try_wait().unwrap().is_none(),
                    "{mismatch} mismatch killed PTY"
                );
                assert!(
                    !crate::load_issue_monitor_prefs(&path)
                        .unwrap()
                        .agent_switch_intents[switch_operation]
                        .runtime_exit_confirmed
                );
            }
            let mut prefs = crate::load_issue_monitor_prefs(&path).unwrap();
            prefs.monitor_runtime_stop_requests = vec![request];
            // A prior cleanup attempt may have left its outcome unknown.
            prefs
                .agent_switch_intents
                .get_mut(switch_operation)
                .unwrap()
                .stage = crate::issue_monitor::AgentSwitchStage::Unknown;
            crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
            let process_alive = || is_process_alive(*child_pid);
            // The leader can exit, and its exit observer can persist Stopped, before
            // the next host worker tick. Its child still belongs to the captured PTY.
            std::fs::write(root_exit_file, b"exit").unwrap();
            // test-hygiene: allow-wall-clock-deadline Bound actual PTY leader exit before driving the retained-host tick.
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while entry.handle.try_wait().unwrap().is_none() {
                assert!(
                    // test-hygiene: allow-wall-clock-deadline Real leader exit is the process-lifecycle contract under test.
                    std::time::Instant::now() < deadline,
                    "PTY leader did not exit"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(process_alive(), "fixture child exited with its leader");
            runtime.status = AgentStatus::Stopped;
            runtime
                .save(&gwt_agent::runtime_state_path(sessions, &session.id))
                .unwrap();
            session.status = AgentStatus::Stopped;
            session.save(sessions).unwrap();
            // No window or GUI registry is available: drive the actual retained tick.
            let mut retained =
                BTreeMap::from([((entry.registration.window_id.clone(), 7), entry.clone())]);
            reconcile_retained_projects(
                &BTreeMap::from([(project.path().into(), sessions.clone())]),
                &mut retained,
            );
            // test-hygiene: allow-wall-clock-deadline Bound the real OS descendant exit after the host's captured-tree cleanup.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            // test-hygiene: allow-wall-clock-deadline Observe process exit directly; elapsed time only caps a failed fixture.
            while process_alive() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(
                !process_alive(),
                "retained stop left descendant {child_pid} alive"
            );
            assert!(
                crate::load_issue_monitor_prefs(&path)
                    .unwrap()
                    .agent_switch_intents[switch_operation]
                    .runtime_exit_confirmed,
                "only the owning Host's exact kill/reap can acknowledge switch exit"
            );
            assert_eq!(
                Session::load(&sessions.join("holder.toml")).unwrap().status,
                AgentStatus::Stopped
            );
            let (code, output) = crate::cli::run_collect(&mut env, stop()).unwrap();
            assert_eq!(code, 0, "{output}");
            let output: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
            assert_eq!(
                output["generation_release"], "held",
                "repeated stop must retain the Active contract"
            );
            assert_eq!(
                execution_state::load_owner_generation_ledger(project.path(), owner).unwrap(),
                before,
                "runtime stop must leave Active ECR unchanged"
            );
            assert_eq!(
                crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(),
                    crate::load_issue_monitor_prefs(&path).unwrap()
                )
                .active_count(),
                0
            );
        });
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompletionTiming {
        Never,
        BeforeStop,
        AfterStop,
    }

    fn assert_stop_request_kills_live_tree(timing: CompletionTiming) {
        with_stop_fixture(|fixture| {
            let StopFixture {
                project,
                sessions,
                owner,
                session,
                entry,
                child_pid,
                root_exit_file,
                ..
            } = fixture;
            let complete = || {
                assert!(matches!(
                    execution_state::settle(
                        project.path(),
                        &session.id,
                        execution_state::ExecutionSettlement::Completed
                    )
                    .unwrap(),
                    execution_state::SettleResult::Settled(_)
                ));
            };
            if timing == CompletionTiming::BeforeStop {
                complete();
            }
            let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
            crate::save_issue_monitor_prefs(
                &prefs_path,
                &crate::IssueMonitorPrefs {
                    launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                        issue_number: owner.number,
                        window_id: entry.registration.window_id.clone(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            let mut env = crate::cli::TestEnv::new(project.path().into());
            let (code, output) = crate::cli::run_collect(
                &mut env,
                crate::cli::CliCommand::Issue(crate::cli::IssueCommand::MonitorStop {
                    project_root: Some(project.path().into()),
                    number: owner.number,
                    reason: "stop the live tree".into(),
                    remove_from_queue: true,
                    claim_id: None,
                    delivery_id: None,
                    window_id: Some(entry.registration.window_id.clone()),
                }),
            )
            .unwrap();
            assert_eq!(code, 0, "{output}");
            let output: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
            assert_eq!(
                output["runtime_stop"], "requested",
                "a Completed execution can still own a live launch"
            );
            if timing == CompletionTiming::AfterStop {
                complete();
            }
            let before =
                execution_state::load_owner_generation_ledger(project.path(), *owner).unwrap();
            assert!(!root_exit_file.exists(), "the leader must not exit itself");
            assert!(entry.handle.try_wait().unwrap().is_none());
            assert!(is_process_alive(*child_pid));
            // Call the request handler directly: the ordinary exited-leader
            // prune path cannot make this test pass in place of request dispatch.
            assert!(
                stop_requested_runtimes(project.path(), std::slice::from_ref(entry)).unwrap(),
                "the accepted request must stop its still-live leader"
            );
            assert!(
                entry.handle.try_wait().unwrap().is_some(),
                "stop left its leader alive"
            );
            assert!(
                !is_process_alive(*child_pid),
                "stop left its descendant alive"
            );
            let stopped = Session::load(&sessions.join(format!("{}.toml", session.id))).unwrap();
            assert_eq!(stopped.status, AgentStatus::Stopped);
            assert!(!stopped.restore_window_on_startup);
            assert_eq!(
                execution_state::load_owner_generation_ledger(project.path(), *owner).unwrap(),
                before,
                "runtime cleanup must preserve execution lifecycle"
            );
        });
    }

    #[test]
    fn monitor_stop_request_kills_a_live_leader_and_child() {
        assert_stop_request_kills_live_tree(CompletionTiming::Never);
    }

    #[test]
    fn monitor_stop_captures_a_completed_launch_with_a_live_tree() {
        assert_stop_request_kills_live_tree(CompletionTiming::BeforeStop);
    }

    #[test]
    fn monitor_stop_request_survives_execution_completion() {
        assert_stop_request_kills_live_tree(CompletionTiming::AfterStop);
    }

    #[test]
    fn reaped_leader_keeps_capacity_until_captured_tree_exit_is_confirmed() {
        with_stop_fixture(|fixture| {
            fixture.entry.handle.kill().unwrap();
            fixture.entry.handle.wait().unwrap();
            let entries = std::slice::from_ref(&fixture.entry);
            assert_eq!(
                super::counts(entries),
                BTreeMap::from([(fixture.owner.number, 1)]),
                "a reaped leader does not prove its captured tree exited"
            );
            assert!(crate::session_finalizer::kill_and_reap(
                &fixture.entry.handle,
                &fixture.entry.registration.window_id,
            )
            .unwrap());
            assert!(
                super::counts(entries).is_empty(),
                "capacity is released after captured-tree exit is confirmed"
            );
        });
    }

    #[test]
    fn failed_stop_authority_keeps_runtime_and_accounting_running() {
        with_stop_fixture(|fixture| {
            let request = super::capture_stop_request(
                fixture.project.path(),
                fixture.owner.number,
                "",
                None,
                "2026-01-01T00:00:00Z",
            )
            .unwrap()
            .unwrap();
            let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(fixture.project.path());
            crate::save_issue_monitor_prefs(
                &prefs_path,
                &crate::IssueMonitorPrefs {
                    monitor_runtime_stop_requests: vec![request.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
            let trusted =
                crate::cli::trusted_store::trusted_dir_for_worktree(fixture.project.path())
                    .unwrap();
            let broken_ledger = trusted
                .parent()
                .unwrap()
                .join("execution-owners")
                .join(format!("owner-{}", fixture.owner.number))
                .join("generation-ledger.json");
            std::fs::write(broken_ledger, b"invalid owner ledger").unwrap();
            let session_path = fixture
                .sessions
                .join(format!("{}.toml", fixture.session.id));
            let session_before = std::fs::read(&session_path).unwrap();
            let inventory =
                crate::session_inventory::ProjectSessionCensus::capture(&fixture.sessions)
                    .observe(fixture.project.path());
            super::reconcile_project(
                fixture.project.path(),
                &fixture.sessions,
                std::slice::from_ref(&fixture.entry),
                &inventory,
            )
            .expect("one failed stop must not abort accounting");
            assert!(
                fixture.entry.handle.try_wait().unwrap().is_none(),
                "unknown authority must retain its captured runtime"
            );
            assert_eq!(std::fs::read(session_path).unwrap(), session_before);
            let prefs = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
            assert_eq!(prefs.monitor_runtime_stop_requests, vec![request]);
            let host = prefs
                .monitor_runtime_counts
                .values()
                .find(|host| host.host_pid == std::process::id())
                .expect("fresh host accounting");
            assert_eq!(host.counts, BTreeMap::from([(fixture.owner.number, 1)]));
        });
    }
}
