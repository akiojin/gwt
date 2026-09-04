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

/// Facts read from the exact Worktree's execution diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionTerminalFacts {
    pub(crate) status: gwt::cli::execution_state::ExecutionDiagnosisState,
    pub(crate) binding: gwt::cli::execution_state::ExecutionBindingState,
    pub(crate) owner_number: Option<u64>,
    /// `settlement_severity == "clear"`.
    pub(crate) settlement_clear: bool,
    pub(crate) settlement_obligation_open: bool,
    pub(crate) open_obligations: usize,
}

impl ExecutionTerminalFacts {
    pub(crate) fn from_diagnosis(
        diagnosis: &gwt::cli::execution_state::ExecutionDiagnosisSnapshot,
    ) -> Self {
        Self {
            status: diagnosis.ecr_status,
            binding: diagnosis.binding_state,
            owner_number: diagnosis.owner_number,
            settlement_clear: diagnosis.settlement_severity == "clear",
            settlement_obligation_open: diagnosis.settlement_obligation_open,
            open_obligations: diagnosis.open_obligations.len(),
        }
    }
}

/// Facts read from the repository-scoped Issue Monitor prefs for one window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MonitorTerminalFacts {
    /// The Monitor durably binds this exact window to the linked Issue.
    pub(crate) binds_this_window: bool,
    /// The Monitor durably binds the linked Issue to a *different* window:
    /// this launch was replaced (failover / relaunch), i.e. revoked.
    pub(crate) binds_other_window: bool,
    /// A durable closed-Issue record (Released) or validated Issue-wide
    /// completion (Merged) exists for the linked Issue.
    pub(crate) issue_closed: bool,
    /// The row is parked for a human (`needs_human`).
    pub(crate) needs_human: bool,
    /// The Issue holds a failure record (launch / agent failure, or an
    /// operator stop hold).
    pub(crate) failure_hold: bool,
}

impl MonitorTerminalFacts {
    pub(crate) fn from_monitor(
        monitor: &gwt::IssueMonitorState,
        issue_number: u64,
        window_id: &str,
    ) -> Self {
        let facts = monitor.terminal_window_facts(issue_number, window_id);
        Self {
            binds_this_window: facts.binds_this_window,
            binds_other_window: facts.binds_other_window,
            issue_closed: facts.issue_closed,
            needs_human: facts.needs_human,
            failure_hold: facts.failure_hold,
        }
    }
}

/// Everything the canonical terminal predicate consumes. `None` for a fact
/// means it could not be read; every such reading fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalWindowFacts {
    pub(crate) linked_issue: Option<u64>,
    pub(crate) session_status: Option<gwt_agent::AgentStatus>,
    pub(crate) window_status: WindowProcessStatus,
    pub(crate) execution: Option<ExecutionTerminalFacts>,
    pub(crate) monitor: Option<MonitorTerminalFacts>,
}

/// Why a window is eligible for automatic terminal cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalCloseReason {
    /// FR-044 (a): Completed ECR, Terminal binding, clear settlement, no open
    /// obligation.
    SettledExecution,
    /// FR-044 (b): durable closed / completed Issue record.
    ClosedIssue,
    /// FR-044 (c): the Monitor durably replaced this launch with another
    /// window and this one is no longer running.
    RevokedLaunch,
}

impl TerminalCloseReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SettledExecution => "settled_execution",
            Self::ClosedIssue => "closed_issue",
            Self::RevokedLaunch => "revoked_launch",
        }
    }
}

/// The canonical terminal predicate (FR-044). Pure; every fact is an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalCloseEligibility {
    Eligible(TerminalCloseReason),
    Ineligible(&'static str),
}

