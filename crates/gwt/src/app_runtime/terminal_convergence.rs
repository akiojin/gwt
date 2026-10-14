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
    /// Issue #4783 AC-1: the generation was Blocked by a revoked-launch
    /// release — what `issue.monitor.stop` writes on the ledger. Durable on
    /// the execution side, so it still says "stopped through the Monitor"
    /// after the Monitor prefs lost the hold (a requeue, a prefs reset, a
    /// different store) and until `execution.reopen` clears it.
    pub(crate) launch_revoked: bool,
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
            launch_revoked: diagnosis.ecr_status
                == gwt::cli::execution_state::ExecutionDiagnosisState::Blocked
                && diagnosis.missing_verification.as_deref()
                    == Some(gwt::cli::execution_state::REVOKED_LAUNCH_MISSING_VERIFICATION),
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
    /// Issue #4802 AC-3: the hold is an operator stop through the Monitor.
    pub(crate) monitor_stopped: bool,
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
            monitor_stopped: facts.monitor_stopped,
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
    /// Issue #4802 AC-3: an idle pane whose Issue the Monitor bound to another
    /// window, and whose Session is not the Issue's execution writer — the
    /// duplicate a relaunch left behind.
    LostOwnership,
    /// Issue #4802 AC-3: an idle pane of an Issue the operator stopped
    /// through the Monitor.
    MonitorStopped,
}

impl TerminalCloseReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SettledExecution => "settled_execution",
            Self::ClosedIssue => "closed_issue",
            Self::RevokedLaunch => "revoked_launch",
            Self::LostOwnership => "lost_ownership",
            Self::MonitorStopped => "monitor_stopped",
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
    // Issue #4802 AC-3: ahead of the hold checks, because a Monitor stop is
    // itself a hold, and ahead of the obligation checks, because a pane that
    // lost ownership is not the writer those obligations belong to.
    if let Some(reason) = orphaned_idle_pane_reason(facts, monitor, session_status) {
        return Eligible(reason);
    }
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

/// Issue #4802 AC-3: an idle pane nothing owns any more.
///
/// The close predicate used to reach only a fully Stopped revoked launch, so
/// the prompt-ready duplicates a relaunch left behind and the panes of Issues
/// the operator stopped stayed on the canvas indefinitely. Both are closed
/// here, and only while idle: a pane in a turn (Running / Starting / Waiting)
/// or held for recovery (Interrupted) is never a candidate, which keeps the
/// #3482 rule that a working pane is not killed. A pane the Monitor still
/// binds is never orphaned, and an unreadable or corrupt execution record
/// fails closed.
fn orphaned_idle_pane_reason(
    facts: &TerminalWindowFacts,
    monitor: &MonitorTerminalFacts,
    session_status: gwt_agent::AgentStatus,
) -> Option<TerminalCloseReason> {
    use gwt::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

    if monitor.binds_this_window {
        return None;
    }
    let idle = matches!(
        facts.window_status,
        WindowProcessStatus::Idle | WindowProcessStatus::Stopped
    ) && matches!(
        session_status,
        gwt_agent::AgentStatus::Idle | gwt_agent::AgentStatus::Stopped
    );
    if !idle {
        return None;
    }
    let execution = facts.execution.as_ref()?;
    if execution.status == ExecutionDiagnosisState::Corrupt
        || execution.binding == ExecutionBindingState::Corrupt
    {
        return None;
    }
    if monitor.monitor_stopped {
        return Some(TerminalCloseReason::MonitorStopped);
    }
    // The Session that still holds the execution generation is the writer:
    // closing it would strand the Work, whichever window the Monitor binds.
    if monitor.binds_other_window
        && !monitor.needs_human
        && execution.binding != ExecutionBindingState::Bound
    {
        return Some(TerminalCloseReason::LostOwnership);
    }
    None
}

