//! Issue #3927 (SPEC #3340 Phase 10S, AS-40〜45 / FR-044〜049): runtime-owned
//! terminal convergence for Issue-linked Agent windows.
//!
//! The runtime, not the PM, closes an Agent window once its Work is
//! canonically terminal: the execution record settled cleanly, the Issue has a
//! durable closed record, or the Issue Monitor durably replaced the launch.
//! The design is one observer plus the existing detached finalizer:
//!
//! 1. A short tick on the Tao thread captures only immutable facts about each
//!    Issue-linked Agent window (window id, Session id, project root, runtime
//!    status, lifecycle generation) and schedules one background scan.
//! 2. The worker reads the Session, the execution diagnosis, and the
//!    repository-scoped Monitor prefs off the event loop and classifies each
//!    window with [`classify_terminal_window`]. A window that is still
//!    Monitor-owned is settled through the internal exact terminal-delivery
//!    control first; close eligibility is returned only after that commit.
//! 3. The Tao thread keeps a grace candidate per eligible window. Eligibility
//!    loss or an identity change resets it; user activity does not.
//! 4. At grace expiry the exact window / Session / lifecycle generation is
//!    revalidated and the window is closed through the shared close path,
//!    which marks the Session Stopped and restore-disabled.
//!
//! The same predicate fences automatic restore (FR-047) so a settled or
//! closed window never respawns at startup or Open Project.

use super::*;
use std::time::Duration;

pub(crate) use gwt::project_runtime::installation::{
    classify_terminal_window, RestoreAdmission, TerminalCloseEligibility, TerminalCloseReason,
};
#[cfg(test)]
pub(crate) use gwt::project_runtime::installation::{
    restore_admission_for_facts, ExecutionTerminalFacts, MonitorTerminalFacts, TerminalWindowFacts,
};

/// Immutable facts about one Issue-linked Agent window captured on the Tao
/// thread for the background observer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalWindowSnapshot {
    pub(crate) window_id: String,
    pub(crate) session_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) worktree_path: PathBuf,
    pub(crate) window_status: WindowProcessStatus,
    pub(crate) lifecycle_generation: Option<u64>,
}

/// One typed observation returned by the worker to the Tao thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalWindowObservation {
    pub(crate) window_id: String,
    pub(crate) session_id: String,
    pub(crate) lifecycle_generation: Option<u64>,
    pub(crate) eligibility: TerminalCloseEligibility,
}

/// A window whose terminal eligibility was first observed at `since`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalCloseCandidate {
    pub(crate) session_id: String,
    pub(crate) lifecycle_generation: Option<u64>,
    pub(crate) reason: TerminalCloseReason,
    pub(crate) since: Instant,
}

/// Cadence of the observer tick. The grace (default 60 seconds) is measured
/// from the first eligible observation, so a window closes within one tick
/// after the grace elapses.
pub(crate) const TERMINAL_CONVERGENCE_TICK: Duration = Duration::from_secs(15);

/// Read the configured grace from the profile settings; missing or unreadable
/// settings load the 60-second default.
pub(crate) fn configured_terminal_close_grace(profile_config_path: Option<&Path>) -> Duration {
    let secs = match profile_config_path {
        Some(path) if path.exists() => gwt_config::Settings::load_from_path(path)
            .map(|settings| settings.agent.terminal_close_grace_secs)
            .unwrap_or(gwt_config::agent_config::DEFAULT_TERMINAL_CLOSE_GRACE_SECS),
        Some(_) => gwt_config::agent_config::DEFAULT_TERMINAL_CLOSE_GRACE_SECS,
        None => gwt_config::Settings::load()
            .map(|settings| settings.agent.terminal_close_grace_secs)
            .unwrap_or(gwt_config::agent_config::DEFAULT_TERMINAL_CLOSE_GRACE_SECS),
    };
    Duration::from_secs(secs)
}

/// Read every canonical fact for one persisted Session off the event loop.
pub(crate) use gwt::project_runtime::installation::{
    load_terminal_monitor, read_terminal_window_facts_with_monitor,
};