/// FR-044: decide whether one Issue-linked Agent window may be closed by the
/// runtime. Missing, stale, corrupt, or unreadable facts fail closed; so do
/// NeedsHuman, Error / Interrupted / Blocked, open obligations, and windows
/// the Monitor still tracks without a terminal fact.
pub(crate) fn classify_terminal_window(facts: &TerminalWindowFacts) -> TerminalCloseEligibility {
    use gwt::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};
    use TerminalCloseEligibility::{Eligible, Ineligible};

    let Some(issue_number) = facts.linked_issue else {
        return Ineligible("no_linked_issue");
    };
    // An Error pane is a diagnostic the operator may still need to read
    // (AS-42 / AS-45); pre-PTY Monitor failures converge through their own
    // acknowledged path instead.
    if facts.window_status == WindowProcessStatus::Error {
        return Ineligible("window_error");
    }
    let Some(session_status) = facts.session_status else {
        return Ineligible("session_unreadable");
    };
    if session_status == gwt_agent::AgentStatus::Interrupted {
        return Ineligible("session_interrupted");
    }
    let Some(monitor) = facts.monitor.as_ref() else {
        return Ineligible("monitor_unreadable");
    };
    if monitor.needs_human {
        return Ineligible("needs_human");
    }
    if monitor.failure_hold {
        return Ineligible("failure_hold");
    }
    let Some(execution) = facts.execution.as_ref() else {
        return Ineligible("execution_unreadable");
    };
    if matches!(
        execution.status,
        ExecutionDiagnosisState::Blocked | ExecutionDiagnosisState::Corrupt
    ) || execution.binding == ExecutionBindingState::Corrupt
    {
        return Ineligible("execution_blocked_or_corrupt");
    }
    if execution.settlement_obligation_open || execution.open_obligations > 0 {
        return Ineligible("obligation_open");
    }

    // (b) durable closed / completed Issue record.
    if monitor.issue_closed {
        return Eligible(TerminalCloseReason::ClosedIssue);
    }
    // (a) fully settled execution for this exact owner.
    if execution.status == ExecutionDiagnosisState::Completed
        && execution.binding == ExecutionBindingState::Terminal
        && execution.settlement_clear
        && execution.owner_number == Some(issue_number)
    {
        return Eligible(TerminalCloseReason::SettledExecution);
    }
    // (c) the Monitor durably replaced this launch and nothing runs here.
    if monitor.binds_other_window
        && !monitor.binds_this_window
        && facts.window_status == WindowProcessStatus::Stopped
        && session_status == gwt_agent::AgentStatus::Stopped
    {
        return Eligible(TerminalCloseReason::RevokedLaunch);
    }
    if monitor.binds_this_window {
        return Ineligible("monitor_tracking_unsettled");
    }
    Ineligible("not_terminal")
}

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
pub(crate) fn read_terminal_window_facts(
    session: &gwt_agent::Session,
    window_id: &str,
    window_status: WindowProcessStatus,
    project_root: &Path,
) -> TerminalWindowFacts {
    let Some(issue_number) = session.linked_issue_number else {
        return TerminalWindowFacts {
            linked_issue: None,
            session_status: Some(session.status),
            window_status,
            execution: None,
            monitor: None,
        };
    };
    let execution = Some(ExecutionTerminalFacts::from_diagnosis(
        &gwt::cli::execution_state::diagnose_for_projection(
            &session.worktree_path,
            Some(&session.id),
        ),
    ));
    let monitor =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(project_root))
            .ok()
            .map(|prefs| {
                let monitor =
                    gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
                MonitorTerminalFacts::from_monitor(&monitor, issue_number, window_id)
            });
    TerminalWindowFacts {
        linked_issue: Some(issue_number),
        session_status: Some(session.status),
        window_status,
        execution,
        monitor,
    }
}

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
    snapshots
        .into_iter()
        .map(|snapshot| {
            let session_path = sessions_dir.join(format!("{}.toml", snapshot.session_id));
            let eligibility = match gwt_agent::Session::load_and_migrate(&session_path) {
                Ok(session) => {
                    let facts = read_terminal_window_facts(
                        &session,
                        &snapshot.window_id,
                        snapshot.window_status,
                        &snapshot.project_root,
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
        let spawn = self.blocking_tasks.try_spawn(move || {
            let grace = configured_terminal_close_grace(profile_config_path.as_deref());
            let observations =
                observe_terminal_windows_in_background(&sessions_dir, snapshots, commit_timeout);
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

    /// FR-047: decide whether automatic restore may spawn `session`. Returns
    /// the terminal reason when the persisted window is already eligible;
    /// the caller then disables restore and removes the placeholder.
    pub(crate) fn restore_admission_terminal_reason(
        &self,
        session: &gwt_agent::Session,
        project_root: &Path,
        window_id: &str,
    ) -> Option<TerminalCloseReason> {
        session.linked_issue_number?;
        let facts = read_terminal_window_facts(
            session,
            window_id,
            WindowProcessStatus::Stopped,
            project_root,
        );
        match classify_terminal_window(&facts) {
            TerminalCloseEligibility::Eligible(reason) => Some(reason),
            TerminalCloseEligibility::Ineligible(_) => None,
        }
    }

    /// FR-047: persist the restore refusal (Session Stopped + restore
    /// disabled) and drop the paused placeholder so nothing spawns.
    pub(crate) fn refuse_terminal_session_restore(
        &mut self,
        tab_id: &str,
        session_id: &str,
        reason: TerminalCloseReason,
    ) {
        match gwt_agent::update_session_if_changed(&self.sessions_dir, session_id, |session| {
            session.restore_window_on_startup = false;
            if session.status != gwt_agent::AgentStatus::Stopped {
                session.update_status(gwt_agent::AgentStatus::Stopped);
            }
            Ok(())
        }) {
            Ok(_) => tracing::info!(
                target: "gwt.pane.teardown",
                session_id,
                reason = reason.as_str(),
                "automatic restore refused: the linked Work is terminal"
            ),
            Err(error) => tracing::warn!(
                target: "gwt.pane.teardown",
                session_id,
                %error,
                "automatic restore refused, but the Session could not be marked restore-disabled"
            ),
        }
        self.remove_stale_paused_agent_window(tab_id, session_id);
        let _ = self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

    fn settled_execution(owner: u64) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            status: ExecutionDiagnosisState::Completed,
            binding: ExecutionBindingState::Terminal,
            owner_number: Some(owner),
            settlement_clear: true,
            settlement_obligation_open: false,
            open_obligations: 0,
        }
    }

    fn active_execution(owner: u64) -> ExecutionTerminalFacts {
        ExecutionTerminalFacts {
            status: ExecutionDiagnosisState::Active,
            binding: ExecutionBindingState::Bound,
            owner_number: Some(owner),
            settlement_clear: false,
            settlement_obligation_open: false,
            open_obligations: 1,
        }
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