/// Issue #4441: translate one close-side ineligibility cause into a
/// restore-side admission.
///
/// [`classify_terminal_window`] answers a single question — *may the runtime
/// close this window?* — so every fact it cannot prove comes back as
/// `Ineligible`, which is the safe answer for closing. Restore has the
/// opposite polarity, and it used to read the whole `Ineligible` family as
/// permission to spawn. That inversion is why `issue.monitor.stop` respawned
/// the row it stopped: the stop writes `failed_issues`, which reads as
/// `failure_hold`, which the close side reports as "do not close" and restore
/// then took as "do spawn". The operator's only lever recreated the window it
/// was pressed to remove.
///
/// So the mapping is an allowlist. Only the causes that positively establish
/// that the Work is still live admit a restore; everything else refuses, and
/// **a cause added to the close predicate later refuses by default** instead of
/// silently becoming a new way to respawn a finished window.
fn restore_admission_for_ineligible(cause: &'static str) -> RestoreAdmission {
    match cause {
        // The Work is live, or there is no Issue-linked Work to read at all
        // (a PM pane, a manual launch). These are the restores that should
        // happen.
        //
        // `execution_blocked_or_corrupt` and `session_interrupted` belong here:
        // a blocked execution is recovered by adopting and reopening it, and an
        // interrupted Session is precisely what auto-resume exists to continue
        // ([`gwt_agent::Session::exact_auto_resume_candidate`] accepts it).
        "no_linked_issue"
        | "not_terminal"
        | "monitor_tracking_unsettled"
        | "obligation_open"
        | "execution_blocked_or_corrupt"
        | "session_interrupted" => RestoreAdmission::Admit,
        // Issue #4441 (AC-3): the Issue Monitor is holding this row. A hold is
        // reversible, so the placeholder and the restore flag stay: releasing
        // the row brings the window back.
        "needs_human" | "failure_hold" => RestoreAdmission::RefuseHeld(cause),
        // Issue #4143: the canonical facts could not be read. Spawning on an
        // unreadable fact is what turned one descriptor exhaustion (#4142) into
        // 254 respawned windows, 135 of which died before PTY start. An
        // unreadable fact is no evidence that the window is finished either, so
        // the placeholder survives for the next generation to answer.
        //
        // `window_error` is the same answer for a different reason: an error
        // pane is a diagnostic the operator may still need, and respawning it
        // automatically is what #4143 (AC-3) removed.
        "session_unreadable" | "monitor_unreadable" | "execution_unreadable" | "window_error" => {
            RestoreAdmission::RefuseUnprovable(cause)
        }
        // A cause this function does not name is, by definition, one it could
        // not establish. Refusing here is what keeps a later addition to the
        // close predicate from becoming a new way to respawn a finished window.
        _ => RestoreAdmission::RefuseUnprovable(cause),
    }
}