/// Settle a successful terminal delivery for a window the Monitor still owns:
/// daemon control first, exact-CAS local fallback otherwise. `Ok` means the
/// slot release is committed; anything else keeps the window ineligible.
pub(crate) fn settle_issue_monitor_terminal_delivery_in_background(
    project_root: &Path,
    window_id: &str,
    issue_number: u64,
    commit_timeout: Duration,
) -> Result<(), String> {
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path)
        .map_err(|error| format!("Issue Monitor prefs could not be read: {error}"))?;
    let monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let target = gwt::IssueMonitorStopTarget {
        issue_number,
        claim_id: monitor.live_claim_id(issue_number),
        delivery_id: monitor.pending_launch_delivery_id(issue_number),
        window_id: Some(window_id.to_string()),
    };
    #[cfg(unix)]
    let publication = {
        let payload = gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "terminal_delivered": {
                    "window_id": window_id,
                    "issue_number": target.issue_number,
                    "claim_id": target.claim_id.as_deref(),
                    "delivery_id": target.delivery_id.as_deref(),
                }
            }),
            std::process::id(),
        );
        gwt::daemon_publisher::publish_issue_monitor_control(project_root, payload)
    };
    #[cfg(not(unix))]
    let publication = Err(
        gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
            "Issue Monitor daemon control is unavailable on this platform".to_string(),
        ),
    );
    match publication {
        Ok(()) => Ok(()),
        Err(error) if error.allows_local_fallback() => {
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                Instant::now() + commit_timeout,
            );
            gwt::try_mutate_issue_monitor_prefs_without_authority_fence(&prefs_path, |prefs| {
                let mut monitor = gwt::IssueMonitorState::with_prefs(
                    gwt::IssueMonitorConfig::default(),
                    prefs.clone(),
                );
                monitor
                    .settle_exact_terminal_delivery(&target)
                    .map_err(|mismatch| {
                        std::io::Error::other(format!(
                            "terminal delivery settlement refused: {mismatch:?}"
                        ))
                    })?;
                *prefs = monitor.prefs();
                Ok(())
            })
            .map(|_| ())
            .map_err(|error| format!("local terminal delivery settlement failed: {error}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// The background observer: classify every snapshot and settle Monitor-owned
/// deliveries before granting eligibility. Never touches the GUI.
pub(crate) fn observe_terminal_windows_in_background(
    sessions_dir: &Path,
    snapshots: Vec<TerminalWindowSnapshot>,
    commit_timeout: Duration,
) -> Vec<TerminalWindowObservation> {
    observe_terminal_windows_with_monitor_loader(
        sessions_dir,
        snapshots,
        commit_timeout,
        load_terminal_monitor,
    )
}

fn observe_terminal_windows_with_monitor_loader(
    sessions_dir: &Path,
    snapshots: Vec<TerminalWindowSnapshot>,
    commit_timeout: Duration,
    mut load_monitor: impl FnMut(&Path) -> Option<gwt::IssueMonitorState>,
) -> Vec<TerminalWindowObservation> {
    // A tick shares each repository snapshot, including unreadable prefs. Settlement
    // below still reads fresh state and commits through its own exact CAS.
    let mut monitors = HashMap::<PathBuf, Option<gwt::IssueMonitorState>>::new();
    snapshots
        .into_iter()
        .map(|snapshot| {
            let session_path = sessions_dir.join(format!("{}.toml", snapshot.session_id));
            let eligibility = match gwt_agent::Session::load(&session_path) {
                Ok(session) => {
                    let monitor = if session.linked_issue_number.is_some() {
                        let prefs_path =
                            gwt::issue_monitor_prefs_path_for_repo_path(&snapshot.project_root);
                        monitors
                            .entry(prefs_path)
                            .or_insert_with_key(|path| load_monitor(path))
                            .as_ref()
                    } else {
                        None
                    };
                    let facts = read_terminal_window_facts_with_monitor(
                        &session,
                        &snapshot.window_id,
                        snapshot.window_status,
                        monitor,
                    );
                    let eligibility = classify_terminal_window(&facts);
                    let monitor_owned = facts
                        .monitor
                        .as_ref()
                        .is_some_and(|monitor| monitor.binds_this_window);
                    match (eligibility, session.linked_issue_number) {
                        (TerminalCloseEligibility::Eligible(reason), Some(issue_number))
                            if monitor_owned =>
                        {
                            match settle_issue_monitor_terminal_delivery_in_background(
                                &snapshot.project_root,
                                &snapshot.window_id,
                                issue_number,
                                commit_timeout,
                            ) {
                                Ok(()) => {
                                    tracing::info!(
                                        target: "gwt.pane.teardown",
                                        window_id = %snapshot.window_id,
                                        issue_number,
                                        reason = reason.as_str(),
                                        "settled the Issue Monitor terminal delivery before automatic close"
                                    );
                                    TerminalCloseEligibility::Eligible(reason)
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        target: "gwt.pane.teardown",
                                        window_id = %snapshot.window_id,
                                        issue_number,
                                        %error,
                                        "terminal delivery settlement failed; the window is retained"
                                    );
                                    TerminalCloseEligibility::Ineligible("settlement_failed")
                                }
                            }
                        }
                        (eligibility, _) => eligibility,
                    }
                }
                Err(_) => TerminalCloseEligibility::Ineligible("session_unreadable"),
            };
            TerminalWindowObservation {
                window_id: snapshot.window_id,
                session_id: snapshot.session_id,
                lifecycle_generation: snapshot.lifecycle_generation,
                eligibility,
            }
        })
        .collect()
}

impl AppRuntime {
    /// FR-045: the Tao-thread half of the observer tick. Closes candidates
    /// whose grace elapsed, then captures immutable facts for the next scan.
    pub(crate) fn terminal_convergence_tick_events(&mut self) -> Vec<OutboundEvent> {
        self.terminal_convergence_tick_events_at(Instant::now())
    }

    pub(crate) fn terminal_convergence_tick_events_at(
        &mut self,
        now: Instant,
    ) -> Vec<OutboundEvent> {
        let events = self.close_expired_terminal_window_candidates_at(now);
        self.schedule_terminal_convergence_scan();
        events
    }

    /// Snapshot every Issue-linked Agent window and run one background scan
    /// if none is in flight. Only in-memory state is read here.
    pub(crate) fn schedule_terminal_convergence_scan(&mut self) {
        if self.terminal_convergence_scan_in_flight {
            return;
        }
        let snapshots = self.terminal_window_snapshots();
        if snapshots.is_empty() {
            self.terminal_close_candidates.clear();
            return;
        }
        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let profile_config_path = self.profile_config_path.clone();
        let commit_timeout = self.issue_monitor_fallback_commit_timeout;
        self.terminal_convergence_scan_in_flight = true;
        let fallback_grace = self.terminal_close_grace;
        let spawn = self.blocking_tasks.try_spawn(move || {
            // The in-flight flag is cleared only by the completion event, so
            // a panicking worker must still send one (with no observations,
            // which drops every candidate) or the observer would stay wedged.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let grace = configured_terminal_close_grace(profile_config_path.as_deref());
                let observations = observe_terminal_windows_in_background(
                    &sessions_dir,
                    snapshots,
                    commit_timeout,
                );
                (grace, observations)
            }));
            let (grace, observations) = match result {
                Ok(observed) => observed,
                Err(panic) => {
                    let detail = panic
                        .downcast_ref::<&str>()
                        .map(|message| (*message).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    tracing::error!(
                        target: "gwt.pane.teardown",
                        %detail,
                        "terminal convergence observer panicked; no window is eligible this scan"
                    );
                    (fallback_grace, Vec::new())
                }
            };
            proxy.send(UserEvent::TerminalConvergenceObserved {
                grace,
                observations,
            });
        });
        if let Err(error) = spawn {
            self.terminal_convergence_scan_in_flight = false;
            tracing::warn!(%error, "failed to spawn the terminal convergence observer");
        }
    }

    pub(crate) fn terminal_window_snapshots(&self) -> Vec<TerminalWindowSnapshot> {
        let generations = self
            .window_lifecycle_generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.active_agent_sessions
            .values()
            .filter_map(|active| {
                // Until authenticated Ready commits this launch, the current
                // generation still describes its predecessor, not this pane.
                if self
                    .pending_fresh_execution_launches
                    .get(&active.window_id)
                    .is_some_and(|pending| pending.binding.session_id == active.session_id)
                {
                    return None;
                }
                let address = self.window_lookup.get(&active.window_id)?;
                let tab = self.tab(&address.tab_id)?;
                let window = tab.workspace.window(&address.raw_id)?;
                Some(TerminalWindowSnapshot {
                    window_id: active.window_id.clone(),
                    session_id: active.session_id.clone(),
                    project_root: tab.project_root.clone(),
                    worktree_path: active.worktree_path.clone(),
                    window_status: window.status,
                    lifecycle_generation: generations.get(&active.window_id).copied(),
                })
            })
            .collect()
    }

    /// Apply one scan's observations: start a grace candidate for each
    /// eligible window, reset it on eligibility loss or identity change, and
    /// drop candidates for windows the scan no longer saw.
    pub(crate) fn terminal_convergence_observed_events(
        &mut self,
        grace: Duration,
        observations: Vec<TerminalWindowObservation>,
    ) -> Vec<OutboundEvent> {
        self.terminal_convergence_observed_events_at(grace, observations, Instant::now())
    }

    pub(crate) fn terminal_convergence_observed_events_at(
        &mut self,
        grace: Duration,
        observations: Vec<TerminalWindowObservation>,
        now: Instant,
    ) -> Vec<OutboundEvent> {
        self.terminal_convergence_scan_in_flight = false;
        self.terminal_close_grace = grace;
        let observed: HashSet<String> = observations
            .iter()
            .map(|observation| observation.window_id.clone())
            .collect();
        self.terminal_close_candidates
            .retain(|window_id, _| observed.contains(window_id));
        for observation in observations {
            match observation.eligibility {
                TerminalCloseEligibility::Eligible(reason) => {
                    let identity_matches = self
                        .terminal_close_candidates
                        .get(&observation.window_id)
                        .is_some_and(|candidate| {
                            candidate.session_id == observation.session_id
                                && candidate.lifecycle_generation
                                    == observation.lifecycle_generation
                        });
                    if !identity_matches {
                        tracing::info!(
                            target: "gwt.pane.teardown",
                            window_id = %observation.window_id,
                            reason = reason.as_str(),
                            grace_secs = grace.as_secs(),
                            "agent window became eligible for automatic terminal close"
                        );
                        self.terminal_close_candidates.insert(
                            observation.window_id.clone(),
                            TerminalCloseCandidate {
                                session_id: observation.session_id,
                                lifecycle_generation: observation.lifecycle_generation,
                                reason,
                                since: now,
                            },
                        );
                    }
                }
                TerminalCloseEligibility::Ineligible(_) => {
                    self.terminal_close_candidates
                        .remove(&observation.window_id);
                }
            }
        }
        self.close_expired_terminal_window_candidates_at(now)
    }

    /// FR-046: close every candidate whose grace elapsed, after revalidating
    /// the raw window, the linked Session, and the process-local lifecycle
    /// generation. Any mismatch drops the candidate without closing.
    pub(crate) fn close_expired_terminal_window_candidates_at(
        &mut self,
        now: Instant,
    ) -> Vec<OutboundEvent> {
        let grace = self.terminal_close_grace;
        let expired: Vec<(String, TerminalCloseCandidate)> = self
            .terminal_close_candidates
            .iter()
            .filter(|(_, candidate)| now.saturating_duration_since(candidate.since) >= grace)
            .map(|(window_id, candidate)| (window_id.clone(), candidate.clone()))
            .collect();
        let mut events = Vec::new();
        for (window_id, candidate) in expired {
            self.terminal_close_candidates.remove(&window_id);
            let session_matches = self
                .active_agent_sessions
                .get(&window_id)
                .is_some_and(|active| active.session_id == candidate.session_id);
            let generation_matches = self
                .window_lifecycle_generations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&window_id)
                .copied()
                == candidate.lifecycle_generation;
            if !self.window_lookup.contains_key(&window_id)
                || !session_matches
                || !generation_matches
            {
                tracing::info!(
                    target: "gwt.pane.teardown",
                    window_id = %window_id,
                    session_matches,
                    generation_matches,
                    "automatic terminal close skipped: window identity changed during grace"
                );
                continue;
            }
            tracing::info!(
                target: "gwt.pane.teardown",
                window_id = %window_id,
                session_id = %candidate.session_id,
                reason = candidate.reason.as_str(),
                "closing agent window after terminal convergence grace"
            );
            // The Monitor settlement was committed by the observer, so the
            // close must not publish a second (`window_closed`) transition.
            events.extend(self.close_window_after_issue_monitor_finalize_events(&window_id));
        }
        events
    }

    /// FR-047 / Issue #4143 (AC-2): decide whether automatic restore may spawn
    /// `session`.
    ///
    /// `window_id` is the persisted placeholder when one exists. An orphan
    /// Session (no persisted window) has no identity to compare against the
    /// Monitor binding, so only the identity-free facts — a closed Issue or
    /// a settled execution — can refuse it; `RevokedLaunch` needs the exact
    /// window and is never inferred from a missing one.
    #[cfg(test)]
    pub(crate) fn restore_work_terminality(
        &self,
        session: &gwt_agent::Session,
        project_root: &Path,
        window_id: Option<&str>,
    ) -> RestoreAdmission {
        gwt::project_runtime::installation::restore_saved_session_work_terminality(
            session,
            project_root,
            window_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

    #[test]
    fn non_completed_diagnoses_skip_the_additional_identity_read() {
        let session = gwt_agent::Session::new(
            Path::new("unused"),
            "work/issue-42",
            gwt_agent::AgentId::Codex,
        );
        for status in [
            ExecutionDiagnosisState::Active,
            ExecutionDiagnosisState::Blocked,
            ExecutionDiagnosisState::Missing,
            ExecutionDiagnosisState::Corrupt,
        ] {
            let mut execution = active_execution(42);
            execution.status = status;
            let mut reads = 0;
            assert!(!execution.matches_completed_session(&session, |_| {
                reads += 1;
                Ok(None)
            }));
            assert_eq!(
                reads, 0,
                "{status:?} never uses the Completed identity fence"
            );
        }
    }

    #[test]
    fn observer_reads_each_monitor_path_once_per_tick_including_failures() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions_dir = temp.path().join("sessions");
        let projects = [temp.path().join("project-a"), temp.path().join("project-b")];
        for project in &projects {
            std::fs::create_dir_all(project).expect("project dir");
        }
        let mut session =
            gwt_agent::Session::new(&projects[0], "work/issue-42", gwt_agent::AgentId::Codex);
        session.linked_issue_number = Some(42);
        session.update_status(gwt_agent::AgentStatus::Idle);
        session.save(&sessions_dir).expect("save session");
        let snapshots: Vec<_> = (0..4)
            .map(|index| TerminalWindowSnapshot {
                window_id: format!("tab-1::agent-{index}"),
                session_id: session.id.clone(),
                project_root: projects[index / 2].clone(),
                worktree_path: projects[0].clone(),
                window_status: WindowProcessStatus::Running,
                lifecycle_generation: Some(1),
            })
            .collect();
        let paths = projects.map(|project| gwt::issue_monitor_prefs_path_for_repo_path(&project));
        assert_ne!(paths[0], paths[1]);
        let mut reads = HashMap::<PathBuf, usize>::new();
        // Swap the failed path next tick: neither successful nor failed reads survive it.
        for (tick, failed_path) in paths.iter().enumerate() {
            let observations = observe_terminal_windows_with_monitor_loader(
                &sessions_dir,
                snapshots.clone(),
                Duration::from_secs(5),
                |path| {
                    *reads.entry(path.to_path_buf()).or_default() += 1;
                    (path != failed_path).then(|| {
                        gwt::IssueMonitorState::with_prefs(
                            gwt::IssueMonitorConfig::default(),
                            gwt::IssueMonitorPrefs::default(),
                        )
                    })
                },
            );
            assert_eq!(observations.len(), snapshots.len());
            for (index, observation) in observations.iter().enumerate() {
                assert_eq!(
                    observation.eligibility
                        == TerminalCloseEligibility::Ineligible("monitor_unreadable"),
                    paths[index / 2] == *failed_path,
                    "each window uses this tick's repository snapshot"
                );
            }
            for path in &paths {
                assert_eq!(
                    reads[path],
                    tick + 1,
                    "each repository is read once per tick, including failures"
                );
            }
        }
    }

    /// Issue #4783 AC-R1/R4: a fresh launch sees its predecessor's
    /// worktree record until it activates its own generation. That record
    /// must never authorize closing the new Session.
    #[test]
    fn completed_predecessor_does_not_close_a_fresh_session() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(gwt_core::process::hidden_command("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert!(gwt_core::process::hidden_command("git")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
                "--allow-empty"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        assert!(gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/fixture/terminal-4783.git"
            ])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let mut predecessor =
            gwt_agent::Session::new(&repo, "work/issue-42", gwt_agent::AgentId::Codex);
        predecessor.linked_issue_number = Some(42);
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            42,
            &predecessor.id,
            "gwt-execute",
            false,
        )
        .unwrap();
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &repo,
                &predecessor.id,
                gwt::cli::execution_state::ExecutionSettlement::Completed,
            )
            .unwrap(),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
        let mut fresh = gwt_agent::Session::new(&repo, "work/issue-42", gwt_agent::AgentId::Codex);
        fresh.linked_issue_number = Some(42);
        fresh.update_status(gwt_agent::AgentStatus::Running);
        let monitor = gwt::IssueMonitorState::with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::IssueMonitorPrefs {
                launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::fresh".to_string(),
                }],
                ..Default::default()
            },
        );
        let mut facts = read_terminal_window_facts_with_monitor(
            &fresh,
            "tab-1::fresh",
            WindowProcessStatus::Running,
            Some(&monitor),
        );
        // Settlement delivery is tested separately; isolate the identity fence.
        let execution = facts.execution.as_mut().unwrap();
        assert_eq!(execution.status, ExecutionDiagnosisState::Completed);
        assert_eq!(execution.binding, ExecutionBindingState::Terminal);
        execution.settlement_clear = true;
        execution.settlement_obligation_open = false;
        execution.open_obligations = 0;
        assert_eq!(
            classify_terminal_window(&facts),
            TerminalCloseEligibility::Ineligible("execution_session_mismatch"),
            "a completed predecessor cannot settle the new launch",
        );
    }

    /// Issue #4441: every ineligibility cause the close predicate can emit is
    /// answered by name on the restore side.
    ///
    /// The defect this guards is drift, not a wrong verdict: a cause added to
    /// [`classify_terminal_window`] for the close side used to become a new way
    /// for restore to respawn a finished window, silently, because the restore
    /// mapping treated "not closable" as "spawnable". Scanning the source keeps
    /// the check honest without a second list to maintain.
    #[test]
    fn every_close_ineligibility_cause_is_answered_by_the_restore_mapping() {
        let source = include_str!("../project_runtime/installation.rs");
        let classify = source
            .split_once("pub fn classify_terminal_window")
            .expect("close predicate")
            .1
            .split_once("\nfn restore_admission_for_ineligible")
            .expect("restore mapping follows the close predicate")
            .0;
        let mapping = source
            .split_once("fn restore_admission_for_ineligible")
            .expect("restore mapping")
            .1
            .split_once("\n/// Issue #4143 (AC-2)")
            .expect("restore mapping body")
            .0;

        let causes = classify
            .match_indices("Ineligible(\"")
            .map(|(index, marker)| {
                let rest = &classify[index + marker.len()..];
                rest.split_once('"').expect("terminated cause").0
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            causes.len() >= 8,
            "expected the close predicate's cause set, found {causes:?}"
        );

        let unmapped = causes
            .iter()
            .filter(|cause| !mapping.contains(&format!("\"{cause}\"")))
            .collect::<Vec<_>>();
        assert!(
            unmapped.is_empty(),
            "restore_admission_for_ineligible must answer each close-side cause by name, \
             or it will inherit the close side's polarity for it; unmapped: {unmapped:?}"
        );
    }

    fn settled_execution(owner: u64) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            status: ExecutionDiagnosisState::Completed,
            binding: ExecutionBindingState::Terminal,
            owner_number: Some(owner),
            session_matches_execution: true,
            settlement_clear: true,
            settlement_obligation_open: false,
            open_obligations: 0,
            launch_revoked: false,
        }
    }

    fn active_execution(owner: u64) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            status: ExecutionDiagnosisState::Active,
            binding: ExecutionBindingState::Bound,
            owner_number: Some(owner),
            session_matches_execution: true,
            settlement_clear: false,
            settlement_obligation_open: false,
            open_obligations: 1,
            launch_revoked: false,
        }
    }

    /// What `issue.monitor.stop` leaves on the ledger: a Blocked generation
    /// whose `missing_verification` is the revoked-launch settlement.
    fn revoked_execution(owner: u64) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            status: ExecutionDiagnosisState::Blocked,
            binding: ExecutionBindingState::Terminal,
            owner_number: Some(owner),
            session_matches_execution: true,
            settlement_clear: false,
            settlement_obligation_open: false,
            open_obligations: 0,
            launch_revoked: true,
        }
    }

    /// Issue #4783 AC-1: a generation the Monitor revoked is refused even
    /// when the Monitor prefs carry no hold any more; ordinary terminal
    /// Blocked generations also require explicit recovery.
    #[test]
    fn restore_refuses_a_monitor_revoked_generation_without_a_prefs_hold() {
        let placeholder = |execution: ExecutionTerminalFacts| TerminalWindowFacts {
            window_status: WindowProcessStatus::Stopped,
            session_status: Some(gwt_agent::AgentStatus::Stopped),
            ..facts(Some(execution), Some(MonitorTerminalFacts::default()))
        };
        assert_eq!(
            restore_admission_for_facts(&placeholder(revoked_execution(42)), true),
            RestoreAdmission::RefuseHeld("launch_revoked")
        );
        assert_eq!(
            restore_admission_for_facts(&placeholder(revoked_execution(42)), false),
            RestoreAdmission::RefuseHeld("launch_revoked"),
            "an orphan Session of a revoked generation does not come back either"
        );
        let mut recoverable = revoked_execution(42);
        recoverable.launch_revoked = false;
        assert_eq!(
            restore_admission_for_facts(&placeholder(recoverable), true),
            RestoreAdmission::RefuseHeld("execution_blocked"),
            "terminal Blocked generations require explicit recovery"
        );
        // The marker is read only off a Blocked diagnosis.
        let mut diagnosis =
            gwt::cli::execution_state::diagnose_for_projection(std::path::Path::new("."), None);
        diagnosis.ecr_status = ExecutionDiagnosisState::Blocked;
        diagnosis.missing_verification =
            Some(gwt::cli::execution_state::REVOKED_LAUNCH_MISSING_VERIFICATION.to_string());
        assert!(ExecutionTerminalFacts::from_diagnosis(&diagnosis).launch_revoked);
        diagnosis.ecr_status = ExecutionDiagnosisState::Active;
        assert!(!ExecutionTerminalFacts::from_diagnosis(&diagnosis).launch_revoked);
    }

    fn tracked_monitor() -> MonitorTerminalFacts {
        MonitorTerminalFacts {
            binds_this_window: true,
            ..MonitorTerminalFacts::default()
        }
    }

    fn facts(
        execution: Option<ExecutionTerminalFacts>,
        monitor: Option<MonitorTerminalFacts>,
    ) -> TerminalWindowFacts {
        TerminalWindowFacts {
            linked_issue: Some(42),
            session_status: Some(gwt_agent::AgentStatus::Idle),
            window_status: WindowProcessStatus::Running,
            execution,
            monitor,
        }
    }

    // T-623 / AS-40〜42: the positives and their paired fail-closed negatives.
    #[test]
    fn settled_execution_is_eligible_even_while_monitor_tracks_the_window() {
        assert_eq!(
            classify_terminal_window(&facts(Some(settled_execution(42)), Some(tracked_monitor()))),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::SettledExecution)
        );
    }

    #[test]
    fn closed_issue_record_is_eligible() {
        let monitor = MonitorTerminalFacts {
            issue_closed: true,
            ..MonitorTerminalFacts::default()
        };
        assert_eq!(
            classify_terminal_window(&facts(Some(active_execution(42)), Some(monitor.clone()))),
            TerminalCloseEligibility::Ineligible("obligation_open"),
            "an open obligation still retains the window"
        );
        let mut execution = active_execution(42);
        execution.open_obligations = 0;
        assert_eq!(
            classify_terminal_window(&facts(Some(execution), Some(monitor))),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::ClosedIssue)
        );
    }

    #[test]
    fn revoked_launch_is_eligible_only_for_a_stopped_superseded_window() {
        let monitor = MonitorTerminalFacts {
            binds_other_window: true,
            ..MonitorTerminalFacts::default()
        };
        let mut execution = active_execution(42);
        execution.open_obligations = 0;
        let mut live = facts(Some(execution.clone()), Some(monitor.clone()));
        assert_eq!(
            classify_terminal_window(&live),
            TerminalCloseEligibility::Ineligible("not_terminal"),
            "a superseded window that still runs is retained"
        );
        live.window_status = WindowProcessStatus::Stopped;
        live.session_status = Some(gwt_agent::AgentStatus::Stopped);
        assert_eq!(
            classify_terminal_window(&live),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::RevokedLaunch)
        );
    }

    fn idle_pane(
        execution: ExecutionTerminalFacts,
        monitor: MonitorTerminalFacts,
    ) -> TerminalWindowFacts {
        TerminalWindowFacts {
            window_status: WindowProcessStatus::Idle,
            ..facts(Some(execution), Some(monitor))
        }
    }

    fn duplicate_monitor() -> MonitorTerminalFacts {
        MonitorTerminalFacts {
            binds_other_window: true,
            ..MonitorTerminalFacts::default()
        }
    }

    fn stopped_monitor() -> MonitorTerminalFacts {
        // What `IssueMonitorState::stop_only` leaves: the binding revoked,
        // the row parked for a human, and a failure record naming the stop.
        MonitorTerminalFacts {
            needs_human: true,
            failure_hold: true,
            monitor_stopped: true,
            ..MonitorTerminalFacts::default()
        }
    }

    fn non_writer(execution: ExecutionTerminalFacts) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            binding: ExecutionBindingState::Stale,
            ..execution
        }
    }

    /// Issue #4802 AC-3: an idle duplicate whose Issue the Monitor bound to
    /// another window is closed, even though the execution still has open
    /// obligations; they belong to the writer, not to this pane.
    #[test]
    fn idle_pane_that_lost_ownership_is_eligible() {
        assert_eq!(
            classify_terminal_window(&idle_pane(
                non_writer(active_execution(42)),
                duplicate_monitor()
            )),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::LostOwnership)
        );
    }

    /// Issue #4802 AC-3 / #3482: a duplicate that is working, is held for
    /// recovery, or still holds the execution generation is never closed.
    #[test]
    fn lost_ownership_never_closes_a_working_or_writing_pane() {
        for status in [
            WindowProcessStatus::Running,
            WindowProcessStatus::Starting,
            WindowProcessStatus::Waiting,
        ] {
            let mut working = idle_pane(non_writer(active_execution(42)), duplicate_monitor());
            working.window_status = status;
            assert!(
                !matches!(
                    classify_terminal_window(&working),
                    TerminalCloseEligibility::Eligible(_)
                ),
                "{status:?} pane must be retained"
            );
        }
        let mut in_turn = idle_pane(non_writer(active_execution(42)), duplicate_monitor());
        in_turn.session_status = Some(gwt_agent::AgentStatus::Running);
        assert!(!matches!(
            classify_terminal_window(&in_turn),
            TerminalCloseEligibility::Eligible(_)
        ));
        let mut interrupted = idle_pane(non_writer(active_execution(42)), duplicate_monitor());
        interrupted.session_status = Some(gwt_agent::AgentStatus::Interrupted);
        assert_eq!(
            classify_terminal_window(&interrupted),
            TerminalCloseEligibility::Ineligible("session_interrupted")
        );
        assert_eq!(
            classify_terminal_window(&idle_pane(active_execution(42), duplicate_monitor())),
            TerminalCloseEligibility::Ineligible("obligation_open"),
            "the execution writer keeps its pane"
        );
        let mut corrupt = non_writer(active_execution(42));
        corrupt.binding = ExecutionBindingState::Corrupt;
        assert_eq!(
            classify_terminal_window(&idle_pane(corrupt, duplicate_monitor())),
            TerminalCloseEligibility::Ineligible("execution_blocked_or_corrupt")
        );
        let still_bound = MonitorTerminalFacts {
            binds_this_window: true,
            ..MonitorTerminalFacts::default()
        };
        assert_eq!(
            classify_terminal_window(&idle_pane(non_writer(active_execution(42)), still_bound)),
            TerminalCloseEligibility::Ineligible("obligation_open")
        );
    }

    /// Issue #4802 AC-3: the idle pane of an Issue stopped through the
    /// Monitor is closed; a pane still in a turn is not.
    #[test]
    fn idle_pane_of_a_monitor_stopped_issue_is_eligible() {
        assert_eq!(
            classify_terminal_window(&idle_pane(active_execution(42), stopped_monitor())),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::MonitorStopped)
        );
        let mut running = idle_pane(active_execution(42), stopped_monitor());
        running.window_status = WindowProcessStatus::Running;
        running.session_status = Some(gwt_agent::AgentStatus::Running);
        assert_eq!(
            classify_terminal_window(&running),
            TerminalCloseEligibility::Ineligible("needs_human")
        );
        // A failure hold that is not an operator stop keeps its pane.
        let failed = MonitorTerminalFacts {
            failure_hold: true,
            ..MonitorTerminalFacts::default()
        };
        assert_eq!(
            classify_terminal_window(&idle_pane(active_execution(42), failed)),
            TerminalCloseEligibility::Ineligible("failure_hold")
        );
    }

    /// Issue #4802 AC-4: a restart restores no window for a stopped, held or
    /// completed Issue, and still restores live Work.
    #[test]
    fn restore_refuses_stopped_held_and_completed_issues() {
        let placeholder = |execution: ExecutionTerminalFacts, monitor: MonitorTerminalFacts| {
            TerminalWindowFacts {
                window_status: WindowProcessStatus::Stopped,
                session_status: Some(gwt_agent::AgentStatus::Stopped),
                ..facts(Some(execution), Some(monitor))
            }
        };
        let restore = |facts: TerminalWindowFacts| restore_admission_for_facts(&facts, true);

        assert_eq!(
            restore(placeholder(active_execution(42), stopped_monitor())),
            RestoreAdmission::RefuseHeld("monitor_stopped")
        );
        assert_eq!(
            restore(placeholder(
                active_execution(42),
                MonitorTerminalFacts {
                    failure_hold: true,
                    ..MonitorTerminalFacts::default()
                }
            )),
            RestoreAdmission::RefuseHeld("failure_hold")
        );
        assert_eq!(
            restore(placeholder(
                active_execution(42),
                MonitorTerminalFacts {
                    needs_human: true,
                    ..MonitorTerminalFacts::default()
                }
            )),
            RestoreAdmission::RefuseHeld("needs_human")
        );
        let mut completed_unsettled = settled_execution(42);
        completed_unsettled.settlement_clear = false;
        completed_unsettled.settlement_obligation_open = true;
        assert_eq!(
            restore(placeholder(
                completed_unsettled,
                MonitorTerminalFacts::default()
            )),
            RestoreAdmission::RefuseCompletedWork,
            "a Completed execution is finished Work even with an open obligation"
        );
        assert_eq!(
            restore(placeholder(settled_execution(42), tracked_monitor())),
            RestoreAdmission::RefuseTerminal(TerminalCloseReason::SettledExecution)
        );
        assert_eq!(
            restore(placeholder(
                non_writer(active_execution(42)),
                duplicate_monitor()
            )),
            RestoreAdmission::RefuseTerminal(TerminalCloseReason::LostOwnership),
            "a duplicate placeholder does not come back"
        );
        assert_eq!(
            restore_admission_for_facts(
                &placeholder(non_writer(active_execution(42)), duplicate_monitor()),
                false
            ),
            RestoreAdmission::Admit,
            "an orphan Session has no window identity to judge"
        );
        assert_eq!(
            restore(placeholder(active_execution(42), tracked_monitor())),
            RestoreAdmission::Admit,
            "live Work still restores"
        );
    }

    #[test]
    fn fail_closed_exclusions_retain_the_window() {
        let settled = settled_execution(42);
        let mut manual = facts(Some(settled.clone()), Some(tracked_monitor()));
        manual.linked_issue = None;
        assert_eq!(
            classify_terminal_window(&manual),
            TerminalCloseEligibility::Ineligible("no_linked_issue")
        );

        let unsettled = facts(Some(active_execution(42)), Some(tracked_monitor()));
        assert_eq!(
            classify_terminal_window(&unsettled),
            TerminalCloseEligibility::Ineligible("obligation_open")
        );
        let mut tracked_not_terminal = facts(Some(active_execution(42)), Some(tracked_monitor()));
        tracked_not_terminal
            .execution
            .as_mut()
            .unwrap()
            .open_obligations = 0;
        assert_eq!(
            classify_terminal_window(&tracked_not_terminal),
            TerminalCloseEligibility::Ineligible("monitor_tracking_unsettled")
        );

        let needs_human = MonitorTerminalFacts {
            needs_human: true,
            issue_closed: true,
            ..MonitorTerminalFacts::default()
        };
        assert_eq!(
            classify_terminal_window(&facts(Some(settled.clone()), Some(needs_human))),
            TerminalCloseEligibility::Ineligible("needs_human")
        );
        let failure_hold = MonitorTerminalFacts {
            failure_hold: true,
            ..MonitorTerminalFacts::default()
        };
        assert_eq!(
            classify_terminal_window(&facts(Some(settled.clone()), Some(failure_hold))),
            TerminalCloseEligibility::Ineligible("failure_hold")
        );

        let mut error_window = facts(Some(settled.clone()), Some(tracked_monitor()));
        error_window.window_status = WindowProcessStatus::Error;
        assert_eq!(
            classify_terminal_window(&error_window),
            TerminalCloseEligibility::Ineligible("window_error")
        );
        let mut interrupted = facts(Some(settled.clone()), Some(tracked_monitor()));
        interrupted.session_status = Some(gwt_agent::AgentStatus::Interrupted);
        assert_eq!(
            classify_terminal_window(&interrupted),
            TerminalCloseEligibility::Ineligible("session_interrupted")
        );
        let mut blocked = settled.clone();
        blocked.status = ExecutionDiagnosisState::Blocked;
        assert_eq!(
            classify_terminal_window(&facts(Some(blocked), Some(tracked_monitor()))),
            TerminalCloseEligibility::Ineligible("execution_blocked_or_corrupt")
        );
        let mut open_obligation = settled.clone();
        open_obligation.settlement_obligation_open = true;
        assert_eq!(
            classify_terminal_window(&facts(Some(open_obligation), Some(tracked_monitor()))),
            TerminalCloseEligibility::Ineligible("obligation_open")
        );
        let mut foreign_owner = settled.clone();
        foreign_owner.owner_number = Some(7);
        assert_eq!(
            classify_terminal_window(&facts(Some(foreign_owner), Some(tracked_monitor()))),
            TerminalCloseEligibility::Ineligible("monitor_tracking_unsettled"),
            "a settled record for another owner is not this window's settlement"
        );

        assert_eq!(
            classify_terminal_window(&facts(None, Some(tracked_monitor()))),
            TerminalCloseEligibility::Ineligible("execution_unreadable")
        );
        assert_eq!(
            classify_terminal_window(&facts(Some(settled.clone()), None)),
            TerminalCloseEligibility::Ineligible("monitor_unreadable")
        );
        let mut unreadable_session = facts(Some(settled), Some(tracked_monitor()));
        unreadable_session.session_status = None;
        assert_eq!(
            classify_terminal_window(&unreadable_session),
            TerminalCloseEligibility::Ineligible("session_unreadable")
        );
    }
}