/// Issue #4143 (AC-2): the admission decision for one restore candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreAdmission {
    /// The Work is live, or the Session carries no Issue-linked Work at all.
    Admit,
    /// The Work is provably terminal: disable restore and drop the
    /// placeholder so the window stops coming back.
    RefuseTerminal(TerminalCloseReason),
    /// The owner is closed, but cleanup must retain its diagnostic window.
    RefuseRetainedTerminal,
    /// Issue #4802 (AC-4): the execution reached Completed, so the Work is
    /// finished even while a settlement obligation is open. Do not spawn;
    /// keep the placeholder as the diagnostic.
    RefuseCompletedWork,
    /// Issue #4441 (AC-3): the Issue Monitor is holding this row. Do not
    /// spawn, but keep the placeholder — the hold is reversible.
    RefuseHeld(&'static str),
    /// The canonical facts could not be read. Do not spawn, but keep the
    /// placeholder: the next generation may be able to prove the answer.
    RefuseUnprovable(&'static str),
}

/// FR-047 / Issue #4143 (AC-2) / Issue #4802 (AC-4): the restore decision for
/// one Issue-linked Session, from the canonical facts. `has_window` is whether
/// a persisted placeholder names the exact window; an orphan Session has no
/// identity to compare against the Monitor binding.
pub(crate) fn restore_admission_for_facts(
    facts: &TerminalWindowFacts,
    has_window: bool,
) -> RestoreAdmission {
    match classify_terminal_window(facts) {
        TerminalCloseEligibility::Eligible(
            TerminalCloseReason::RevokedLaunch | TerminalCloseReason::LostOwnership,
        ) if !has_window => RestoreAdmission::Admit,
        // Issue #4441 AC-3 / #4802 AC-4: a stop is a reversible hold, so
        // nothing spawns but the placeholder stays for the release.
        TerminalCloseEligibility::Eligible(TerminalCloseReason::MonitorStopped) => {
            RestoreAdmission::RefuseHeld("monitor_stopped")
        }
        TerminalCloseEligibility::Eligible(reason) => RestoreAdmission::RefuseTerminal(reason),
        // Reopened #4143: diagnostic retention is not permission to
        // restart a closed owner's process. Keep its placeholder, but
        // refuse automatic spawn even for Blocked/open-obligation ECRs.
        // This outranks the per-cause mapping below, which answers "could
        // this cause be established", not "is this owner finished".
        TerminalCloseEligibility::Ineligible(_)
            if facts
                .monitor
                .as_ref()
                .is_some_and(|monitor| monitor.issue_closed) =>
        {
            RestoreAdmission::RefuseRetainedTerminal
        }
        // Issue #4783 AC-1: a generation Blocked by `issue.monitor.stop` is
        // not the recoverable Blocked that `execution_blocked_or_corrupt`
        // admits below — the operator stopped this Work. The Monitor-side
        // hold (`failure_hold` / `monitor_stopped`) refuses it while the
        // prefs carry it; the ledger marker refuses it after they stop
        // carrying it, which is how a stopped Issue's window came back on
        // the next restart. Held, not terminal: the placeholder stays, and
        // `execution.reopen` lifts the marker.
        TerminalCloseEligibility::Ineligible(_)
            if facts
                .execution
                .as_ref()
                .is_some_and(|execution| execution.launch_revoked) =>
        {
            RestoreAdmission::RefuseHeld("launch_revoked")
        }
        // Issue #4802 AC-4: an execution that reached Completed is
        // finished Work even while a settlement obligation is still open.
        // Restarting its agent is what brought finished Issues' windows
        // back after a restart; the placeholder is retained as the
        // diagnostic, but nothing spawns.
        TerminalCloseEligibility::Ineligible(_)
            if facts.execution.as_ref().is_some_and(|execution| {
                execution.status == gwt::cli::execution_state::ExecutionDiagnosisState::Completed
            }) =>
        {
            RestoreAdmission::RefuseCompletedWork
        }
        TerminalCloseEligibility::Ineligible(cause) => restore_admission_for_ineligible(cause),
    }
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
    pub(crate) fn restore_work_terminality(
        &self,
        session: &gwt_agent::Session,
        project_root: &Path,
        window_id: Option<&str>,
    ) -> RestoreAdmission {
        if session.linked_issue_number.is_none() {
            // A Session with no Issue link has no Work whose terminality this
            // predicate can read (a PM pane, a manual launch). Restore keeps
            // the "everything the user did not explicitly close" rule.
            return RestoreAdmission::Admit;
        }
        let facts = read_terminal_window_facts(
            session,
            window_id.unwrap_or_default(),
            WindowProcessStatus::Stopped,
            project_root,
        );
        restore_admission_for_facts(&facts, window_id.is_some())
    }

    /// Persist a terminal/empty restore refusal and remove its placeholder.
    pub(crate) fn remove_refused_session_restore(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window_id: Option<&str>,
        reason: &str,
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
                reason,
                "automatic restore refused: removing the stopped placeholder"
            ),
            Err(error) => tracing::warn!(
                target: "gwt.pane.teardown",
                session_id,
                %error,
                "automatic restore refused, but the Session could not be marked restore-disabled"
            ),
        }
        self.remove_stale_paused_agent_window(tab_id, session_id, window_id);
        let _ = self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

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
        let source = include_str!("terminal_convergence.rs");
        let classify = source
            .split_once("pub(crate) fn classify_terminal_window")
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
            settlement_clear: false,
            settlement_obligation_open: false,
            open_obligations: 0,
            launch_revoked: true,
        }
    }

    /// Issue #4783 AC-1: a generation the Monitor revoked is refused even
    /// when the Monitor prefs carry no hold any more; an ordinary Blocked
    /// generation keeps today's recoverable admission.
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
            RestoreAdmission::Admit,
            "a Blocked generation nobody revoked is still recovered by restore"
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
