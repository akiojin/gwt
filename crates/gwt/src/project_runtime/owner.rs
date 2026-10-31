//! The existing daemon's single physical pane store and enqueue-only mailbox.
use super::events::{OwnerCommand, OwnerEvent, OwnerReply, OwnerRequest};
use super::presence::{GuiPresence, ResidencySnapshot};
use super::{
    installation::{InstallAgentLaunchInput, PreparedAgentLaunchSuccess},
    launch_payload::{ActiveAgentSession, AgentLaunchError},
};
use crate::{
    agent_capability::AgentCapabilityIssuer,
    pane_runtime::{self, PaneRuntimeEvent},
    project_transport::{ClientHub, TransportEvent},
    WindowProcessStatus,
};
use gwt_terminal::{pty::SpawnConfig, Pane, PtyHandle};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
};
use tokio::sync::{broadcast, oneshot};

fn spawn_owner_worker<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> thread::JoinHandle<T> {
    #[cfg(any(test, feature = "test-gh-guard"))]
    let home = gwt_core::test_support::gwt_home_override();
    #[cfg(any(test, feature = "test-gh-guard"))]
    let clock = gwt_core::operation_deadline::ScopedOperationClock::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    thread::spawn(move || {
        let _dispatch = tracing::dispatcher::set_default(&dispatch);
        #[cfg(any(test, feature = "test-gh-guard"))]
        let _home = home
            .as_ref()
            .map(gwt_core::test_support::ScopedGwtHome::set);
        #[cfg(any(test, feature = "test-gh-guard"))]
        let _clock = clock.map(gwt_core::operation_deadline::ScopedOperationClock::set);
        work()
    })
}

pub(crate) struct WindowRuntime {
    project_root: PathBuf,
    pub(crate) policy: super::runtime_events::RuntimePolicyState,
    pub(crate) _initial_prompt_file: Option<Arc<tempfile::TempPath>>,
    pub(crate) started_at_ms: u64,
    child_started_at: Option<u64>,
    pub(crate) current: Arc<AtomicBool>,
    pub(crate) registry_cleanup: Option<super::agent_dispatch::OwnedWindowRecordCleanup>,
    close_handoff: Option<crate::session_finalizer::CloseHandoffReservation>,
    close_monitor_snapshot: Option<crate::IssueMonitorWindowSnapshot>,
    pm_lifecycle: Option<super::pm_delivery::PmLifecycleCommit>,
    rollback_monitor_failure: Option<(super::monitor::MonitorLaunch, String)>,
    member_cleanup: Option<(gwt_agent::Session, String)>,
    detail: Option<String>,
    last_output_at: Option<String>,
    pub(crate) pty: Arc<PtyHandle>,
    pub(crate) incarnation: u64,
    pub(crate) pane: Arc<Mutex<Pane>>,
    pub(crate) output_thread: Option<thread::JoinHandle<()>>,
    pub(crate) status_thread: Option<thread::JoinHandle<()>>,
    pty_status: crate::WindowProcessStatus,
    hook_status: Option<crate::WindowProcessStatus>,
    output_bytes: u64,
    pub(crate) agent: Option<AgentRuntimeRecord>,
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.current.store(false, Ordering::Release);
        self.pty.revoke_input_generation();
        if let Some(agent) = &self.agent {
            agent.current.store(false, Ordering::Release);
        }
        if self.pty.tree_exit_confirmed()
            && self.output_thread.is_none()
            && self.status_thread.is_none()
        {
            return;
        }
        // Reader/status workers retain the PTY. Dispose of the captured
        // physical resources off the actor, without locking the Pane.
        let pty = self.pty.clone();
        let output_thread = self.output_thread.take();
        let status_thread = self.status_thread.take();
        let prompt_file = self._initial_prompt_file.take();
        spawn_owner_worker(move || {
            let _prompt_file = prompt_file;
            if !pty.tree_exit_confirmed() {
                match crate::session_finalizer::kill_and_reap(&pty, "dropped-owner-runtime") {
                    Ok(true) => {}
                    result => {
                        tracing::error!(?result, "dropped owner runtime tree exit is unconfirmed")
                    }
                }
            }
            pty.release_descriptors();
            for worker in [output_thread, status_thread].into_iter().flatten() {
                if worker.join().is_err() {
                    tracing::error!("dropped owner runtime worker panicked");
                }
            }
        });
    }
}

pub(crate) struct AgentRuntimeRecord {
    pub(crate) feedback: Option<super::launch_payload::LaunchFeedbackContext>,
    monitor_owner: Option<(PathBuf, String)>,
    monitor_terminal_failure: Option<(String, Option<crate::IssueMonitorFailure>)>,
    monitor_answer_inflight: bool,
    binding_repair_inflight: bool,
    monitor_source: Option<crate::runtime_daemon_events::IssueMonitorAgentFailureSource>,
    monitor_commit_inflight: bool,
    monitor_ready: bool,
    monitor_retry_at: Option<std::time::Instant>,
    monitor_startup_at: Option<std::time::Instant>,
    monitor_adopt_claim: bool,
    monitor_ack_replies: Vec<oneshot::Sender<OwnerReply>>,
    pub(crate) current: Arc<AtomicBool>,
    pub(crate) active: ActiveAgentSession,
    pub(crate) snapshot: gwt_agent::Session,
    pub(crate) capability_token: Option<String>,
    pub(crate) issuer: AgentCapabilityIssuer,
    pub(crate) pending_fresh: Option<super::launch_payload::PendingFreshExecutionLaunch>,
    pub(crate) pending_continue: Option<super::launch_payload::PendingContinueWork>,
    purpose_title: Option<String>,
    readiness_deadline: Option<ReadinessDeadline>,
}

struct AgentOperation {
    session_id: Option<String>,
    early_events: VecDeque<TransportEvent>,
    request: serde_json::Value,
    id: String,
    incarnation: u64,
    current: Arc<AtomicBool>,
    replies: Vec<oneshot::Sender<OwnerReply>>,
    cancel_replies: Vec<oneshot::Sender<OwnerReply>>,
    cancellation_cleaned: bool,
    manual_predecessor_stopped: Option<gwt_agent::SessionExecutionIdentity>,
    retained_manual_failure: Option<super::events::OwnerRetainedManualLaunchFailure>,
    pending_continue: Option<super::launch_payload::PendingContinueWork>,
    failed_session: Option<gwt_agent::Session>,
    result: Option<OwnerReply>,
}

impl AgentOperation {
    fn pending_monitor_rejection_issue(&self) -> Option<u64> {
        if self.result.is_some() || self.request["type"] != "reject_monitor_launch" {
            return None;
        }
        // Cancellation does not finish the worker's exact canonical commit.
        self.request
            .pointer("/rejection/expected_delivery/issue_number")
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                self.request
                    .pointer("/rejection/expected_handoff/issue_number")
                    .and_then(serde_json::Value::as_u64)
            })
    }

    fn pending_monitor_rejection_matches_source(
        &self,
        owner: crate::cli::execution_state::ExecutionOwnerKey,
        worktree_path: &Path,
        branch: Option<&str>,
        source_session_id: Option<&str>,
    ) -> bool {
        if self.pending_monitor_rejection_issue() != Some(owner.number) {
            return false;
        }
        let Some(rejection) = self.request.get("rejection").and_then(|value| {
            serde_json::from_value::<super::events::OwnerMonitorLaunchRejection>(value.clone()).ok()
        }) else {
            return false;
        };
        let (source, handoff) = match (
            rejection.expected_session.as_deref(),
            rejection.expected_handoff.as_deref(),
        ) {
            (Some(source), Some(handoff)) => (source, handoff),
            (None, None) => {
                // An initial auth refusal captures a delivery, not an asking
                // Session. Strict target resolution has already verified this
                // path's project membership and its actual Git branch/owner.
                return rejection
                    .expected_delivery
                    .as_deref()
                    .is_some_and(|delivery| {
                        delivery.issue_number == owner.number
                            && delivery.linked_issue_kind
                                == match owner.kind {
                                    crate::cli::execution_state::ExecutionOwnerKind::Spec => {
                                        crate::LinkedIssueKind::Spec
                                    }
                                    crate::cli::execution_state::ExecutionOwnerKind::Issue => {
                                        crate::LinkedIssueKind::Issue
                                    }
                                }
                            && branch.is_some_and(|branch| {
                                !delivery.branch_name.trim().is_empty()
                                    && crate::launch_runtime::normalize_branch_name(branch)
                                        == crate::launch_runtime::normalize_branch_name(
                                            delivery.branch_name.trim(),
                                        )
                            })
                    });
            }
            _ => return false,
        };
        let Ok(Some(identity)) = gwt_agent::SessionExecutionIdentity::from_session(source) else {
            return false;
        };
        handoff.session_id == identity.session_id
            && handoff.issue_number == owner.number
            && identity.linked_issue_number == Some(owner.number)
            && identity.execution_binding.owner_number == owner.number
            && identity.execution_binding.owner_kind == owner.kind.as_str()
            && source_session_id == Some(identity.session_id.as_str())
            && branch.is_some_and(|branch| {
                crate::launch_runtime::normalize_branch_name(branch)
                    == crate::launch_runtime::normalize_branch_name(&identity.branch)
            })
            && super::launch_finalization::path_matches(&identity.worktree_path, worktree_path)
    }
}

type FreshReadinessResult =
    Result<Option<crate::AgentExecutionContinuationReceipt>, crate::AgentWorkspaceUpdateError>;
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ReadinessFence {
    id: String,
    incarnation: u64,
    operation_id: String,
    binding: gwt_agent::SessionExecutionBinding,
    token: Option<String>,
}
#[derive(Clone)]
enum ReadinessPending {
    Fresh(Box<super::launch_payload::PendingFreshExecutionLaunch>),
    Continue(Box<super::launch_payload::PendingContinueWork>),
}
impl ReadinessPending {
    fn binding(&self) -> &gwt_agent::SessionExecutionBinding {
        match self {
            Self::Fresh(pending) => &pending.binding,
            Self::Continue(pending) => &pending.binding,
        }
    }
    fn operation_id(&self) -> &str {
        match self {
            Self::Fresh(pending) => &pending.operation_id,
            Self::Continue(pending) => &pending.operation_id,
        }
    }
    fn identity(&self) -> Option<gwt_agent::SessionExecutionIdentity> {
        match self {
            Self::Fresh(pending) => Some(pending.session_identity.clone()),
            Self::Continue(pending) => {
                super::launch_finalization::pending_continue_work_session_identity(pending).ok()
            }
        }
    }
    fn matches_agent(&self, agent: &AgentRuntimeRecord) -> bool {
        match self {
            Self::Fresh(expected) => agent.pending_fresh.as_ref().is_some_and(|pending| {
                pending.binding == expected.binding && pending.operation_id == expected.operation_id
            }),
            Self::Continue(expected) => agent.pending_continue.as_ref().is_some_and(|pending| {
                pending.binding == expected.binding && pending.operation_id == expected.operation_id
            }),
        }
    }
}
pub(crate) enum ReadinessOutcome {
    Fresh(super::continuation::FreshExecutionFinalizationOutcome),
    Continue(super::continuation::ContinueWorkFinalizationOutcome),
}
impl ReadinessOutcome {
    fn committed(&self) -> bool {
        matches!(
            self,
            Self::Fresh(super::continuation::FreshExecutionFinalizationOutcome::Committed)
                | Self::Continue(super::continuation::ContinueWorkFinalizationOutcome::Committed)
        )
    }
    fn rolled_back_detail(&self) -> Option<&str> {
        match self {
            Self::Fresh(super::continuation::FreshExecutionFinalizationOutcome::RolledBack(
                detail,
            ))
            | Self::Continue(super::continuation::ContinueWorkFinalizationOutcome::RolledBack {
                detail,
            }) => Some(detail),
            _ => None,
        }
    }
}
struct ReadinessJob {
    fence: ReadinessFence,
    pending: ReadinessPending,
    current: Arc<AtomicBool>,
    replies: Vec<(String, mpsc::Sender<FreshReadinessResult>)>,
    teardown_after_rollback: bool,
}
struct ReadinessDeadline {
    watch: super::launch_readiness::ContinueWorkReadinessWatch,
    at: std::time::Instant,
}
struct DeferredReadinessClose {
    incarnation: u64,
    cause: CloseCause,
}

pub(crate) struct AgentPrepared {
    monitor_failure_committed: bool,
    launch_failure: Option<super::events::OwnerLaunchFailure>,
    restore: Option<super::installation::OwnerSavedSessionRestore>,
    retirement_reason: Option<String>,
    registry_cleanup: Option<super::agent_dispatch::OwnedWindowRecordCleanup>,
    cancelled_cleanup: bool,
    durable_candidate_removed: bool,
    purpose_title: Option<String>,
    operation_id: String,
    input: InstallAgentLaunchInput,
    result: Result<PreparedAgentLaunchSuccess, AgentLaunchError>,
}

#[derive(Clone, Copy)]
pub(crate) enum CloseCause {
    ExplicitClose,
    NaturalExit,
    DiagnosticExit,
    PmGenericCrash,
    GuiLifetime,
}
impl CloseCause {
    fn restore_policy(self) -> Option<bool> {
        match self {
            Self::ExplicitClose | Self::NaturalExit | Self::DiagnosticExit => Some(false),
            Self::PmGenericCrash | Self::GuiLifetime => None,
        }
    }

    fn is_diagnostic(self) -> bool {
        matches!(self, Self::DiagnosticExit | Self::PmGenericCrash)
    }
}

/// Exact cleanup evidence stays in the existing owner until teardown commits.
pub(crate) struct ClosingRuntime {
    incarnation: u64,
    pm_worktree: bool,
}

fn runtime_uses_pm_worktree(runtime: &WindowRuntime) -> bool {
    runtime.agent.as_ref().is_some_and(|agent| {
        crate::pm_registry::is_canonical_pm_worktree(&agent.snapshot.worktree_path)
    })
}

pub(crate) enum Retirement {
    StoppedWindow(Box<super::agent_dispatch::StoppedWindowRetirement>),
    SwitchStoppedWindow {
        proof: Box<crate::issue_monitor::IssueMonitorRuntimeStopRequest>,
        exit: StoppedOwnerWindowExit,
        cleanup: Box<super::agent_dispatch::OwnedWindowRecordCleanup>,
    },
    Runtime {
        runtime: Box<WindowRuntime>,
        cause: CloseCause,
    },
    Pane {
        pane: Box<Pane>,
        operation_id: Option<String>,
        _initial_prompt_file: Option<Arc<tempfile::TempPath>>,
    },
    Agent {
        success: Box<PreparedAgentLaunchSuccess>,
        sessions_dir: PathBuf,
        operation_id: String,
        durable_candidate_removed: bool,
        registry_cleanup: Option<super::agent_dispatch::OwnedWindowRecordCleanup>,
        restore: Option<Box<super::installation::OwnerSavedSessionRestore>>,
    },
}

pub(crate) enum StoppedOwnerWindowExit {
    CapturedPty(Arc<PtyHandle>),
    DiagnosticClosed(Box<gwt_agent::Session>),
}

pub(crate) struct ShellRegistryPublication {
    id: String,
    incarnation: u64,
    cleanup: Option<super::agent_dispatch::OwnedWindowRecordCleanup>,
    result: Result<(), String>,
    receipt: serde_json::Value,
    reply: Option<oneshot::Sender<OwnerReply>>,
    operation_id: Option<String>,
}

pub(crate) struct LegacyFocusPublication {
    id: String,
    incarnation: u64,
    current: Arc<AtomicBool>,
    token: String,
    source: gwt_agent::Session,
    result: Result<gwt_agent::Session, String>,
    client_id: String,
    operation_id: String,
    outcome: super::continuation::CachedContinueWorkOutcome,
    reply: oneshot::Sender<OwnerReply>,
}

pub(crate) enum OwnerMessage {
    #[cfg(any(test, feature = "test-gh-guard"))]
    SetOperationClock(Option<std::time::Instant>, oneshot::Sender<()>),
    #[cfg(any(test, feature = "test-gh-guard"))]
    SuppressMobileCommitReply {
        ready: super::events::OwnerMobileInputReady,
        reply: oneshot::Sender<Result<Arc<AtomicBool>, String>>,
    },
    AgentSwitchFinished {
        drove: bool,
        result: OwnerReply,
        reply: oneshot::Sender<OwnerReply>,
    },
    AgentSwitchSourceExited(Box<super::agent_switch::AgentSwitchSourceExited>),
    LegacyFocusPublished(Box<LegacyFocusPublication>),
    BoardNotificationFinished {
        result: Result<(), String>,
        reply: oneshot::Sender<OwnerReply>,
    },
    NamedAssignmentFinished(Result<(), String>),
    MonitorRejectionFinished {
        operation_id: String,
        result: OwnerReply,
    },
    PermissionCheckFinished {
        operation_id: String,
        result: OwnerReply,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    SetIssueClientFactory {
        factory: super::launch::RuntimeIssueClientFactory,
        reply: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    RegisterTestAgentClient {
        id: String,
        reply: oneshot::Sender<Arc<crate::project_transport::ClientQueue>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    AgentDispatchTestBarrier {
        wait_for_self_closes: bool,
        reply: oneshot::Sender<()>,
    },
    AutonomousAnswerFinished {
        id: String,
        incarnation: u64,
        outcome: Result<super::monitor::OwnerAnswerDelivery, String>,
        reply: oneshot::Sender<OwnerReply>,
    },
    MonitorStartupFailed {
        id: String,
        incarnation: u64,
        watch_at: std::time::Instant,
        result: Result<bool, String>,
    },
    MonitorNotificationFinished {
        result: Result<(), String>,
    },
    MonitorCommitted {
        id: String,
        incarnation: u64,
        ready: bool,
        stale_runtime: Option<(String, u64, u64)>,
        expired: bool,
        result:
            Result<Option<crate::runtime_daemon_events::IssueMonitorAgentFailureSource>, String>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    DisconnectPreparingMailbox {
        reply: oneshot::Sender<Result<(), String>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    PauseCapacityRetry {
        id: String,
        incarnation: u64,
        checkpoint: Arc<super::test_support::OwnerPolicyRetryCheckpoint>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    PauseNextAgentLaunch {
        checkpoint: Arc<super::test_support::OwnerLaunchCheckpoint>,
        reply: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    LaunchPreparedContinuation {
        config: Box<gwt_agent::LaunchConfig>,
        pending: Box<super::launch_payload::PendingContinueWork>,
        reply: oneshot::Sender<OwnerReply>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    SetPtyStartGate {
        gate: super::test_support::OwnerTestGate,
        reply: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    ProbeRuntime {
        id: String,
        incarnation: u64,
        reply: oneshot::Sender<Result<super::test_support::OwnerRuntimeProbe, String>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    ObserveRuntimePolicy {
        id: String,
        incarnation: u64,
        wall: chrono::DateTime<chrono::Utc>,
        now: std::time::Instant,
        reply: oneshot::Sender<Result<(), String>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    TickRuntimePolicy {
        now: std::time::Instant,
        reply: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    CheckReadinessDeadline {
        id: String,
        incarnation: u64,
        watch: super::launch_readiness::ContinueWorkReadinessWatch,
        reply: oneshot::Sender<Result<(), String>>,
    },
    #[cfg(any(test, feature = "test-gh-guard"))]
    CheckMonitorDeadlines {
        now: std::time::Instant,
        reply: oneshot::Sender<()>,
    },
    AgentDispatchFinished(Box<super::agent_dispatch::AgentDispatchCompletion>),
    ShellRegistryPublished(Box<ShellRegistryPublication>),
    ProcessLaunchFailed {
        operation_id: String,
        detail: String,
        cleanup: OwnerReply,
    },
    CommitAgentSelfClose(crate::agent_capability::AgentSelfCloseCapabilityTicket),
    AgentSelfCloseFinished(crate::agent_capability::AgentSelfCloseCapabilityTicket),
    NativeWorkClosed {
        result: Result<bool, String>,
        reply: oneshot::Sender<OwnerReply>,
    },
    OwnerLayoutCommitted {
        result: Result<crate::persistence::PersistedWindowRegistry, String>,
        reply: oneshot::Sender<OwnerReply>,
    },
    Request(OwnerRequest, oneshot::Sender<OwnerReply>),
    Prepared {
        id: String,
        incarnation: u64,
        pane: Result<Box<Pane>, String>,
        reply: Option<oneshot::Sender<OwnerReply>>,
        operation_id: Option<String>,
        initial_prompt_file: Option<Arc<tempfile::TempPath>>,
    },
    Closed {
        id: String,
        incarnation: u64,
        runtime: Box<WindowRuntime>,
        cause: CloseCause,
        result: Result<(), String>,
    },
    Retired {
        id: String,
        incarnation: u64,
        captured: Retirement,
        result: Result<(), String>,
        reply: Option<oneshot::Sender<OwnerReply>>,
        cancellation: Option<String>,
    },
    AgentMaterialized {
        operation_id: String,
        id: String,
        incarnation: u64,
        session_id: String,
        ack: mpsc::Sender<()>,
    },
    ReadinessFinalized {
        fence: ReadinessFence,
        outcome: ReadinessOutcome,
        snapshot: Option<Box<gwt_agent::Session>>,
    },
    AgentPrepared(Box<AgentPrepared>),
    ManualPredecessorStopped {
        operation_id: String,
        launch_incarnation: u64,
        predecessor: gwt_agent::SessionExecutionIdentity,
        id: String,
        incarnation: u64,
        runtime: Box<WindowRuntime>,
        result: Result<(), String>,
        ack: mpsc::Sender<()>,
    },
    InputWritten {
        reply: oneshot::Sender<OwnerReply>,
        result: Result<Option<super::events::OwnerInputObservation>, String>,
    },
    MobileInputWritten {
        identity: super::events::OwnerMobileInputIdentity,
        data: Vec<u8>,
        transaction: Arc<super::pm_delivery::MobileInputTransaction>,
        attempted: bool,
        result: Result<(), String>,
        observation: Option<super::events::OwnerInputObservation>,
    },
    PmWakeWritten {
        operation_id: String,
        reply: oneshot::Sender<OwnerReply>,
        result: Result<super::pm_delivery::PmWakeWrite, String>,
    },
    PmWakeAcknowledged {
        result: Result<(), String>,
    },
    Pane(PaneRuntimeEvent),
    Transport(TransportEvent),
    GuiAttached(oneshot::Sender<Result<u64, String>>),
    GuiDetached(u64),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct OwnerHandle {
    pub(crate) sender: mpsc::Sender<OwnerMessage>,
    events: broadcast::Sender<OwnerEvent>,
    residency: tokio::sync::watch::Receiver<ResidencySnapshot>,
}
impl OwnerHandle {
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_set_issue_client_factory(
        &self,
        factory: super::launch::RuntimeIssueClientFactory,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::SetIssueClientFactory { factory, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before Issue client setup".to_string())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_register_agent_client(
        &self,
        id: String,
    ) -> Result<Arc<crate::project_transport::ClientQueue>, String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::RegisterTestAgentClient { id, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before client registration".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_agent_dispatch_barrier(
        &self,
        wait_for_self_closes: bool,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::AgentDispatchTestBarrier {
                wait_for_self_closes,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before dispatch barrier".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) fn test_commit_self_close(
        &self,
        ticket: crate::agent_capability::AgentSelfCloseCapabilityTicket,
    ) -> Result<(), String> {
        self.sender
            .send(OwnerMessage::CommitAgentSelfClose(ticket))
            .map_err(|_| "project owner unavailable".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) fn test_pane_event(&self, event: PaneRuntimeEvent) -> Result<(), String> {
        self.sender
            .send(OwnerMessage::Pane(event))
            .map_err(|_| "project owner unavailable".into())
    }
    pub(crate) fn residency(&self) -> tokio::sync::watch::Receiver<ResidencySnapshot> {
        self.residency.clone()
    }
    pub(crate) async fn attach_gui(&self) -> Result<u64, String> {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::GuiAttached(send))
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before GUI attachment".to_string())?
    }
    pub(crate) fn detach_gui(&self, id: u64) {
        let _ = self.sender.send(OwnerMessage::GuiDetached(id));
    }
    pub async fn request(&self, request: OwnerRequest) -> OwnerReply {
        let (send, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::Request(request, send))
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before acknowledgement".to_string())?
    }
    pub fn transport_event(&self, event: TransportEvent) {
        let _ = self.sender.send(OwnerMessage::Transport(event));
    }
    pub fn subscribe(&self) -> broadcast::Receiver<OwnerEvent> {
        self.events.subscribe()
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub async fn test_set_operation_clock(
        &self,
        clock: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::SetOperationClock(clock, reply))
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before clock observation".to_string())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub async fn test_suppress_mobile_commit_reply(
        &self,
        ready: super::events::OwnerMobileInputReady,
    ) -> Result<Arc<AtomicBool>, String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::SuppressMobileCommitReply { ready, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before receipt boundary".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    #[doc(hidden)]
    pub async fn test_probe_runtime(
        &self,
        id: &str,
        incarnation: u64,
    ) -> Result<super::test_support::OwnerRuntimeProbe, String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::ProbeRuntime {
                id: id.into(),
                incarnation,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before exact probe".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_observe_runtime_policy(
        &self,
        id: &str,
        incarnation: u64,
        wall: chrono::DateTime<chrono::Utc>,
        now: std::time::Instant,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::ObserveRuntimePolicy {
                id: id.into(),
                incarnation,
                wall,
                now,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before policy observation".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_tick_runtime_policy(
        &self,
        now: std::time::Instant,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::TickRuntimePolicy { now, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before policy tick".to_string())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_check_monitor_deadlines(
        &self,
        now: std::time::Instant,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::CheckMonitorDeadlines { now, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before Monitor deadline tick".to_string())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_check_readiness_deadline(
        &self,
        id: &str,
        incarnation: u64,
        watch: super::launch_readiness::ContinueWorkReadinessWatch,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::CheckReadinessDeadline {
                id: id.into(),
                incarnation,
                watch,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before readiness deadline check".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub async fn test_pause_capacity_retry(
        &self,
        id: String,
        incarnation: u64,
        checkpoint: Arc<super::test_support::OwnerPolicyRetryCheckpoint>,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::PauseCapacityRetry {
                id,
                incarnation,
                checkpoint,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before retry checkpoint setup".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    #[doc(hidden)]
    pub async fn test_pause_next_agent_launch(
        &self,
        checkpoint: Arc<super::test_support::OwnerLaunchCheckpoint>,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::PauseNextAgentLaunch { checkpoint, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before launch checkpoint setup".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) fn test_agent_switch_source_exited(
        &self,
        proof: crate::issue_monitor::IssueMonitorRuntimeStopRequest,
        pty: Arc<PtyHandle>,
    ) -> Result<(), String> {
        self.sender
            .send(OwnerMessage::AgentSwitchSourceExited(Box::new(
                super::agent_switch::AgentSwitchSourceExited {
                    operation_id: "actual-owner-switch-fixture".into(),
                    proof,
                    pty,
                },
            )))
            .map_err(|_| "project owner unavailable".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub(crate) async fn test_launch_prepared_continuation(
        &self,
        config: Box<gwt_agent::LaunchConfig>,
        pending: Box<super::launch_payload::PendingContinueWork>,
    ) -> OwnerReply {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::LaunchPreparedContinuation {
                config,
                pending,
                reply,
            })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "owner stopped before prepared launch".to_string())?
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    #[doc(hidden)]
    pub async fn test_set_pty_start_gate(
        &self,
        gate: super::test_support::OwnerTestGate,
    ) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::SetPtyStartGate { gate, reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before PTY gate setup".into())
    }
    #[cfg(any(test, feature = "test-gh-guard"))]
    pub async fn test_disconnect_preparing_mailbox(&self) -> Result<(), String> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(OwnerMessage::DisconnectPreparingMailbox { reply })
            .map_err(|_| "project owner unavailable".to_string())?;
        receive
            .await
            .map_err(|_| "project owner stopped before mailbox disconnect".to_string())?
    }
    pub(crate) fn stop(&self) {
        let (send, _) = oneshot::channel();
        let _ = self.sender.send(OwnerMessage::Shutdown(send));
    }
    pub async fn shutdown(&self) {
        let (send, receive) = oneshot::channel();
        if self.sender.send(OwnerMessage::Shutdown(send)).is_ok() {
            let _ = receive.await;
        }
    }
}

type DeferredAgentFrontend = (
    String,
    crate::agent_capability::AgentCapabilityGrant,
    crate::agent_capability::AgentFrontendRequest,
);

pub(crate) struct OwnerState {
    #[cfg(any(test, feature = "test-gh-guard"))]
    test_operation_clock: Option<std::time::Instant>,
    issue_client_factory: super::launch::RuntimeIssueClientFactory,
    #[cfg(any(test, feature = "test-gh-guard"))]
    test_dispatch_barriers: Vec<(bool, oneshot::Sender<()>)>,
    policy_usage_accounts: Vec<gwt_core::usage::ProviderUsage>,
    policy_capacity_resumes: HashMap<String, super::runtime_events::ProviderCapacityResume>,
    #[cfg(any(test, feature = "test-gh-guard"))]
    next_capacity_checkpoint: Option<(
        String,
        u64,
        Arc<super::test_support::OwnerPolicyRetryCheckpoint>,
    )>,
    #[cfg(any(test, feature = "test-gh-guard"))]
    next_launch_checkpoint: Option<Arc<super::test_support::OwnerLaunchCheckpoint>>,
    #[cfg(any(test, feature = "test-gh-guard"))]
    test_pty_start_gate: Option<super::test_support::OwnerTestGate>,
    agent_registry: Result<super::agent_dispatch::OwnerWindowRegistry, String>,
    shared_registry_initialization: Option<Result<(), String>>,
    registry_projection: crate::persistence::PersistedWindowRegistry,
    pty_writers: super::agent_dispatch::PtyWriterRegistry,
    agent_dispatch_jobs: usize,
    deferred_agent_frontend: HashMap<(String, u64), Vec<DeferredAgentFrontend>>,
    pending_agent_self_closes: HashMap<String, super::agent_dispatch::PendingAgentSelfClose>,
    pub(crate) root: PathBuf,
    pub(crate) project_key: String,
    pub(crate) runtimes: HashMap<String, WindowRuntime>,
    pub(crate) clients: ClientHub,
    pub(crate) issuer: AgentCapabilityIssuer,
    pub(crate) host_instance_id: String,
    pub(crate) sender: mpsc::Sender<OwnerMessage>,
    pub(crate) pending_transport: VecDeque<TransportEvent>,
    pub(crate) pending: HashMap<String, u64>,
    pub(crate) closing: HashMap<String, ClosingRuntime>,
    pub(crate) next_incarnation: u64,
    pub(crate) events: broadcast::Sender<OwnerEvent>,
    stopping: bool,
    gui_presence: GuiPresence,
    next_gui_presence_id: u64,
    residency: tokio::sync::watch::Sender<ResidencySnapshot>,
    shutdown_replies: Vec<oneshot::Sender<()>>,
    retiring: usize,
    input_jobs: usize,
    switch_job_inflight: bool,
    pm_wake_permits: super::pm_delivery::PmWakePermits,
    agent_operations: HashMap<String, AgentOperation>,
    retirements: HashMap<(String, u64), Option<Retirement>>,
    readiness: HashMap<String, ReadinessJob>,
    close_replies: HashMap<(String, u64), Vec<oneshot::Sender<OwnerReply>>>,
    pub(crate) continue_work_outcomes:
        HashMap<String, super::continuation::CachedContinueWorkOutcome>,
    pub(crate) continue_work_waiters: HashMap<String, HashSet<String>>,
    deferred_readiness_closes: HashMap<String, DeferredReadinessClose>,
}
impl OwnerState {
    pub(crate) fn start(
        root: PathBuf,
        project_key: String,
        clients: ClientHub,
        issuer: AgentCapabilityIssuer,
        host_instance_id: String,
        startup_intent: gwt_core::daemon::DaemonStartIntent,
    ) -> OwnerHandle {
        let (sender, receiver) = mpsc::channel();
        let (events, _) = broadcast::channel(1024);
        let (residency, residency_projection) = tokio::sync::watch::channel(ResidencySnapshot {
            startup_intent,
            exit_started: false,
        });
        let handle = OwnerHandle {
            sender: sender.clone(),
            events: events.clone(),
            residency: residency_projection,
        };
        let agent_registry = super::agent_dispatch::load_owner_registry(&root);
        let registry_projection = agent_registry
            .as_ref()
            .ok()
            .and_then(|registry| {
                registry
                    .lock()
                    .ok()
                    .map(|registry| registry.persisted().clone())
            })
            .unwrap_or_else(crate::persistence::empty_workspace_state);
        let state = Self {
            issue_client_factory: super::launch::default_issue_client_factory(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_dispatch_barriers: Vec::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_operation_clock: None,
            policy_usage_accounts: Vec::new(),
            policy_capacity_resumes: HashMap::new(),
            agent_registry,
            shared_registry_initialization: None,
            registry_projection,
            pty_writers: Arc::new(std::sync::RwLock::new(HashMap::new())),
            agent_dispatch_jobs: 0,
            deferred_agent_frontend: HashMap::new(),
            pending_agent_self_closes: HashMap::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_capacity_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_launch_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_pty_start_gate: None,
            root,
            project_key,
            runtimes: HashMap::new(),
            clients,
            issuer,
            host_instance_id,
            sender,
            events,
            pending: HashMap::new(),
            closing: HashMap::new(),
            next_incarnation: 0,
            pending_transport: VecDeque::new(),
            stopping: false,
            gui_presence: GuiPresence::new(startup_intent, std::time::Instant::now()),
            next_gui_presence_id: 0,
            residency,
            shutdown_replies: Vec::new(),
            retiring: 0,
            input_jobs: 0,
            switch_job_inflight: false,
            pm_wake_permits: Default::default(),
            agent_operations: HashMap::new(),
            retirements: HashMap::new(),
            readiness: HashMap::new(),
            close_replies: HashMap::new(),
            continue_work_outcomes: HashMap::new(),
            continue_work_waiters: HashMap::new(),
            deferred_readiness_closes: HashMap::new(),
        };
        spawn_owner_worker(move || {
            #[cfg(windows)]
            let _verification_cap_relief =
                crate::verification_cap_relief::spawn(state.pty_writers.clone());
            let mut state = state;
            if let Err(error) = state.initialize_shared_registry() {
                tracing::warn!(%error, "owner startup waits for shared Agent registry recovery");
            }
            let writers = state.pty_writers.clone();
            let _monitor_reconciler = crate::monitor_duplicate_runtime::spawn(
                state.root.clone(), gwt_core::paths::gwt_sessions_dir(), move || {
                    let writers = writers.read().ok()?;
                    Some(writers.values().filter_map(|entry| Some(crate::monitor_duplicate_runtime::RegisteredMonitorRuntime {
                        registration: entry.monitor_runtime.clone()?, handle: entry.handle.clone(),
                    })).collect())
                },
            ).map_err(|error| tracing::error!(target: "gwt.monitor.runtime", %error, "failed to start owner Monitor runtime reconciliation")).ok();
            let shutdown_replies = state.run(receiver);
            drop(_monitor_reconciler);
            #[cfg(windows)]
            drop(_verification_cap_relief);
            // Completion includes all producers captured by this owner lifetime.
            for reply in shutdown_replies {
                let _ = reply.send(());
            }
        });
        handle
    }

    /// Initial activation takes Session leases before prefs/registry locks.
    /// Run before any startup or request-specific Session lease, and retain a
    /// failed attempt so Health can diagnose and retry after explicit recovery.
    fn initialize_shared_registry(&mut self) -> Result<(), String> {
        if matches!(self.shared_registry_initialization, Some(Ok(()))) {
            return Ok(());
        }
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            gwt_core::operation_deadline::now() + std::time::Duration::from_secs(5),
        );
        if let Err(error) = crate::agent_registry::ensure_active_for_project(&self.root) {
            let error = format!("Shared Agent registry initialization failed: {error}");
            self.shared_registry_initialization = Some(Err(error.clone()));
            return Err(error);
        }
        let sessions = gwt_core::paths::gwt_sessions_dir();
        super::startup::reconcile_durable_launches(&self.root, &sessions);
        if let Ok(registry) = &self.agent_registry {
            match super::agent_dispatch::backfill_owner_pane_names(&self.root, &sessions, registry)
            {
                Ok(snapshot) => {
                    if self.registry_projection != snapshot {
                        self.registry_projection = snapshot;
                        let _ = self.events.send(OwnerEvent::Invalidated);
                    }
                }
                Err(error) => tracing::warn!(%error, "owner Agent name backfill failed"),
            }
        }
        self.shared_registry_initialization = Some(Ok(()));
        Ok(())
    }

    fn run(mut self, receiver: mpsc::Receiver<OwnerMessage>) -> Vec<oneshot::Sender<()>> {
        loop {
            if !self.stopping && self.gui_presence.expired(std::time::Instant::now()) {
                self.begin_shutdown(None);
            }
            if self.finish_shutdown_if_drained() {
                break;
            }
            self.check_readiness_deadlines(std::time::Instant::now());
            self.tick_runtime_policies(std::time::Instant::now());
            self.check_monitor_deadlines(std::time::Instant::now());
            let deadline = self
                .next_readiness_deadline()
                .into_iter()
                .chain(self.next_policy_deadline())
                .chain(self.next_monitor_deadline())
                .chain(
                    (!self.stopping)
                        .then(|| self.gui_presence.deadline())
                        .flatten(),
                )
                .min();
            let message = if let Some(deadline) = deadline {
                match receiver
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match receiver.recv() {
                    Ok(message) => message,
                    Err(_) => break,
                }
            };
            #[cfg(any(test, feature = "test-gh-guard"))]
            let _clock = self
                .test_operation_clock
                .map(gwt_core::operation_deadline::ScopedOperationClock::set);
            match message {
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::SetOperationClock(clock, reply) => {
                    self.test_operation_clock = clock;
                    let _ = reply.send(());
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::SuppressMobileCommitReply { ready, reply } => {
                    let result = self
                        .pm_wake_permits
                        .mobile(&ready.identity)
                        .and_then(|transaction| transaction.suppress_commit_reply(&ready));
                    let _ = reply.send(result);
                }
                OwnerMessage::AgentSwitchFinished {
                    drove,
                    result,
                    reply,
                } => {
                    self.input_jobs -= 1;
                    if drove {
                        self.switch_job_inflight = false;
                    }
                    let _ = reply.send(result);
                }
                OwnerMessage::AgentSwitchSourceExited(exited) => self.retire_switch_source(exited),
                OwnerMessage::BoardNotificationFinished { result, reply } => {
                    self.input_jobs -= 1;
                    let _ = reply.send(result.map(|_| json!({"ok":true})));
                }
                OwnerMessage::NamedAssignmentFinished(result) => {
                    self.input_jobs -= 1;
                    if let Err(error) = result {
                        tracing::warn!(%error, "named Issue assignment could not be recorded");
                    }
                }
                OwnerMessage::MonitorRejectionFinished {
                    operation_id,
                    result,
                }
                | OwnerMessage::PermissionCheckFinished {
                    operation_id,
                    result,
                } => {
                    self.input_jobs -= 1;
                    self.finish_agent_operation(&operation_id, result);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::SetIssueClientFactory { factory, reply } => {
                    self.issue_client_factory = factory;
                    let _ = reply.send(());
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::RegisterTestAgentClient { id, reply } => {
                    let _ = reply.send(self.clients.register_pane_for_test(id));
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::AgentDispatchTestBarrier {
                    wait_for_self_closes,
                    reply,
                } => {
                    self.test_dispatch_barriers
                        .push((wait_for_self_closes, reply));
                }
                OwnerMessage::AutonomousAnswerFinished {
                    id,
                    incarnation,
                    outcome,
                    reply,
                } => {
                    self.input_jobs -= 1;
                    let result = match outcome {
                        Ok(outcome) => {
                            if let Some(agent) = self
                                .runtimes
                                .get_mut(&id)
                                .filter(|runtime| runtime.incarnation == incarnation)
                                .and_then(|runtime| runtime.agent.as_mut())
                            {
                                agent.feedback = Some(outcome.feedback);
                                agent.monitor_owner =
                                    Some((self.root.clone(), self.host_instance_id.clone()));
                                agent.monitor_source = outcome.source;
                                if outcome.submitted && outcome.result.is_err() {
                                    agent.monitor_retry_at = Some(
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(1),
                                    );
                                }
                            }
                            outcome.result
                        }
                        Err(error) => Err(error),
                    };
                    if let Some(agent) = self
                        .runtimes
                        .get_mut(&id)
                        .filter(|runtime| runtime.incarnation == incarnation)
                        .and_then(|runtime| runtime.agent.as_mut())
                    {
                        agent.monitor_answer_inflight = false;
                    }
                    let _ = reply.send(result);
                    self.resume_deferred_close(&id, incarnation);
                }

                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::DisconnectPreparingMailbox { reply } => {
                    if self.pending.len() != 1
                        || !self.runtimes.is_empty()
                        || !self.closing.is_empty()
                        || !self.readiness.is_empty()
                        || !self.retirements.is_empty()
                        || self.retiring != 0
                    {
                        let _ = reply.send(Err("mailbox disconnect requires one preparing launch and no installed resources".into()));
                        continue;
                    }
                    // Close the actual receiver before acknowledging so the
                    // paused worker must exercise its production SendError path.
                    drop(receiver);
                    let _ = reply.send(Ok(()));
                    return std::mem::take(&mut self.shutdown_replies);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::PauseCapacityRetry {
                    id,
                    incarnation,
                    checkpoint,
                    reply,
                } => {
                    let result = self.exact(&id, incarnation).map(|_| ());
                    if result.is_ok() {
                        self.next_capacity_checkpoint = Some((id, incarnation, checkpoint));
                    }
                    let _ = reply.send(result);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::PauseNextAgentLaunch { checkpoint, reply } => {
                    self.next_launch_checkpoint = Some(checkpoint);
                    let _ = reply.send(());
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::LaunchPreparedContinuation {
                    config,
                    pending,
                    reply,
                } => {
                    self.apply_continue_action(
                        pending.client_id.clone(),
                        pending.operation_id.clone(),
                        pending.work_id.clone(),
                        super::continuation::ContinueWorkAction::PreparedLaunch { config, pending },
                        reply,
                    );
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::SetPtyStartGate { gate, reply } => {
                    self.test_pty_start_gate = Some(gate);
                    let _ = reply.send(());
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::ProbeRuntime {
                    id,
                    incarnation,
                    reply,
                } => {
                    let result = self.exact(&id, incarnation).and_then(|runtime| {
                        let writer = self
                            .pty_writers
                            .read()
                            .map_err(|error| error.to_string())?
                            .get(&id)
                            .filter(|entry| {
                                Arc::ptr_eq(&entry.current, &runtime.current)
                                    && Arc::ptr_eq(&entry.handle, &runtime.pty)
                            })
                            .cloned();
                        Ok(super::test_support::OwnerRuntimeProbe {
                            id: id.clone(),
                            incarnation,
                            output_bytes: runtime.output_bytes,
                            pane: runtime.pane.clone(),
                            pty: runtime.pty.clone(),
                            current: runtime.current.clone(),
                            policy: runtime.policy.clone(),
                            status: Self::runtime_status(runtime),
                            detail: runtime.detail.clone(),
                            capacity_resume: runtime
                                .agent
                                .as_ref()
                                .and_then(|agent| {
                                    self.policy_capacity_resumes.get(&agent.active.session_id)
                                })
                                .map(|resume| super::test_support::OwnerCapacityResumeProbe {
                                    due: resume.due,
                                    attempted: resume.attempted,
                                }),
                            monitor_startup_at: runtime
                                .agent
                                .as_ref()
                                .and_then(|agent| agent.monitor_startup_at),
                            readiness_watch: runtime.agent.as_ref().and_then(|agent| {
                                agent
                                    .readiness_deadline
                                    .as_ref()
                                    .map(|deadline| deadline.watch.clone())
                            }),
                            pending_fresh_execution: runtime
                                .agent
                                .as_ref()
                                .and_then(|agent| agent.pending_fresh.clone()),
                            readiness_inflight: self.readiness_inflight(&id, incarnation),
                            readiness_waiters: self
                                .readiness
                                .get(&id)
                                .filter(|job| job.fence.incarnation == incarnation)
                                .map_or(0, |job| job.replies.len()),
                            writer,
                            issuer: self.issuer.clone(),
                            session: runtime.agent.as_ref().map(|agent| agent.snapshot.clone()),
                            active_session: runtime
                                .agent
                                .as_ref()
                                .map(|agent| agent.active.clone()),
                            capability_token: runtime
                                .agent
                                .as_ref()
                                .and_then(|agent| agent.capability_token.clone()),
                            grant: runtime.agent.as_ref().and_then(|agent| {
                                agent
                                    .capability_token
                                    .as_deref()
                                    .and_then(|token| agent.issuer.grant_for_test(token))
                            }),
                        })
                    });
                    let _ = reply.send(result);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::ObserveRuntimePolicy {
                    id,
                    incarnation,
                    wall,
                    now,
                    reply,
                } => {
                    let result = self.exact(&id, incarnation).map(|_| ());
                    if result.is_ok() {
                        self.observe_owner_screen_policy(&id, wall, now, None);
                    }
                    let _ = reply.send(result);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::TickRuntimePolicy { now, reply } => {
                    self.tick_runtime_policies(now);
                    let _ = reply.send(());
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::CheckReadinessDeadline {
                    id,
                    incarnation,
                    watch,
                    reply,
                } => {
                    let now = std::time::Instant::now();
                    let result = self
                        .runtimes
                        .get_mut(&id)
                        .filter(|runtime| runtime.incarnation == incarnation)
                        .and_then(|runtime| runtime.agent.as_mut())
                        .and_then(|agent| agent.readiness_deadline.as_mut())
                        .filter(|deadline| deadline.watch.operation_id == watch.operation_id)
                        .ok_or_else(|| "exact readiness deadline is no longer current".to_string())
                        .map(|deadline| {
                            deadline.watch = watch;
                            deadline.at = now;
                        });
                    if result.is_ok() {
                        self.check_readiness_deadlines(now);
                    }
                    let _ = reply.send(result);
                }
                #[cfg(any(test, feature = "test-gh-guard"))]
                OwnerMessage::CheckMonitorDeadlines { now, reply } => {
                    self.check_monitor_deadlines(now);
                    let _ = reply.send(());
                }
                OwnerMessage::AgentDispatchFinished(completion) => {
                    self.agent_dispatch_finished(*completion)
                }
                OwnerMessage::ShellRegistryPublished(publication) => {
                    self.shell_registry_published(*publication)
                }
                OwnerMessage::ProcessLaunchFailed {
                    operation_id,
                    detail,
                    cleanup,
                } => {
                    self.agent_dispatch_jobs -= 1;
                    if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                        operation.cancellation_cleaned = cleanup.is_ok();
                        if cleanup.is_ok() {
                            operation.failed_session = None;
                        }
                    }
                    let error = match cleanup {
                        Ok(_) => detail,
                        Err(error) => format!("{detail}; cleanup requires reconciliation: {error}"),
                    };
                    self.finish_agent_operation(&operation_id, Err(error));
                }
                OwnerMessage::CommitAgentSelfClose(ticket) => {
                    self.commit_owner_agent_self_close(ticket)
                }
                OwnerMessage::AgentSelfCloseFinished(ticket) => {
                    self.pending_agent_self_closes.remove(ticket.id());
                    self.issuer.finish_self_close(&ticket);
                }
                OwnerMessage::NativeWorkClosed { result, reply } => {
                    self.agent_dispatch_jobs -= 1;
                    if result.as_ref().is_ok_and(|changed| *changed) {
                        let _ = self.events.send(OwnerEvent::Invalidated);
                    }
                    let _ = reply.send(result.map(|changed| json!({"changed":changed})));
                    self.resume_agent_transport();
                }
                OwnerMessage::OwnerLayoutCommitted { result, reply } => {
                    self.agent_dispatch_jobs -= 1;
                    if let Ok(snapshot) = &result {
                        let names_changed = snapshot.windows.iter().any(|row| {
                            self.registry_projection
                                .windows
                                .iter()
                                .find(|previous| previous.id == row.id)
                                .and_then(|previous| previous.agent_name.as_ref())
                                != row.agent_name.as_ref()
                        });
                        self.registry_projection = snapshot.clone();
                        if names_changed {
                            let _ = self.events.send(OwnerEvent::Invalidated);
                        }
                    }
                    let _ = reply.send(result.map(|_| json!({"updated":true})));
                    self.resume_agent_transport();
                }
                OwnerMessage::GuiAttached(reply) => {
                    if self.stopping {
                        let _ = reply.send(Err("project owner is shutting down".into()));
                    } else {
                        self.next_gui_presence_id += 1;
                        let id = self.next_gui_presence_id;
                        self.gui_presence.attach(id);
                        if reply.send(Ok(id)).is_err() {
                            self.gui_presence.detach(id, std::time::Instant::now());
                        }
                    }
                }
                OwnerMessage::GuiDetached(id) => {
                    self.gui_presence.detach(id, std::time::Instant::now())
                }
                OwnerMessage::Request(ref request, _)
                    if self.stopping
                        && matches!(
                            request.command,
                            OwnerCommand::Health | OwnerCommand::Close { .. }
                        ) =>
                {
                    if let OwnerMessage::Request(request, reply) = message {
                        self.request(request, reply);
                    }
                }
                OwnerMessage::Request(_, reply) if self.stopping => {
                    let _ = reply.send(Err("project owner is shutting down".into()));
                }
                OwnerMessage::Request(request, reply) => self.request(request, reply),
                OwnerMessage::LegacyFocusPublished(publication) => {
                    self.legacy_focus_published(*publication)
                }
                OwnerMessage::Prepared {
                    id,
                    incarnation,
                    pane,
                    reply,
                    operation_id,
                    initial_prompt_file,
                } => {
                    self.process_prepared(
                        id,
                        incarnation,
                        pane.map(|pane| *pane),
                        initial_prompt_file,
                        operation_id,
                        reply,
                    );
                }
                OwnerMessage::Retired {
                    id,
                    incarnation,
                    captured,
                    result,
                    reply,
                    cancellation,
                } => {
                    self.retiring -= 1;
                    if self.pending.get(&id) == Some(&incarnation) {
                        self.pending.remove(&id);
                    }
                    let operation_id = match &captured {
                        Retirement::Agent { operation_id, .. } => Some(operation_id.clone()),
                        Retirement::Pane { operation_id, .. } => operation_id.clone(),
                        _ => None,
                    };
                    let detail = cancellation.as_deref().unwrap_or("exact cleanup retry");
                    let restore_outcome = match &captured {
                        Retirement::Agent {
                            restore: Some(restore),
                            ..
                        } => Some(restore.outcome(detail)),
                        _ => None,
                    };
                    let failure = result
                        .as_ref()
                        .err()
                        .map(|error| format!("{detail}; cleanup requires reconciliation: {error}"));
                    if let Retirement::Agent { success, .. } = &captured {
                        if let Some(pending) = success.pending_continue_work.as_ref() {
                            if result.is_ok()
                                && super::launch_finalization::pending_execution_activation_status(
                                    pending,
                                ) == Some(false)
                            {
                                self.continue_work_outcome(pending, super::continuation::CachedContinueWorkOutcome {
                                    work_id: pending.work_id.clone(), outcome: crate::ContinueWorkOutcomeKind::Failed,
                                    message: Some(format!("Continue work launch failed before activation: {detail}")), error_code: Some("launch_failed".into()), retryable: true,
                                }, true);
                            } else if result.is_err() {
                                self.continue_work_outcome(pending, super::continuation::CachedContinueWorkOutcome {
                                    work_id: pending.work_id.clone(), outcome: crate::ContinueWorkOutcomeKind::Failed,
                                    message: Some("The failed continuation could not commit its exact cleanup; retry reconciliation.".into()),
                                    error_code: Some("continuation_reconciliation_required".into()), retryable: true,
                                }, false);
                            }
                        }
                    }
                    if result.is_ok() {
                        if let Retirement::SwitchStoppedWindow { proof, .. } = &captured {
                            let _ = self.events.send(OwnerEvent::WindowRemoved {
                                id: id.clone(),
                                incarnation,
                                session_id: Some(proof.session_id.clone()),
                            });
                        }
                    }
                    if result.is_err() {
                        self.retirements
                            .insert((id.clone(), incarnation), Some(captured));
                    } else {
                        self.retirements.remove(&(id.clone(), incarnation));
                    }
                    if let Some(operation_id) = operation_id {
                        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                            operation.cancellation_cleaned = result.is_ok();
                        }
                        self.finish_agent_operation(
                            &operation_id,
                            restore_outcome.map(Ok).unwrap_or_else(|| {
                                Err(failure.clone().unwrap_or_else(|| {
                                    cancellation.clone().unwrap_or_else(|| {
                                        "agent launch cancelled before installation".into()
                                    })
                                }))
                            }),
                        );
                    }
                    if let Some(reply) = reply {
                        let result = match (result, cancellation) {
                            (Err(_), _) => Err(failure.expect("failed cleanup detail")),
                            (Ok(()), Some(reason)) => Err(reason),
                            (Ok(()), None) => {
                                Ok(json!({"closed":true,"id":id,"incarnation":incarnation}))
                            }
                        };
                        let _ = reply.send(result);
                    }
                    self.refresh_registry_projection();
                    self.resume_agent_transport();
                }
                OwnerMessage::Closed {
                    id,
                    incarnation,
                    runtime,
                    cause,
                    result,
                } => self.closed(id, incarnation, runtime, cause, result),
                OwnerMessage::AgentMaterialized {
                    operation_id,
                    id,
                    incarnation,
                    session_id,
                    ack,
                } => {
                    if let Some(operation) =
                        self.agent_operations
                            .get_mut(&operation_id)
                            .filter(|operation| {
                                operation.id == id && operation.incarnation == incarnation
                            })
                    {
                        operation.session_id = Some(session_id);
                    }
                    let _ = ack.send(());
                }
                OwnerMessage::ReadinessFinalized {
                    fence,
                    outcome,
                    snapshot,
                } => self.readiness_finalized(fence, outcome, snapshot.map(|snapshot| *snapshot)),
                OwnerMessage::ManualPredecessorStopped {
                    operation_id,
                    launch_incarnation,
                    predecessor,
                    id,
                    incarnation,
                    runtime,
                    result,
                    ack,
                } => {
                    self.retiring -= 1;
                    if result.is_ok() {
                        if let Some(operation) = self
                            .agent_operations
                            .get_mut(&operation_id)
                            .filter(|operation| {
                                operation.incarnation == launch_incarnation
                                    && operation.result.is_none()
                                    && operation.current.load(Ordering::Acquire)
                                    && operation
                                        .request
                                        .get("metadata")
                                        .and_then(|metadata| metadata.get("manual_successor"))
                                        .and_then(|choice| choice.get("expected_predecessor"))
                                        == serde_json::to_value(&predecessor).ok().as_ref()
                            })
                        {
                            operation.manual_predecessor_stopped = Some(predecessor);
                        }
                        self.retirements.remove(&(id.clone(), incarnation));
                        self.publish_status(
                            id,
                            incarnation,
                            crate::WindowProcessStatus::Stopped,
                            None,
                            true,
                        );
                        let _ = self.events.send(OwnerEvent::Invalidated);
                    } else {
                        self.retirements.insert(
                            (id, incarnation),
                            Some(Retirement::Runtime {
                                runtime,
                                cause: CloseCause::ExplicitClose,
                            }),
                        );
                    }
                    let _ = ack.send(());
                }
                OwnerMessage::AgentPrepared(prepared) => self.agent_prepared(*prepared),
                OwnerMessage::MonitorStartupFailed {
                    id,
                    incarnation,
                    watch_at,
                    result,
                } => {
                    self.input_jobs -= 1;
                    let current = self.runtimes.get(&id).is_some_and(|runtime| {
                        runtime.incarnation == incarnation
                            && runtime
                                .agent
                                .as_ref()
                                .is_some_and(|agent| agent.monitor_startup_at == Some(watch_at))
                    });
                    if current {
                        match result {
                            Ok(true) => {
                                let runtime = self.runtimes.remove(&id).unwrap();
                                let (reply, _) = oneshot::channel();
                                self.start_close(id, runtime, CloseCause::ExplicitClose, reply);
                            }
                            Ok(false) => {
                                self.runtimes
                                    .get_mut(&id)
                                    .unwrap()
                                    .agent
                                    .as_mut()
                                    .unwrap()
                                    .monitor_startup_at = None
                            }
                            Err(error) => {
                                tracing::warn!(%error, window_id = %id, "Monitor startup failure awaits exact durable confirmation")
                            }
                        }
                    }
                }
                OwnerMessage::MonitorNotificationFinished { result } => {
                    self.input_jobs -= 1;
                    if let Err(error) = result {
                        tracing::warn!(%error, "owner Monitor notification is unconfirmed");
                    }
                }
                OwnerMessage::MonitorCommitted {
                    id,
                    incarnation,
                    ready,
                    stale_runtime,
                    expired,
                    result,
                } => {
                    self.input_jobs -= 1;
                    let mut commit_again = false;
                    let mut cleanup_stale = None;
                    let mut cleanup_expired = false;
                    if let Some(agent) = self
                        .runtimes
                        .get_mut(&id)
                        .filter(|runtime| runtime.incarnation == incarnation)
                        .and_then(|runtime| runtime.agent.as_mut())
                    {
                        agent.monitor_commit_inflight = false;
                        let receipt = match result {
                            Ok(source) => {
                                let acknowledged = source.is_some();
                                agent.monitor_source = source;
                                agent.monitor_adopt_claim = false;
                                agent.monitor_retry_at = None;
                                commit_again = agent.monitor_ready
                                    && !ready
                                    && agent.current.load(Ordering::Acquire);
                                if acknowledged {
                                    cleanup_stale = stale_runtime;
                                }
                                Ok(
                                    json!({"acknowledged":acknowledged,"id":id,"incarnation":incarnation}),
                                )
                            }
                            Err(error) => {
                                tracing::warn!(%error, window_id = %id, "owner Monitor commit awaits reconciliation");
                                agent.monitor_retry_at = (!expired).then(|| {
                                    std::time::Instant::now() + std::time::Duration::from_secs(1)
                                });
                                cleanup_expired = expired;
                                Err(error)
                            }
                        };
                        for reply in agent.monitor_ack_replies.drain(..) {
                            let _ = reply.send(receipt.clone());
                        }
                    }
                    if cleanup_expired
                        && self
                            .runtimes
                            .get(&id)
                            .is_some_and(|runtime| runtime.incarnation == incarnation)
                    {
                        let runtime = self.runtimes.remove(&id).unwrap();
                        let (reply, _) = oneshot::channel();
                        self.start_close(id.clone(), runtime, CloseCause::ExplicitClose, reply);
                    }
                    if commit_again {
                        self.commit_monitor_launch(&id);
                    }
                    self.resume_deferred_close(&id, incarnation);
                    if let Some((stale_id, stale_incarnation, issue_number)) = cleanup_stale {
                        if self.runtimes.get(&stale_id).is_some_and(|runtime| {
                            runtime.incarnation == stale_incarnation
                                && Self::is_failed_monitor_runtime(runtime, issue_number)
                        }) && !self.pending.contains_key(&stale_id)
                            && !self.closing.contains_key(&stale_id)
                        {
                            let runtime = self
                                .runtimes
                                .remove(&stale_id)
                                .expect("captured stale owner pane");
                            let (reply, _) = oneshot::channel();
                            self.start_close(stale_id, runtime, CloseCause::ExplicitClose, reply);
                        }
                    }
                }
                OwnerMessage::InputWritten { reply, result } => {
                    self.input_jobs -= 1;
                    let _ = reply.send(
                        result.map(|observation| json!({"ok":true,
                            "has_unsent_user_input":observation.map(|value| value.has_unsent_user_input),
                            "input_observation":observation})),
                    );
                }
                OwnerMessage::MobileInputWritten {
                    identity,
                    data,
                    transaction,
                    attempted,
                    result,
                    observation,
                } => {
                    self.input_jobs -= 1;
                    if attempted && self.exact(&identity.id, identity.incarnation).is_ok() {
                        self.begin_policy_input(&identity.id, &data);
                    }
                    self.pm_wake_permits.finish(&identity.operation_id);
                    transaction.finish(attempted, result, observation);
                }
                OwnerMessage::PmWakeWritten {
                    operation_id,
                    reply,
                    result,
                } => {
                    self.input_jobs -= 1;
                    self.pm_wake_permits.finish(&operation_id);
                    let _ = reply.send(result.map(|status| json!({"status":status})));
                }
                OwnerMessage::PmWakeAcknowledged { result } => {
                    self.input_jobs -= 1;
                    if let Err(error) = result {
                        tracing::warn!(%error, "owner PM wake acknowledgement is unconfirmed");
                    }
                }
                OwnerMessage::Pane(event) => self.pane_event(event),
                OwnerMessage::Transport(event) => self.handle_transport_event(event),
                OwnerMessage::Shutdown(reply) => self.begin_shutdown(Some(reply)),
            }
            #[cfg(any(test, feature = "test-gh-guard"))]
            if self.agent_dispatch_jobs == 0
                && self.pending_transport.is_empty()
                && self.deferred_agent_frontend.is_empty()
            {
                let mut retained = Vec::new();
                for (wait_for_self_closes, barrier) in self.test_dispatch_barriers.drain(..) {
                    if wait_for_self_closes
                        && (!self.pending_agent_self_closes.is_empty()
                            || self.retiring != 0
                            || !self.closing.is_empty())
                    {
                        retained.push((wait_for_self_closes, barrier));
                    } else {
                        let _ = barrier.send(());
                    }
                }
                self.test_dispatch_barriers = retained;
            }
        }
        std::mem::take(&mut self.shutdown_replies)
    }
    fn begin_shutdown(&mut self, reply: Option<oneshot::Sender<()>>) {
        self.stopping = true;
        self.pm_wake_permits.cancel_mobile_for_pane(None);
        self.residency.send_replace(ResidencySnapshot {
            startup_intent: self.gui_presence.intent(),
            exit_started: true,
        });
        for operation in self.agent_operations.values() {
            let repairing = self.runtimes.get(&operation.id).is_some_and(|runtime| {
                runtime.incarnation == operation.incarnation
                    && Arc::ptr_eq(&runtime.current, &operation.current)
                    && runtime
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.binding_repair_inflight)
            });
            if !repairing {
                operation.current.store(false, Ordering::Release);
            }
        }
        if let Some(reply) = reply {
            self.shutdown_replies.push(reply);
        }
        let runtimes = self.runtimes.drain().collect::<Vec<_>>();
        for (id, runtime) in runtimes {
            let awaiting_origin = self.pending_agent_self_closes.values().any(|pending| {
                pending.window_id == id
                    && pending.window_lifecycle_generation == runtime.incarnation
                    && runtime
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.snapshot.id == pending.session_id)
            });
            if awaiting_origin {
                // DirectAcceptance owns the bounded origin response attempt.
                // Its completion must choose the explicit close before a
                // lifetime drain can retire this exact runtime.
                runtime.current.store(false, Ordering::Release);
                runtime.pty.revoke_input_generation();
                if let Some(agent) = &runtime.agent {
                    agent.current.store(false, Ordering::Release);
                }
                self.runtimes.insert(id, runtime);
                continue;
            }
            let (reply, _) = oneshot::channel();
            self.start_close(id, runtime, CloseCause::GuiLifetime, reply);
        }
    }
    fn finish_shutdown_if_drained(&mut self) -> bool {
        self.stopping
            && self.pending.is_empty()
            && self.closing.is_empty()
            && self.retiring == 0
            && self.input_jobs == 0
            && self.agent_dispatch_jobs == 0
            && self.deferred_agent_frontend.is_empty()
            && self.pending_agent_self_closes.is_empty()
            && self.runtimes.is_empty()
            && self.retirements.is_empty()
            && self.readiness.is_empty()
            && self.close_replies.is_empty()
            && self.deferred_readiness_closes.is_empty()
    }
    pub(crate) fn install_pane(
        &mut self,
        id: String,
        incarnation: u64,
        pane: Pane,
        current: Arc<AtomicBool>,
    ) -> OwnerReply {
        if self.runtimes.contains_key(&id)
            || self.closing.contains_key(&id)
            || self
                .retirements
                .keys()
                .any(|(retired_id, _)| retired_id == &id)
        {
            self.retire_uninstalled(id, incarnation, Ok(pane), None, "pane already exists");
            return Err("pane already exists".into());
        }
        self.next_incarnation = self.next_incarnation.max(incarnation);
        let child_pid = pane.pty().process_id();
        let child_started_at = child_pid.and_then(crate::process::host_process_start_time);
        let started_at_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let pty = pane.shared_pty();
        let pane = Arc::new(Mutex::new(pane));
        let send = self.sender.clone();
        let output_thread = pane_runtime::spawn_output_thread(
            id.clone(),
            incarnation,
            pane.clone(),
            current.clone(),
            move |event| {
                let _ = send.send(OwnerMessage::Pane(event));
            },
        );
        let send = self.sender.clone();
        let status_thread = pane_runtime::spawn_status_thread(
            id.clone(),
            incarnation,
            pane.clone(),
            move |event| {
                let _ = send.send(OwnerMessage::Pane(event));
            },
        );
        self.runtimes.insert(
            id.clone(),
            WindowRuntime {
                project_root: self.root.clone(),
                policy: Default::default(),
                incarnation,
                _initial_prompt_file: None,
                registry_cleanup: None,
                close_handoff: None,
                close_monitor_snapshot: None,
                pm_lifecycle: None,
                rollback_monitor_failure: None,
                member_cleanup: None,
                started_at_ms,
                child_started_at,
                current,
                detail: None,
                last_output_at: None,
                pty,
                pane,
                output_thread: Some(output_thread),
                status_thread: Some(status_thread),
                pty_status: crate::WindowProcessStatus::Running,
                hook_status: None,
                output_bytes: 0,
                agent: None,
            },
        );
        self.register_owner_writer(&id)?;
        Ok(
            json!({"id":id,"incarnation":incarnation,"child_pid":child_pid,"host_pid":std::process::id(),"host_instance_id":self.host_instance_id,"started_at_ms":started_at_ms}),
        )
    }
    fn retire_switch_source(&mut self, exited: Box<super::agent_switch::AgentSwitchSourceExited>) {
        let proof = &exited.proof;
        let id = &proof.window_id;
        if !exited.pty.tree_exit_confirmed()
            || exited.pty.process_id() != Some(proof.child_pid)
            || proof.host_pid != std::process::id()
            || crate::process::host_process_start_time(proof.host_pid)
                != Some(proof.host_started_at)
        {
            return;
        }
        if let Some(closing) = self.closing.get(id) {
            if closing.incarnation != proof.runtime_incarnation {
                return;
            }
            // Observe the already-owned close receipt, never poll or start a
            // competing finalizer. Shutdown tracks this continuation as input.
            let (reply, done) = oneshot::channel();
            self.close_replies
                .entry((id.clone(), proof.runtime_incarnation))
                .or_default()
                .push(reply);
            self.input_jobs += 1;
            let sender = self.sender.clone();
            spawn_owner_worker(move || {
                let result = done
                    .blocking_recv()
                    .unwrap_or_else(|_| Err("owner close receipt unavailable".into()));
                if result.is_ok() {
                    let _ = sender.send(OwnerMessage::AgentSwitchSourceExited(exited));
                }
                let (reply, _) = oneshot::channel();
                let _ = sender.send(OwnerMessage::AgentSwitchFinished {
                    drove: false,
                    result,
                    reply,
                });
            });
            return;
        }
        if let Some(runtime) = self.runtimes.get(id) {
            if runtime.incarnation != proof.runtime_incarnation
                || !Arc::ptr_eq(&runtime.pty, &exited.pty)
                || runtime.agent.as_ref().is_none_or(|agent| {
                    agent.active.session_id != proof.session_id
                        || gwt_agent::SessionExecutionIdentity::from_session(&agent.snapshot)
                            .ok()
                            .flatten()
                            .as_ref()
                            != Some(&proof.execution_identity)
                })
            {
                return;
            }
            tracing::info!(operation_id = %exited.operation_id, window_id = %id, "retiring exact exited Switch source in project owner");
            let mut runtime = self.runtimes.remove(id).unwrap();
            self.capture_close_monitor_snapshot(id, &mut runtime);
            let (reply, _) = oneshot::channel();
            self.start_close(id.clone(), runtime, CloseCause::ExplicitClose, reply);
            return;
        }
        if self.pending.contains_key(id)
            || self
                .retirements
                .keys()
                .any(|(candidate, _)| candidate == id)
            || !self.agent_operations.values().any(|operation| {
                operation.id == *id
                    && operation.incarnation == proof.runtime_incarnation
                    && operation.session_id.as_deref() == Some(proof.session_id.as_str())
                    && operation.result.as_ref().is_some_and(|result| {
                        result.as_ref().is_ok_and(|receipt| {
                            receipt["closed"] == true && receipt["diagnostic_exit"] == true
                        })
                    })
            })
        {
            return;
        }
        let Some(raw) = id.strip_prefix(&format!("{}::", self.project_key)) else {
            return;
        };
        if !self
            .registry_projection
            .windows
            .iter()
            .any(|row| row.id == raw && row.session_id.as_deref() == Some(&proof.session_id))
        {
            return;
        }
        let Ok(registry) = &self.agent_registry else {
            return;
        };
        let cleanup = super::agent_dispatch::OwnedWindowRecordCleanup {
            registry: registry.clone(),
            registry_path: crate::persistence::workspace_state_path(&self.root),
            root: self.root.clone(),
            project_key: self.project_key.clone(),
            canonical_id: id.clone(),
            expected_session_id: Some(proof.session_id.clone()),
        };
        self.start_retirement(
            id.clone(),
            proof.runtime_incarnation,
            Retirement::SwitchStoppedWindow {
                proof: Box::new(exited.proof),
                exit: StoppedOwnerWindowExit::CapturedPty(exited.pty),
                cleanup: Box::new(cleanup),
            },
            None,
            None,
        );
    }

    fn begin_agent_switch(
        &mut self,
        action: Option<crate::issue_monitor::AgentSwitchAction>,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        if self.stopping {
            let _ = reply.send(Err("project owner is stopping".into()));
            return;
        }
        let drove = action.is_none();
        if drove && self.switch_job_inflight {
            let _ = reply.send(Ok(json!({"busy":true})));
            return;
        }
        if drove {
            self.switch_job_inflight = true;
        }
        let root = self.root.clone();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let entries = self
            .pty_writers
            .read()
            .map(|entries| {
                entries
                    .values()
                    .filter(|entry| {
                        entry.project_key.as_str() == self.project_key
                            && entry.monitor_runtime.is_some()
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let writers = self.pty_writers.clone();
        let sender = self.sender.clone();
        self.input_jobs += 1;
        spawn_owner_worker(move || {
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                gwt_core::operation_deadline::now() + std::time::Duration::from_secs(60),
            );
            let result = match action {
                Some(action) => super::agent_switch::apply_action(&root, &sessions, action),
                None => super::agent_switch::drive_pending(
                    &root,
                    &sessions,
                    &entries,
                    &writers,
                    |exited| {
                        let _ =
                            sender.send(OwnerMessage::AgentSwitchSourceExited(Box::new(exited)));
                    },
                ),
            }
            .map(|snapshot| json!({"snapshot":snapshot}));
            let _ = sender.send(OwnerMessage::AgentSwitchFinished {
                drove,
                result,
                reply,
            });
        });
    }

    fn request(&mut self, request: OwnerRequest, reply: oneshot::Sender<OwnerReply>) {
        if request.project_key != self.project_key {
            let _ = reply.send(Err("project key mismatch".into()));
            return;
        }
        if !self.stopping {
            if matches!(request.command, OwnerCommand::Health) {
                // GUI clients verify Health before sending a launch command.
                // Retrying here makes external registry recovery reachable on
                // the same owner rather than leaving discovery permanently degraded.
                let _ = self.initialize_shared_registry();
            } else if matches!(
                request.command,
                OwnerCommand::LaunchAgent { .. }
                    | OwnerCommand::RestoreSavedSession { .. }
                    | OwnerCommand::ContinueWork { .. }
                    | OwnerCommand::AgentSwitch { .. }
                    | OwnerCommand::DriveAgentSwitch
                    | OwnerCommand::UpdateLayout { .. }
            ) {
                if let Err(error) = self.initialize_shared_registry() {
                    let _ = reply.send(Err(error));
                    return;
                }
            }
        }
        if matches!(
            request.command,
            OwnerCommand::PrepareMobileInput { .. }
                | OwnerCommand::CommitMobileInput { .. }
                | OwnerCommand::CancelMobileInput { .. }
                | OwnerCommand::MobileInputOutcome { .. }
                | OwnerCommand::SettleMobileInput { .. }
        ) {
            let deferred_reply = match &request.command {
                OwnerCommand::PrepareMobileInput { identity, .. } => Some((identity.clone(), true)),
                OwnerCommand::CommitMobileInput { ready } => Some((ready.identity.clone(), false)),
                _ => None,
            };
            let result = self.mobile_input_command(request.command);
            if result.is_ok() {
                if let Some((identity, preparing)) = deferred_reply {
                    if let Ok(transaction) = self.pm_wake_permits.mobile(&identity) {
                        transaction.reply_when_observed(reply, preparing);
                        return;
                    }
                }
            }
            let _ = reply.send(result);
            return;
        }
        if matches!(
            request.command,
            OwnerCommand::AgentSwitch { .. } | OwnerCommand::DriveAgentSwitch
        ) {
            let action = match request.command {
                OwnerCommand::AgentSwitch { action } => Some(*action),
                OwnerCommand::DriveAgentSwitch => None,
                _ => unreachable!(),
            };
            self.begin_agent_switch(action, reply);
            return;
        }
        if let OwnerCommand::NotifyBoardDecision {
            entry_id,
            session_id,
        } = request.command
        {
            self.notify_board_decision(entry_id, session_id, reply);
            return;
        }
        if matches!(
            &request.command,
            OwnerCommand::LaunchAgent { .. }
                | OwnerCommand::LaunchShell { .. }
                | OwnerCommand::LaunchConfiguredShell { .. }
                | OwnerCommand::RestoreSavedSession { .. }
                | OwnerCommand::ContinueWork { .. }
                | OwnerCommand::Close { .. }
                | OwnerCommand::CloseStoppedSession { .. }
        ) {
            let issues = match &request.command {
                OwnerCommand::LaunchAgent {
                    config, metadata, ..
                } => [
                    config.linked_issue_number,
                    metadata
                        .launch_feedback_context
                        .as_ref()
                        .and_then(|feedback| feedback.issue_monitor_launch_binding_issue_number()),
                ],
                OwnerCommand::RestoreSavedSession {
                    expected_source, ..
                } => [expected_source.linked_issue_number, None],
                _ => [None, None],
            };
            let continuing = matches!(&request.command, OwnerCommand::ContinueWork { .. });
            for operation in self.agent_operations.values_mut() {
                if let Some(issue) = operation.pending_monitor_rejection_issue() {
                    // The matching producer is rejected after immutable replay,
                    // without invalidating the original rejection worker first.
                    if !continuing && !issues.contains(&Some(issue)) {
                        operation.current.store(false, Ordering::Release);
                    }
                }
            }
        }
        if matches!(&request.command, OwnerCommand::CheckLaunchPermission { .. }) {
            self.check_launch_permission(request.command, reply);
            return;
        }
        if matches!(&request.command, OwnerCommand::RejectMonitorLaunch { .. }) {
            self.reject_monitor_launch(request.command, reply);
            return;
        }
        if let OwnerCommand::DeliverAutonomousAnswer {
            id,
            incarnation,
            issue_number,
            delivery_id,
        } = request.command
        {
            self.deliver_owner_answer(id, incarnation, issue_number, delivery_id, reply);
            return;
        }
        if matches!(
            request.command,
            OwnerCommand::AcknowledgeMonitorLaunch { .. } | OwnerCommand::AdoptMonitorLaunch { .. }
        ) {
            self.acknowledge_owner_monitor(request.command, reply);
            return;
        }
        if let OwnerCommand::ProviderUsageSnapshot { accounts } = request.command {
            self.apply_policy_usage_snapshot(accounts);
            let _ = reply.send(Ok(json!({"ok":true})));
            return;
        }
        if matches!(
            request.command,
            OwnerCommand::LaunchShell { .. }
                | OwnerCommand::LaunchConfiguredShell { .. }
                | OwnerCommand::RestoreSavedSession { .. }
                | OwnerCommand::LaunchAgent { .. }
                | OwnerCommand::ContinueWork { .. }
        ) {
            if let Err(error) = &self.agent_registry {
                let _ = reply.send(Err(error.clone()));
                return;
            }
        }
        if matches!(request.command, OwnerCommand::LaunchConfiguredShell { .. }) {
            self.launch_configured_shell(request.command, reply);
            return;
        }
        if matches!(request.command, OwnerCommand::RestoreSavedSession { .. }) {
            self.restore_saved_session(request.command, reply);
            return;
        }
        if let OwnerCommand::CloseWork {
            work_id,
            close_kind,
        } = request.command
        {
            let live = self
                .runtimes
                .values()
                .filter_map(|runtime| {
                    runtime
                        .agent
                        .as_ref()
                        .map(|agent| agent.active.session_id.clone())
                })
                .chain(
                    self.agent_operations
                        .values()
                        .filter(|operation| operation.result.is_none())
                        .filter_map(|operation| operation.session_id.clone()),
                )
                .collect();
            self.agent_dispatch_jobs += 1;
            let sender = self.sender.clone();
            let root = self.root.clone();
            spawn_owner_worker(move || {
                let result = super::agent_dispatch::close_work_from_native(
                    &root,
                    &gwt_core::paths::gwt_sessions_dir(),
                    &work_id,
                    &close_kind,
                    &live,
                );
                let _ = sender.send(OwnerMessage::NativeWorkClosed { result, reply });
            });
            return;
        }
        if let OwnerCommand::UpdateLayout { layout } = request.command {
            self.update_owner_layout(layout, reply);
            return;
        }
        if let OwnerCommand::CloseStoppedSession {
            session_id,
            expected,
        } = request.command
        {
            self.close_stopped_owner_session(session_id, expected, reply);
            return;
        }
        if let OwnerCommand::PendingCancel { id, operation_id } = request.command {
            self.cancel_pending(id, operation_id, None, reply);
            return;
        }
        if let OwnerCommand::PendingCancelExact {
            id,
            operation_id,
            expected_request,
        } = request.command
        {
            self.cancel_pending(id, operation_id, Some(expected_request), reply);
            return;
        }
        if let OwnerCommand::ContinueWork {
            client_id,
            operation_id,
            work_id,
        } = request.command
        {
            self.continue_work(client_id, operation_id, work_id, reply);
            return;
        }
        if matches!(request.command, OwnerCommand::LaunchAgent { .. }) {
            self.launch_agent(request.command, reply);
            return;
        }
        if let OwnerCommand::LaunchShell {
            id,
            command,
            args,
            cols,
            rows,
        } = request.command
        {
            if !id
                .strip_prefix(&format!("{}::", self.project_key))
                .is_some_and(|raw| !raw.is_empty())
            {
                let _ = reply.send(Err("pane is outside the fixed project scope".into()));
                return;
            }
            if self.runtimes.contains_key(&id)
                || self.pending.contains_key(&id)
                || self.closing.contains_key(&id)
                || self
                    .retirements
                    .keys()
                    .any(|(retired_id, _)| retired_id == &id)
            {
                let _ = reply.send(Err("pane already exists".into()));
                return;
            }
            self.next_incarnation += 1;
            let incarnation = self.next_incarnation;
            self.pending.insert(id.clone(), incarnation);
            let cwd = self.root.clone();
            let send = self.sender.clone();
            spawn_owner_worker(move || {
                let config = SpawnConfig {
                    command,
                    args,
                    cols,
                    rows,
                    env: Default::default(),
                    remove_env: Vec::new(),
                    cwd: Some(cwd),
                };
                let pane = super::spawn_unbound_pane(&id, config, None, incarnation, None, false);
                if let Err(error) = send.send(OwnerMessage::Prepared {
                    id,
                    incarnation,
                    pane: pane.map(Box::new),
                    reply: Some(reply),
                    operation_id: None,
                    initial_prompt_file: None,
                }) {
                    if let OwnerMessage::Prepared {
                        id, pane: Ok(pane), ..
                    } = error.0
                    {
                        let _ = crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &id);
                    }
                }
            });
            return;
        }
        if matches!(
            request.command,
            OwnerCommand::Input { .. } | OwnerCommand::SendInput { .. }
        ) {
            let (id, incarnation, bytes, text) = match request.command {
                OwnerCommand::Input {
                    id,
                    incarnation,
                    data,
                } => (id, incarnation, Some(data), None),
                OwnerCommand::SendInput {
                    id,
                    incarnation,
                    text,
                } => (id, incarnation, None, Some(text)),
                _ => unreachable!(),
            };
            let entry = self.exact(&id, incarnation).and_then(|runtime| {
                let writers = self.pty_writers.read().map_err(|error| error.to_string())?;
                writers
                    .get(&id)
                    .filter(|entry| {
                        entry.current.load(Ordering::Acquire)
                            && Arc::ptr_eq(&entry.current, &runtime.current)
                            && Arc::ptr_eq(&entry.handle, &runtime.pty)
                    })
                    .cloned()
                    .ok_or_else(|| "pane has no current physical writer".into())
            });
            match entry {
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
                Ok(entry) => {
                    self.begin_policy_input(
                        &id,
                        bytes
                            .as_deref()
                            .unwrap_or_else(|| text.as_deref().unwrap_or_default().as_bytes()),
                    );
                    self.input_jobs += 1;
                    let writers = self.pty_writers.clone();
                    let sender = self.sender.clone();
                    spawn_owner_worker(move || {
                        let result = match bytes {
                            Some(bytes) => super::agent_dispatch::write_current_raw_input(
                                &writers, &id, &entry, &bytes,
                            ),
                            None => super::agent_dispatch::write_current_pane_input(
                                &writers,
                                &id,
                                &entry,
                                text.as_deref().expect("typed input"),
                            ),
                        };
                        let result = result
                            .map(|()| super::events::OwnerInputObservation::capture(&entry.handle));
                        let _ = sender.send(OwnerMessage::InputWritten { reply, result });
                    });
                }
            }
            return;
        }
        if let OwnerCommand::CancelPmWake {
            id,
            incarnation,
            host_instance_id,
            operation_id,
        } = &request.command
        {
            let result = if host_instance_id != &self.host_instance_id {
                Err("PM wake owner identity changed".into())
            } else {
                self.exact(id, *incarnation)
                    .map(|_| ())
                    .and_then(|()| self.pm_wake_permits.cancel(operation_id, id, *incarnation))
            };
            let _ = reply.send(result.map(|()| json!({"ok":true})));
            return;
        }
        if let OwnerCommand::PmWakeInput {
            id,
            incarnation,
            host_instance_id,
            delivery,
        } = request.command
        {
            let captured = (|| {
                if host_instance_id != self.host_instance_id {
                    return Err("PM wake owner identity changed".into());
                }
                let runtime = self.exact(&id, incarnation)?;
                let writers = self.pty_writers.read().map_err(|error| error.to_string())?;
                let entry = writers
                    .get(&id)
                    .filter(|entry| {
                        Arc::ptr_eq(&entry.current, &runtime.current)
                            && Arc::ptr_eq(&entry.handle, &runtime.pty)
                    })
                    .cloned()
                    .ok_or("PM wake has no current physical writer")?;
                drop(writers);
                let session = runtime
                    .agent
                    .as_ref()
                    .map(|agent| agent.active.session_id.clone());
                let pane = runtime.pane.clone();
                let output_seq = pane.lock().map_err(|error| error.to_string())?.output_seq();
                let permit =
                    self.pm_wake_permits
                        .start(&delivery.operation_id, &id, incarnation)?;
                Ok::<_, String>((entry, permit, session, pane, output_seq))
            })();
            match captured {
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
                Ok((entry, permit, session, pane, output_seq)) => {
                    self.input_jobs += 1;
                    let writers = self.pty_writers.clone();
                    let root = self.root.clone();
                    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
                    let sender = self.sender.clone();
                    spawn_owner_worker(move || {
                        let result = super::pm_delivery::deliver_pm_wake(
                            &root,
                            &id,
                            &entry,
                            &writers,
                            &delivery,
                            &permit,
                            (
                                &sessions_dir,
                                session.as_deref(),
                                incarnation,
                                &pane,
                                output_seq,
                            ),
                        );
                        let _ = sender.send(OwnerMessage::PmWakeWritten {
                            operation_id: delivery.operation_id,
                            reply,
                            result,
                        });
                    });
                }
            }
            return;
        }
        let result = match request.command {
            OwnerCommand::PromoteStandalone => {
                self.gui_presence.promote();
                self.residency.send_replace(ResidencySnapshot {
                    startup_intent: self.gui_presence.intent(),
                    exit_started: false,
                });
                Ok(json!({"startup_intent":self.gui_presence.intent()}))
            }
            OwnerCommand::Health => {
                let initialization_error = self
                    .shared_registry_initialization
                    .as_ref()
                    .and_then(|result| result.as_ref().err());
                let health = json!({"startup_intent":self.gui_presence.intent(),"gui_presence_count":self.gui_presence.count(),"pid":std::process::id(),"host_started_at":crate::process::host_process_start_time(std::process::id()),"host_instance_id":self.host_instance_id,"project_key":self.project_key,"panes":self.runtimes.len(),"pending_transport":self.pending_transport.len(),"draining":self.stopping,"reconciliation_required":self.retirements.values().any(Option::is_some)||self.agent_registry.is_err()||initialization_error.is_some(),"agent_registry_initialization_error":initialization_error,"reconciliation":self.retirements.iter().filter_map(|((id,incarnation),captured)| captured.as_ref().map(|_| json!({"id":id,"incarnation":incarnation}))).collect::<Vec<_>>()});
                #[cfg(any(test, feature = "test-gh-guard"))]
                let health = {
                    let mut health = health;
                    health["test_pty_writers"] = json!(self
                        .pty_writers
                        .read()
                        .map(|writers| writers.len())
                        .unwrap_or(usize::MAX));
                    health
                };
                Ok(health)
            }
            OwnerCommand::ListPanes => Ok(json!({
                "panes":self.runtimes.iter().filter_map(|(id,runtime)| self.pane_snapshot(id,runtime.incarnation).ok()).collect::<Vec<_>>(),
                "canonical_rows":self.registry_projection,
                "pending":self.pending.iter().map(|(id,incarnation)| json!({"id":id,"incarnation":incarnation})).collect::<Vec<_>>(),
                "closing":self.closing.iter().map(|(id,closing)| json!({"id":id,"incarnation":closing.incarnation})).collect::<Vec<_>>(),
                "reconciliation":self.retirements.keys().map(|(id,incarnation)| json!({"id":id,"incarnation":incarnation})).collect::<Vec<_>>(),
            })),

            OwnerCommand::Close { id, incarnation } => {
                if self
                    .closing
                    .get(&id)
                    .is_some_and(|closing| closing.incarnation == incarnation)
                {
                    self.close_replies
                        .entry((id, incarnation))
                        .or_default()
                        .push(reply);
                    return;
                }
                if let Some(captured) = self
                    .retirements
                    .get_mut(&(id.clone(), incarnation))
                    .and_then(Option::take)
                {
                    match captured {
                        Retirement::Runtime { runtime, cause } => {
                            self.retirements.remove(&(id.clone(), incarnation));
                            self.start_close(id, *runtime, cause, reply);
                        }
                        captured => {
                            self.start_retirement(id, incarnation, captured, Some(reply), None)
                        }
                    }
                    return;
                }
                if !self
                    .runtimes
                    .get(&id)
                    .is_some_and(|runtime| runtime.incarnation == incarnation)
                {
                    Err("pane incarnation mismatch".into())
                } else {
                    let runtime = self.runtimes.remove(&id).expect("validated exact pane");
                    self.start_close(id, runtime, CloseCause::ExplicitClose, reply);
                    return;
                }
            }
            OwnerCommand::Resize {
                id,
                incarnation,
                cols,
                rows,
            } => self
                .exact(&id, incarnation)
                .and_then(|runtime| {
                    runtime
                        .pane
                        .lock()
                        .map_err(|e| e.to_string())?
                        .resize(cols, rows)
                        .map_err(|e| e.to_string())
                })
                .map(|_| json!({"ok":true})),
            OwnerCommand::Snapshot { id, incarnation } => self.pane_snapshot(&id, incarnation),

            OwnerCommand::AgentSwitch { .. }
            | OwnerCommand::DriveAgentSwitch
            | OwnerCommand::DeliverAutonomousAnswer { .. }
            | OwnerCommand::RejectMonitorLaunch { .. }
            | OwnerCommand::CheckLaunchPermission { .. }
            | OwnerCommand::NotifyBoardDecision { .. }
            | OwnerCommand::AcknowledgeMonitorLaunch { .. }
            | OwnerCommand::AdoptMonitorLaunch { .. }
            | OwnerCommand::ProviderUsageSnapshot { .. }
            | OwnerCommand::UpdateLayout { .. }
            | OwnerCommand::CloseWork { .. }
            | OwnerCommand::CloseStoppedSession { .. }
            | OwnerCommand::LaunchShell { .. }
            | OwnerCommand::LaunchConfiguredShell { .. }
            | OwnerCommand::RestoreSavedSession { .. }
            | OwnerCommand::LaunchAgent { .. }
            | OwnerCommand::Input { .. }
            | OwnerCommand::PrepareMobileInput { .. }
            | OwnerCommand::CommitMobileInput { .. }
            | OwnerCommand::CancelMobileInput { .. }
            | OwnerCommand::MobileInputOutcome { .. }
            | OwnerCommand::SettleMobileInput { .. }
            | OwnerCommand::SendInput { .. }
            | OwnerCommand::PmWakeInput { .. }
            | OwnerCommand::CancelPmWake { .. }
            | OwnerCommand::PendingCancel { .. }
            | OwnerCommand::PendingCancelExact { .. }
            | OwnerCommand::ContinueWork { .. } => unreachable!(),
        };
        let _ = reply.send(result);
    }

    fn mobile_input_command(&mut self, command: OwnerCommand) -> OwnerReply {
        use super::events::OwnerMobileInputIdentity;
        let check_owner = |identity: &OwnerMobileInputIdentity| {
            if identity.host_instance_id != self.host_instance_id
                || !identity.id.starts_with(&format!("{}::", self.project_key))
            {
                return Err("native mobile input owner identity changed".to_string());
            }
            Ok(())
        };
        match command {
            OwnerCommand::PrepareMobileInput { identity, data } => {
                use sha2::{Digest, Sha256};
                check_owner(&identity)?;
                if self.stopping || hex::encode(Sha256::digest(&data)) != identity.data_sha256 {
                    return Err("native mobile input preparation is unavailable".into());
                }
                let runtime = self.exact(&identity.id, identity.incarnation)?;
                if runtime.agent.as_ref().map(|agent| &agent.active.session_id)
                    != identity.session_id.as_ref()
                {
                    return Err("native mobile input Session changed".into());
                }
                let entry = self
                    .pty_writers
                    .read()
                    .map_err(|error| error.to_string())?
                    .get(&identity.id)
                    .filter(|entry| {
                        Arc::ptr_eq(&entry.current, &runtime.current)
                            && Arc::ptr_eq(&entry.handle, &runtime.pty)
                            && entry.current.load(Ordering::Acquire)
                    })
                    .cloned()
                    .ok_or("native mobile input has no current physical writer")?;
                let (transaction, created) = self.pm_wake_permits.prepare_mobile(&identity)?;
                if created {
                    self.input_jobs += 1;
                    let transaction = transaction.clone();
                    let sender = self.sender.clone();
                    spawn_owner_worker(move || {
                        let mut attempted = false;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let reservation = entry
                                .handle
                                .reserve_input_transaction()
                                .map_err(|error| error.to_string())?;
                            reservation
                                .write_ordinary_input_authorized(&data, |commit| {
                                    let refused = |details| {
                                        gwt_terminal::TerminalError::PtyIoError { details }
                                    };
                                    transaction.wait_for_commit().map_err(refused)?;
                                    if !entry.current.load(Ordering::Acquire)
                                        || !transaction.can_write()
                                    {
                                        return Err(refused(
                                            "native prepared input is no longer current".into(),
                                        ));
                                    }
                                    commit(&mut || attempted = true)
                                })
                                .map_err(|error| error.to_string())
                            // Reservation Drop releases the physical writer before completion.
                        }))
                        .unwrap_or_else(|_| Err("native prepared input worker panicked".into()));
                        let observation =
                            super::events::OwnerInputObservation::capture(&entry.handle);
                        let _ = sender.send(OwnerMessage::MobileInputWritten {
                            identity,
                            data,
                            transaction,
                            attempted,
                            result,
                            observation,
                        });
                    });
                }
                serde_json::to_value(transaction.outcome()?).map_err(|error| error.to_string())
            }
            OwnerCommand::CommitMobileInput { ready } => {
                check_owner(&ready.identity)?;
                let transaction = self.pm_wake_permits.mobile(&ready.identity)?;
                let current = self
                    .exact(&ready.identity.id, ready.identity.incarnation)
                    .is_ok_and(|runtime| {
                        runtime.agent.as_ref().map(|agent| &agent.active.session_id)
                            == ready.identity.session_id.as_ref()
                    });
                if self.stopping || !current {
                    self.pm_wake_permits.cancel_mobile(&ready.identity)?;
                    return Err("native prepared input Session or pane changed".into());
                }
                transaction.commit(&ready)?;
                serde_json::to_value(transaction.outcome()?).map_err(|error| error.to_string())
            }
            OwnerCommand::CancelMobileInput { identity } => {
                check_owner(&identity)?;
                serde_json::to_value(self.pm_wake_permits.cancel_mobile(&identity)?)
                    .map_err(|error| error.to_string())
            }
            OwnerCommand::MobileInputOutcome { identity } => {
                check_owner(&identity)?;
                serde_json::to_value(self.pm_wake_permits.mobile(&identity)?.outcome()?)
                    .map_err(|error| error.to_string())
            }
            OwnerCommand::SettleMobileInput { ready } => {
                check_owner(&ready.identity)?;
                self.pm_wake_permits.settle_mobile(&ready)?;
                Ok(json!({"ok":true}))
            }
            _ => unreachable!("private mobile transaction command"),
        }
    }
    fn pane_snapshot(&self, id: &str, incarnation: u64) -> OwnerReply {
        self.runtimes.get(id).filter(|runtime| runtime.incarnation == incarnation)
            .ok_or_else(|| "pane incarnation mismatch".to_string()).and_then(|runtime| {
            let pane = runtime.pane.lock().map_err(|error| error.to_string())?;
            let agent = runtime.agent.as_ref();
            Ok(json!({
                "id":id, "incarnation":incarnation, "screen":pane.screen().contents(),
                "screen_ansi":pane.screen().contents_formatted(), "screen_seq":pane.output_seq(), "seq":pane.output_seq(),
                "has_unsent_user_input":pane.has_unsent_user_input(),
                "input_observation":super::events::OwnerInputObservation::capture(&runtime.pty),
                "child_pid":runtime.pty.process_id(), "started_at_ms":runtime.started_at_ms,
                "raw_status":runtime.pty_status, "status":Self::runtime_status(runtime), "detail":runtime.detail,
                "last_output_at":runtime.last_output_at,
                "exit_confirmed":pane.last_exit().is_some(), "exit_code":pane.last_exit().map(|exit| exit.exit_code),
                "session_id":agent.map(|agent| &agent.active.session_id),
                "agent_id":agent.map(|agent| &agent.active.agent_id),
                "agent_name":agent.and_then(|agent| agent.snapshot.agent_name.as_deref()),
                "display_name":agent.map(|agent| &agent.active.display_name),
                "purpose_title":agent.and_then(|agent| agent.purpose_title.as_deref()),
                "branch":agent.map(|agent| &agent.active.branch_name),
                "runtime_target":agent.map(|agent| &agent.active.runtime_target),
                "worktree_path":agent.map(|agent| &agent.active.worktree_path),
                "closing":self.closing.get(id).is_some_and(|closing| closing.incarnation == incarnation),
            }))
        })
    }
    /// Read-only retry evidence; the ordinary preparation/claim path still
    /// reacquires its canonical leases before any subsequent launch mutation.
    fn manual_retry_preparation_is_exact(
        sessions: &Path,
        stopped: &gwt_agent::SessionExecutionIdentity,
        operation_id: &str,
    ) -> Result<bool, String> {
        use crate::cli::execution_state as execution;
        let owner = execution::ExecutionOwnerKey {
            kind: match stopped.execution_binding.owner_kind.as_str() {
                "issue" => execution::ExecutionOwnerKind::Issue,
                "spec" => execution::ExecutionOwnerKind::Spec,
                _ => return Err("manual retry source owner is not canonical".into()),
            },
            number: stopped.execution_binding.owner_number,
        };
        if !matches!(
            execution::classify_exact_session_runtime(sessions, stopped)
                .map_err(|error| error.to_string())?,
            execution::ExactSessionRuntimeDisposition::Terminal(_)
        ) {
            return Err("manual retry source has no exact terminal runtime proof".into());
        }
        let attempt = execution::continuation_attempt_for_operation(
            &stopped.worktree_path,
            owner,
            operation_id,
        )
        .map_err(|error| error.to_string())?;
        let current = execution::current_execution_binding(&stopped.worktree_path, owner)
            .map_err(|error| error.to_string())?
            .ok_or("manual retry has no current execution binding")?;
        let prepared = if let Some(attempt) = attempt {
            if attempt.status != execution::ContinuationAttemptStatus::Prepared
                || attempt.predecessor.generation_id
                    != stopped.execution_binding.identity.generation_id
                || attempt.predecessor_generation_content_hash != current.ledger_head_hash
                || attempt.request.initial_session_id
                    != super::continuation::manual_successor_stable_component(
                        "manual-successor",
                        operation_id,
                    )
            {
                return Err("manual retry no longer has its exact Prepared operation".into());
            }
            let binding = execution::prepared_successor_execution_binding(
                &stopped.worktree_path,
                owner,
                &attempt.request,
            )
            .map_err(|error| error.to_string())?;
            match gwt_agent::inspect_session_path(
                &sessions.join(format!("{}.toml", attempt.request.initial_session_id)),
            ) {
                gwt_agent::SessionPathState::Missing => {}
                gwt_agent::SessionPathState::Present(candidate) => {
                    let identity = gwt_agent::SessionExecutionIdentity::from_session(&candidate)
                        .map_err(|error| error.to_string())?
                        .ok_or("manual retry candidate is unbound")?;
                    let recovery =
                        crate::session_launch::recovery::durable_launch_recovery_session_identity(
                            sessions,
                            &candidate.id,
                        )?;
                    if recovery.as_ref() != Some(&identity)
                        || identity.execution_binding.identity != binding
                        || !matches!(
                            execution::classify_exact_session_runtime(sessions, &identity)
                                .map_err(|error| error.to_string())?,
                            execution::ExactSessionRuntimeDisposition::Absent
                        )
                    {
                        return Err("manual retry candidate changed or has runtime evidence".into());
                    }
                }
                gwt_agent::SessionPathState::Error(error) => return Err(error.to_string()),
            }
            true
        } else {
            false
        };
        let expected = &stopped.execution_binding.identity;
        let projection = execution::load(&stopped.worktree_path)
            .map_err(|error| error.to_string())?
            .ok_or("manual retry has no current execution projection")?;
        if current.generation_id != expected.generation_id
            || current.binding_id != expected.binding_id
            || (!prepared && current != *expected)
            || projection.owner_kind != owner.kind
            || projection.owner_number != owner.number
            || projection.primary_session_id != stopped.session_id
        {
            return Err(
                "manual retry predecessor is no longer the current generation and Session binding"
                    .into(),
            );
        }
        let exact = execution::with_current_runtime_cleanup_session_execution_identity_lease(
            sessions,
            stopped,
            || {
                let source = gwt_agent::Session::load(
                    &sessions.join(format!("{}.toml", stopped.session_id)),
                )?;
                if source.status != gwt_agent::AgentStatus::Stopped
                    || gwt_agent::read_session_active_launch_handshake_under_lease(
                        sessions, stopped,
                    )?
                    .is_some()
                {
                    return Ok(false);
                }
                Ok(prepared
                    || gwt_agent::read_session_manual_handoff_under_lease(sessions, stopped)?
                        .is_some())
            },
        )
        .map_err(|error| error.to_string())?
        .transpose()
        .map_err(|error: std::io::Error| error.to_string())?;
        if exact != Some(true) {
            return Err("manual retry source authority changed after its exact stop".into());
        }
        Ok(prepared)
    }

    fn cancel_pending(
        &mut self,
        id: String,
        operation_id: String,
        expected_request: Option<serde_json::Value>,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        if let Some(expected) = expected_request.as_ref() {
            let identity = serde_json::from_value::<OwnerCommand>(expected.clone())
                .ok()
                .and_then(|command| match command {
                    OwnerCommand::LaunchAgent {
                        id, operation_id, ..
                    }
                    | OwnerCommand::LaunchConfiguredShell {
                        id, operation_id, ..
                    }
                    | OwnerCommand::RestoreSavedSession {
                        id, operation_id, ..
                    } => Some((id, operation_id)),
                    _ => None,
                });
            if identity
                .as_ref()
                .is_none_or(|(expected_id, expected_operation)| {
                    expected_id != &id || expected_operation != &operation_id
                })
                || operation_id.trim().is_empty()
                || !id
                    .strip_prefix(&format!("{}::", self.project_key))
                    .is_some_and(|raw| !raw.is_empty())
            {
                let _ = reply.send(Err(
                    "exact cancellation requires its original fixed-project launch request".into(),
                ));
                return;
            }
        }
        let Some(operation) = self.agent_operations.get_mut(&operation_id) else {
            // Cancel may arrive over HTTP before the corresponding LaunchAgent.
            // Keep the exact command in this same operation-cache tombstone.
            // A later different payload cannot inherit its cancellation proof.
            self.agent_operations.insert(
                operation_id.clone(),
                AgentOperation {
                    session_id: None,
                    early_events: VecDeque::new(),
                    request: expected_request.unwrap_or(serde_json::Value::Null),
                    id: id.clone(),
                    incarnation: 0,
                    current: Arc::new(AtomicBool::new(false)),
                    replies: Vec::new(),
                    cancel_replies: Vec::new(),
                    cancellation_cleaned: true,
                    manual_predecessor_stopped: None,
                    retained_manual_failure: None,
                    pending_continue: None,
                    failed_session: None,
                    result: Some(Err("launch cancelled before owner admission".into())),
                },
            );
            let _ = reply.send(Ok(
                json!({"cancelled":true,"id":id,"operation_id":operation_id}),
            ));
            return;
        };
        if expected_request
            .as_ref()
            .is_some_and(|expected| operation.request != *expected)
        {
            // Another immutable command owns this key. This caller has no
            // candidate here to drain, including when the old entry is legacy Null.
            let _ = reply.send(Ok(json!({
                "cancelled":false, "request_matches":false,
                "id":id, "operation_id":operation_id,
            })));
            return;
        }
        if operation.id != id {
            let _ = reply.send(Err("operation identity belongs to another pane".into()));
            return;
        }
        let repairing = self.runtimes.get(&id).is_some_and(|runtime| {
            runtime.incarnation == operation.incarnation
                && Arc::ptr_eq(&runtime.current, &operation.current)
                && runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.binding_repair_inflight)
        });
        if !repairing {
            operation.current.store(false, Ordering::Release);
            operation.retained_manual_failure = None;
        }
        if operation.result.is_none() {
            operation.cancel_replies.push(reply);
            return;
        }
        if operation.cancellation_cleaned {
            let _ = reply.send(Ok(
                json!({"cancelled":true,"id":id,"operation_id":operation_id}),
            ));
            return;
        }
        if operation.failed_session.is_some() {
            let in_flight = !operation.cancel_replies.is_empty();
            operation.cancel_replies.push(reply);
            if !in_flight {
                self.close_failed_manual_launch(&operation_id);
            }
            return;
        }
        let incarnation = operation.incarnation;
        if self
            .runtimes
            .get(&id)
            .is_some_and(|runtime| runtime.incarnation == incarnation)
            || self
                .closing
                .get(&id)
                .is_some_and(|closing| closing.incarnation == incarnation)
            || self.retirements.contains_key(&(id.clone(), incarnation))
        {
            self.request(
                OwnerRequest {
                    project_key: self.project_key.clone(),
                    command: OwnerCommand::Close { id, incarnation },
                },
                reply,
            );
        } else {
            let _ = reply.send(Err(
                "cancelled launch cleanup requires exact reconciliation".into(),
            ));
        }
    }
    fn continue_work(
        &mut self,
        client_id: String,
        operation_id: String,
        work_id: String,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        use super::continuation::{
            ContinueWorkInput, ContinueWorkPane, ContinueWorkPending, ContinueWorkView,
        };
        let panes: Vec<_> = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                let agent = runtime.agent.as_ref()?;
                Some(ContinueWorkPane {
                    id,
                    active: &agent.active,
                    token: agent.capability_token.as_deref(),
                    live: !self.closing.contains_key(id)
                        && !matches!(
                            Self::runtime_status(runtime),
                            crate::WindowProcessStatus::Stopped | crate::WindowProcessStatus::Error
                        ),
                })
            })
            .collect();
        let pending: Vec<_> = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                Some(ContinueWorkPending {
                    id,
                    pending: runtime.agent.as_ref()?.pending_continue.as_ref()?,
                })
            })
            .chain(
                self.agent_operations
                    .values()
                    .filter(|operation| operation.result.is_none())
                    .filter_map(|operation| {
                        Some(ContinueWorkPending {
                            id: &operation.id,
                            pending: operation.pending_continue.as_ref()?,
                        })
                    }),
            )
            .collect();
        let profile = gwt_core::paths::gwt_config_path();
        let is_git = !matches!(
            gwt_git::detect_repo_type(&self.root),
            gwt_git::RepoType::NonRepo
        );
        let action = super::continuation::coordinate_continue_work_with_monitor_rejection_guard(
            ContinueWorkInput {
                project_root: &self.root,
                sessions_dir: &gwt_core::paths::gwt_sessions_dir(),
                profile_config_path: Some(&profile),
                is_git,
                migration_pending: false,
                client_id: &client_id,
                operation_id: &operation_id,
                work_id: &work_id,
            },
            ContinueWorkView {
                panes: &panes,
                pending: &pending,
                outcomes: &self.continue_work_outcomes,
                issuer: Some(&self.issuer),
            },
            |owner, worktree_path, branch, source_session_id| {
                self.agent_operations.values().any(|operation| {
                    operation.pending_monitor_rejection_matches_source(
                        owner,
                        worktree_path,
                        branch,
                        source_session_id,
                    )
                })
            },
        );
        if matches!(&action, super::continuation::ContinueWorkAction::Failure { failure, cacheable: false }
            if failure.code == "monitor_rejection_in_progress")
        {
            let _ = reply.send(Err("an agent operation is in progress".into()));
            return;
        }
        self.apply_continue_action(client_id, operation_id, work_id, action, reply);
    }
    fn refresh_owner_session_snapshot(&mut self, id: &str) {
        if let Some(runtime) = self.runtimes.get_mut(id) {
            if let Some(agent) = runtime.agent.as_mut() {
                // Inspection promotion may have installed a repaired binding.
                // Refresh only this same durable Session incarnation.
                if let Ok(snapshot) = gwt_agent::Session::load(
                    &gwt_core::paths::gwt_sessions_dir()
                        .join(format!("{}.toml", agent.active.session_id)),
                ) {
                    if snapshot.id == agent.snapshot.id
                        && snapshot.created_at == agent.snapshot.created_at
                        && snapshot.worktree_path == agent.snapshot.worktree_path
                        && snapshot.branch == agent.snapshot.branch
                        && snapshot.agent_id == agent.snapshot.agent_id
                    {
                        agent.snapshot = snapshot;
                    }
                }
            }
        }
    }
    fn publish_legacy_focus(
        &mut self,
        id: String,
        client_id: String,
        operation_id: String,
        outcome: super::continuation::CachedContinueWorkOutcome,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let runtime = self.runtimes.get_mut(&id).expect("classified live runtime");
        let agent = runtime.agent.as_mut().expect("classified live owner agent");
        if agent.binding_repair_inflight {
            let _ = reply.send(Err(
                "the exact legacy binding repair is still being published; retry after it settles"
                    .into(),
            ));
            return;
        }
        let Some(token) = agent.capability_token.clone() else {
            let _ = reply.send(Err(
                "legacy binding repair lost its actual capability".into()
            ));
            return;
        };
        agent.binding_repair_inflight = true;
        let incarnation = runtime.incarnation;
        let current = runtime.current.clone();
        let source = agent.snapshot.clone();
        let pty = runtime.pty.clone();
        let child_started_at = runtime.child_started_at;
        let issuer = agent.issuer.clone();
        let sender = self.sender.clone();
        self.agent_dispatch_jobs += 1;
        spawn_owner_worker(move || {
            let result = publish_repaired_inspection_runtime(
                &source,
                incarnation,
                &current,
                &pty,
                child_started_at,
                &issuer,
                &token,
            );
            let _ = sender.send(OwnerMessage::LegacyFocusPublished(Box::new(
                LegacyFocusPublication {
                    id,
                    incarnation,
                    current,
                    token,
                    source,
                    result,
                    client_id,
                    operation_id,
                    outcome,
                    reply,
                },
            )));
        });
    }

    fn legacy_focus_published(&mut self, publication: LegacyFocusPublication) {
        self.agent_dispatch_jobs -= 1;
        let LegacyFocusPublication {
            id,
            incarnation,
            current,
            token,
            source,
            result,
            client_id,
            operation_id,
            outcome,
            reply,
        } = publication;
        let closing = self.stopping
            || self
                .closing
                .get(&id)
                .is_some_and(|closing| closing.incarnation == incarnation);
        let exact = current.load(Ordering::Acquire)
            && self.runtimes.get(&id).is_some_and(|runtime| {
                runtime.incarnation == incarnation
                    && Arc::ptr_eq(&runtime.current, &current)
                    && runtime.agent.as_ref().is_some_and(|agent| {
                        agent.binding_repair_inflight
                            && agent.capability_token.as_deref() == Some(token.as_str())
                            && same_legacy_session_source(&agent.snapshot, &source)
                    })
            });
        if exact {
            self.runtimes
                .get_mut(&id)
                .unwrap()
                .agent
                .as_mut()
                .unwrap()
                .binding_repair_inflight = false;
        }
        let result = result.and_then(|snapshot| {
            if !exact {
                return Err("legacy binding repair target changed before publication ACK".into());
            }
            let binding = snapshot
                .execution_binding
                .as_ref()
                .ok_or("repaired Session lost its binding")?;
            let runtime = self.runtimes.get_mut(&id).expect("exact repaired runtime");
            let agent = runtime.agent.as_mut().expect("exact repaired agent");
            if !agent.issuer.active_token_is_current(&token, binding) {
                return Err(
                    "legacy binding repair capability changed before publication ACK".into(),
                );
            }
            agent.snapshot = snapshot;
            if closing {
                return Err(
                    "the exact pane closed while its legacy binding repair was being published"
                        .into(),
                );
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                let _ = self.events.send(OwnerEvent::FocusExisting {
                    id: id.clone(),
                    incarnation,
                });
                self.publish_continue_outcome(&client_id, &operation_id, outcome, true, false);
                let _ = reply.send(self.pane_snapshot(&id, incarnation));
            }
            Err(error) => {
                self.publish_continue_outcome(
                    &client_id,
                    &operation_id,
                    super::continuation::CachedContinueWorkOutcome {
                        work_id: outcome.work_id,
                        outcome: crate::ContinueWorkOutcomeKind::ConflictUnknown,
                        message: Some(error.clone()),
                        error_code: Some("continuation_reconciliation_required".into()),
                        retryable: true,
                    },
                    false,
                    false,
                );
                let _ = reply.send(Err(error));
            }
        }
        self.resume_deferred_agent_frontend(&id, incarnation);
        self.resume_deferred_close(&id, incarnation);
    }

    fn focus_owner_pane(&mut self, id: &str) {
        self.refresh_owner_session_snapshot(id);
        if let Some(runtime) = self.runtimes.get(id) {
            let _ = self.events.send(OwnerEvent::FocusExisting {
                id: id.into(),
                incarnation: runtime.incarnation,
            });
        } else if let Some(operation) = self
            .agent_operations
            .values()
            .find(|operation| operation.id == id && operation.result.is_none())
        {
            let _ = self.events.send(OwnerEvent::FocusExisting {
                id: id.into(),
                incarnation: operation.incarnation,
            });
        }
    }
    fn publish_continue_outcome(
        &mut self,
        client_id: &str,
        operation_id: &str,
        outcome: super::continuation::CachedContinueWorkOutcome,
        cacheable: bool,
        fanout: bool,
    ) {
        if cacheable {
            self.continue_work_outcomes
                .insert(operation_id.into(), outcome.clone());
        }
        let mut clients = vec![client_id.to_string()];
        if fanout {
            if let Some(waiters) = self.continue_work_waiters.remove(operation_id) {
                clients.extend(waiters);
            }
        }
        clients.sort();
        clients.dedup();
        for client_id in clients {
            let _ = self.events.send(OwnerEvent::ContinueWorkOutcome {
                client_id,
                operation_id: operation_id.into(),
                work_id: outcome.work_id.clone(),
                outcome: outcome.outcome,
                message: outcome.message.clone(),
                error_code: outcome.error_code.clone(),
                retryable: outcome.retryable,
            });
        }
    }
    fn apply_continue_action(
        &mut self,
        client_id: String,
        operation_id: String,
        work_id: String,
        action: super::continuation::ContinueWorkAction,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        use super::continuation::{CachedContinueWorkOutcome, ContinueWorkAction};
        match action {
            ContinueWorkAction::Failure { failure, cacheable } => {
                self.publish_continue_outcome(
                    &client_id,
                    &operation_id,
                    CachedContinueWorkOutcome {
                        work_id,
                        outcome: failure.outcome,
                        message: Some(failure.message),
                        error_code: Some(failure.code.into()),
                        retryable: failure.retryable,
                    },
                    cacheable,
                    false,
                );
                let _ = reply.send(Ok(json!({"accepted":true,"operation_id":operation_id})));
            }
            ContinueWorkAction::Pending { window_id } => {
                self.continue_work_waiters
                    .entry(operation_id.clone())
                    .or_default()
                    .insert(client_id);
                self.focus_owner_pane(&window_id);
                let incarnation = self
                    .runtimes
                    .get(&window_id)
                    .map(|runtime| runtime.incarnation)
                    .or_else(|| {
                        self.agent_operations
                            .values()
                            .find(|operation| operation.id == window_id)
                            .map(|operation| operation.incarnation)
                    });
                let _ = reply.send(Ok(json!({"pending":true,"id":window_id,"incarnation":incarnation,"operation_id":operation_id})));
            }
            ContinueWorkAction::FocusExisting { window_id, outcome } => {
                if self
                    .runtimes
                    .get(&window_id)
                    .and_then(|runtime| runtime.agent.as_ref())
                    .is_some_and(|agent| agent.snapshot.execution_binding.is_none())
                {
                    self.publish_legacy_focus(window_id, client_id, operation_id, outcome, reply);
                    return;
                }
                self.focus_owner_pane(&window_id);
                self.publish_continue_outcome(&client_id, &operation_id, outcome, true, false);
                let receipt = self.pane_snapshot(&window_id, self.runtimes[&window_id].incarnation);
                let _ = reply.send(receipt);
            }
            ContinueWorkAction::Replay {
                outcome,
                focus,
                completed_window,
                clear_operation,
                refresh_projection,
            } => {
                if let Some(id) = completed_window {
                    if let Some(agent) = self
                        .runtimes
                        .get_mut(&id)
                        .and_then(|runtime| runtime.agent.as_mut())
                    {
                        if agent
                            .pending_continue
                            .as_ref()
                            .is_some_and(|pending| pending.operation_id == operation_id)
                        {
                            agent.pending_continue = None;
                            agent.readiness_deadline = None;
                        }
                    }
                    self.refresh_owner_session_snapshot(&id);
                }
                if clear_operation {
                    for runtime in self.runtimes.values_mut() {
                        if let Some(agent) = runtime.agent.as_mut() {
                            if agent
                                .pending_continue
                                .as_ref()
                                .is_some_and(|pending| pending.operation_id == operation_id)
                            {
                                agent.pending_continue = None;
                                agent.readiness_deadline = None;
                            }
                        }
                    }
                    for operation in self.agent_operations.values_mut() {
                        if operation
                            .pending_continue
                            .as_ref()
                            .is_some_and(|pending| pending.operation_id == operation_id)
                        {
                            operation.pending_continue = None;
                        }
                    }
                }
                if let Some(id) = focus.as_deref() {
                    self.focus_owner_pane(id);
                }
                self.publish_continue_outcome(
                    &client_id,
                    &operation_id,
                    outcome,
                    clear_operation || refresh_projection,
                    true,
                );
                if refresh_projection {
                    let _ = self.events.send(OwnerEvent::Invalidated);
                }
                let _ = reply.send(Ok(
                    json!({"accepted":true,"operation_id":operation_id,"id":focus}),
                ));
            }
            ContinueWorkAction::PreparedLaunch { config, pending } => {
                let id = format!(
                    "{}::continue-{}",
                    self.project_key,
                    uuid::Uuid::new_v4().simple()
                );
                let metadata = super::launch_payload::OwnerLaunchMetadata {
                    workspace_resume_context: Some(pending.resume_context.clone()),
                    ..Default::default()
                };
                self.launch_agent_inner(
                    OwnerCommand::LaunchAgent {
                        id,
                        operation_id,
                        tab_id: self.project_key.clone(),
                        config,
                        metadata: Box::new(metadata),
                        cols: 80,
                        rows: 24,
                    },
                    reply,
                    Some(*pending),
                    None,
                );
            }
        }
    }
    fn restore_saved_session(&mut self, command: OwnerCommand, reply: oneshot::Sender<OwnerReply>) {
        let request = match serde_json::to_value(&command) {
            Ok(request) => request,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let OwnerCommand::RestoreSavedSession {
            id,
            operation_id,
            source_session_id,
            expected_source,
            origin,
            cols,
            rows,
        } = command
        else {
            unreachable!()
        };
        if !matches!(
            origin,
            gwt_agent::SessionLaunchOrigin::AutomaticRestore
                | gwt_agent::SessionLaunchOrigin::UserRestart
        ) || source_session_id != expected_source.id
            || !gwt_agent::validate_session_id_path_component(&source_session_id).is_ok()
            || !id
                .strip_prefix(&format!("{}::", self.project_key))
                .is_some_and(|raw| !raw.is_empty())
        {
            let _ = reply.send(Err(
                "restore requires an exact source, origin, and fixed-project pane identity".into(),
            ));
            return;
        }
        let reserved = self.runtimes.values().any(|runtime| {
            runtime.agent.as_ref().is_some_and(|agent| {
                crate::launch_runtime::same_worktree_path(
                    &agent.snapshot.worktree_path,
                    &expected_source.worktree_path,
                )
            })
        }) || self
            .agent_operations
            .values()
            .filter(|operation| operation.result.is_none())
            .any(|operation| {
                serde_json::from_value::<OwnerCommand>(operation.request.clone())
                    .ok()
                    .is_some_and(|command| {
                        let worktree = match command {
                            OwnerCommand::LaunchAgent { config, .. } => config.working_dir,
                            OwnerCommand::RestoreSavedSession {
                                expected_source, ..
                            } => Some(expected_source.worktree_path),
                            _ => None,
                        };
                        worktree.as_ref().is_some_and(|path| {
                            crate::launch_runtime::same_worktree_path(
                                path,
                                &expected_source.worktree_path,
                            )
                        })
                    })
            });
        let registry = match &self.agent_registry {
            Ok(registry) => registry.clone(),
            Err(error) => {
                let _ = reply.send(Err(error.clone()));
                return;
            }
        };
        let placeholders = self
            .registry_projection
            .windows
            .iter()
            .filter(|row| row.session_id.as_deref() == Some(&source_session_id))
            .cloned()
            .collect();
        // This routing draft grants no authority. The worker rereads the exact
        // saved snapshot and rebuilds the complete config before preparation.
        let mut config =
            gwt_agent::AgentLaunchBuilder::new(expected_source.agent_id.clone()).build();
        config.working_dir = Some(expected_source.worktree_path.clone());
        config.branch = Some(expected_source.branch.clone());
        config.linked_issue_number = expected_source.linked_issue_number;
        config.predecessor_session_id = Some(source_session_id);
        config.automatic_restore = origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore;
        let metadata = Box::new(super::launch_payload::OwnerLaunchMetadata::default());
        let restore = super::installation::OwnerSavedSessionRestore {
            request,
            source: *expected_source,
            origin,
            registry,
            placeholders,
            worktree_reserved: reserved,
            registry_after: None,
            launch_started: false,
            member_reservation: None,
            refusal: None,
        };
        self.launch_agent_inner(
            OwnerCommand::LaunchAgent {
                id,
                operation_id,
                tab_id: self.project_key.clone(),
                config: Box::new(config),
                metadata,
                cols,
                rows,
            },
            reply,
            None,
            Some(restore),
        );
    }

    fn launch_configured_shell(
        &mut self,
        command: OwnerCommand,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let request = match serde_json::to_value(&command) {
            Ok(request) => request,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let OwnerCommand::LaunchConfiguredShell {
            id,
            operation_id,
            mut config,
            cols,
            rows,
        } = command
        else {
            unreachable!()
        };
        if operation_id.trim().is_empty()
            || !id
                .strip_prefix(&format!("{}::", self.project_key))
                .is_some_and(|raw| !raw.is_empty())
        {
            let _ = reply.send(Err(
                "shell launch requires a fixed-project pane and operation identity".into(),
            ));
            return;
        }
        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
            if operation.request.is_null() && operation.cancellation_cleaned && operation.id == id {
                let _ = reply.send(Err("launch cancelled before owner admission".into()));
            } else if operation.request != request {
                let _ = reply.send(Err(
                    "operation identity is already bound to a different launch".into(),
                ));
            } else if let Some(result) = &operation.result {
                let _ = reply.send(result.clone());
            } else {
                operation.replies.push(reply);
            }
            return;
        }
        if self.runtimes.contains_key(&id)
            || self.pending.contains_key(&id)
            || self.closing.contains_key(&id)
            || self
                .retirements
                .keys()
                .any(|(retired_id, _)| retired_id == &id)
        {
            let _ = reply.send(Err("pane already exists".into()));
            return;
        }
        self.next_incarnation += 1;
        let incarnation = self.next_incarnation;
        let current = Arc::new(AtomicBool::new(true));
        self.pending.insert(id.clone(), incarnation);
        self.agent_operations.insert(
            operation_id.clone(),
            AgentOperation {
                session_id: None,
                early_events: VecDeque::new(),
                request,
                id: id.clone(),
                incarnation,
                current: current.clone(),
                replies: vec![reply],
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: None,
            },
        );
        let live_session_ids = self
            .runtimes
            .values()
            .filter_map(|runtime| runtime.agent.as_ref())
            .map(|agent| agent.active.session_id.clone())
            .collect();
        let root = self.root.clone();
        let project_key = self.project_key.clone();
        let sender = self.sender.clone();
        let events = self.events.clone();
        spawn_owner_worker(move || {
            let prepared = (|| {
                if let Some(path) = config.working_dir.as_deref().filter(|path| path.exists()) {
                    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
                        path,
                        gwt_core::daemon::RuntimeTarget::Host,
                    )
                    .map_err(|error| error.to_string())?;
                    if scope.repo_hash != project_key {
                        return Err("shell worktree belongs to another project".into());
                    }
                }
                super::installation::prepare_owner_shell_launch(
                    &root,
                    &id,
                    &mut config,
                    &live_session_ids,
                    &current,
                    |message| {
                        let _ = events.send(OwnerEvent::LaunchProgress {
                            operation_id: operation_id.clone(),
                            id: id.clone(),
                            incarnation,
                            message: message.into(),
                        });
                    },
                )
            })();
            let _ = events.send(OwnerEvent::Invalidated);
            let mut initial_prompt_file = None;
            let pane = prepared.and_then(|launch| {
                if !current.load(Ordering::Acquire) {
                    return Err("shell launch cancelled before PTY spawn".into());
                }
                initial_prompt_file = launch.initial_prompt_file;
                let pane = super::spawn_unbound_pane(
                    &id,
                    SpawnConfig {
                        command: launch.command,
                        args: launch.args,
                        cols,
                        rows,
                        env: launch.env,
                        remove_env: launch.remove_env,
                        cwd: launch.cwd,
                    },
                    None,
                    incarnation,
                    None,
                    false,
                )?;
                Ok(pane)
            });
            if let Err(error) = sender.send(OwnerMessage::Prepared {
                id,
                incarnation,
                pane: pane.map(Box::new),
                reply: None,
                operation_id: Some(operation_id),
                initial_prompt_file,
            }) {
                if let OwnerMessage::Prepared {
                    id, pane: Ok(pane), ..
                } = error.0
                {
                    let _ = crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &id);
                }
            }
        });
    }

    fn check_launch_permission(
        &mut self,
        command: OwnerCommand,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let request = match serde_json::to_value(&command) {
            Ok(request) => request,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let OwnerCommand::CheckLaunchPermission {
            operation_id,
            config,
        } = command
        else {
            unreachable!()
        };
        if uuid::Uuid::parse_str(&operation_id).is_err() {
            let _ = reply.send(Err("permission audit operation must be a UUID".into()));
            return;
        }
        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
            if operation.request != request {
                let _ = reply.send(Err("permission audit operation changed".into()));
            } else if let Some(result) = operation.result.as_ref() {
                let _ = reply.send(result.clone());
            } else {
                operation.replies.push(reply);
            }
            return;
        }
        let current = Arc::new(AtomicBool::new(true));
        self.agent_operations.insert(
            operation_id.clone(),
            AgentOperation {
                session_id: None,
                early_events: VecDeque::new(),
                request,
                id: format!("permission-check:{operation_id}"),
                incarnation: 0,
                current: current.clone(),
                replies: vec![reply],
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: None,
            },
        );
        let root = self.root.clone();
        let project_key = self.project_key.clone();
        let host_instance_id = self.host_instance_id.clone();
        let sender = self.sender.clone();
        self.input_jobs += 1;
        spawn_owner_worker(move || {
            let result = (|| {
                let config_hash = super::events::OwnerLaunchPermissionCheck::config_hash(&config)?;
                let record = super::launch::check_launch_permission(
                    &root,
                    &project_key,
                    &operation_id,
                    &config,
                    &current,
                )?;
                serde_json::to_value(super::events::OwnerLaunchPermissionCheck {
                    operation_id: operation_id.clone(),
                    project_key,
                    host_instance_id,
                    config_hash,
                    blocked: record.is_some(),
                    record,
                })
                .map_err(|error| error.to_string())
            })();
            let _ = sender.send(OwnerMessage::PermissionCheckFinished {
                operation_id,
                result,
            });
        });
    }

    fn reject_monitor_launch(&mut self, command: OwnerCommand, reply: oneshot::Sender<OwnerReply>) {
        let request = match serde_json::to_value(&command) {
            Ok(request) => request,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let OwnerCommand::RejectMonitorLaunch { rejection } = command else {
            unreachable!()
        };
        let operation_id = rejection.operation_id.clone();
        let issue = rejection
            .expected_delivery
            .as_deref()
            .map(|delivery| delivery.issue_number)
            .or_else(|| {
                rejection
                    .expected_handoff
                    .as_deref()
                    .map(|handoff| handoff.issue_number)
            });
        if uuid::Uuid::parse_str(&operation_id).is_err()
            || issue.is_none_or(|issue| issue == 0)
            || rejection.expected_delivery.is_none()
                && (rejection.expected_handoff.is_none()
                    || rejection.expected_session.is_none()
                    || rejection.expected_profile_readset.is_none())
        {
            let _ = reply.send(Err("Monitor rejection identity is invalid".into()));
            return;
        }
        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
            if operation.request != request {
                let _ = reply.send(Err("Monitor rejection operation changed".into()));
            } else if let Some(result) = operation.result.as_ref() {
                let _ = reply.send(result.clone());
            } else {
                operation.replies.push(reply);
            }
            return;
        }
        let issue = issue.unwrap();
        let preparing = self.agent_operations.values().any(|operation| {
            operation.result.is_none()
                && [
                    operation
                        .request
                        .pointer("/config/linked_issue_number")
                        .and_then(serde_json::Value::as_u64),
                    operation
                        .request
                        .pointer("/rejection/expected_delivery/issue_number")
                        .and_then(serde_json::Value::as_u64),
                    operation
                        .request
                        .pointer("/rejection/expected_handoff/issue_number")
                        .and_then(serde_json::Value::as_u64),
                    operation
                        .request
                        .pointer("/expected_source/linked_issue_number")
                        .and_then(serde_json::Value::as_u64),
                ]
                .contains(&Some(issue))
        });
        if self.stopping
            || !self.closing.is_empty()
            || !self.pending.is_empty()
            || self.retiring != 0
            || !self.retirements.is_empty()
            || preparing
            || self.runtimes.values().any(|runtime| {
                runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.snapshot.linked_issue_number == Some(issue))
            })
        {
            let _ = reply.send(Err(
                "Monitor rejection has a live, preparing or closing owner".into(),
            ));
            return;
        }
        let id = format!("{}::preflight-reject-{operation_id}", self.project_key);
        let current = Arc::new(AtomicBool::new(true));
        self.agent_operations.insert(
            operation_id.clone(),
            AgentOperation {
                session_id: None,
                early_events: VecDeque::new(),
                request,
                id: id.clone(),
                incarnation: 0,
                current: current.clone(),
                replies: vec![reply],
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: None,
            },
        );
        let snapshot = self.owner_monitor_window_snapshot(&id);
        let root = self.root.clone();
        let materializer_id = self.host_instance_id.clone();
        let sender = self.sender.clone();
        #[cfg(any(test, feature = "test-gh-guard"))]
        let checkpoint = self
            .next_launch_checkpoint
            .take()
            .map(|checkpoint| (checkpoint, self.issuer.clone()));
        self.input_jobs += 1;
        spawn_owner_worker(move || {
            let result = super::monitor::reject_preflight_launch(
                &root,
                &id,
                &materializer_id,
                current,
                &rejection,
                snapshot,
                #[cfg(any(test, feature = "test-gh-guard"))]
                checkpoint,
            );
            let _ = sender.send(OwnerMessage::MonitorRejectionFinished {
                operation_id,
                result,
            });
        });
    }

    fn launch_agent(&mut self, command: OwnerCommand, reply: oneshot::Sender<OwnerReply>) {
        self.launch_agent_inner(command, reply, None, None);
    }
    fn launch_agent_inner(
        &mut self,
        command: OwnerCommand,
        reply: oneshot::Sender<OwnerReply>,
        continuation: Option<super::launch_payload::PendingContinueWork>,
        mut restore: Option<super::installation::OwnerSavedSessionRestore>,
    ) {
        if let Err(error) = &self.agent_registry {
            let _ = reply.send(Err(error.clone()));
            return;
        }
        let request = match restore
            .as_ref()
            .map(|restore| Ok(restore.request.clone()))
            .unwrap_or_else(|| serde_json::to_value(&command))
        {
            Ok(request) => request,
            Err(error) => {
                let _ = reply.send(Err(error.to_string()));
                return;
            }
        };
        let OwnerCommand::LaunchAgent {
            id,
            operation_id,
            tab_id,
            mut config,
            mut metadata,
            cols,
            rows,
        } = command
        else {
            unreachable!()
        };
        if config
            .working_dir
            .as_deref()
            .is_some_and(crate::pm_registry::is_canonical_pm_worktree)
            && (self.closing.values().any(|closing| closing.pm_worktree)
                || self
                    .retirements
                    .values()
                    .flatten()
                    .any(|retirement| match retirement {
                        Retirement::Runtime { runtime, .. } => runtime_uses_pm_worktree(runtime),
                        Retirement::Agent { success, .. } => {
                            crate::pm_registry::is_canonical_pm_worktree(
                                &success.active_session.worktree_path,
                            )
                        }
                        _ => false,
                    }))
        {
            let _ = reply.send(Err(
                "the canonical PM worktree is still closing or requires reconciliation".into(),
            ));
            return;
        }
        if continuation.is_none() {
            config.execution_intent = metadata
                .manual_successor
                .clone()
                .map(|choice| choice.into_intent())
                .unwrap_or_default();
        }
        if metadata
            .launch_feedback_context
            .as_ref()
            .and_then(|feedback| feedback.issue_monitor_project_root.as_deref())
            .is_some_and(|project_root| {
                gwt_core::paths::project_scope_hash(project_root).as_str() != self.project_key
            })
        {
            let _ = reply.send(Err("launch feedback belongs to another project".into()));
            return;
        }
        if id.trim().is_empty() || operation_id.trim().is_empty() {
            let _ = reply.send(Err("pane and operation identity must not be empty".into()));
            return;
        }
        let rejection_in_progress = self.agent_operations.values().any(|operation| {
            operation
                .pending_monitor_rejection_issue()
                .is_some_and(|issue| {
                    config.linked_issue_number == Some(issue)
                        || metadata
                            .launch_feedback_context
                            .as_ref()
                            .and_then(|feedback| {
                                feedback.issue_monitor_launch_binding_issue_number()
                            })
                            == Some(issue)
                })
        });
        let retry_stopped_predecessor = metadata.manual_successor.as_ref().and_then(|choice| {
            let operation = self.agent_operations.get(&operation_id)?;
            let stopped = operation.manual_predecessor_stopped.as_ref()?;
            (operation.id == id
                && operation.request == request
                && operation.result.as_ref().is_some_and(Result::is_err)
                && operation.retained_manual_failure.is_some()
                && operation.current.load(Ordering::Acquire)
                && choice.expected_predecessor.as_deref() == Some(stopped)
                && Self::manual_retry_preparation_is_exact(
                    &gwt_core::paths::gwt_sessions_dir(),
                    stopped,
                    &operation_id,
                )
                .is_ok()
                && !self.pending.contains_key(&id)
                && !self.runtimes.contains_key(&id)
                && !self.closing.contains_key(&id)
                && !self
                    .retirements
                    .keys()
                    .any(|(retired_id, _)| retired_id == &id))
            .then(|| stopped.clone())
        });
        if (continuation.is_some() || retry_stopped_predecessor.is_some())
            && self
                .agent_operations
                .get(&operation_id)
                .is_some_and(|operation| {
                    operation.result.as_ref().is_some_and(Result::is_err)
                        && !operation.request.is_null()
                })
            && !self
                .retirements
                .keys()
                .any(|(retired_id, _)| retired_id == &id)
        {
            if rejection_in_progress {
                let _ = reply.send(Err("an agent operation is in progress".into()));
                return;
            }
            self.agent_operations.remove(&operation_id);
        }
        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
            if operation.request.is_null() && operation.cancellation_cleaned && operation.id == id {
                let _ = reply.send(Err("launch cancelled before owner admission".into()));
            } else if operation.request != request {
                let _ = reply.send(Err(
                    "operation identity is already bound to a different launch".into(),
                ));
            } else if let Some(result) = &operation.result {
                let _ = reply.send(result.clone());
            } else {
                operation.replies.push(reply);
            }
            return;
        }
        if rejection_in_progress {
            let _ = reply.send(Err("an agent operation is in progress".into()));
            return;
        }
        let is_monitor = metadata
            .launch_feedback_context
            .as_ref()
            .is_some_and(|feedback| feedback.issue_monitor_issue_number.is_some());
        let is_review = config
            .env_vars
            .get("GWT_REVIEW_DISPATCH")
            .is_some_and(|value| value == "1");
        if continuation.is_none()
            && restore.is_none()
            && !is_monitor
            && !is_review
            && metadata.manual_successor.is_none()
        {
            let branch = config
                .branch
                .as_deref()
                .map(crate::launch_runtime::normalize_branch_name);
            let existing = self
                .runtimes
                .iter()
                .find_map(|(existing_id, runtime)| {
                    let agent = runtime.agent.as_ref()?;
                    (!self.closing.contains_key(existing_id)
                        && agent.snapshot.agent_id == config.agent_id
                        && !matches!(
                            Self::runtime_status(runtime),
                            crate::WindowProcessStatus::Stopped | crate::WindowProcessStatus::Error
                        )
                        && (branch.as_deref().is_some_and(|branch| {
                            crate::launch_runtime::normalize_branch_name(&agent.active.branch_name)
                                == branch
                        }) || config.working_dir.as_deref().is_some_and(|path| {
                            crate::launch_runtime::same_worktree_path(
                                path,
                                &agent.active.worktree_path,
                            )
                        })))
                    .then(|| (existing_id.clone(), runtime.incarnation))
                })
                .or_else(|| {
                    let key = super::installation::inflight_launch_key(&self.project_key, &config)?;
                    self.agent_operations
                        .values()
                        .filter(|operation| {
                            operation.result.is_none() && operation.current.load(Ordering::Acquire)
                        })
                        .find_map(|operation| {
                            let config = serde_json::from_value::<gwt_agent::LaunchConfig>(
                                operation.request.get("config")?.clone(),
                            )
                            .ok()?;
                            (super::installation::inflight_launch_key(&self.project_key, &config)
                                .as_ref()
                                == Some(&key))
                            .then(|| (operation.id.clone(), operation.incarnation))
                        })
                });
            if let Some((existing_id, incarnation)) = existing {
                let receipt = self
                    .pane_snapshot(&existing_id, incarnation)
                    .unwrap_or_else(
                        |_| json!({"id":existing_id,"incarnation":incarnation,"pending":true}),
                    );
                self.agent_operations.insert(
                    operation_id.clone(),
                    AgentOperation {
                        session_id: None,
                        early_events: VecDeque::new(),
                        request,
                        id,
                        incarnation,
                        current: Arc::new(AtomicBool::new(false)),
                        replies: Vec::new(),
                        cancel_replies: Vec::new(),
                        cancellation_cleaned: true,
                        manual_predecessor_stopped: None,
                        retained_manual_failure: None,
                        pending_continue: None,
                        failed_session: None,
                        result: Some(Ok(receipt.clone())),
                    },
                );
                self.focus_owner_pane(&existing_id);
                let _ = reply.send(Ok(receipt));
                return;
            }
        }
        if self.runtimes.contains_key(&id)
            || self.pending.contains_key(&id)
            || self.closing.contains_key(&id)
            || self
                .retirements
                .keys()
                .any(|(retired_id, _)| retired_id == &id)
        {
            let _ = reply.send(Err("pane already exists".into()));
            return;
        }
        if let Some(path) = config.working_dir.as_deref().filter(|path| path.exists()) {
            let scope = gwt_core::daemon::RuntimeScope::from_project_root(
                path,
                gwt_core::daemon::RuntimeTarget::Host,
            );
            if !scope.is_ok_and(|scope| scope.repo_hash == self.project_key)
                && !super::continuation::worktree_matches_project_state(path, &self.root)
            {
                let _ = reply.send(Err("launch worktree belongs to another project".into()));
                return;
            }
        }
        let manual_stop = if let Some(choice) =
            metadata.manual_successor.as_ref().filter(|choice| {
                retry_stopped_predecessor.is_none()
                    && choice.predecessor_kind
                        == gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive
            }) {
            let Some(expected) = choice.expected_predecessor.as_deref() else {
                let _ = reply.send(Err(
                    "Active manual successor requires an exact predecessor Session".into(),
                ));
                return;
            };
            let installed = self
                .runtimes
                .iter()
                .filter_map(|(id, runtime)| {
                    let agent = runtime.agent.as_ref()?;
                    (agent.active.session_id == expected.session_id
                        || agent.snapshot.id == expected.session_id)
                        .then(|| id.clone())
                })
                .collect::<Vec<_>>();
            if installed.len() > 1 {
                let _ = reply.send(Err(
                    "The exact holder has multiple installed runtimes".into()
                ));
                return;
            }
            if let Some(predecessor_id) = installed.into_iter().next() {
                let runtime = &self.runtimes[&predecessor_id];
                let agent = runtime.agent.as_ref().expect("installed predecessor");
                let exact = matches!(choice.expected_runtime,
                    Some(gwt_agent::ManualLaunchRuntimeEvidence::Proof(proof))
                        if proof.host_pid == std::process::id()
                            && proof.runtime_incarnation == runtime.incarnation)
                    && agent.active.session_id == expected.session_id
                    && !self.closing.contains_key(&predecessor_id)
                    && !self.readiness.contains_key(&predecessor_id)
                    && agent.pending_fresh.is_none()
                    && agent.pending_continue.is_none()
                    && gwt_agent::SessionExecutionIdentity::from_session(&agent.snapshot)
                        .ok()
                        .flatten()
                        .as_ref()
                        == Some(expected)
                    && agent.capability_token.is_some();
                if !exact {
                    let _ = reply.send(Err(
                        "The exact holder runtime or Session changed before termination".into(),
                    ));
                    return;
                }
                let runtime = self
                    .runtimes
                    .remove(&predecessor_id)
                    .expect("exact manual predecessor");
                self.retirements
                    .insert((predecessor_id.clone(), runtime.incarnation), None);
                self.retiring += 1;
                Some((predecessor_id, runtime, expected.clone()))
            } else {
                // An already terminal historical source has no local child to
                // stop. The existing worker must validate the full immutable
                // Session/binding and captured terminal/Absent proof under leases.
                None
            }
        } else {
            None
        };
        self.next_incarnation += 1;
        let incarnation = self.next_incarnation;
        let current = Arc::new(AtomicBool::new(true));
        self.pending.insert(id.clone(), incarnation);
        self.agent_operations.insert(
            operation_id.clone(),
            AgentOperation {
                session_id: None,
                early_events: VecDeque::new(),
                request,
                id: id.clone(),
                incarnation,
                current: current.clone(),
                replies: vec![reply],
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: retry_stopped_predecessor.clone(),
                retained_manual_failure: None,
                pending_continue: continuation.clone(),
                failed_session: None,
                result: None,
            },
        );
        let snapshot = self.owner_monitor_window_snapshot(&id);
        let snapshot_tabs: std::collections::BTreeSet<String> = snapshot
            .windows
            .iter()
            .filter_map(|window| {
                window
                    .window_id
                    .split_once("::")
                    .map(|(tab, _)| tab.to_string())
            })
            .chain(std::iter::once(self.project_key.clone()))
            .collect();
        let known_names: HashSet<_> = self
            .runtimes
            .values()
            .filter_map(|runtime| runtime.agent.as_ref()?.snapshot.agent_name.clone())
            .collect();
        let owner_registry = self.agent_registry.clone();
        let owner_registry_path = crate::persistence::workspace_state_path(&self.root);
        let cleanup_project_key = self.project_key.clone();
        let root = self.root.clone();
        let issuer = self.issuer.clone();
        let materializer_id = self.host_instance_id.clone();
        let sender = self.sender.clone();
        let events = self.events.clone();
        let live_pm_session_ids = self
            .runtimes
            .values()
            .filter_map(|runtime| {
                runtime
                    .agent
                    .as_ref()
                    .map(|agent| agent.active.session_id.clone())
            })
            .collect();
        #[cfg(any(test, feature = "test-gh-guard"))]
        let launch_checkpoint = self.next_launch_checkpoint.take();
        let linked_issue_number = config.linked_issue_number;
        #[cfg(any(test, feature = "test-gh-guard"))]
        let test_pty_start_gate = self.test_pty_start_gate.clone();
        spawn_owner_worker(move || {
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                gwt_core::operation_deadline::now() + std::time::Duration::from_secs(120),
            );
            let sessions_dir = gwt_core::paths::gwt_sessions_dir();
            let expected_registry_row: Result<
                Option<crate::persistence::PersistedWindowState>,
                String,
            > = (|| {
                let registry = owner_registry.as_ref().map_err(Clone::clone)?;
                let raw = id
                    .strip_prefix(&format!(
                        "{}::",
                        gwt_core::paths::resolve_project_scope(&root).hash.as_str()
                    ))
                    .filter(|raw| !raw.is_empty())
                    .ok_or("agent pane is outside fixed project identity")?;
                let registry = registry.lock().map_err(|error| error.to_string())?;
                let row = registry.window(raw).cloned();
                let source = config.predecessor_session_id.as_deref().or_else(|| {
                    metadata
                        .manual_successor
                        .as_ref()
                        .and_then(|choice| choice.expected_predecessor.as_ref())
                        .map(|identity| identity.session_id.as_str())
                });
                if row
                    .as_ref()
                    .and_then(|row| row.session_id.as_deref())
                    .is_some_and(|session| Some(session) != source)
                {
                    return Err(
                        "existing window Session does not match the typed launch predecessor"
                            .into(),
                    );
                }
                Ok(row)
            })();

            let restored_monitor_launch = restore.as_ref().and_then(|restore| {
                let row = expected_registry_row.as_ref().ok()?;
                match super::monitor::capture_restored_launch(
                    &root,
                    &id,
                    incarnation,
                    &materializer_id,
                    current.clone(),
                    &restore.source,
                    row.as_ref(),
                ) {
                    Ok(captured) => captured,
                    Err(error) => {
                        tracing::warn!(%error, "restored Monitor source capture remains unknown");
                        None
                    }
                }
            });

            let had_manual_predecessor = manual_stop.is_some();
            let manual_stop_result = if let Some((predecessor_id, mut runtime, identity)) =
                manual_stop
            {
                let predecessor_incarnation = runtime.incarnation;
                let result = (|| {
                    let agent = runtime.agent.as_ref().expect("captured manual predecessor");
                    crate::session_finalizer::stop_exact_runtime(
                        &sessions_dir,
                        &identity,
                        predecessor_incarnation,
                        &runtime.pty,
                        &predecessor_id,
                        agent
                            .capability_token
                            .as_ref()
                            .map(|token| (agent.issuer.clone(), token.clone())),
                        true,
                    )?;
                    if !crate::session_finalizer::kill_and_reap(&runtime.pty, &predecessor_id)
                        .map_err(|error| error.to_string())?
                    {
                        return Err("The exact holder child has not exited".to_string());
                    }
                    runtime.pty.release_descriptors();
                    if let Some(worker) = runtime.output_thread.take() {
                        worker
                            .join()
                            .map_err(|_| "holder output worker panicked".to_string())?;
                    }
                    if let Some(worker) = runtime.status_thread.take() {
                        worker
                            .join()
                            .map_err(|_| "holder status worker panicked".to_string())?;
                    }
                    if !crate::session_finalizer::persist_exact_terminal_after_exit(
                        &runtime.pty,
                        &sessions_dir,
                        &identity,
                        predecessor_incarnation,
                        &predecessor_id,
                    )
                    .map_err(|error| error.to_string())?
                    {
                        return Err("The exact holder terminal proof changed before preparation"
                            .to_string());
                    }
                    if let Some((snapshot, reservation)) = runtime.member_cleanup.as_ref() {
                        crate::agent_names::release_reaped_project_member(
                            &root,
                            &sessions_dir,
                            snapshot,
                            reservation,
                            &runtime.pty,
                        )?;
                        runtime.member_cleanup = None;
                    }
                    Ok(())
                })();
                let (ack, accepted) = mpsc::channel();
                let sent = sender.send(OwnerMessage::ManualPredecessorStopped {
                    operation_id: operation_id.clone(),
                    launch_incarnation: incarnation,
                    predecessor: identity,
                    id: predecessor_id,
                    incarnation: predecessor_incarnation,
                    runtime: Box::new(runtime),
                    result: result.clone(),
                    ack,
                });
                if sent.is_ok() {
                    let _ = accepted.recv();
                    result
                } else {
                    Err("owner stopped before manual holder retirement".into())
                }
            } else {
                Ok(())
            };
            #[cfg(any(test, feature = "test-gh-guard"))]
            if let Some(checkpoint) = launch_checkpoint.as_ref().filter(|checkpoint| {
                had_manual_predecessor
                    && manual_stop_result.is_ok()
                    && checkpoint.stage
                        == super::test_support::OwnerLaunchStage::ManualPredecessorStoppedBeforePreparation
            }) {
                let predecessor = metadata
                    .manual_successor
                    .as_ref()
                    .and_then(|choice| choice.expected_predecessor.as_deref());
                checkpoint.pause(super::test_support::OwnerLaunchEvidence {
                    id: id.clone(),
                    incarnation,
                    operation_id: operation_id.clone(),
                    session_id: predecessor.map(|identity| identity.session_id.clone()),
                    current: current.clone(),
                    session: predecessor.and_then(|identity| {
                        gwt_agent::Session::load(
                            &sessions_dir.join(format!("{}.toml", identity.session_id)),
                        )
                        .ok()
                    }),
                    pty: None,
                    issuer: issuer.clone(),
                    grant: None,
                    pending_continue_work: None,
                    pending_fresh_execution: None,
                });
            }
            let retained_manual_predecessor = metadata
                .manual_successor
                .as_ref()
                .filter(|_| {
                    manual_stop_result.is_ok()
                        && (had_manual_predecessor || retry_stopped_predecessor.is_some())
                })
                .and_then(|choice| choice.expected_predecessor.as_deref())
                .cloned();
            let is_restart = continuation.is_none()
                && metadata.inherited_agent_name.is_some()
                && config.predecessor_session_id.is_some();
            let mut input = InstallAgentLaunchInput {
                auto_resume_source_snapshot: restore.as_ref().map(|restore| restore.source.clone()),
                auto_resume_source_session_id: config
                    .predecessor_session_id
                    .clone()
                    .filter(|_| config.automatic_restore || is_restart),
                current,
                failure_session: None,
                physical_spawn_started: false,
                failed_launch: None,
                had_window: true,
                is_continue_work: continuation.is_some(),
                issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
                launch_feedback_context: None,
                live_pm_session_ids,
                materializer_id,
                origin: if let Some(restore) = restore.as_ref() {
                    restore.origin
                } else if config.automatic_restore {
                    gwt_agent::SessionLaunchOrigin::AutomaticRestore
                } else if is_restart {
                    gwt_agent::SessionLaunchOrigin::UserRestart
                } else {
                    gwt_agent::SessionLaunchOrigin::Launch
                },
                pending_continue_work: continuation,
                pending_fresh_execution: None,
                pending_pm_project_root: None,
                project_root: root.clone(),
                removed_genesis_session: None,
                sessions_dir: sessions_dir.clone(),
                stage_id: incarnation,
                tab_id,
                window_id: id.clone(),
                workspace_resume_context: None,
                incarnation,
            };
            let output_id = id.clone();
            let output_operation_id = operation_id.clone();
            let mut purpose_title = None;
            let result = manual_stop_result
                .map_err(AgentLaunchError::from)
                .and_then(|()| {
                    if let Some(restore) = restore.as_mut() {
                        super::installation::prepare_owner_saved_session_restore(
                            &root,
                            &sessions_dir,
                            restore,
                            &mut config,
                            &mut metadata,
                            &input.current,
                        )
                        .map_err(AgentLaunchError::from)?;
                        input.auto_resume_source_session_id = Some(restore.source.id.clone());
                        input.auto_resume_source_snapshot = Some(restore.source.clone());
                    }
                    expected_registry_row
                        .as_ref()
                        .map_err(|error| AgentLaunchError::from(error.clone()))?;
                    super::continuation::prepare_manual_successor_before_pane(
                        &root,
                        &sessions_dir,
                        Some(&issuer),
                        &mut config,
                        match retry_stopped_predecessor.as_ref() {
                            Some(stopped) => Self::manual_retry_preparation_is_exact(
                                &sessions_dir,
                                stopped,
                                &operation_id,
                            )
                            .map_err(AgentLaunchError::from)?,
                            None => false,
                        },
                    )
                    .map_err(AgentLaunchError::from)?;
                    let host_project_roots = super::installation::owner_host_project_roots(&root)
                        .map_err(AgentLaunchError::from)?;
                    let mut admitted = super::installation::prepare_owner_agent_admission(
                        super::installation::OwnerAgentAdmissionInput {
                            project_root: &root,
                            sessions_dir: &sessions_dir,
                            issue_link_cache_dir: &input.issue_link_cache_dir,
                            window_id: &id,
                            materializer_id: &input.materializer_id,
                            host_project_roots: &host_project_roots,
                            snapshot,
                            snapshot_tabs,
                            known_names: &known_names,
                            metadata: *metadata,
                            is_continue_work: input.is_continue_work,
                            restore_source: input.auto_resume_source_snapshot.as_ref(),
                        },
                        &mut config,
                    )
                    .map_err(AgentLaunchError::from)?;
                    input.workspace_resume_context = admitted.workspace_resume_context.take();
                    input.launch_feedback_context = admitted.launch_feedback_context.take();
                    input.pending_pm_project_root = config
                        .working_dir
                        .as_deref()
                        .filter(|path| crate::pm_registry::is_canonical_pm_worktree(path))
                        .map(|_| root.clone());
                    purpose_title = admitted.purpose_title.take();
                    if let Some(restore) = restore.as_mut() {
                        restore.launch_started = true;
                    }
                    #[cfg(any(test, feature = "test-gh-guard"))]
                    if let Some(checkpoint) = launch_checkpoint.as_ref().filter(|checkpoint| {
                        checkpoint.stage == super::test_support::OwnerLaunchStage::FinalizedBeforeRuntimePublication
                    }).cloned() {
                        let id = id.clone();
                        let operation_id = operation_id.clone();
                        let current = input.current.clone();
                        let issuer = issuer.clone();
                        crate::session_launch::set_finalized_session_post_save_hook_for_test(Box::new(move |session| {
                            checkpoint.pause(super::test_support::OwnerLaunchEvidence {
                                id, incarnation, operation_id, current,
                                session_id: Some(session.id.clone()), session: Some(session.clone()),
                                pty: None, issuer, grant: None,
                                pending_continue_work: None, pending_fresh_execution: None,
                            });
                            Ok(())
                        }));
                    }
                    super::launch::prepare_agent_launch(
                        super::launch::AgentLaunchInput {
                            sessions_dir: sessions_dir.clone(),
                            project_root: root.to_string_lossy().into_owned(),
                            window_id: id.clone(),
                            config: *config,
                            profile_config_path: gwt_core::paths::gwt_config_path(),
                            agent_capability_issuer: Some(issuer.clone()),
                            prepared_manual_launch_claim: admitted.prepared_manual_launch_claim,
                            issue_monitor_trust_candidate: admitted.issue_monitor_trust_candidate,
                            issue_monitor_launch: admitted.issue_monitor_launch,
                        },
                        move |event| {
                            let event = match event {
                                super::launch::AgentLaunchEvent::Progress { message } => {
                                    OwnerEvent::LaunchProgress {
                                        operation_id: output_operation_id.clone(),
                                        id: output_id.clone(),
                                        incarnation,
                                        message,
                                    }
                                }
                                super::launch::AgentLaunchEvent::TerminalOutput { data } => {
                                    OwnerEvent::PreparationOutput {
                                        operation_id: output_operation_id.clone(),
                                        id: output_id.clone(),
                                        incarnation,
                                        data,
                                    }
                                }
                                super::launch::AgentLaunchEvent::Toast {
                                    level,
                                    message,
                                    issue_number,
                                } => OwnerEvent::Notice {
                                    level,
                                    message,
                                    issue_number,
                                },
                            };
                            let _ = events.send(event);
                        },
                    )
                });
            let mut materialized_snapshot = result.as_ref().ok().and_then(|launch| {
                gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", launch.1))).ok()
            });
            if let Ok(launch) = &result {
                let (ack, registered) = mpsc::channel();
                if sender
                    .send(OwnerMessage::AgentMaterialized {
                        operation_id: operation_id.clone(),
                        id: id.clone(),
                        incarnation,
                        session_id: launch.1.clone(),
                        ack,
                    })
                    .is_ok()
                {
                    let _ = registered.recv();
                } else {
                    input.current.store(false, Ordering::Release);
                }
            }
            let token = result.as_ref().ok().and_then(|launch| {
                launch
                    .0
                    .env
                    .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
                    .cloned()
            });
            // Unbound preparation publishes a provisional sidecar before a
            // physical child exists. Capture it before installation so failure
            // can retire only this exact pre-child record under its writer lease.
            let provisional_runtime = materialized_snapshot.as_ref().map(|snapshot| {
                let path = gwt_agent::runtime_state_path(&sessions_dir, &snapshot.id);
                match std::fs::read(path) {
                    Ok(bytes) => Ok(Some(bytes)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error.to_string()),
                }
            });
            #[cfg(any(test, feature = "test-gh-guard"))]
            if let Some(checkpoint) = launch_checkpoint.as_ref().filter(|checkpoint| {
                checkpoint.stage == super::test_support::OwnerLaunchStage::MaterializedBeforeInstall
            }) {
                checkpoint.pause(super::test_support::OwnerLaunchEvidence {
                    id: id.clone(),
                    incarnation,
                    operation_id: operation_id.clone(),
                    session_id: materialized_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.id.clone()),
                    current: input.current.clone(),
                    session: materialized_snapshot.clone(),
                    pty: None,
                    issuer: issuer.clone(),
                    pending_continue_work: input.pending_continue_work.clone(),
                    pending_fresh_execution: input.pending_fresh_execution.clone().or_else(|| {
                        let launch = result.as_ref().ok()?;
                        let nonce = launch
                            .0
                            .env
                            .get(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV)?;
                        super::launch_finalization::pending_fresh_execution_launch_from_session(
                            &sessions_dir,
                            &launch.1,
                            &launch.4,
                            &launch.11.agent_project_root,
                            launch.6,
                            launch.7.clone(),
                            input.workspace_resume_context.clone(),
                            input.launch_feedback_context.clone(),
                            nonce,
                            &launch.5,
                        )
                        .ok()
                    }),
                    grant: token
                        .as_deref()
                        .and_then(|token| issuer.grant_for_test(token)),
                });
            }
            let mut retirement_reason = None;
            let mut registry_cleanup = None;
            let result = result
                .and_then(|launch| {
                    super::installation::install_agent_launch(&mut input, launch, || {
                        #[cfg(any(test, feature = "test-gh-guard"))]
                        if let Some(gate) = test_pty_start_gate.as_ref() {
                            return Ok((gate.program.clone(), gate.args.clone()));
                        }
                        let executable =
                            std::env::current_exe().map_err(|error| error.to_string())?;
                        let gate =
                            crate::pty_start_gate::resolve_pty_start_gate_program(&executable)?;
                        Ok((
                            gate,
                            vec![crate::pty_start_gate::PTY_START_GATE_ARG.to_string()],
                        ))
                    })
                    .map_err(Into::into)
                })
                .map(|mut success| {
                    if let Err(error) = success.pane.resize(cols, rows) {
                        retirement_reason = Some(format!("agent pane resize failed: {error}"));
                    }
                    if let (Ok(registry), Ok(expected), Some(snapshot)) = (
                        &owner_registry,
                        &expected_registry_row,
                        success.session_snapshot.as_ref(),
                    ) {
                        match super::agent_dispatch::owned_process_window_state(
                            &root,
                            &id,
                            Some(snapshot),
                            Some(&success.active_session),
                            purpose_title.clone(),
                            expected.as_ref(),
                        ) {
                            Ok(row) => {
                                let cleanup = super::agent_dispatch::OwnedWindowRecordCleanup {
                                    registry: registry.clone(),
                                    registry_path: owner_registry_path.clone(),
                                    root: root.clone(),
                                    project_key: cleanup_project_key.clone(),
                                    canonical_id: id.clone(),
                                    expected_session_id: Some(snapshot.id.clone()),
                                };
                                match super::agent_dispatch::publish_owned_process_window(
                                    registry,
                                    &root,
                                    &id,
                                    expected.as_ref(),
                                    row,
                                    &input.current,
                                ) {
                                    Ok(()) => registry_cleanup = Some(cleanup),
                                    Err(error) => {
                                        if error.record_changed {
                                            registry_cleanup = Some(cleanup);
                                        }
                                        retirement_reason = Some(format!(
                                            "owner registry publication failed: {}",
                                            error.detail
                                        ));
                                    }
                                }
                            }
                            Err(error) => retirement_reason = Some(error),
                        }
                    }
                    success
                });
            let mut unspawned_runtime_retired = false;
            if result.is_err()
                && !input.physical_spawn_started
                && input.failed_launch.is_none()
                && input.current.load(Ordering::Acquire)
                && input.pending_fresh_execution.is_some()
                && retained_manual_predecessor.is_some()
            {
                if let Some((active, snapshot)) = input.failure_session.clone() {
                    match retire_unspawned_launch_runtime(
                        &sessions_dir,
                        &snapshot,
                        provisional_runtime.as_ref(),
                    ) {
                        Ok(stopped) => {
                            materialized_snapshot = Some(stopped.clone());
                            input.failure_session = Some((active, stopped));
                            unspawned_runtime_retired = true;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "manual no-child provisional runtime remains unconfirmed")
                        }
                    }
                }
            }
            let mut durable_candidate_removed = input.removed_genesis_session.is_some();
            if result.is_err() {
                if let Some(pending) = input.pending_continue_work.as_ref() {
                    match super::launch_finalization::rollback_worker_continue_work(
                        &sessions_dir,
                        pending,
                    ) {
                        Ok(true) => durable_candidate_removed = true,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed owner continuation retained exact recovery")
                        }
                    }
                }

                let preserve = input
                    .pending_fresh_execution
                    .as_ref()
                    .is_some_and(|pending| {
                        super::launch_finalization::pending_fresh_execution_activation_status(
                            pending,
                        ) != Some(false)
                    });
                let retain_manual_preparation = !input.physical_spawn_started
                    && input.failed_launch.is_none()
                    && input.current.load(Ordering::Acquire)
                    && retained_manual_predecessor.as_ref().is_some_and(|stopped| {
                        Self::manual_retry_preparation_is_exact(
                            &sessions_dir,
                            stopped,
                            &operation_id,
                        )
                        .is_ok()
                    });
                if let Some(pending) = input
                    .pending_fresh_execution
                    .as_ref()
                    .filter(|_| !retain_manual_preparation)
                {
                    match super::launch_finalization::rollback_worker_fresh_execution(
                        &sessions_dir,
                        pending,
                    ) {
                        Ok(true) => durable_candidate_removed = true,
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed agent launch retained fresh execution recovery")
                        }
                    }
                }
                if !preserve {
                    if let Some(token) = token {
                        issuer.revoke_token(&token);
                    }
                }
            }
            if result.is_err()
                && input.failed_launch.is_none()
                && input.pending_continue_work.is_none()
                && input.pending_fresh_execution.is_none()
            {
                if let Some((active, snapshot)) = input.failure_session.clone() {
                    match retire_unspawned_launch_runtime(
                        &sessions_dir,
                        &snapshot,
                        provisional_runtime.as_ref(),
                    ) {
                        Ok(stopped) => {
                            materialized_snapshot = Some(stopped.clone());
                            input.failure_session = Some((active, stopped));
                            unspawned_runtime_retired = true;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed launch provisional runtime remains unconfirmed")
                        }
                    }
                }
            }
            let mut restore_cleanup_complete = true;
            if let (Err(error), Some(restore)) = (&result, restore.as_mut()) {
                if input.failed_launch.is_none() {
                    if let Err(cleanup) = super::installation::finish_owner_restore_failure(
                        &root,
                        &sessions_dir,
                        restore,
                        &error.detail,
                    ) {
                        restore_cleanup_complete = false;
                        restore.refuse(
                            "restore_reconciliation_required",
                            format!(
                                "{}; cleanup requires reconciliation: {cleanup}",
                                error.detail
                            ),
                            true,
                        );
                    }
                }
            }
            if result.is_err()
                && input.failed_launch.is_none()
                && input.origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore
            {
                if let Some((_, snapshot)) = input.failure_session.as_ref() {
                    match crate::session_finalizer::remove_restored_stopped_view(
                        &sessions_dir,
                        snapshot,
                    ) {
                        Ok(true) => {}
                        outcome => {
                            restore_cleanup_complete = false;
                            if let Some(restore) = restore.as_mut() {
                                restore.refuse("restore_reconciliation_required",
                                    format!("Failed restore candidate cleanup remains unconfirmed: {outcome:?}"), true);
                            }
                        }
                    }
                }
            }
            let compensated_cleanup = !input.physical_spawn_started
                && input.failed_launch.is_none()
                && input
                    .removed_genesis_session
                    .as_ref()
                    .is_some_and(|removed| {
                        matches!(
                            gwt_agent::inspect_session_path(
                                &sessions_dir.join(format!("{}.toml", removed.session_id)),
                            ),
                            gwt_agent::SessionPathState::Missing
                        ) && crate::session_launch::durable_launch_recovery_path(
                            &sessions_dir,
                            &removed.session_id,
                        )
                        .is_ok_and(|path| {
                            std::fs::symlink_metadata(path)
                                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                        })
                    });
            let cancelled_cleanup = restore_cleanup_complete
                && (!input.current.load(Ordering::Acquire) || compensated_cleanup)
                && (!input.physical_spawn_started || unspawned_runtime_retired)
                && result.is_err()
                && materialized_snapshot.as_ref().is_none_or(|snapshot| {
                    match gwt_agent::inspect_session_path(
                        &sessions_dir.join(format!("{}.toml", snapshot.id)),
                    ) {
                        gwt_agent::SessionPathState::Missing => true,
                        gwt_agent::SessionPathState::Present(_)
                            if snapshot.execution_binding.is_none() =>
                        {
                            crate::session_finalizer::remove_restored_stopped_view(
                                &sessions_dir,
                                snapshot,
                            )
                            .unwrap_or(false)
                        }
                        _ => false,
                    }
                });
            let cancelled_cleanup = cancelled_cleanup
                || (restore_cleanup_complete
                    && restore
                        .as_ref()
                        .is_some_and(|restore| restore.refusal.is_some())
                    && result.is_err()
                    && input.failed_launch.is_none());
            if let Err(error) = &result {
                let source = restore
                    .as_ref()
                    .map(|restore| &restore.source)
                    .or(input.auto_resume_source_snapshot.as_ref())
                    .or_else(|| input.failure_session.as_ref().map(|(_, snapshot)| snapshot))
                    .or(materialized_snapshot.as_ref());
                let sanitized = super::launch::sanitize_launch_log_error(&error.detail);
                tracing::error!(target: "gwt::agent_launch", stage = "launch_complete",
                    window_id = %input.window_id, tab_id = %input.tab_id,
                    raw_window_id = input.window_id.rsplit_once("::").map_or(input.window_id.as_str(), |(_, raw)| raw),
                    session_id = source.map(|session| session.id.as_str()).unwrap_or("unknown"),
                    agent_id = source.map(|session| session.agent_id.command()).unwrap_or("unknown"),
                    branch = source.map(|session| session.branch.as_str()).unwrap_or("unknown"),
                    error = %sanitized, "window launch failed");
                crate::error_report::report_error_and_publish(
                    gwt_core::error_ledger::ErrorKind::LaunchFailure,
                    sanitized,
                    gwt_core::error_ledger::ErrorTarget {
                        window_id: Some(input.window_id.clone()),
                        session_id: source.map(|session| session.id.clone()),
                        project_root: Some(input.project_root.display().to_string()),
                        issue: source
                            .and_then(|session| session.linked_issue_number)
                            .or(linked_issue_number),
                    },
                );
            }
            let mut monitor_failure_committed = false;
            if let Err(error) = &result {
                let launch = restored_monitor_launch.or_else(|| {
                    let feedback = input.launch_feedback_context.as_ref().filter(|feedback| {
                        feedback.issue_monitor_delivery_id.is_some()
                            || feedback.issue_monitor_autonomous_handoff.is_some()
                    })?;
                    Some(super::monitor::MonitorLaunch {
                        root: root.clone(),
                        id: id.clone(),
                        incarnation,
                        session_id: input
                            .failure_session
                            .as_ref()
                            .map(|(_, session)| session.id.clone())
                            .unwrap_or_default(),
                        agent_id: input
                            .failure_session
                            .as_ref()
                            .map(|(active, _)| active.agent_id.clone())
                            .unwrap_or_default(),
                        materializer_id: input.materializer_id.clone(),
                        feedback: feedback.clone(),
                        current: input.current.clone(),
                        source: None,
                    })
                });
                if let Some(launch) = launch.filter(|_| {
                    input.failed_launch.is_none()
                        && (!input.physical_spawn_started
                            || unspawned_runtime_retired
                            || durable_candidate_removed)
                }) {
                    let no_delivery_target = input.failure_session.as_ref()
                        .filter(|_| launch.feedback.issue_monitor_delivery_id.is_none()
                            && launch.feedback.issue_monitor_autonomous_handoff.is_some())
                        .and_then(|(_, snapshot)| {
                            let issue = launch.feedback.issue_monitor_launch_binding_issue_number()?;
                            super::installation::autonomous_handoff_delivery_target_for_session(
                                snapshot, issue, &launch.id, None, &launch.materializer_id,
                            ).inspect_err(|error| {
                                tracing::warn!(%error, "unspawned answer target proof is unavailable");
                            }).ok()
                        });
                    match launch.commit_unspawned_failure(
                        &error.detail,
                        error.retryable,
                        no_delivery_target.as_ref(),
                    ) {
                        Ok(true) => {
                            let cleanup = (|| -> Result<(), String> {
                                if let Some((_, snapshot)) = input
                                    .failure_session
                                    .as_ref()
                                    .filter(|_| !durable_candidate_removed)
                                {
                                    if !crate::session_finalizer::remove_restored_stopped_view(
                                        &sessions_dir,
                                        snapshot,
                                    )
                                    .map_err(|error| error.to_string())?
                                    {
                                        return Err(
                                            "failed Monitor Session changed before exact cleanup"
                                                .into(),
                                        );
                                    }
                                }
                                if let (Ok(registry), Ok(expected)) =
                                    (&owner_registry, &expected_registry_row)
                                {
                                    super::agent_dispatch::OwnedWindowRecordCleanup {
                                        registry: registry.clone(),
                                        registry_path: owner_registry_path.clone(),
                                        root: root.clone(),
                                        project_key: cleanup_project_key.clone(),
                                        canonical_id: id.clone(),
                                        expected_session_id: expected
                                            .as_ref()
                                            .and_then(|row| row.session_id.clone()),
                                    }
                                    .finish()?;
                                }
                                if let Some(pending) = input.pending_fresh_execution.as_ref() {
                                    crate::session_launch::recovery::clear_durable_launch_recovery(
                                        &sessions_dir,
                                        &pending.binding.session_id,
                                    )
                                    .map_err(|error| error.to_string())?;
                                }
                                Ok(())
                            })();
                            match cleanup {
                                Ok(()) => monitor_failure_committed = true,
                                Err(error) => {
                                    tracing::warn!(%error, "failed Monitor admission cleanup remains unconfirmed")
                                }
                            }
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed Monitor admission remains unconfirmed")
                        }
                    }
                }
            }
            let launch_failure = match (&result, input.failure_session.clone()) {
                (Err(error), Some((active, snapshot)))
                    if input.origin != gwt_agent::SessionLaunchOrigin::AutomaticRestore
                        && input.failed_launch.is_none()
                        && input.pending_continue_work.is_none()
                        && input.pending_fresh_execution.is_none()
                        && unspawned_runtime_retired
                        && !monitor_failure_committed
                        && input.current.load(Ordering::Acquire) =>
                {
                    let publication = (|| -> Result<gwt_agent::Session, String> {
                        let registry = owner_registry.as_ref().map_err(Clone::clone)?;
                        let expected = expected_registry_row.as_ref().map_err(Clone::clone)?;
                        match gwt_agent::update_session_if_matches_with(
                            &sessions_dir, &snapshot,
                            |current, expected| Ok(serde_json::to_value(current)? == serde_json::to_value(expected)?),
                            |current| {
                                let mut row = super::agent_dispatch::owned_process_window_state(
                                    &root, &id, Some(current), Some(&active), purpose_title.clone(), expected.as_ref(),
                                ).map_err(std::io::Error::other)?;
                                row.status = crate::WindowProcessStatus::Error;
                                row.dynamic_title_detail = Some(super::launch::sanitize_launch_log_error(&error.detail));
                                super::agent_dispatch::publish_owned_process_window(
                                    registry, &root, &id, expected.as_ref(), row, &input.current,
                                ).map_err(|failure| std::io::Error::other(failure.detail))?;
                                Ok(current.clone())
                            },
                        ).map_err(|error| error.to_string())? {
                            gwt_agent::SessionSnapshotUpdateOutcome::Updated(snapshot) => Ok(snapshot),
                            _ => Err("failed manual launch Session changed before diagnostic publication".into()),
                        }
                    })();
                    match publication {
                        Ok(published) => {
                            let session_fingerprint =
                                super::events::launch_failure_session_fingerprint(&published)
                                    .expect(
                                    "a persisted Session has a serializable diagnostic fingerprint",
                                );
                            input.failure_session = Some((active, published));
                            Some(super::events::OwnerLaunchFailure {
                                id: id.clone(),
                                incarnation,
                                host_pid: std::process::id(),
                                host_instance_id: input.materializer_id.clone(),
                                session_id: snapshot.id.clone(),
                                session_fingerprint,
                                detail: error.detail.clone(),
                                error_code: error.error_code.into(),
                                retryable: error.retryable,
                            })
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed manual launch diagnostic remains unpublished");
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(restore) = restore.as_mut().filter(|restore| {
                restore
                    .refusal
                    .as_ref()
                    .is_some_and(|refusal| refusal.remove_placeholder)
            }) {
                restore.registry_after = Some(
                    restore
                        .registry
                        .lock()
                        .map(|registry| registry.persisted().clone())
                        .map_err(|error| error.to_string()),
                );
            }
            let completion = AgentPrepared {
                monitor_failure_committed,
                launch_failure,
                restore,
                retirement_reason,
                registry_cleanup,
                cancelled_cleanup,
                durable_candidate_removed,
                purpose_title,
                operation_id,
                input,
                result,
            };
            #[cfg(any(test, feature = "test-gh-guard"))]
            if let Some(checkpoint) = launch_checkpoint.filter(|checkpoint| {
                checkpoint.stage == super::test_support::OwnerLaunchStage::PreparedBeforeMailbox
            }) {
                let success = completion
                    .result
                    .as_ref()
                    .ok()
                    .or(completion.input.failed_launch.as_deref());
                checkpoint.pause(super::test_support::OwnerLaunchEvidence {
                    id: completion.input.window_id.clone(),
                    incarnation,
                    operation_id: completion.operation_id.clone(),
                    session_id: completion
                        .result
                        .as_ref()
                        .ok()
                        .map(|success| success.active_session.session_id.clone())
                        .or_else(|| {
                            completion
                                .input
                                .failed_launch
                                .as_ref()
                                .map(|success| success.active_session.session_id.clone())
                        }),
                    current: completion.input.current.clone(),
                    session: success.and_then(|success| {
                        success
                            .session_snapshot
                            .clone()
                            .or_else(|| success.cached_session.clone())
                    }),
                    pty: success.map(|success| success.pane.shared_pty()),
                    issuer: issuer.clone(),
                    pending_continue_work: completion.input.pending_continue_work.clone(),
                    pending_fresh_execution: completion.input.pending_fresh_execution.clone(),
                    grant: success
                        .and_then(|success| success.capability_token.as_deref())
                        .and_then(|token| issuer.grant_for_test(token)),
                });
            }
            if let Err(error) = sender.send(OwnerMessage::AgentPrepared(Box::new(completion))) {
                if let OwnerMessage::AgentPrepared(mut prepared) = error.0 {
                    let captured = match prepared.result {
                        Ok(success) => Some(Box::new(success)),
                        Err(_) => prepared.input.failed_launch.take(),
                    };
                    if let Some(success) = captured {
                        if let Err(error) = abandon_agent_launch(
                            &success,
                            &prepared.input.sessions_dir,
                            &issuer,
                            prepared.durable_candidate_removed,
                            prepared.registry_cleanup.as_ref(),
                        ) {
                            tracing::error!(%error, "lost owner launch completion retained exact recovery evidence");
                        }
                    }
                }
            }
        });
    }
    pub(crate) fn failed_launch_diagnostic_snapshot(
        &self,
        id: &str,
        session_id: &str,
    ) -> Option<(u64, gwt_agent::Session, Vec<u8>)> {
        let raw = id.strip_prefix(&format!("{}::", self.project_key))?;
        let row = self.registry_projection.windows.iter().find(|row| {
            row.id == raw
                && row.session_id.as_deref() == Some(session_id)
                && row.status == crate::WindowProcessStatus::Error
        })?;
        self.agent_operations.values().find_map(|operation| {
            let session = operation.failed_session.as_ref()?;
            if operation.id != id
                || session.id != session_id
                || !operation.current.load(Ordering::Acquire)
                || row.agent_id.as_deref() != Some(session.agent_id.command())
            {
                return None;
            }
            let receipt = operation.result.as_ref()?.as_ref().ok()?;
            let failure: super::events::OwnerLaunchFailure =
                serde_json::from_value(receipt.get("launch_failure")?.clone()).ok()?;
            if failure.id != id
                || failure.incarnation != operation.incarnation
                || failure.host_pid != std::process::id()
                || failure.host_instance_id != self.host_instance_id
                || failure.session_id != session.id
                || super::events::launch_failure_session_fingerprint(session)
                    .ok()
                    .as_deref()
                    != Some(failure.session_fingerprint.as_str())
            {
                return None;
            }
            Some((
                operation.incarnation,
                session.clone(),
                super::agent_dispatch::launch_error_terminal_bytes(&failure.detail),
            ))
        })
    }

    fn close_failed_manual_launch(&mut self, operation_id: &str) {
        let Some(expected) = self
            .agent_operations
            .get(operation_id)
            .and_then(|operation| operation.failed_session.clone())
        else {
            return;
        };
        let (reply, cleanup) = oneshot::channel();
        self.close_stopped_owner_session(expected.id.clone(), Box::new(expected), reply);
        self.agent_dispatch_jobs += 1;
        let sender = self.sender.clone();
        let operation_id = operation_id.to_string();
        spawn_owner_worker(move || {
            let cleanup = cleanup.blocking_recv().unwrap_or_else(|_| {
                Err("owner stopped before failed manual launch cleanup".into())
            });
            let _ = sender.send(OwnerMessage::ProcessLaunchFailed {
                operation_id,
                detail: "manual launch cancelled before GUI installation".into(),
                cleanup,
            });
        });
    }

    fn finish_agent_operation(&mut self, operation_id: &str, mut result: OwnerReply) {
        if let Some(operation) = self.agent_operations.get_mut(operation_id) {
            // Exact compensation is immutable readback even if a later
            // physical/registry cleanup attempt still needs reconciliation.
            if let Some(Ok(captured)) = operation.result.as_ref().filter(|cached| {
                cached
                    .as_ref()
                    .is_ok_and(|value| value.get("compensated_launch_failure").is_some())
            }) {
                let mut captured = captured.clone();
                if let Err(error) = &result {
                    captured["compensated_launch_failure"]["error"] = json!(error);
                }
                result = Ok(captured);
            }
            operation.result = Some(result.clone());
            let wire_result = match (&result, operation.retained_manual_failure.as_ref()) {
                (Err(_), Some(failure)) if operation.current.load(Ordering::Acquire) => {
                    Ok(json!({"retained_manual_failure": failure}))
                }
                _ => result.clone(),
            };
            for reply in operation.replies.drain(..) {
                let _ = reply.send(wire_result.clone());
            }
            let cancellation = if operation.cancellation_cleaned {
                Ok(json!({"cancelled":true,"id":operation.id,"operation_id":operation_id}))
            } else {
                Err("cancelled launch cleanup requires exact reconciliation".to_string())
            };
            for reply in operation.cancel_replies.drain(..) {
                let _ = reply.send(cancellation.clone());
            }
            if result.is_err()
                || result
                    .as_ref()
                    .is_ok_and(|value| value.get("compensated_launch_failure").is_some())
            {
                for event in operation.early_events.drain(..) {
                    if let TransportEvent::FreshExecutionReadyResend { reply, .. } = event {
                        Self::refuse_readiness(
                            &reply,
                            "the exact launch failed before readiness installation",
                        );
                    }
                }
            }
        }
    }
    fn agent_prepared(&mut self, prepared: AgentPrepared) {
        let AgentPrepared {
            monitor_failure_committed,
            launch_failure,
            restore,
            retirement_reason,
            registry_cleanup,
            cancelled_cleanup,
            durable_candidate_removed,
            purpose_title,
            operation_id,
            mut input,
            result,
        } = prepared;
        let id = input.window_id;
        let incarnation = input.incarnation;
        let current = self
            .agent_operations
            .get(&operation_id)
            .is_some_and(|operation| operation.id == id && operation.incarnation == incarnation)
            && self.pending.get(&id) == Some(&incarnation)
            && !self.stopping
            && input.current.load(Ordering::Acquire)
            && !self.runtimes.contains_key(&id)
            && !self.closing.contains_key(&id)
            && !self
                .retirements
                .keys()
                .any(|(candidate, _)| candidate == &id);
        let success = match result {
            Err(error) => {
                if monitor_failure_committed {
                    if self.pending.get(&id) == Some(&incarnation) {
                        self.pending.remove(&id);
                    }
                    if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                        operation.cancellation_cleaned = true;
                    }
                    self.refresh_registry_projection();
                    let _ = self.events.send(OwnerEvent::LaunchFailed {
                        id: id.clone(),
                        incarnation,
                        detail: super::monitor::launch_failure_message(&error.detail),
                    });
                    let _ = self.events.send(OwnerEvent::WindowRemoved {
                        id: id.clone(),
                        incarnation,
                        session_id: None,
                    });
                    self.finish_agent_operation(
                        &operation_id,
                        Ok(json!({
                            "id": id, "incarnation": incarnation, "host_pid": std::process::id(),
                            "host_instance_id": self.host_instance_id,
                            "monitor_failure_closed": { "detail": error.detail },
                        })),
                    );
                    return;
                }

                if let Some(removed_genesis_session) = input.removed_genesis_session.take() {
                    let failure = super::events::OwnerCompensatedLaunchFailure {
                        error: error.detail.clone(),
                        removed_genesis_session,
                    };
                    if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                        operation.result = Some(Ok(json!({
                            "id": id, "incarnation": incarnation, "host_pid": std::process::id(),
                            "host_instance_id": self.host_instance_id,
                            "compensated_launch_failure": failure,
                        })));
                    }
                }
                if let Some(failure) = launch_failure {
                    if self.pending.get(&id) == Some(&incarnation) {
                        self.pending.remove(&id);
                    }
                    if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                        operation.failed_session = input
                            .failure_session
                            .as_ref()
                            .map(|(_, snapshot)| snapshot.clone());
                        operation.result = Some(Ok(json!({
                            "id":failure.id, "incarnation":failure.incarnation,
                            "host_pid":failure.host_pid, "host_instance_id":failure.host_instance_id,
                            "session_id":failure.session_id, "launch_failure":failure,
                        })));
                    }
                    self.refresh_registry_projection();
                    if !current {
                        self.close_failed_manual_launch(&operation_id);
                    } else {
                        let receipt = self
                            .agent_operations
                            .get(&operation_id)
                            .and_then(|operation| operation.result.clone())
                            .expect("captured failed launch operation");
                        self.finish_agent_operation(&operation_id, receipt);
                    }
                    return;
                }
                if let Some(failed) = input.failed_launch.take() {
                    self.start_retirement(
                        id,
                        incarnation,
                        Retirement::Agent {
                            restore: restore.map(Box::new),
                            success: failed,
                            sessions_dir: input.sessions_dir,
                            operation_id,
                            durable_candidate_removed,
                            registry_cleanup,
                        },
                        None,
                        Some(error.detail),
                    );
                    return;
                }
                if let Some(pending) = input.pending_continue_work.as_ref() {
                    if durable_candidate_removed {
                        self.continue_work_outcome(
                            pending,
                            super::continuation::CachedContinueWorkOutcome {
                                work_id: pending.work_id.clone(),
                                outcome: crate::ContinueWorkOutcomeKind::Failed,
                                message: Some(format!(
                                    "Continue work launch failed before activation: {}",
                                    error.detail
                                )),
                                error_code: Some(error.error_code.into()),
                                retryable: error.retryable,
                            },
                            true,
                        );
                    } else if super::launch_finalization::pending_execution_activation_status(
                        pending,
                    ) != Some(true)
                    {
                        self.continue_work_outcome(pending, super::continuation::CachedContinueWorkOutcome {
                            work_id: pending.work_id.clone(), outcome: crate::ContinueWorkOutcomeKind::Failed,
                            message: Some("The failed continuation could not commit its exact cleanup; retry reconciliation.".into()),
                            error_code: Some("continuation_reconciliation_required".into()), retryable: true,
                        }, false);
                    }
                }
                if self.pending.get(&id) == Some(&incarnation) {
                    self.pending.remove(&id);
                }
                let retained = current
                    && !input.physical_spawn_started
                    && input.failed_launch.is_none()
                    && self
                        .agent_operations
                        .get(&operation_id)
                        .and_then(|operation| operation.manual_predecessor_stopped.as_ref())
                        .is_some_and(|stopped| {
                            Self::manual_retry_preparation_is_exact(
                                &input.sessions_dir,
                                stopped,
                                &operation_id,
                            )
                            .is_ok()
                        });
                if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
                    operation.cancellation_cleaned = cancelled_cleanup;
                    if retained {
                        operation.retained_manual_failure =
                            Some(super::events::OwnerRetainedManualLaunchFailure {
                                id: id.clone(),
                                operation_id: operation_id.clone(),
                                incarnation,
                                host_pid: std::process::id(),
                                host_instance_id: self.host_instance_id.clone(),
                                detail: error.detail.clone(),
                            });
                    }
                }
                let result = restore
                    .as_ref()
                    .map(|restore| Ok(restore.outcome(&error.detail)))
                    .unwrap_or_else(|| Err(error.detail));
                if let Some(restore) = restore
                    .as_ref()
                    .filter(|restore| restore.registry_after.is_some())
                {
                    match restore
                        .registry_after
                        .as_ref()
                        .expect("captured restore readback")
                    {
                        Ok(snapshot) => {
                            // A sibling worker may have published newer rows. Apply only
                            // this exact source's committed deletions, never an older layout.
                            self.registry_projection.windows.retain(|row| {
                                row.session_id.as_deref() != Some(restore.source.id.as_str())
                                    || !restore
                                        .placeholders
                                        .iter()
                                        .any(|expected| expected.id == row.id)
                                    || snapshot
                                        .windows
                                        .iter()
                                        .any(|remaining| remaining.id == row.id)
                            });
                        }
                        Err(error) => {
                            self.finish_agent_operation(
                                &operation_id,
                                Err(format!(
                                    "restore registry readback requires reconciliation: {error}"
                                )),
                            );
                            return;
                        }
                    }
                } else {
                    self.refresh_registry_projection();
                }
                self.finish_agent_operation(&operation_id, result);
                return;
            }
            Ok(success) => success,
        };
        if !current || success.session_snapshot.is_none() || retirement_reason.is_some() {
            self.start_retirement(
                id,
                incarnation,
                Retirement::Agent {
                    restore: restore.map(Box::new),
                    success: Box::new(success),
                    sessions_dir: input.sessions_dir,
                    operation_id,
                    durable_candidate_removed: false,
                    registry_cleanup,
                },
                None,
                Some(
                    retirement_reason
                        .unwrap_or_else(|| "agent launch cancelled before installation".into()),
                ),
            );
            return;
        }
        self.pending.remove(&id);
        if let Some(operation) = self.agent_operations.get_mut(&operation_id) {
            operation.pending_continue = None;
        }
        let snapshot = success.session_snapshot.expect("validated durable Session");
        let member_cleanup = success
            .member_reservation
            .clone()
            .map(|reservation| (snapshot.clone(), reservation));
        let named_assignment = snapshot.clone();
        let predecessor_session_id = success.predecessor_session_id.clone();
        let session_id = success.active_session.session_id.clone();
        let awaiting_readiness =
            success.pending_fresh_execution.is_some() || success.pending_continue_work.is_some();
        let initial_prompt_file = success.initial_prompt_file;
        let readiness_deadline = success
            .pending_fresh_execution
            .as_ref()
            .map(|pending| &pending.operation_id)
            .or_else(|| {
                success
                    .pending_continue_work
                    .as_ref()
                    .map(|pending| &pending.operation_id)
            })
            .map(|operation_id| {
                let watch =
                    super::launch_readiness::ContinueWorkReadinessWatch::new(operation_id.clone());
                let at = std::time::Instant::now() + watch.delay();
                ReadinessDeadline { watch, at }
            });
        let record = AgentRuntimeRecord {
            feedback: input.launch_feedback_context.clone(),
            monitor_owner: Some((input.project_root.clone(), input.materializer_id.clone())),
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: !awaiting_readiness,
            monitor_retry_at: None,
            monitor_startup_at: input
                .launch_feedback_context
                .as_ref()
                .filter(|feedback| {
                    !feedback.issue_monitor_review_dispatch
                        && feedback.issue_monitor_autonomous_handoff.is_none()
                        && feedback
                            .issue_monitor_launch_binding_issue_number()
                            .is_some()
                        && feedback
                            .issue_monitor_session_mode
                            .unwrap_or(gwt_agent::SessionMode::Normal)
                            == gwt_agent::SessionMode::Normal
                })
                .map(|_| std::time::Instant::now() + std::time::Duration::from_secs(180)),
            current: input.current.clone(),
            active: success.active_session,
            snapshot,
            capability_token: success.capability_token,
            issuer: self.issuer.clone(),
            pending_fresh: success.pending_fresh_execution,
            pending_continue: success.pending_continue_work,
            purpose_title,
            readiness_deadline,
        };
        let result =
            self.install_pane(id.clone(), incarnation, success.pane, input.current.clone());
        // A failed writer-index publication must not turn a bound Session into
        // an unbound physical-only cleanup. Install its captured metadata first.
        if let Some(runtime) = self
            .runtimes
            .get_mut(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
        {
            runtime._initial_prompt_file = initial_prompt_file;
            runtime.registry_cleanup = registry_cleanup;
            runtime.member_cleanup = member_cleanup;
            runtime.agent = Some(record);
            // A newly installed physical Session has not yet reported a
            // provider hook. Preserve the existing launch Starting state.
            runtime.hook_status = Some(crate::WindowProcessStatus::Starting);
        }
        let result = result.and_then(|mut receipt| {
            self.register_owner_writer(&id)?;
            receipt["session_id"] = json!(session_id);
            receipt["operation_id"] = json!(operation_id);
            receipt["awaiting_readiness"] = json!(awaiting_readiness);
            Ok(receipt)
        });
        if result.is_ok() && awaiting_readiness {
            let status = Self::runtime_status(&self.runtimes[&id]);
            self.publish_status(
                id.clone(),
                incarnation,
                status,
                Some("Waiting for authenticated SessionStart...".into()),
                false,
            );
        }
        if result.is_err()
            && self
                .runtimes
                .get(&id)
                .is_some_and(|runtime| runtime.incarnation == incarnation)
        {
            let runtime = self
                .runtimes
                .remove(&id)
                .expect("exact failed agent insertion");
            let (reply, _) = oneshot::channel();
            self.start_close(id.clone(), runtime, CloseCause::ExplicitClose, reply);
        }
        self.refresh_registry_projection();
        if result.is_ok()
            && named_assignment.agent_name.is_some()
            && named_assignment.linked_issue_number.is_some()
            && !named_assignment.review_dispatch
        {
            self.input_jobs += 1;
            let sender = self.sender.clone();
            let factory = self.issue_client_factory.clone();
            spawn_owner_worker(move || {
                let result = super::launch::record_named_launch_assignment(
                    &named_assignment,
                    predecessor_session_id.as_deref(),
                    &gwt_core::paths::gwt_sessions_dir(),
                    &factory,
                );
                let _ = sender.send(OwnerMessage::NamedAssignmentFinished(result));
            });
        }
        self.finish_agent_operation(&operation_id, result);
        self.commit_monitor_launch(&id);
        let early_events = self
            .agent_operations
            .get_mut(&operation_id)
            .map(|operation| std::mem::take(&mut operation.early_events))
            .unwrap_or_default();
        // Replay directly before consuming the next mailbox item. A re-enqueue
        // would let a later close overtake an already authenticated SessionStart.
        for event in early_events {
            self.handle_transport_event(event);
        }
        let _ = self.events.send(OwnerEvent::AgentStarted {
            id,
            incarnation,
            session_id,
            awaiting_readiness,
        });
        let _ = self.events.send(OwnerEvent::Invalidated);
    }

    fn owner_monitor_window_snapshot(
        &self,
        excluded_id: &str,
    ) -> crate::IssueMonitorWindowSnapshot {
        let observed_at = chrono::Utc::now().to_rfc3339();
        let windows = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                let agent = runtime.agent.as_ref()?;
                Some(crate::IssueMonitorWindowObservation {
                    monitor_owned: crate::cli::execution_state::session_launch_route_from_session(
                        &agent.snapshot,
                    ) == gwt_agent::LaunchRoute::Autonomous,
                    window_id: id.clone(),
                    issue_number: agent.snapshot.linked_issue_number,
                    status: Self::runtime_status(runtime),
                    review_dispatch: agent.snapshot.review_dispatch,
                    hold_reason: None,
                    last_output_at: None,
                    session_id: Some(agent.active.session_id.clone()),
                })
            })
            .chain(
                self.agent_operations
                    .values()
                    .filter(|operation| {
                        operation.result.is_none()
                            && operation.id != excluded_id
                            && operation.request["type"] != "check_launch_permission"
                    })
                    .map(|operation| {
                        let metadata = operation
                            .request
                            .get("metadata")
                            .and_then(|value| value.get("launch_feedback_context"));
                        crate::IssueMonitorWindowObservation {
                            monitor_owned: metadata
                                .and_then(|feedback| feedback.get("issue_monitor_issue_number"))
                                .is_some_and(|number| number.is_u64()),
                            window_id: operation.id.clone(),
                            issue_number: operation
                                .request
                                .get("config")
                                .and_then(|config| config.get("linked_issue_number"))
                                .and_then(serde_json::Value::as_u64),
                            status: crate::WindowProcessStatus::Starting,
                            review_dispatch: metadata
                                .and_then(|feedback| feedback.get("issue_monitor_review_dispatch"))
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                            hold_reason: None,
                            last_output_at: None,
                            session_id: operation.session_id.clone(),
                        }
                    }),
            )
            .collect();
        crate::IssueMonitorWindowSnapshot {
            project_tab_id: self.project_key.clone(),
            observed_at,
            windows,
        }
    }

    fn capture_close_monitor_snapshot(&self, id: &str, runtime: &mut WindowRuntime) {
        let monitor_owned = runtime.rollback_monitor_failure.is_some()
            || runtime
                .agent
                .as_ref()
                .and_then(|agent| agent.feedback.as_ref())
                .is_some_and(|feedback| feedback.issue_monitor_issue_number.is_some());
        runtime.close_monitor_snapshot =
            monitor_owned.then(|| self.owner_monitor_window_snapshot(id));
    }

    fn start_close(
        &mut self,
        id: String,
        runtime: WindowRuntime,
        cause: CloseCause,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        self.pm_wake_permits.cancel_mobile_for_pane(Some(&id));
        let incarnation = runtime.incarnation;
        self.close_replies
            .entry((id.clone(), incarnation))
            .or_default()
            .push(reply);
        let mut runtime = runtime;
        if runtime.registry_cleanup.is_none() {
            if let Ok(registry) = &self.agent_registry {
                if let Some(raw) = id.strip_prefix(&format!("{}::", self.project_key)) {
                    let expected_session_id = runtime
                        .agent
                        .as_ref()
                        .map(|agent| agent.snapshot.id.clone());
                    let row = self
                        .registry_projection
                        .windows
                        .iter()
                        .find(|row| row.id == raw);
                    if row.is_none_or(|row| row.session_id == expected_session_id) {
                        runtime.registry_cleanup =
                            Some(super::agent_dispatch::OwnedWindowRecordCleanup {
                                registry: registry.clone(),
                                registry_path: crate::persistence::workspace_state_path(&self.root),
                                root: self.root.clone(),
                                project_key: self.project_key.clone(),
                                canonical_id: id.clone(),
                                expected_session_id,
                            });
                    }
                }
            }
        }
        if runtime
            .agent
            .as_ref()
            .is_some_and(|agent| agent.binding_repair_inflight)
        {
            runtime.pty.revoke_input_generation();
            self.closing.insert(
                id.clone(),
                ClosingRuntime {
                    incarnation,
                    pm_worktree: runtime_uses_pm_worktree(&runtime),
                },
            );
            let deferred = self
                .deferred_readiness_closes
                .entry(id.clone())
                .or_insert(DeferredReadinessClose { incarnation, cause });
            if matches!(cause, CloseCause::ExplicitClose) {
                deferred.cause = cause;
            }
            self.runtimes.insert(id, runtime);
            return;
        }
        runtime.current.store(false, Ordering::Release);
        runtime.pty.revoke_input_generation();
        if let Some(agent) = &runtime.agent {
            agent.current.store(false, Ordering::Release);
        }
        self.closing.insert(
            id.clone(),
            ClosingRuntime {
                incarnation,
                pm_worktree: runtime_uses_pm_worktree(&runtime),
            },
        );
        let readiness_inflight = self.readiness_inflight(&id, incarnation);
        if readiness_inflight {
            let job = self.readiness.get(&id).expect("exact readiness job");
            job.current.store(false, Ordering::Release);
        }
        if readiness_inflight
            || runtime
                .agent
                .as_ref()
                .is_some_and(|agent| agent.monitor_commit_inflight || agent.monitor_answer_inflight)
        {
            let deferred = self
                .deferred_readiness_closes
                .entry(id.clone())
                .or_insert(DeferredReadinessClose { incarnation, cause });
            if matches!(cause, CloseCause::ExplicitClose) {
                deferred.cause = cause;
            }
            self.runtimes.insert(id, runtime);
            return;
        }
        self.spawn_close(id, runtime, cause);
    }
    fn resume_deferred_close(&mut self, id: &str, incarnation: u64) {
        if self.readiness_inflight(id, incarnation)
            || self.runtimes.get(id).is_some_and(|runtime| {
                runtime.incarnation == incarnation
                    && runtime.agent.as_ref().is_some_and(|agent| {
                        agent.monitor_commit_inflight
                            || agent.monitor_answer_inflight
                            || agent.binding_repair_inflight
                    })
            })
        {
            return;
        }
        if self
            .deferred_readiness_closes
            .get(id)
            .is_some_and(|deferred| deferred.incarnation == incarnation)
        {
            let deferred = self
                .deferred_readiness_closes
                .remove(id)
                .expect("exact deferred close");
            if let Some(runtime) = self.runtimes.remove(id) {
                runtime.current.store(false, Ordering::Release);
                runtime.pty.revoke_input_generation();
                if let Some(agent) = runtime.agent.as_ref() {
                    agent.current.store(false, Ordering::Release);
                }
                self.spawn_close(id.into(), runtime, deferred.cause);
            }
        }
    }
    fn spawn_close(&self, id: String, mut runtime: WindowRuntime, cause: CloseCause) {
        if cause.restore_policy() == Some(false) || cause.is_diagnostic() {
            self.capture_close_monitor_snapshot(&id, &mut runtime);
        } else {
            runtime.close_monitor_snapshot = None;
        }
        let incarnation = runtime.incarnation;
        let sender = self.sender.clone();
        let writers = self.pty_writers.clone();
        spawn_owner_worker(move || {
            let registry_result =
                super::agent_dispatch::forget_owner_writer(&writers, &id, &runtime.current);
            let result = close_runtime(&id, &mut runtime, cause).and(registry_result);
            let _ = sender.send(OwnerMessage::Closed {
                id,
                incarnation,
                runtime: Box::new(runtime),
                cause,
                result,
            });
        });
    }
    fn closed(
        &mut self,
        id: String,
        incarnation: u64,
        runtime: Box<WindowRuntime>,
        cause: CloseCause,
        result: Result<(), String>,
    ) {
        let result = if self
            .closing
            .get(&id)
            .is_none_or(|closing| closing.incarnation != incarnation)
        {
            self.retirements.insert(
                (id.clone(), incarnation),
                Some(Retirement::Runtime { runtime, cause }),
            );
            Err("close incarnation replaced".into())
        } else {
            self.closing.remove(&id);
            let result = result.and_then(|()| {
                if cause.is_diagnostic() {
                    self.persist_registry_status(
                        &id,
                        runtime
                            .agent
                            .as_ref()
                            .map(|agent| agent.snapshot.id.as_str()),
                        Self::runtime_status(&runtime),
                    )?;
                }
                Ok(())
            });

            // close_runtime joined the reader before this exact successful
            // ACK. Its last Output may have arrived after retirement and been
            // rejected; carry the authoritative parsed screen with the ACK.
            let final_screen = result.is_ok().then(|| {
                let pane = runtime
                    .pane
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                super::events::OwnerFinalScreen {
                    screen: pane.screen().contents(),
                    screen_ansi: pane.screen().contents_formatted(),
                    screen_seq: pane.output_seq(),
                }
            });
            if let Some(frame) = final_screen.as_ref() {
                use base64::Engine;
                self.dispatch_project(
                    crate::BackendEvent::TerminalSnapshot {
                        id: id.clone(),
                        data_base64: base64::engine::general_purpose::STANDARD
                            .encode(&frame.screen_ansi),
                    },
                    Some(frame.screen_seq),
                );
            }
            let natural_terminal = (matches!(cause, CloseCause::NaturalExit)
                || cause.is_diagnostic())
                && result.is_ok();
            if natural_terminal {
                let diagnostic_exit = cause.is_diagnostic();
                let status = Self::runtime_status(&runtime);
                // Capture this proof while the successful finalizer still owns
                // the exact reaped PTY. Later historical cleanup must not fill
                // missing process identities from a newer sidecar.
                let diagnostic_runtime = diagnostic_exit
                    .then(|| {
                        if !runtime.pty.tree_exit_confirmed()
                            || runtime.pty.try_wait().ok()?.is_none()
                        {
                            return None;
                        }
                        let agent = runtime.agent.as_ref()?;
                        let identity =
                            gwt_agent::SessionExecutionIdentity::from_session(&agent.snapshot)
                                .ok()
                                .flatten()?;
                        Some(crate::issue_monitor::IssueMonitorRuntimeStopRequest {
                            issue_number: agent.snapshot.linked_issue_number?,
                            window_id: id.clone(),
                            session_id: agent.snapshot.id.clone(),
                            execution_identity: identity,
                            host_pid: std::process::id(),
                            host_started_at: crate::process::host_process_start_time(
                                std::process::id(),
                            )
                            .filter(|birth| *birth > 0)?,
                            runtime_incarnation: incarnation,
                            child_pid: runtime.pty.process_id().filter(|pid| *pid > 0)?,
                            child_started_at: runtime
                                .child_started_at
                                .filter(|birth| *birth > 0)?,
                            requested_at: chrono::Utc::now().to_rfc3339(),
                        })
                    })
                    .flatten();
                // Replaying the accepted operation must carry its completed
                // lifecycle even when the GUI missed the initial reply/event.
                for operation in self
                    .agent_operations
                    .values_mut()
                    .filter(|operation| operation.id == id && operation.incarnation == incarnation)
                {
                    let diagnostic_runtime = diagnostic_runtime.as_ref().filter(|proof| {
                        operation.session_id.as_deref() == Some(proof.session_id.as_str())
                    });
                    if let Some(Ok(receipt)) = operation.result.as_mut() {
                        receipt["closed"] = json!(true);
                        receipt["natural_exit"] = json!(!diagnostic_exit);
                        receipt["diagnostic_exit"] = json!(diagnostic_exit);
                        receipt["status"] = json!(status);
                        receipt["detail"] = json!(runtime.detail);
                        if let Some(proof) = diagnostic_runtime {
                            receipt["diagnostic_close_runtime"] = json!(proof);
                        }
                    }
                }
                self.dispatch_project(
                    crate::BackendEvent::TerminalStatus {
                        id: id.clone(),
                        status,
                        detail: runtime.detail.clone(),
                        error_code: None,
                        retryable: None,
                    },
                    None,
                );
                let _ = self.events.send(OwnerEvent::Status {
                    id: id.clone(),
                    incarnation,
                    status,
                    detail: runtime.detail.clone(),
                    exit_confirmed: true,
                    raw_status: runtime.pty_status,
                    exit_code: runtime
                        .pane
                        .lock()
                        .ok()
                        .and_then(|pane| pane.last_exit().map(|exit| exit.exit_code)),
                });
            }
            let pm_lifecycle = runtime
                .pm_lifecycle
                .as_ref()
                .and_then(super::pm_delivery::PmLifecycleCommit::outcome)
                .cloned();
            if result.is_ok() {
                if let Some(outcome) = pm_lifecycle.as_ref() {
                    let _ = self.events.send(OwnerEvent::PmLifecycle {
                        id: id.clone(),
                        incarnation,
                        outcome: outcome.clone(),
                    });
                }
                if matches!(cause, CloseCause::ExplicitClose | CloseCause::NaturalExit) {
                    let _ = self.events.send(OwnerEvent::WindowRemoved {
                        id: id.clone(),
                        incarnation,
                        session_id: runtime
                            .agent
                            .as_ref()
                            .map(|agent| agent.snapshot.id.clone()),
                    });
                }
            }
            let _ = self.events.send(OwnerEvent::Closed {
                id: id.clone(),
                incarnation,
                error: result.as_ref().err().cloned(),
                final_screen,
            });
            if result.is_err() {
                self.retirements.insert(
                    (id.clone(), incarnation),
                    Some(Retirement::Runtime { runtime, cause }),
                );
            }
            result.map(|_| json!({"closed":true,"id":id,"incarnation":incarnation,"pm_lifecycle":pm_lifecycle}))
        };
        for reply in self
            .close_replies
            .remove(&(id, incarnation))
            .unwrap_or_default()
        {
            let _ = reply.send(result.clone());
        }
        self.refresh_registry_projection();
        self.resume_agent_transport();
    }
    fn finish_process_launch(
        &mut self,
        operation_id: Option<&str>,
        reply: Option<oneshot::Sender<OwnerReply>>,
        result: OwnerReply,
        cleaned: bool,
    ) {
        if let Some(operation_id) = operation_id {
            if let Some(operation) = self.agent_operations.get_mut(operation_id) {
                operation.cancellation_cleaned = cleaned;
            }
            self.finish_agent_operation(operation_id, result.clone());
        }
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }

    fn close_failed_process_launch(
        &mut self,
        id: String,
        runtime: WindowRuntime,
        operation_id: Option<String>,
        reply: Option<oneshot::Sender<OwnerReply>>,
        detail: String,
    ) {
        let (closed, receive) = oneshot::channel();
        self.start_close(id, runtime, CloseCause::ExplicitClose, closed);
        if let Some(operation_id) = operation_id {
            self.agent_dispatch_jobs += 1;
            let sender = self.sender.clone();
            spawn_owner_worker(move || {
                let cleanup = receive.blocking_recv().unwrap_or_else(|_| {
                    Err("owner stopped before failed process cleanup acknowledgement".into())
                });
                let _ = sender.send(OwnerMessage::ProcessLaunchFailed {
                    operation_id,
                    detail,
                    cleanup,
                });
            });
        } else if let Some(reply) = reply {
            let _ = reply.send(Err(detail));
        }
    }

    fn process_prepared(
        &mut self,
        id: String,
        incarnation: u64,
        pane: Result<Pane, String>,
        initial_prompt_file: Option<Arc<tempfile::TempPath>>,
        operation_id: Option<String>,
        reply: Option<oneshot::Sender<OwnerReply>>,
    ) {
        let current = operation_id.as_ref().and_then(|operation_id| {
            self.agent_operations
                .get(operation_id)
                .filter(|operation| operation.id == id && operation.incarnation == incarnation)
                .map(|operation| operation.current.clone())
        });
        let cancelled = self.stopping
            || self.pending.get(&id) != Some(&incarnation)
            || (operation_id.is_some()
                && current
                    .as_ref()
                    .is_none_or(|current| !current.load(Ordering::Acquire)));
        if cancelled {
            match pane {
                Ok(pane) => self.start_retirement(
                    id,
                    incarnation,
                    Retirement::Pane {
                        pane: Box::new(pane),
                        operation_id,
                        _initial_prompt_file: initial_prompt_file,
                    },
                    reply,
                    Some("launch was cancelled before installation".into()),
                ),
                Err(error) => {
                    if self.pending.get(&id) == Some(&incarnation) {
                        self.pending.remove(&id);
                    }
                    self.finish_process_launch(operation_id.as_deref(), reply, Err(error), true);
                }
            }
            return;
        }
        self.pending.remove(&id);
        let current = current.unwrap_or_else(|| Arc::new(AtomicBool::new(true)));
        let result =
            pane.and_then(|pane| self.install_pane(id.clone(), incarnation, pane, current));
        match result {
            Ok(mut receipt) => {
                let runtime = self.runtimes.get_mut(&id).expect("installed exact process");
                runtime._initial_prompt_file = initial_prompt_file;
                receipt["session_id"] = serde_json::Value::Null;
                if let Err(error) = self.register_owner_writer(&id) {
                    let runtime = self
                        .runtimes
                        .remove(&id)
                        .expect("failed process writer registration");
                    self.close_failed_process_launch(id, runtime, operation_id, reply, error);
                    return;
                }
                self.publish_shell_registry(id, incarnation, receipt, reply, operation_id);
            }
            Err(error) => {
                if self
                    .runtimes
                    .get(&id)
                    .is_some_and(|runtime| runtime.incarnation == incarnation)
                {
                    let runtime = self
                        .runtimes
                        .remove(&id)
                        .expect("exact failed shell insertion");
                    self.close_failed_process_launch(id, runtime, operation_id, reply, error);
                } else {
                    self.finish_process_launch(operation_id.as_deref(), reply, Err(error), true);
                }
            }
        }
    }

    fn retire_uninstalled(
        &mut self,
        id: String,
        incarnation: u64,
        pane: Result<Pane, String>,
        reply: Option<oneshot::Sender<OwnerReply>>,
        reason: &str,
    ) {
        match pane {
            Ok(pane) => self.start_retirement(
                id,
                incarnation,
                Retirement::Pane {
                    pane: Box::new(pane),
                    operation_id: None,
                    _initial_prompt_file: None,
                },
                reply,
                Some(reason.into()),
            ),
            Err(error) => {
                if self.pending.get(&id) == Some(&incarnation) {
                    self.pending.remove(&id);
                }
                if let Some(reply) = reply {
                    let _ = reply.send(Err(format!("{reason}: {error}")));
                }
            }
        }
    }
    fn start_retirement(
        &mut self,
        id: String,
        incarnation: u64,
        captured: Retirement,
        reply: Option<oneshot::Sender<OwnerReply>>,
        cancellation: Option<String>,
    ) {
        self.retiring += 1;
        self.retirements.insert((id.clone(), incarnation), None);
        let sender = self.sender.clone();
        let issuer = self.issuer.clone();
        spawn_owner_worker(move || {
            let mut captured = captured;
            let result = match &mut captured {
                Retirement::Agent {
                    success,
                    sessions_dir,
                    durable_candidate_removed,
                    registry_cleanup,
                    restore,
                    ..
                } => abandon_agent_launch(
                    success,
                    sessions_dir,
                    &issuer,
                    *durable_candidate_removed,
                    registry_cleanup.as_ref(),
                )
                .and_then(|()| {
                    restore.as_mut().map_or(Ok(()), |restore| {
                        super::installation::finish_owner_restore_failure(
                            &success.project_root,
                            sessions_dir,
                            restore,
                            cancellation
                                .as_deref()
                                .unwrap_or("agent launch cancelled before installation"),
                        )
                    })
                }),
                Retirement::StoppedWindow(capture) => capture.finish(),
                Retirement::SwitchStoppedWindow {
                    proof,
                    exit,
                    cleanup,
                } => (|| {
                    let sessions = gwt_core::paths::gwt_sessions_dir();
                    crate::cli::execution_state::with_exact_owned_runtime_cleanup_lease(
                        &sessions,
                        &proof.execution_identity,
                        gwt_agent::ManualLaunchRuntimeProof {
                            host_pid: proof.host_pid,
                            runtime_incarnation: proof.runtime_incarnation,
                        },
                        || {
                            let snapshot = gwt_agent::Session::load(
                                &sessions.join(format!("{}.toml", proof.session_id)),
                            )?;
                            let runtime = gwt_agent::SessionRuntimeState::load(
                                &gwt_agent::runtime_state_path_for_pid(
                                    &sessions,
                                    proof.host_pid,
                                    &proof.session_id,
                                ),
                            )?;
                            let exact_exit = match exit {
                                StoppedOwnerWindowExit::CapturedPty(pty) => {
                                    pty.tree_exit_confirmed()
                                        && pty.process_id() == Some(proof.child_pid)
                                }
                                StoppedOwnerWindowExit::DiagnosticClosed(expected) => {
                                    serde_json::to_value(&snapshot)?
                                        == serde_json::to_value(expected)?
                                }
                            };
                            if !exact_exit
                                || proof.host_pid != std::process::id()
                                || proof.host_started_at == 0
                                || crate::process::host_process_start_time(proof.host_pid)
                                    != Some(proof.host_started_at)
                                || proof.child_pid == 0
                                || proof.child_started_at == 0
                                || gwt_agent::SessionExecutionIdentity::from_session(&snapshot)
                                    .map_err(std::io::Error::other)?
                                    .as_ref()
                                    != Some(&proof.execution_identity)
                                || !matches!(
                                    snapshot.status,
                                    gwt_agent::AgentStatus::Stopped
                                        | gwt_agent::AgentStatus::Interrupted
                                )
                                || snapshot.restore_window_on_startup
                                || runtime.status != gwt_agent::AgentStatus::Stopped
                                || !proof.matches_runtime(&snapshot.id, proof.host_pid, &runtime)
                            {
                                return Err(std::io::Error::other(
                                    "Switch source terminal identity or tree exit changed",
                                ));
                            }
                            if matches!(&*exit, StoppedOwnerWindowExit::DiagnosticClosed(_)) {
                                let observations =
                                    crate::session_inventory::observe_session(&snapshot, &sessions);
                                if !observations.sessions.is_empty()
                                    || !observations.uncertainties.is_empty()
                                {
                                    return Err(std::io::Error::other(
                                        "stopped diagnostic has live or uncertain runtime evidence",
                                    ));
                                }
                            }
                            // Exact physical close already committed the Session.
                            // Keep its terminal sidecar; remove only its exact
                            // canvas record under the same cleanup authority.
                            cleanup.finish().map_err(std::io::Error::other)
                        },
                    )
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Switch source cleanup authority changed".into())
                })(),
                Retirement::Pane { pane, .. } => {
                    crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &id)
                        .map_err(|error| error.to_string())
                        .and_then(|reaped| {
                            if reaped {
                                pane.pty().invalidate_input_generation();
                                Ok(())
                            } else {
                                Err("owned process tree did not exit".into())
                            }
                        })
                }
                Retirement::Runtime { .. } => {
                    unreachable!("installed runtime retries use its exact close finalizer")
                }
            };
            let _ = sender.send(OwnerMessage::Retired {
                id,
                incarnation,
                captured,
                result,
                reply,
                cancellation,
            });
        });
    }
    fn exact(&self, id: &str, incarnation: u64) -> Result<&WindowRuntime, String> {
        self.runtimes
            .get(id)
            .filter(|runtime| runtime.incarnation == incarnation && !self.closing.contains_key(id))
            .ok_or_else(|| "pane incarnation mismatch".into())
    }
    fn refuse_readiness(reply: &mpsc::Sender<FreshReadinessResult>, reason: &str) {
        let _ = reply.send(Err(crate::AgentWorkspaceUpdateError::new(
            crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
            reason,
        )));
    }
    fn resend_fresh_readiness(
        &mut self,
        grant: crate::agent_capability::AgentCapabilityGrant,
        request: crate::AgentExecutionContinuationRequest,
        reply: mpsc::Sender<FreshReadinessResult>,
    ) {
        if let Err(error) = request.validate() {
            let _ = reply.send(Err(error));
            return;
        }
        let principal = grant.principal();
        let Some(binding) = principal.execution_binding() else {
            Self::refuse_readiness(
                &reply,
                "readiness resend requires an exact bound Host capability",
            );
            return;
        };
        if self.stopping
            || !principal.authorizes_project_root(&self.root)
            || !self.issuer.grant_is_current(&grant)
        {
            Self::refuse_readiness(
                &reply,
                "project readiness owner or capability is no longer current",
            );
            return;
        }
        if let Some(operation) = self.agent_operations.values_mut().find(|operation| {
            operation.result.is_none()
                && operation.session_id.as_deref() == Some(principal.session_id())
        }) {
            operation
                .early_events
                .push_back(TransportEvent::FreshExecutionReadyResend {
                    grant,
                    request,
                    reply,
                });
            return;
        }
        let prepared = principal.prepared_execution_binding().is_some();
        let candidate = self.runtimes.iter().find_map(|(id, runtime)| {
            let agent = runtime.agent.as_ref()?;
            let pending = agent.pending_fresh.as_ref()?;
            (pending.binding == *binding
                && pending.binding.session_id == principal.session_id()
                && super::launch_finalization::path_matches(
                    &pending.project_root,
                    principal.canonical_project_root(),
                ))
            .then(|| (id.clone(), pending.clone(), agent.capability_token.clone()))
        });
        let Some((id, pending, token)) = candidate else {
            if prepared {
                Self::refuse_readiness(
                    &reply,
                    "the matching fresh launch is no longer pending; run execution.status",
                );
            } else {
                let _ = reply.send(Ok(None));
            }
            return;
        };
        if prepared && request.readiness_nonce.as_deref() != Some(pending.readiness_nonce.as_str())
        {
            Self::refuse_readiness(
                &reply,
                "execution.continue requires the original authenticated launch readiness nonce",
            );
            return;
        }
        let Some(token) = token.filter(|token| grant.matches_token(token)) else {
            Self::refuse_readiness(
                &reply,
                "Prepared Host capability changed before readiness resend",
            );
            return;
        };
        if self.closing.contains_key(&id) {
            Self::refuse_readiness(&reply, "the exact fresh launch is closing");
            return;
        }
        if let Some(job) = self.readiness.get_mut(&id) {
            if job.fence.binding == pending.binding
                && job.fence.token.as_deref() == Some(token.as_str())
            {
                job.replies.push((request.operation_id, reply));
                return;
            }
            Self::refuse_readiness(&reply, "a different exact readiness completion is pending");
            return;
        }
        if !prepared
            && super::launch_finalization::pending_fresh_execution_activation_status(&pending)
                != Some(true)
        {
            Self::refuse_readiness(
                &reply,
                "Active capability has no authenticated activation for this pending fresh launch",
            );
            return;
        }
        if self.begin_readiness(&id, request.readiness_nonce, !prepared, None) {
            self.readiness
                .get_mut(&id)
                .expect("accepted readiness job")
                .replies
                .push((request.operation_id, reply));
        } else {
            Self::refuse_readiness(&reply, "the exact fresh launch cannot accept readiness");
        }
    }
    fn readiness_inflight(&self, id: &str, incarnation: u64) -> bool {
        self.readiness
            .get(id)
            .is_some_and(|job| job.fence.incarnation == incarnation)
    }
    fn next_readiness_deadline(&self) -> Option<std::time::Instant> {
        if self.stopping {
            return None;
        }
        self.runtimes
            .iter()
            .filter(|(id, _)| !self.closing.contains_key(*id) && !self.readiness.contains_key(*id))
            .filter_map(|(_, runtime)| {
                runtime
                    .agent
                    .as_ref()?
                    .readiness_deadline
                    .as_ref()
                    .map(|deadline| deadline.at)
            })
            .min()
    }
    fn check_readiness_deadlines(&mut self, now: std::time::Instant) {
        use super::launch_readiness::{
            continue_work_readiness_decision, ReadinessDeadlineDecision, ReadinessPaneEvidence,
        };
        if self.stopping {
            return;
        }
        let due: Vec<String> = self
            .runtimes
            .iter()
            .filter(|(id, _)| !self.closing.contains_key(*id) && !self.readiness.contains_key(*id))
            .filter_map(|(id, runtime)| {
                runtime
                    .agent
                    .as_ref()?
                    .readiness_deadline
                    .as_ref()
                    .filter(|deadline| deadline.at <= now)
                    .map(|_| id.clone())
            })
            .collect();
        for id in due {
            let runtime = &self.runtimes[&id];
            let agent = runtime.agent.as_ref().expect("deadline agent");
            let deadline = agent.readiness_deadline.as_ref().expect("due deadline");
            let pending = agent
                .pending_fresh
                .as_ref()
                .map(|pending| (&pending.operation_id, &pending.binding.session_id))
                .or_else(|| {
                    agent
                        .pending_continue
                        .as_ref()
                        .map(|pending| (&pending.operation_id, &pending.binding.session_id))
                });
            let Some((operation_id, session_id)) =
                pending.filter(|(operation_id, _)| **operation_id == deadline.watch.operation_id)
            else {
                self.runtimes
                    .get_mut(&id)
                    .unwrap()
                    .agent
                    .as_mut()
                    .unwrap()
                    .readiness_deadline = None;
                continue;
            };
            let evidence = if agent.active.session_id != *session_id {
                ReadinessPaneEvidence::Foreign
            } else if runtime.pty_status == crate::WindowProcessStatus::Running {
                ReadinessPaneEvidence::Live
            } else {
                ReadinessPaneEvidence::Dead
            };
            let decision =
                continue_work_readiness_decision(&deadline.watch, evidence, runtime.output_bytes);
            let _ = operation_id;
            match decision {
                ReadinessDeadlineDecision::Extend(watch) => {
                    let at = now + watch.delay();
                    self.runtimes
                        .get_mut(&id)
                        .unwrap()
                        .agent
                        .as_mut()
                        .unwrap()
                        .readiness_deadline = Some(ReadinessDeadline { watch, at });
                }
                ReadinessDeadlineDecision::HandOff { watch, detail } => {
                    let at = now + watch.delay();
                    let runtime = self.runtimes.get_mut(&id).unwrap();
                    runtime.agent.as_mut().unwrap().readiness_deadline =
                        Some(ReadinessDeadline { watch, at });
                    if let Some(detail) = detail {
                        let agent = runtime.agent.as_ref().unwrap();
                        let owner_root = agent
                            .pending_fresh
                            .as_ref()
                            .map(|pending| (pending.owner, pending.project_root.as_path()))
                            .or_else(|| {
                                agent
                                    .pending_continue
                                    .as_ref()
                                    .map(|pending| (pending.owner, pending.project_root.as_path()))
                            });
                        let detail = super::launch_readiness::record_readiness_handoff(
                            &id,
                            &detail,
                            Some(&agent.active),
                            owner_root,
                        );
                        let incarnation = runtime.incarnation;
                        let status = Self::runtime_status(runtime);
                        self.publish_status(id.clone(), incarnation, status, Some(detail), false);
                    }
                }
                ReadinessDeadlineDecision::Abort { detail, pane } => {
                    self.begin_readiness(&id, None, false, Some((detail, pane)));
                }
            }
        }
    }
    fn begin_readiness(
        &mut self,
        id: &str,
        nonce: Option<String>,
        repair: bool,
        abort: Option<(String, super::launch_readiness::LaunchPaneDisposition)>,
    ) -> bool {
        if self.stopping || self.closing.contains_key(id) {
            return false;
        }
        let Some(runtime) = self.runtimes.get(id) else {
            return false;
        };
        let Some(agent) = runtime.agent.as_ref() else {
            return false;
        };
        let pending = if let Some(pending) = agent.pending_fresh.clone() {
            ReadinessPending::Fresh(Box::new(pending))
        } else if let Some(pending) = agent.pending_continue.clone() {
            ReadinessPending::Continue(Box::new(pending))
        } else {
            return false;
        };
        let fence = ReadinessFence {
            id: id.into(),
            incarnation: runtime.incarnation,
            operation_id: pending.operation_id().into(),
            binding: pending.binding().clone(),
            token: agent.capability_token.clone(),
        };
        if let Some(existing) = self.readiness.get(id) {
            return existing.fence == fence;
        }
        let current = agent.current.clone();
        let active = agent.active.clone();
        let active_is_live = !matches!(
            Self::runtime_status(runtime),
            crate::WindowProcessStatus::Stopped | crate::WindowProcessStatus::Error
        );
        // Match the original active_agent_sessions set, including stopped records.
        let live_sessions = self
            .runtimes
            .values()
            .filter_map(|runtime| {
                runtime
                    .agent
                    .as_ref()
                    .map(|agent| agent.active.session_id.clone())
            })
            .collect();
        let teardown_after_rollback = abort.as_ref().is_none_or(|(_, pane)| {
            *pane == super::launch_readiness::LaunchPaneDisposition::Teardown
        });
        self.readiness.insert(
            id.into(),
            ReadinessJob {
                fence: fence.clone(),
                pending: pending.clone(),
                current: current.clone(),
                replies: Vec::new(),
                teardown_after_rollback,
            },
        );
        let issuer = agent.issuer.clone();
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                gwt_core::operation_deadline::now() + std::time::Duration::from_secs(120),
            );
            let sessions = gwt_core::paths::gwt_sessions_dir();
            let cache = gwt_core::paths::gwt_cache_dir();
            let outcome = match &pending {
                ReadinessPending::Fresh(pending) => {
                    use super::continuation::FreshExecutionFinalizationOutcome::{
                        Retained, RolledBack,
                    };
                    let outcome = if !current.load(Ordering::Acquire) {
                        Retained
                    } else if let Some((detail, _)) = abort {
                        match super::launch_finalization::rollback_worker_fresh_execution(&sessions, pending) {
                            Ok(true) => RolledBack(detail),
                            _ if super::launch_finalization::pending_fresh_execution_activation_status(pending) == Some(true) =>
                                repair_fresh_readiness(pending, &active, &fence, &issuer, &sessions, &cache, &current),
                            _ => Retained,
                        }
                    } else if repair {
                        repair_fresh_readiness(
                            pending, &active, &fence, &issuer, &sessions, &cache, &current,
                        )
                    } else {
                        super::continuation::finalize_fresh_execution_launch(
                            pending,
                            Some(&active),
                            fence.token.as_deref(),
                            Some(&issuer),
                            nonce.as_deref(),
                            &sessions,
                            &cache,
                            &live_sessions,
                            || current.load(Ordering::Acquire),
                        )
                    };
                    ReadinessOutcome::Fresh(outcome)
                }
                ReadinessPending::Continue(pending) => {
                    use super::continuation::ContinueWorkFinalizationOutcome::{
                        Retained, RolledBack,
                    };
                    let outcome = if let Some((detail, _)) = abort {
                        if !current.load(Ordering::Acquire) {
                            Retained { failure: None }
                        } else {
                            match super::launch_finalization::rollback_worker_continue_work(&sessions, pending) {
                                Ok(true) => RolledBack { detail },
                                Ok(false) => super::continuation::finalize_continue_work_launch(
                                    pending, Some(&active), fence.token.as_deref(), Some(&issuer), Some(&pending.readiness_nonce),
                                    &sessions, &live_sessions, active_is_live, || current.load(Ordering::Acquire)),
                                Err(_) if super::launch_finalization::pending_execution_activation_status(pending) == Some(true) =>
                                    super::continuation::finalize_continue_work_launch(
                                        pending, Some(&active), fence.token.as_deref(), Some(&issuer), Some(&pending.readiness_nonce),
                                        &sessions, &live_sessions, active_is_live, || current.load(Ordering::Acquire)),
                                Err(error) => Retained { failure: Some(if error == "continuation candidate Session changed before exact rollback" {
                                    super::continuation::ContinueWorkFailure::conflict("The failed continuation candidate no longer matches its exact Agent identity.")
                                } else {
                                    super::continuation::ContinueWorkFailure::failed("continuation_reconciliation_required", "The failed continuation could not commit its exact cleanup; retry reconciliation.", true)
                                }) },
                            }
                        }
                    } else {
                        super::continuation::finalize_continue_work_launch(
                            pending,
                            Some(&active),
                            fence.token.as_deref(),
                            Some(&issuer),
                            nonce.as_deref(),
                            &sessions,
                            &live_sessions,
                            active_is_live,
                            || current.load(Ordering::Acquire),
                        )
                    };
                    ReadinessOutcome::Continue(outcome)
                }
            };
            let snapshot = pending.identity().and_then(|identity| {
                gwt_agent::Session::load(&sessions.join(format!("{}.toml", identity.session_id)))
                    .ok()
                    .filter(|snapshot| {
                        gwt_agent::SessionExecutionIdentity::from_session(snapshot)
                            .ok()
                            .flatten()
                            .as_ref()
                            == Some(&identity)
                    })
            });
            let _ = sender.send(OwnerMessage::ReadinessFinalized {
                fence,
                outcome,
                snapshot: snapshot.map(Box::new),
            });
        });
        true
    }
    fn readiness_finalized(
        &mut self,
        fence: ReadinessFence,
        outcome: ReadinessOutcome,
        snapshot: Option<gwt_agent::Session>,
    ) {
        if !self
            .readiness
            .get(&fence.id)
            .is_some_and(|job| job.fence == fence)
        {
            return;
        }
        let job = self
            .readiness
            .remove(&fence.id)
            .expect("checked exact readiness job");
        let exact = self.runtimes.get(&fence.id).is_some_and(|runtime| {
            runtime.incarnation == fence.incarnation
                && runtime.agent.as_ref().is_some_and(|agent| {
                    agent.capability_token == fence.token && job.pending.matches_agent(agent)
                })
        });
        let closing = self.stopping
            || self
                .closing
                .get(&fence.id)
                .is_some_and(|closing| closing.incarnation == fence.incarnation);
        if exact {
            if let Some(deadline) = self
                .runtimes
                .get_mut(&fence.id)
                .unwrap()
                .agent
                .as_mut()
                .unwrap()
                .readiness_deadline
                .as_mut()
            {
                let now = std::time::Instant::now();
                if deadline.at <= now {
                    deadline.at = now + deadline.watch.delay();
                }
            }
        }
        // Even Retained may follow an irreversible activation. Refresh only the
        // captured exact identity before an explicit deferred close uses it.
        let mut snapshot_current = false;
        if exact {
            if let Some(snapshot) = snapshot.filter(|snapshot| {
                gwt_agent::SessionExecutionIdentity::from_session(snapshot)
                    .ok()
                    .flatten()
                    == job.pending.identity()
            }) {
                self.runtimes
                    .get_mut(&fence.id)
                    .unwrap()
                    .agent
                    .as_mut()
                    .unwrap()
                    .snapshot = snapshot;
                snapshot_current = true;
            }
        }
        let readback = match &job.pending {
            ReadinessPending::Fresh(pending) => {
                super::continuation::fresh_execution_commit_readback_matches(
                    &pending.worktree_path,
                    pending.owner,
                    &pending.session_identity,
                )
            }
            ReadinessPending::Continue(pending) => {
                super::continuation::continue_work_commit_readback_matches(pending)
            }
        };
        let committed = exact
            && outcome.committed()
            && snapshot_current
            && readback
            && fence
                .token
                .as_deref()
                .is_some_and(|token| self.issuer.active_token_is_current(token, &fence.binding));
        #[cfg(any(test, feature = "test-gh-guard"))]
        if !committed || closing {
            let token_current = fence
                .token
                .as_deref()
                .is_some_and(|token| self.issuer.active_token_is_current(token, &fence.binding));
            eprintln!("owner readiness finalization refused: outcome_committed={}, exact={exact}, snapshot_current={snapshot_current}, readback={readback}, token_current={token_current}, closing={closing}",
                outcome.committed());
        }
        if committed {
            if let Some(agent) = self
                .runtimes
                .get_mut(&fence.id)
                .and_then(|runtime| runtime.agent.as_mut())
            {
                agent.monitor_ready = true;
            }
            self.commit_monitor_launch(&fence.id);
            let agent = self
                .runtimes
                .get_mut(&fence.id)
                .unwrap()
                .agent
                .as_mut()
                .unwrap();
            agent.readiness_deadline = None;
            self.runtimes.get_mut(&fence.id).unwrap().detail = None;
            let agent = self
                .runtimes
                .get_mut(&fence.id)
                .unwrap()
                .agent
                .as_mut()
                .unwrap();
            match &job.pending {
                ReadinessPending::Fresh(_) => agent.pending_fresh = None,
                ReadinessPending::Continue(_) => agent.pending_continue = None,
            }
        }
        for (operation_id, reply) in job.replies {
            let result = if committed && !closing {
                match &job.pending {
                    ReadinessPending::Fresh(pending) => {
                        super::continuation::fresh_execution_readiness_receipt(
                            pending,
                            operation_id,
                        )
                        .map(Some)
                    }
                    ReadinessPending::Continue(_) => {
                        Err(super::continuation::fresh_execution_readiness_conflict())
                    }
                }
            } else {
                Err(super::continuation::fresh_execution_readiness_conflict())
            };
            let _ = reply.send(result);
        }
        if exact {
            if let ReadinessPending::Continue(pending) = &job.pending {
                if committed {
                    self.continue_work_outcome(
                        pending,
                        super::continuation::CachedContinueWorkOutcome {
                            work_id: pending.work_id.clone(),
                            outcome: pending.outcome,
                            message: None,
                            error_code: None,
                            retryable: false,
                        },
                        true,
                    );
                } else if let Some(detail) = outcome.rolled_back_detail() {
                    self.continue_work_outcome(
                        pending,
                        super::continuation::CachedContinueWorkOutcome {
                            work_id: pending.work_id.clone(),
                            outcome: crate::ContinueWorkOutcomeKind::Failed,
                            message: Some(format!(
                                "Continue work launch failed before activation: {detail}"
                            )),
                            error_code: Some("launch_failed".into()),
                            retryable: true,
                        },
                        true,
                    );
                } else if let ReadinessOutcome::Continue(
                    super::continuation::ContinueWorkFinalizationOutcome::Retained {
                        failure: Some(failure),
                    },
                ) = &outcome
                {
                    self.continue_work_outcome(
                        pending,
                        super::continuation::CachedContinueWorkOutcome {
                            work_id: pending.work_id.clone(),
                            outcome: failure.outcome,
                            message: Some(failure.message.clone()),
                            error_code: Some(failure.code.into()),
                            retryable: failure.retryable,
                        },
                        false,
                    );
                }
            }
            if let Some(detail) = outcome.rolled_back_detail() {
                // The exact candidate is already rolled back. Preserve its
                // actual Monitor source until physical cleanup can settle it.
                let monitor_failure = self
                    .monitor_launch(&fence.id)
                    .map(|launch| (launch, detail.to_string()));
                let runtime = self.runtimes.get_mut(&fence.id).unwrap();
                runtime.rollback_monitor_failure = monitor_failure;
                if let Some(agent) = runtime.agent.take() {
                    if let Some(token) = agent.capability_token {
                        agent.issuer.revoke_token(&token);
                    }
                }
                let _ = self.events.send(OwnerEvent::LaunchFailed {
                    id: fence.id.clone(),
                    incarnation: fence.incarnation,
                    detail: detail.into(),
                });
                if job.teardown_after_rollback
                    && !self.deferred_readiness_closes.contains_key(&fence.id)
                {
                    if let Some(runtime) = self.runtimes.remove(&fence.id) {
                        let (reply, _) = oneshot::channel();
                        self.start_close(
                            fence.id.clone(),
                            runtime,
                            CloseCause::ExplicitClose,
                            reply,
                        );
                    }
                }
            }
            let _ = self.events.send(OwnerEvent::Invalidated);
        }
        self.resume_deferred_agent_frontend(&fence.id, fence.incarnation);
        self.resume_deferred_close(&fence.id, fence.incarnation);
    }
    fn continue_work_outcome(
        &mut self,
        pending: &super::launch_payload::PendingContinueWork,
        outcome: super::continuation::CachedContinueWorkOutcome,
        cacheable: bool,
    ) {
        self.publish_continue_outcome(
            &pending.client_id,
            &pending.operation_id,
            outcome,
            cacheable,
            true,
        );
    }

    fn refresh_registry_projection(&mut self) {
        if let Ok(registry) = &self.agent_registry {
            if let Ok(registry) = registry.try_lock() {
                self.registry_projection = registry.persisted().clone();
            }
        }
    }
    fn resume_agent_transport(&mut self) {
        let events = std::mem::take(&mut self.pending_transport);
        for event in events {
            self.handle_transport_event(event);
        }
    }
    fn dispatch_agent_frontend(
        &mut self,
        client_id: String,
        grant: crate::agent_capability::AgentCapabilityGrant,
        request: crate::agent_capability::AgentFrontendRequest,
    ) {
        use super::agent_dispatch::{
            AgentCloseCompletion, AgentDispatchCapture, FleetPhysicalProof, PaneProjection,
            PaneSnapshotInput,
        };
        use crate::agent_capability::AgentFrontendRequest;
        if self.stopping {
            self.clients.unregister_with_close_frame(
                &client_id,
                Some(crate::project_transport::AGENT_AUTHORITY_UNAVAILABLE_CLOSE),
            );
            return;
        }
        if let AgentFrontendRequest::Name { request_id, .. } = &request {
            if let Err(error) = self.initialize_shared_registry() {
                self.clients
                    .dispatch(vec![crate::project_transport::OutboundEvent::reply(
                        client_id,
                        crate::BackendEvent::PaneNameResult {
                            request_id: request_id.clone(),
                            ok: false,
                            agent_name: None,
                            error: Some(error),
                        },
                    )]);
                return;
            }
        }
        let target = match &request {
            AgentFrontendRequest::CloseWindow { id, .. }
            | AgentFrontendRequest::CloseFleetPane { id, .. }
            | AgentFrontendRequest::RecoverRestoredWindow { id, .. } => Some(id.clone()),
            _ => None,
        };
        if let Some((id, incarnation)) = target.as_ref().and_then(|id| {
            self.runtimes
                .get(id)
                .map(|runtime| (id.clone(), runtime.incarnation))
        }) {
            if self.readiness_inflight(&id, incarnation)
                || self
                    .runtimes
                    .get(&id)
                    .and_then(|runtime| runtime.agent.as_ref())
                    .is_some_and(|agent| agent.binding_repair_inflight)
            {
                self.deferred_agent_frontend
                    .entry((id, incarnation))
                    .or_default()
                    .push((client_id, grant, request));
                return;
            }
        }
        let registry = match &self.agent_registry {
            Ok(registry) => registry.clone(),
            Err(_) => {
                self.clients.unregister_with_close_frame(
                    &client_id,
                    Some(crate::project_transport::AGENT_AUTHORITY_UNAVAILABLE_CLOSE),
                );
                return;
            }
        };
        let rows = match registry.try_lock() {
            Ok(registry) => registry.persisted().clone(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.pending_transport
                    .push_back(TransportEvent::AgentFrontend {
                        client_id,
                        grant,
                        request,
                    });
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                self.clients.unregister_with_close_frame(
                    &client_id,
                    Some(crate::project_transport::AGENT_AUTHORITY_UNAVAILABLE_CLOSE),
                );
                return;
            }
        };
        let mut physical = HashMap::new();
        let mut generations = HashMap::new();
        let mut snapshots = Vec::new();
        let mut panes = Vec::new();
        let mut wire_rows = Vec::new();
        for row in rows.windows {
            let id = format!("{}::{}", self.project_key, row.id);
            let runtime = self.runtimes.get(&id);
            let mut wire = row.clone();
            if let Some(runtime) = runtime {
                wire.status = Self::runtime_status(runtime);
                wire.runtime_started_at_ms = Some(runtime.started_at_ms);
                if let Some(agent) = &runtime.agent {
                    wire.agent_name = agent.snapshot.agent_name.clone().or(wire.agent_name);
                    wire.session_id = Some(agent.snapshot.id.clone());
                }
                physical.insert(
                    id.clone(),
                    FleetPhysicalProof {
                        runtime_present: true,
                        child_pid: runtime.pty.process_id(),
                        child_started_at: runtime
                            .pty
                            .process_id()
                            .and_then(crate::process::host_process_start_time),
                    },
                );
                generations.insert(id.clone(), runtime.incarnation);
            }
            wire.is_pm = wire.session_id.as_deref().is_some_and(|session| {
                crate::pm_registry::session_is_registered_pm(
                    &crate::pm_registry::pm_prefs_path_for_repo_path(&self.root),
                    session,
                )
            });
            let failed_diagnostic = wire
                .session_id
                .as_deref()
                .and_then(|session| self.failed_launch_diagnostic_snapshot(&id, session));
            snapshots.push(PaneSnapshotInput {
                id: id.clone(),
                pane: runtime.map(|runtime| runtime.pane.clone()),
                cached_snapshot: None,
                launch_error_snapshot: failed_diagnostic
                    .as_ref()
                    .map(|(_, _, bytes)| bytes.clone()),
                launch_error_source: failed_diagnostic.map(|(_, session, _)| session),
            });
            super::agent_dispatch::qualify_window_projection(&mut wire, &self.project_key);
            wire_rows.push(wire.clone());
            panes.push(PaneProjection {
                project_root: self.root.clone(),
                tab_id: self.project_key.clone(),
                window: wire,
                active: runtime
                    .and_then(|runtime| runtime.agent.as_ref())
                    .map(|agent| agent.active.clone()),
            });
        }
        let close = target.and_then(|id| {
            if self.closing.contains_key(&id) {
                return None;
            }
            let row_exists = panes.iter().any(|pane| pane.window.id == id);
            if !row_exists {
                return None;
            }
            let runtime = self.runtimes.remove(&id);
            let incarnation = runtime.as_ref().map_or(0, |runtime| runtime.incarnation);
            self.closing.insert(
                id.clone(),
                ClosingRuntime {
                    incarnation,
                    pm_worktree: runtime.as_ref().is_some_and(runtime_uses_pm_worktree),
                },
            );
            let expected_session_id = panes
                .iter()
                .find(|pane| pane.window.id == id)
                .and_then(|pane| pane.window.session_id.clone());
            let mut runtime = runtime;
            if let Some(runtime) = runtime.as_mut() {
                self.capture_close_monitor_snapshot(&id, runtime);
                runtime.registry_cleanup = Some(super::agent_dispatch::OwnedWindowRecordCleanup {
                    registry: registry.clone(),
                    registry_path: crate::persistence::workspace_state_path(&self.root),
                    root: self.root.clone(),
                    project_key: self.project_key.clone(),
                    canonical_id: id.clone(),
                    expected_session_id: expected_session_id.clone(),
                });
            }
            Some(AgentCloseCompletion {
                id,
                runtime,
                accepted: false,
                result: Ok(()),
                expected_session_id,
                husk: None,
            })
        });
        let kind = if gwt_git::worktree::main_worktree_root(&self.root).is_ok() {
            crate::ProjectKind::Git
        } else {
            crate::ProjectKind::NonRepo
        };
        let running_agents = super::agent_dispatch::collect_running_agents(&wire_rows);
        let capture = AgentDispatchCapture {
            root: self.root.clone(),
            registry_path: crate::persistence::workspace_state_path(&self.root),
            sessions_dir: gwt_core::paths::gwt_sessions_dir(),
            issuer: self.issuer.clone(),
            clients: self.clients.clone(),
            panes,
            snapshots,
            workspace: crate::ProjectTabView {
                id: self.project_key.clone(),
                project_key: self.project_key.clone(),
                title: self
                    .root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                project_root: self.root.display().to_string(),
                project_scope: self.project_key.clone(),
                kind,
                workspace: crate::WorkspaceView {
                    windows: wire_rows,
                    work_items: Vec::new(),
                },
                running_agent_count: running_agents.len() as u32,
                running_agents,
            },
            physical,
            generations,
            writers: self.pty_writers.clone(),
            registry,
        };
        self.agent_dispatch_jobs += 1;
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let completion = capture.run(client_id, grant, request, close);
            if let Err(error) =
                sender.send(OwnerMessage::AgentDispatchFinished(Box::new(completion)))
            {
                if let OwnerMessage::AgentDispatchFinished(mut completion) = error.0 {
                    if let Some(close) = completion.close.as_mut() {
                        if let Some(runtime) = close.runtime.as_mut() {
                            let _ = close_runtime(&close.id, runtime, CloseCause::ExplicitClose);
                        }
                    }
                }
            }
        });
    }
    fn agent_dispatch_finished(
        &mut self,
        mut completion: super::agent_dispatch::AgentDispatchCompletion,
    ) {
        self.agent_dispatch_jobs -= 1;
        if let Some(close) = completion.close.take() {
            let incarnation = close
                .runtime
                .as_ref()
                .map_or(0, |runtime| runtime.incarnation);
            if close.accepted {
                if let Some(runtime) = close.runtime {
                    self.closed(
                        close.id,
                        incarnation,
                        Box::new(runtime),
                        CloseCause::ExplicitClose,
                        close.result,
                    );
                } else if close.result.is_ok() {
                    self.closing.remove(&close.id);
                } else if let Some(husk) = close.husk {
                    self.closing.remove(&close.id);
                    self.retirements.insert(
                        (close.id, 0),
                        Some(Retirement::StoppedWindow(Box::new(husk))),
                    );
                }
            } else if self
                .closing
                .get(&close.id)
                .is_some_and(|closing| closing.incarnation == incarnation)
            {
                self.closing.remove(&close.id);
                if let Some(runtime) = close.runtime {
                    if self.stopping {
                        let (reply, _) = oneshot::channel();
                        self.start_close(close.id, runtime, CloseCause::GuiLifetime, reply);
                    } else {
                        self.runtimes.insert(close.id, runtime);
                    }
                }
            }
        }
        if let Some(acceptance) = completion.acceptance.take() {
            let pending = acceptance.pending;
            let valid = self
                .runtimes
                .get(&pending.window_id)
                .is_some_and(|runtime| {
                    runtime.incarnation == pending.window_lifecycle_generation
                        && runtime
                            .agent
                            .as_ref()
                            .is_some_and(|agent| agent.snapshot.id == pending.session_id)
                });
            if valid {
                let ticket = pending.ticket.clone();
                let sender = self.sender.clone();
                let direct = crate::agent_capability::AgentSelfCloseDirectAcceptance::new(
                    acceptance.request_id,
                    pending.window_id.clone(),
                    ticket.clone(),
                    Arc::new(move |ticket| {
                        let _ = sender.send(OwnerMessage::CommitAgentSelfClose(ticket));
                    }),
                );
                self.pending_agent_self_closes
                    .insert(ticket.id().into(), pending);
                if let Err(direct) = acceptance.responder.send(direct) {
                    let ticket = direct.disarm();
                    self.pending_agent_self_closes.remove(ticket.id());
                    self.issuer.rollback_self_close(&ticket);
                }
            } else {
                self.issuer.rollback_self_close(&pending.ticket);
                acceptance.responder.refuse(
                    pending.window_id,
                    "pane changed before self-close acceptance".into(),
                );
            }
        }
        super::agent_dispatch::apply_dispatch_outcome(
            &self.clients,
            &completion.client_id,
            completion.outcome,
        );
        self.refresh_registry_projection();
        if completion.invalidated {
            let _ = self.events.send(OwnerEvent::Invalidated);
        }
        self.resume_agent_transport();
    }
    fn resume_deferred_agent_frontend(&mut self, id: &str, incarnation: u64) {
        for (client, grant, request) in self
            .deferred_agent_frontend
            .remove(&(id.into(), incarnation))
            .unwrap_or_default()
        {
            self.dispatch_agent_frontend(client, grant, request);
        }
    }
    fn commit_owner_agent_self_close(
        &mut self,
        ticket: crate::agent_capability::AgentSelfCloseCapabilityTicket,
    ) {
        let Some(pending) = self.pending_agent_self_closes.get(ticket.id()).cloned() else {
            return;
        };
        let valid = self
            .runtimes
            .get(&pending.window_id)
            .is_some_and(|runtime| {
                runtime.incarnation == pending.window_lifecycle_generation
                    && runtime
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.snapshot.id == pending.session_id)
            });
        if !valid {
            self.pending_agent_self_closes.remove(ticket.id());
            self.issuer.finish_self_close(&ticket);
            return;
        }
        let mut runtime = self
            .runtimes
            .remove(&pending.window_id)
            .expect("exact accepted self-close");
        if let Ok(registry) = &self.agent_registry {
            runtime.registry_cleanup = Some(super::agent_dispatch::OwnedWindowRecordCleanup {
                registry: registry.clone(),
                registry_path: crate::persistence::workspace_state_path(&self.root),
                root: self.root.clone(),
                project_key: self.project_key.clone(),
                canonical_id: pending.window_id.clone(),
                expected_session_id: Some(pending.session_id),
            });
        }
        let (reply, receive) = oneshot::channel();
        self.start_close(pending.window_id, runtime, CloseCause::ExplicitClose, reply);
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let _ = receive.blocking_recv();
            let _ = sender.send(OwnerMessage::AgentSelfCloseFinished(ticket));
        });
    }
    fn notify_board_decision(
        &mut self,
        entry_id: String,
        session_id: String,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let mut candidates = self.runtimes.iter().filter(|(_, runtime)| {
            Self::runtime_status(runtime) == crate::WindowProcessStatus::Idle
                && runtime.current.load(Ordering::Acquire)
                && runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.active.session_id == session_id)
        });
        let Some((id, runtime)) = candidates.next() else {
            let _ = reply.send(Ok(
                json!({"ok":false,"reason":"Session is not idle at this owner"}),
            ));
            return;
        };
        if candidates.next().is_some() {
            let _ = reply.send(Err(
                "Board notification Session has multiple runtimes".into()
            ));
            return;
        }
        let id = id.clone();
        let expected = self
            .pty_writers
            .read()
            .ok()
            .and_then(|writers| writers.get(&id).cloned())
            .filter(|entry| {
                Arc::ptr_eq(&entry.handle, &runtime.pty)
                    && Arc::ptr_eq(&entry.current, &runtime.current)
                    && entry.current.load(Ordering::Acquire)
            });
        let Some(expected) = expected else {
            let _ = reply.send(Err("Board notification has no exact current writer".into()));
            return;
        };
        let Some(ordinary_revision) = expected
            .handle
            .ordinary_input_revision()
            .filter(|_| !expected.handle.has_unsent_user_input())
        else {
            let _ = reply.send(Ok(
                json!({"ok":false,"reason":"Session has unsent user input"}),
            ));
            return;
        };
        let max_submit_attempts = if matches!(
            runtime.agent.as_ref().unwrap().active.agent_id.as_str(),
            "claude" | "codex"
        ) {
            2
        } else {
            1
        };
        let root = self.root.clone();
        let writers = self.pty_writers.clone();
        let sender = self.sender.clone();
        self.input_jobs += 1;
        spawn_owner_worker(move || {
            let result = super::pm_delivery::deliver_board_notification(
                &root,
                id,
                super::pm_delivery::BoardDecisionNotice {
                    entry_id,
                    session_id,
                },
                max_submit_attempts,
                expected,
                writers,
                ordinary_revision,
            );
            let _ = sender.send(OwnerMessage::BoardNotificationFinished { result, reply });
        });
    }

    fn update_owner_layout(
        &mut self,
        layout: crate::persistence::PersistedWindowRegistry,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let registry = match &self.agent_registry {
            Ok(registry) => registry.clone(),
            Err(error) => {
                let _ = reply.send(Err(error.clone()));
                return;
            }
        };
        self.agent_dispatch_jobs += 1;
        let sender = self.sender.clone();
        let root = self.root.clone();
        spawn_owner_worker(move || {
            let result = (|| {
                {
                    let mut registry = registry.lock().map_err(|error| error.to_string())?;
                    registry.merge_owner_layout(&layout);
                    crate::persistence::save_workspace_state_durable(
                        &crate::persistence::workspace_state_path(&root),
                        &registry.persistable_state(),
                    )
                    .map_err(|error| error.to_string())?;
                }
                super::agent_dispatch::backfill_owner_pane_names(
                    &root,
                    &gwt_core::paths::gwt_sessions_dir(),
                    &registry,
                )
            })();
            let _ = sender.send(OwnerMessage::OwnerLayoutCommitted { result, reply });
        });
    }
    fn close_stopped_owner_session(
        &mut self,
        session_id: String,
        expected: Box<gwt_agent::Session>,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        if expected.id != session_id
            || self.runtimes.values().any(|runtime| {
                runtime
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.snapshot.id == session_id)
            })
            || self.agent_operations.values().any(|operation| {
                operation.result.is_none() && operation.session_id.as_deref() == Some(&session_id)
            })
        {
            let _ = reply.send(Err("stopped Session identity is active or changed".into()));
            return;
        }
        let rows = self
            .registry_projection
            .windows
            .iter()
            .filter(|row| row.session_id.as_deref() == Some(&session_id))
            .collect::<Vec<_>>();
        if rows.len() != 1 {
            let _ = reply.send(Err(
                "stopped Session has no unique owner window record".into()
            ));
            return;
        }
        let id = format!("{}::{}", self.project_key, rows[0].id);
        if self.closing.contains_key(&id)
            || self
                .retirements
                .keys()
                .any(|(candidate, _)| candidate == &id)
        {
            let _ = reply.send(Err("stopped Session cleanup is already pending".into()));
            return;
        }
        let registry = match &self.agent_registry {
            Ok(registry) => registry.clone(),
            Err(error) => {
                let _ = reply.send(Err(error.clone()));
                return;
            }
        };
        let diagnostic_runtime = self
            .agent_operations
            .values()
            .filter(|operation| {
                operation.id == id
                    && operation.session_id.as_deref() == Some(session_id.as_str())
                    && !operation.current.load(Ordering::Acquire)
            })
            .filter_map(|operation| {
                let receipt = operation.result.as_ref()?.as_ref().ok()?;
                if receipt["closed"] != true
                    || receipt["diagnostic_exit"] != true
                    || receipt["id"].as_str() != Some(id.as_str())
                    || receipt["session_id"].as_str() != Some(session_id.as_str())
                    || receipt["incarnation"].as_u64() != Some(operation.incarnation)
                {
                    return None;
                }
                let proof = serde_json::from_value::<
                    crate::issue_monitor::IssueMonitorRuntimeStopRequest,
                >(receipt.get("diagnostic_close_runtime")?.clone())
                .ok()?;
                (proof.window_id == id
                    && proof.session_id == session_id
                    && proof.runtime_incarnation == operation.incarnation
                    && receipt["child_pid"].as_u64() == Some(u64::from(proof.child_pid))
                    && receipt["host_pid"].as_u64() == Some(u64::from(proof.host_pid))
                    && proof.host_pid == std::process::id()
                    && crate::process::host_process_start_time(proof.host_pid)
                        == Some(proof.host_started_at)
                    && gwt_agent::SessionExecutionIdentity::from_session(&expected)
                        .ok()
                        .flatten()
                        .as_ref()
                        == Some(&proof.execution_identity))
                .then_some(proof)
            })
            .max_by_key(|proof| proof.runtime_incarnation);
        if let Some(proof) = diagnostic_runtime {
            if self.runtimes.contains_key(&id)
                || self.pending.contains_key(&id)
                || self.agent_operations.values().any(|operation| {
                    operation.id == id
                        && (operation.result.is_none()
                            || operation.incarnation > proof.runtime_incarnation)
                })
            {
                let _ = reply.send(Err("stopped diagnostic has a replacement owner".into()));
                return;
            }
            let cleanup = super::agent_dispatch::OwnedWindowRecordCleanup {
                registry,
                registry_path: crate::persistence::workspace_state_path(&self.root),
                root: self.root.clone(),
                project_key: self.project_key.clone(),
                canonical_id: id.clone(),
                expected_session_id: Some(session_id),
            };
            self.start_retirement(
                id,
                proof.runtime_incarnation,
                Retirement::SwitchStoppedWindow {
                    proof: Box::new(proof),
                    exit: StoppedOwnerWindowExit::DiagnosticClosed(expected),
                    cleanup: Box::new(cleanup),
                },
                Some(reply),
                None,
            );
            return;
        }
        let capture = super::agent_dispatch::StoppedWindowRetirement {
            sessions_dir: gwt_core::paths::gwt_sessions_dir(),
            expected: Some(*expected),
            cleanup: super::agent_dispatch::OwnedWindowRecordCleanup {
                registry,
                registry_path: crate::persistence::workspace_state_path(&self.root),
                root: self.root.clone(),
                project_key: self.project_key.clone(),
                canonical_id: id.clone(),
                expected_session_id: Some(session_id),
            },
        };
        self.start_retirement(
            id,
            0,
            Retirement::StoppedWindow(Box::new(capture)),
            Some(reply),
            None,
        );
    }
    fn register_owner_writer(&self, id: &str) -> Result<(), String> {
        let Some(runtime) = self.runtimes.get(id) else {
            return Err("pane disappeared before writer registration".into());
        };
        let project_key = gwt_core::repo_hash::ProjectKey::parse(&self.project_key)
            .map_err(|error| error.to_string())?;
        let monitor_runtime = runtime.agent.as_ref().and_then(|agent| {
            let snapshot = &agent.snapshot;
            if snapshot.launch_route != gwt_agent::LaunchRoute::Autonomous
                && !snapshot
                    .launch_args
                    .iter()
                    .any(|arg| arg.contains(crate::issue_monitor::ISSUE_MONITOR_LAUNCH_PROVENANCE))
            {
                return None;
            }
            Some(
                crate::monitor_duplicate_runtime::MonitorRuntimeRegistration {
                    window_id: id.into(),
                    session_id: snapshot.id.clone(),
                    issue_number: snapshot.linked_issue_number?,
                    worktree_path: agent.active.worktree_path.clone(),
                    project_root: self.root.clone(),
                    sessions_dir: gwt_core::paths::gwt_sessions_dir(),
                    incarnation: runtime.incarnation,
                    review_dispatch: snapshot.review_dispatch,
                },
            )
        });
        let entry = Arc::new(super::agent_dispatch::PtyWriterEntry {
            project_key,
            handle: runtime.pty.clone(),
            current: runtime.current.clone(),
            monitor_runtime,
            pm_delivery_replay: Mutex::new(None),
        });
        let mut writers = self
            .pty_writers
            .write()
            .map_err(|error| error.to_string())?;
        writers.insert(id.into(), entry);
        Ok(())
    }
    fn publish_shell_registry(
        &mut self,
        id: String,
        incarnation: u64,
        receipt: serde_json::Value,
        reply: Option<oneshot::Sender<OwnerReply>>,
        operation_id: Option<String>,
    ) {
        if !id.starts_with(&format!("{}::", self.project_key)) {
            self.finish_process_launch(operation_id.as_deref(), reply, Ok(receipt), false);
            return;
        }
        let Some(runtime) = self
            .runtimes
            .get(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
        else {
            self.finish_process_launch(
                operation_id.as_deref(),
                reply,
                Err("shell changed before registry publication".into()),
                false,
            );
            return;
        };
        let registry = match &self.agent_registry {
            Ok(registry) => registry.clone(),
            Err(error) => {
                let detail = error.clone();
                let runtime = self
                    .runtimes
                    .remove(&id)
                    .expect("captured failed shell publication");
                self.close_failed_process_launch(id, runtime, operation_id, reply, detail);
                return;
            }
        };
        self.agent_dispatch_jobs += 1;
        let root = self.root.clone();
        let registry_path = crate::persistence::workspace_state_path(&self.root);
        let cleanup_project_key = self.project_key.clone();
        let current = runtime.current.clone();
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let mut cleanup = None;
            let result = (|| {
                let raw = id
                    .strip_prefix(&format!(
                        "{}::",
                        gwt_core::paths::resolve_project_scope(&root).hash.as_str()
                    ))
                    .filter(|raw| !raw.is_empty())
                    .ok_or("shell is outside fixed project identity")?;
                let expected = registry
                    .lock()
                    .map_err(|error| error.to_string())?
                    .window(raw)
                    .cloned();
                if expected.as_ref().is_some_and(|row| {
                    row.session_id.is_some() || row.preset != crate::WindowPreset::Shell
                }) {
                    return Err("shell cannot replace an existing owner Session".into());
                }
                let row = super::agent_dispatch::owned_process_window_state(
                    &root,
                    &id,
                    None,
                    None,
                    None,
                    expected.as_ref(),
                )?;
                let capture = super::agent_dispatch::OwnedWindowRecordCleanup {
                    registry: registry.clone(),
                    registry_path: registry_path.clone(),
                    root: root.clone(),
                    project_key: cleanup_project_key.clone(),
                    canonical_id: id.clone(),
                    expected_session_id: None,
                };
                match super::agent_dispatch::publish_owned_process_window(
                    &registry,
                    &root,
                    &id,
                    expected.as_ref(),
                    row,
                    &current,
                ) {
                    Ok(()) => {
                        cleanup = Some(capture);
                        Ok(())
                    }
                    Err(error) => {
                        if error.record_changed {
                            cleanup = Some(capture);
                        }
                        Err(error.detail)
                    }
                }
            })();
            let _ = sender.send(OwnerMessage::ShellRegistryPublished(Box::new(
                ShellRegistryPublication {
                    id,
                    incarnation,
                    cleanup,
                    result,
                    receipt,
                    reply,
                    operation_id,
                },
            )));
        });
    }
    fn shell_registry_published(&mut self, publication: ShellRegistryPublication) {
        let ShellRegistryPublication {
            id,
            incarnation,
            cleanup,
            result,
            receipt,
            reply,
            operation_id,
        } = publication;
        self.agent_dispatch_jobs -= 1;
        if let Some(runtime) = self
            .runtimes
            .get_mut(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
        {
            if let Some(cleanup) = cleanup {
                runtime.registry_cleanup = Some(cleanup);
            }
            match result {
                Ok(()) if !self.stopping && runtime.current.load(Ordering::Acquire) => {
                    self.finish_process_launch(operation_id.as_deref(), reply, Ok(receipt), false);
                }
                result => {
                    let runtime = self
                        .runtimes
                        .remove(&id)
                        .expect("exact shell registry completion");
                    self.close_failed_process_launch(
                        id,
                        runtime,
                        operation_id,
                        reply,
                        result
                            .err()
                            .unwrap_or_else(|| "owner stopped before shell registry commit".into()),
                    );
                }
            }
        } else {
            self.finish_process_launch(
                operation_id.as_deref(),
                reply,
                Err("shell closed before registry publication acknowledgement".into()),
                false,
            );
        }
        self.refresh_registry_projection();
        self.resume_agent_transport();
    }

    fn handle_transport_event(&mut self, event: TransportEvent) {
        match event {
            TransportEvent::AgentFrontend {
                client_id,
                grant,
                request,
            } => self.dispatch_agent_frontend(client_id, grant, request),
            TransportEvent::ClientPaneSnapshotRepair {
                client_id,
                pane_ids,
            } => {
                let inputs = pane_ids
                    .into_iter()
                    .filter_map(|id| {
                        self.runtimes.get(&id).map(|runtime| {
                            super::agent_dispatch::PaneSnapshotInput {
                                id,
                                pane: Some(runtime.pane.clone()),
                                cached_snapshot: None,
                                launch_error_snapshot: None,
                                launch_error_source: None,
                            }
                        })
                    })
                    .collect();
                self.agent_dispatch_jobs += 1;
                let sender = self.sender.clone();
                spawn_owner_worker(move || {
                    let events =
                        super::agent_dispatch::pane_snapshot_repair_events(&client_id, inputs);
                    let _ = sender.send(OwnerMessage::AgentDispatchFinished(Box::new(
                        super::agent_dispatch::AgentDispatchCompletion {
                            client_id,
                            outcome:
                                super::agent_frontend::AgentFrontendDispatchOutcome::Dispatched(
                                    events,
                                ),
                            close: None,
                            acceptance: None,
                            invalidated: false,
                        },
                    )));
                });
            }
            TransportEvent::FreshExecutionReadyResend {
                grant,
                request,
                reply,
            } => self.resend_fresh_readiness(grant, request, reply),
            TransportEvent::RuntimeHook(event) => self.runtime_hook(event),
            TransportEvent::WorkspaceProjectionChanged { project_root } => {
                if gwt_core::paths::resolve_project_scope(&project_root)
                    .hash
                    .as_str()
                    == self.project_key
                {
                    let _ = self.events.send(OwnerEvent::Invalidated);
                }
            }
            event => self.pending_transport.push_back(event),
        }
    }
    fn runtime_hook(&mut self, mut event: crate::RuntimeHookEvent) {
        if self.stopping {
            return;
        }
        if event.project_root.as_deref().is_some_and(|root| {
            gwt_core::paths::resolve_project_scope(std::path::Path::new(root))
                .hash
                .as_str()
                != self.project_key
        }) {
            return;
        }
        let session_start = event.source_event.as_deref() == Some("SessionStart");
        if session_start && event.gwt_session_id.is_none() {
            return;
        }
        if let Some(operation) = self.agent_operations.values_mut().find(|operation| {
            operation.result.is_none()
                && operation
                    .session_id
                    .as_deref()
                    .is_some_and(|session| event.gwt_session_id.as_deref() == Some(session))
        }) {
            operation
                .early_events
                .push_back(TransportEvent::RuntimeHook(event));
            return;
        }
        let id = [
            event.gwt_session_id.as_deref(),
            event.agent_session_id.as_deref().filter(|_| !session_start),
        ]
        .into_iter()
        .flatten()
        .find_map(|session| {
            self.runtimes.iter().find_map(|(id, runtime)| {
                runtime
                    .agent
                    .as_ref()
                    .filter(|agent| agent.active.session_id == session)
                    .map(|_| id.clone())
            })
        });
        let readiness_nonce = event.continuation_readiness_nonce.take();
        if event.kind != crate::RuntimeHookEventKind::RuntimeState {
            self.dispatch_project(
                crate::BackendEvent::RuntimeHookEvent {
                    event: event.clone(),
                },
                None,
            );
            let _ = self.events.send(OwnerEvent::Hook {
                event: event.clone(),
            });
        }
        let Some(id) = id else {
            return;
        };
        if self.stopping || self.closing.contains_key(&id) {
            return;
        }
        if event.source_event.as_deref() == Some("UserPromptSubmit") {
            let captured = self.runtimes.get(&id).and_then(|runtime| {
                let session = runtime.agent.as_ref()?.active.session_id.clone();
                let entry = self.pty_writers.read().ok()?.get(&id).cloned()?;
                (Arc::ptr_eq(&entry.current, &runtime.current)
                    && Arc::ptr_eq(&entry.handle, &runtime.pty))
                .then_some((session, entry))
            });
            if let Some((session, entry)) = captured {
                self.input_jobs += 1;
                let root = self.root.clone();
                let sender = self.sender.clone();
                let window_id = id.clone();
                spawn_owner_worker(move || {
                    let result = super::pm_delivery::release_acknowledged_wake(
                        &root, &window_id, &session, &entry, None,
                    )
                    .map(|_| ());
                    let _ = sender.send(OwnerMessage::PmWakeAcknowledged { result });
                });
            }
        }
        if session_start {
            self.begin_readiness(&id, readiness_nonce, false, None);
        }
        let hook_status = crate::window_state::runtime_hook_window_state(&event);
        let progress = matches!(
            event.source_event.as_deref(),
            Some("UserPromptSubmit" | "PreToolUse" | "PostToolUse")
        );
        if hook_status.is_none() && !progress {
            return;
        }
        let now = chrono::Utc::now();
        let runtime = self.runtimes.get_mut(&id).expect("matched exact Session");
        let effective_before = Self::runtime_status(runtime);
        if progress {
            if let Some(agent) = runtime.agent.as_mut() {
                agent.monitor_startup_at = None;
            }
        }
        let publish_heartbeat = runtime
            .policy
            .last_heartbeat
            .is_none_or(|last| (now - last).num_seconds() >= 60);
        if publish_heartbeat {
            runtime.policy.last_heartbeat = Some(now);
        }
        if publish_heartbeat {
            self.runtime_monitor_heartbeat(
                &id,
                event.source_event.as_deref(),
                Some(&event.occurred_at),
            );
        }
        let Some(hook_status) = hook_status else {
            return;
        };
        let runtime = self.runtimes.get_mut(&id).expect("matched exact Session");
        runtime.policy.approval = None;
        runtime.policy.approval_settle_at = None;
        runtime.policy.recoverable_error = false;
        runtime.policy.quota_hold = None;
        runtime.policy.quota_candidate_since = None;
        runtime.policy.released_quota_notice = None;
        runtime.hook_status = Some(hook_status);
        if crate::window_state::is_live_agent_hook_state(hook_status) {
            runtime.detail = None;
        } else if event
            .message
            .as_deref()
            .is_some_and(|detail| !detail.trim().is_empty())
        {
            runtime.detail = event.message.clone();
        }
        let incarnation = runtime.incarnation;
        let status = Self::runtime_status(runtime);
        let detail = runtime.detail.clone();
        self.observe_policy_capacity_hook(&id, hook_status);
        if status == crate::WindowProcessStatus::Error {
            self.runtime_monitor_failure(
                &id,
                detail.as_deref().unwrap_or("Agent entered error state"),
                None,
                false,
            );
        }
        // A hook has no physical incarnation and cannot retire a same-Session replacement.
        if effective_before != status {
            self.publish_status(id, incarnation, status, detail, false);
        }
    }
    fn policy_capacity_target(
        &self,
        window_id: &str,
    ) -> Option<(
        crate::monitor_duplicate_runtime::RegisteredMonitorRuntime,
        gwt_agent::SessionExecutionBinding,
    )> {
        let runtime = self.runtimes.get(window_id)?;
        let agent = runtime.agent.as_ref()?;
        let active = &agent.active;
        let writers = self.pty_writers.read().ok()?;
        let entry = writers.get(window_id)?;
        let registration = entry.monitor_runtime.as_ref()?;
        if active.agent_id != "codex"
            || active.runtime_target != gwt_agent::LaunchRuntimeTarget::Host
            || registration.review_dispatch
            || registration.window_id != window_id
            || registration.session_id != active.session_id
            || registration.incarnation != runtime.incarnation
            || registration.worktree_path != active.worktree_path
            || !Arc::ptr_eq(&entry.handle, &runtime.pty)
        {
            return None;
        }
        let binding = agent
            .issuer
            .active_execution_binding_for_token(agent.capability_token.as_ref()?)?;
        if binding.session_id != active.session_id
            || binding.owner_number != registration.issue_number
        {
            return None;
        }
        Some((
            crate::monitor_duplicate_runtime::RegisteredMonitorRuntime {
                registration: registration.clone(),
                handle: entry.handle.clone(),
            },
            binding,
        ))
    }

    fn cancel_policy_capacity_resume(&mut self, window_id: &str) {
        for resume in self
            .policy_capacity_resumes
            .values_mut()
            .filter(|resume| resume.window_id == window_id)
        {
            resume.permit.store(false, Ordering::Release);
            resume.due = None;
        }
    }

    fn observe_policy_capacity_hook(&mut self, window_id: &str, state: WindowProcessStatus) {
        // A trailing Stop is Idle, not renewed activity. Keep its pending
        // retry while Waiting and terminal hook states still revoke it.
        if state == WindowProcessStatus::Idle {
            return;
        }
        self.cancel_policy_capacity_resume(window_id);
        if state != WindowProcessStatus::Running {
            return;
        }
        let Some((target, binding)) = self.policy_capacity_target(window_id) else {
            return;
        };
        let resume = self
            .policy_capacity_resumes
            .entry(binding.session_id.clone())
            .or_insert_with(|| super::runtime_events::ProviderCapacityResume {
                window_id: window_id.to_string(),
                incarnation: target.registration.incarnation,
                binding: binding.clone(),
                due: None,
                input_revision: None,
                attempted: false,
                permit: Arc::new(AtomicBool::new(false)),
            });
        resume.window_id = window_id.to_string();
        resume.incarnation = target.registration.incarnation;
        resume.binding = binding;
    }

    fn arm_policy_capacity_resume(&mut self, window_id: &str) {
        if self
            .runtimes
            .get(window_id)
            .is_some_and(|runtime| runtime.policy.approval.is_some())
            || self
                .runtimes
                .get(window_id)
                .is_some_and(|runtime| runtime.policy.quota_hold.is_some())
            || !self
                .runtimes
                .get(window_id)
                .and_then(|runtime| runtime.policy.api_error.as_ref())
                .is_some_and(|error| {
                    error.transient && error.summary.starts_with("Selected model is at capacity")
                })
        {
            return;
        }
        let Some((target, binding)) = self.policy_capacity_target(window_id) else {
            return;
        };
        let Some(resume) = self.policy_capacity_resumes.get_mut(&binding.session_id) else {
            return;
        };
        if resume.attempted
            || resume.incarnation != target.registration.incarnation
            || resume.binding != binding
        {
            return;
        }
        resume.due = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        resume.input_revision = target.handle.ordinary_input_revision();
        resume.permit = Arc::new(AtomicBool::new(true));
    }

    /// The existing convergence clock schedules a single revocable worker;
    /// no sleep, Session read or execution lease runs on the application loop.
    fn sweep_policy_capacity_resumes(&mut self, now: std::time::Instant) {
        let due: Vec<_> = self
            .policy_capacity_resumes
            .iter()
            .filter(|(_, resume)| !resume.attempted && resume.due.is_some_and(|due| now >= due))
            .map(|(session_id, resume)| (session_id.clone(), resume.window_id.clone()))
            .collect();
        for (session_id, window_id) in due {
            let Some((target, binding)) = self.policy_capacity_target(&window_id) else {
                self.cancel_policy_capacity_resume(&window_id);
                continue;
            };
            if session_id != binding.session_id
                || self.runtimes.get(&window_id).map(Self::runtime_status)
                    != Some(WindowProcessStatus::Waiting)
                || self
                    .runtimes
                    .get(&window_id)
                    .is_some_and(|runtime| runtime.policy.approval.is_some())
                || self
                    .runtimes
                    .get(&window_id)
                    .is_some_and(|runtime| runtime.policy.quota_hold.is_some())
                || !self
                    .runtimes
                    .get(&window_id)
                    .and_then(|runtime| runtime.policy.api_error.as_ref())
                    .is_some_and(|error| {
                        error.transient
                            && error.summary.starts_with("Selected model is at capacity")
                    })
            {
                self.cancel_policy_capacity_resume(&window_id);
                continue;
            }
            let Some(resume) = self.policy_capacity_resumes.get_mut(&session_id) else {
                continue;
            };
            if resume.incarnation != target.registration.incarnation
                || resume.binding != binding
                || target.handle.has_unsent_user_input()
                || resume.input_revision.is_none()
                || target.handle.ordinary_input_revision() != resume.input_revision
            {
                resume.permit.store(false, Ordering::Release);
                resume.due = None;
                continue;
            }
            // An ambiguous outcome consumes the same budget as a success.
            resume.attempted = true;
            resume.due = None;
            let permit = Arc::clone(&resume.permit);
            let input_revision = resume.input_revision;
            let writers = Arc::clone(&self.pty_writers);
            let Some(agent) = self
                .runtimes
                .get(&window_id)
                .and_then(|runtime| runtime.agent.as_ref())
            else {
                continue;
            };
            let issuer = agent.issuer.clone();
            let Some(token) = agent.capability_token.clone() else {
                continue;
            };
            #[cfg(any(test, feature = "test-gh-guard"))]
            let checkpoint =
                if self
                    .next_capacity_checkpoint
                    .as_ref()
                    .is_some_and(|(id, incarnation, _)| {
                        id == &window_id && *incarnation == target.registration.incarnation
                    })
                {
                    self.next_capacity_checkpoint
                        .take()
                        .map(|(_, _, checkpoint)| checkpoint)
                } else {
                    None
                };
            spawn_owner_worker(move || {
                #[cfg(any(test, feature = "test-gh-guard"))]
                let _checkpoint = checkpoint.map(|checkpoint| checkpoint.pause());
                let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                    gwt_core::operation_deadline::now() + std::time::Duration::from_secs(5),
                );
                let result = (|| -> Result<(), String> {
                    let registration = &target.registration;
                    let session = gwt_agent::Session::load(
                        &registration
                            .sessions_dir
                            .join(format!("{}.toml", registration.session_id)),
                    )
                    .map_err(|error| error.to_string())?;
                    if session.launch_route != gwt_agent::LaunchRoute::Autonomous
                        || session.agent_id != gwt_agent::AgentId::Codex
                        || session.execution_binding.as_ref() != Some(&binding)
                        || session.worktree_path != registration.worktree_path
                        || session.linked_issue_number != Some(registration.issue_number)
                    {
                        return Err("the autonomous Monitor Session changed".into());
                    }
                    let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)?
                        .ok_or("the producing Session is unbound")?;
                    let reservation = Arc::clone(&target.handle)
                        .reserve_input_transaction()
                        .map_err(|error| error.to_string())?;
                    let mut body_attempted = false;
                    // Physical submission is the only evidence here. Provider
                    // recovery and its prompt hook are observed independently.
                    let _unverified = super::agent_dispatch::drive_verified_pane_submit(
                        "[gwt] Automated retry after model capacity. Continue the current task within its existing authorization; permission and verification gates still apply.\r",
                        reservation.bracketed_paste_enabled(),
                        1,
                        |bytes| reservation.write_input_authorized(bytes, |commit| {
                            let fail = |details: &str| gwt_terminal::TerminalError::PtyIoError { details: details.into() };
                            gwt_core::operation_deadline::ensure_remaining("capacity retry input").map_err(|error| fail(&error.to_string()))?;
                            if !body_attempted && (!permit.load(Ordering::Acquire) || target.handle.has_unsent_user_input() || target.handle.ordinary_input_revision() != input_revision) {
                                return Err(fail("capacity retry cancelled by newer activity"));
                            }
                            let authorized = crate::cli::execution_state::with_current_active_session_execution_identity_lease(&registration.sessions_dir, &identity, || {
                                let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(&registration.project_root)).map_err(|error| fail(&error.to_string()))?;
                                if !prefs.enabled || !prefs.autonomous_mode || !issuer.active_token_is_current(&token, &binding) {
                                    return Err(fail("autonomous Monitor authority changed"));
                                }
                                let pm = crate::pm_registry::load_pm_prefs(&crate::pm_registry::pm_prefs_path_for_repo_path(&registration.project_root)).map_err(|error| fail(&error.to_string()))?;
                                if pm.settings.paused || pm.registration.as_ref().is_some_and(|pm| pm.session_id == registration.session_id) {
                                    return Err(fail("PM is paused or owns the target Session"));
                                }
                                let current = writers.read().map_err(|_| fail("PTY registry unavailable"))?;
                                if !current.get(&registration.window_id).is_some_and(|entry| Arc::ptr_eq(&entry.handle, &target.handle) && entry.monitor_runtime.as_ref().is_some_and(|current| current.session_id == registration.session_id && current.incarnation == registration.incarnation && current.issue_number == registration.issue_number && current.worktree_path == registration.worktree_path && current.project_root == registration.project_root && !current.review_dispatch)) {
                                    return Err(fail("the Monitor PTY was replaced"));
                                }
                                if body_attempted {
                                    commit(&mut || {})
                                } else {
                                    reservation.with_unchanged_ordinary_input(input_revision.expect("captured ordinary input revision"), || {
                                        if !permit.load(Ordering::Acquire) {
                                            return Err(fail("capacity retry cancelled by newer activity"));
                                        }
                                        commit(&mut || body_attempted = true)
                                    })
                                }
                            }).map_err(|error| fail(&error.to_string()))?;
                            authorized.ok_or_else(|| fail("the producing execution is no longer Active"))?
                        }).map_err(|error| error.to_string()),
                        thread::sleep,
                        || Ok(false),
                    )?;
                    Ok(())
                })();
                if let Err(error) = result {
                    tracing::warn!(%error, session_id = %binding.session_id, "capacity retry refused; existing waiting state and PM notification retained");
                }
            });
        }
    }

    fn policy_preset(runtime: &WindowRuntime) -> crate::WindowPreset {
        if runtime.agent.is_some() {
            crate::WindowPreset::Agent
        } else {
            crate::WindowPreset::Shell
        }
    }

    pub(crate) fn begin_policy_input(&mut self, id: &str, bytes: &[u8]) {
        self.cancel_policy_capacity_resume(id);
        if !bytes.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return;
        }
        if let Some(runtime) = self.runtimes.get_mut(id) {
            if let Some(latch) = runtime.policy.approval.as_mut() {
                latch.resolving_fingerprint = latch.active_fingerprint;
                latch.resolution_started = true;
                runtime.policy.approval_settle_at = None;
            }
        }
    }

    fn next_policy_deadline(&self) -> Option<std::time::Instant> {
        if self.stopping {
            return None;
        }
        let now = std::time::Instant::now();
        let wall = chrono::Utc::now();
        self.runtimes
            .values()
            .flat_map(|runtime| {
                let quota = runtime.policy.quota_candidate_since.map(|first| {
                    let remaining = (first + chrono::Duration::seconds(120) - wall)
                        .to_std()
                        .unwrap_or_default();
                    now + remaining
                });
                [runtime.policy.approval_settle_at, quota]
            })
            .flatten()
            .chain(
                self.policy_capacity_resumes
                    .values()
                    .filter_map(|resume| resume.due),
            )
            .min()
    }

    fn tick_runtime_policies(&mut self, now: std::time::Instant) {
        if self.stopping {
            return;
        }
        let wall = chrono::Utc::now();
        let due: Vec<_> = self
            .runtimes
            .iter()
            .filter(|(_, runtime)| {
                runtime
                    .policy
                    .approval_settle_at
                    .is_some_and(|due| now >= due)
                    || runtime
                        .policy
                        .quota_candidate_since
                        .is_some_and(|first| (wall - first).num_seconds() >= 120)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.observe_owner_screen_policy(&id, wall, now, None);
        }
        self.sweep_policy_capacity_resumes(now);
    }

    fn observe_owner_screen_policy(
        &mut self,
        id: &str,
        now: chrono::DateTime<chrono::Utc>,
        monotonic_now: std::time::Instant,
        captured_screen: Option<String>,
    ) {
        let Some(runtime) = self.runtimes.get_mut(id) else {
            return;
        };
        let before = Self::runtime_status(runtime);
        let preset = Self::policy_preset(runtime);
        let agent_id = runtime
            .agent
            .as_ref()
            .map(|agent| agent.active.agent_id.clone());
        let screen = captured_screen.or_else(|| {
            runtime
                .pane
                .lock()
                .ok()
                .map(|pane| pane.screen().contents())
        });
        let approval_settle_due = runtime
            .policy
            .approval_settle_at
            .is_some_and(|due| monotonic_now >= due);
        if approval_settle_due {
            runtime.policy.approval_settle_at = None;
            let provider = super::runtime_events::approval_provider(agent_id.as_deref());
            if screen.as_deref().is_some_and(|screen| {
                !crate::window_state::has_approval_prompt_evidence(provider, screen)
            }) {
                runtime.policy.approval = None;
            }
        }
        let api_before = runtime.policy.api_error.clone();
        let outcome = super::runtime_events::observe_screen_policy(
            &mut runtime.policy,
            super::runtime_events::ScreenPolicyInput {
                window_id: id,
                preset,
                agent_id: agent_id.as_deref(),
                screen: screen.as_deref(),
                accounts: &self.policy_usage_accounts,
                now,
                monotonic_now,
            },
        );
        if approval_settle_due {
            // Partial prompt evidence keeps Waiting until new output or input.
            // Consuming this deadline must not schedule another idle tick.
            runtime.policy.approval_settle_at = None;
        }
        if let Some(detail) = &outcome.detail {
            runtime.detail = Some(detail.clone());
        } else if outcome.clear_detail {
            runtime.detail = None;
        }
        let api_changed = api_before != runtime.policy.api_error;
        let cancel_retry =
            api_changed || runtime.policy.approval.is_some() || runtime.policy.quota_hold.is_some();
        let incarnation = runtime.incarnation;
        let status = Self::runtime_status(runtime);
        let approval_cleared_status = outcome.approval_reentered.then(|| {
            let mut policy = runtime.policy.clone();
            policy.approval = None;
            super::runtime_events::composed_policy_status(
                &policy,
                runtime.pty_status,
                preset,
                runtime.hook_status,
                runtime.agent.is_some(),
            )
        });
        let detail = runtime.detail.clone();
        if let Some(fingerprint) = outcome.permission_prompt {
            self.record_owner_permission_prompt(id, fingerprint);
        }
        if let Some(failure) = outcome.failure {
            let message = detail.clone().unwrap_or_else(|| match &failure {
                crate::IssueMonitorFailure::CodexDirectoryTrustPrompt => {
                    gwt_core::pane_health::CODEX_DIRECTORY_TRUST_PROMPT_REASON.into()
                }
                _ => "Agent requires provider recovery".into(),
            });
            self.runtime_monitor_failure(id, &message, Some(failure), false);
        }
        if cancel_retry {
            self.cancel_policy_capacity_resume(id);
        }
        if api_changed {
            self.arm_policy_capacity_resume(id);
        }
        if let Some(cleared) = approval_cleared_status {
            self.publish_status(id.to_string(), incarnation, cleared, detail.clone(), false);
        }
        if before != status
            || outcome.detail.is_some()
            || outcome.clear_detail
            || outcome.approval_reentered
        {
            self.publish_status(id.to_string(), incarnation, status, detail, false);
        }
    }

    fn record_owner_permission_prompt(&self, id: &str, fingerprint: u64) {
        let Some(agent) = self
            .runtimes
            .get(id)
            .and_then(|runtime| runtime.agent.as_ref())
        else {
            return;
        };
        if agent.snapshot.launch_route.is_attended() {
            return;
        }
        let worktree = agent.active.worktree_path.clone();
        let session_id = agent.active.session_id.clone();
        let agent_id = agent.active.agent_id.clone();
        let current = agent.current.clone();
        {
            if !current.load(Ordering::Acquire) {
                return;
            }
            let Ok(Some(record)) = crate::cli::execution_state::load(&worktree) else {
                return;
            };
            if record.primary_session_id != session_id
                || matches!(
                    crate::cli::permission_readiness::load(&worktree),
                    Ok(Some(_))
                )
            {
                return;
            }
            if let Err(error) = crate::cli::permission_readiness::record_prompt_regression(
                &worktree,
                record.owner_kind.as_str(),
                record.owner_number,
                &session_id,
                &agent_id,
                fingerprint,
            ) {
                tracing::warn!(%error, owner = record.owner_number, "could not record the permission prompt regression");
            }
        }
    }

    fn apply_policy_usage_snapshot(&mut self, accounts: Vec<gwt_core::usage::ProviderUsage>) {
        let now = chrono::Utc::now();
        let mut changed = Vec::new();
        for (id, runtime) in &mut self.runtimes {
            if super::runtime_events::release_policy_quota(&mut runtime.policy, &accounts, now) {
                runtime.detail = None;
                changed.push((
                    id.clone(),
                    runtime.incarnation,
                    Self::runtime_status(runtime),
                ));
            }
        }
        self.policy_usage_accounts = accounts;
        for (id, incarnation, status) in changed {
            self.publish_status(id, incarnation, status, None, false);
        }
        let ids: Vec<_> = self.runtimes.keys().cloned().collect();
        for id in ids {
            self.observe_owner_screen_policy(&id, now, std::time::Instant::now(), None);
        }
    }

    fn persist_owner_exit_receipt(&self, id: &str) {
        let Some(runtime) = self.runtimes.get(id) else {
            return;
        };
        let Some(agent) = &runtime.agent else {
            return;
        };
        let Some(exit) = runtime
            .pane
            .lock()
            .ok()
            .and_then(|pane| pane.last_exit().cloned())
        else {
            return;
        };
        let expected = agent.snapshot.clone();
        let incarnation = runtime.incarnation;
        let receipt = gwt_agent::SessionExitReceipt {
            exit_code: exit.exit_code,
            signal: exit.signal,
            exited_at: exit.observed_at.into(),
        };
        {
            let sessions = gwt_core::paths::gwt_sessions_dir();
            let result = gwt_agent::update_session_if_matches_with(
                &sessions,
                &expected,
                |current, expected| {
                    Ok(current.id == expected.id
                        && current.created_at == expected.created_at
                        && current.worktree_path == expected.worktree_path
                        && current.agent_id == expected.agent_id
                        && current.execution_binding == expected.execution_binding)
                },
                |session| {
                    let proof = gwt_agent::SessionRuntimeState::load(
                        &gwt_agent::runtime_state_path(&sessions, &session.id),
                    )?;
                    if proof.runtime_incarnation != Some(incarnation) {
                        return Err(std::io::Error::other(
                            "Session runtime changed before exit receipt",
                        ));
                    }
                    session.record_process_exit(&receipt);
                    Ok(())
                },
            );
            if let Err(error) = result {
                tracing::warn!(%error, session_id = %expected.id, "failed to persist exact owner exit receipt");
            }
        }
    }

    fn classify_owner_termination(
        &self,
        id: &str,
        exit_confirmed: bool,
    ) -> crate::IssueMonitorFailureClass {
        use crate::IssueMonitorFailureClass::{Agent, Infrastructure, Unknown};
        if !exit_confirmed {
            return Unknown;
        }
        let Some(runtime) = self.runtimes.get(id) else {
            return Unknown;
        };
        let Some(agent) = &runtime.agent else {
            return Unknown;
        };
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let Ok(Some(identity)) = gwt_agent::SessionExecutionIdentity::from_session(&agent.snapshot)
        else {
            return Unknown;
        };
        let path = gwt_agent::runtime_state_path(&sessions, &agent.snapshot.id);
        let Ok(proof) = gwt_agent::SessionRuntimeState::load(&path) else {
            return Unknown;
        };
        if proof.execution_identity.as_ref() != Some(&identity)
            || proof.runtime_incarnation != Some(runtime.incarnation)
        {
            return Unknown;
        }
        match gwt_agent::has_unresolved_host_bridge_fault(&path, &proof) {
            Ok(true) => Infrastructure,
            Ok(false) => Agent,
            Err(_) => Unknown,
        }
    }

    fn owner_status_event(
        &mut self,
        id: String,
        incarnation: u64,
        raw_status: crate::WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    ) {
        if self.exact(&id, incarnation).is_err() {
            return;
        }
        // Capture current ownership before terminal status revokes its writer.
        // Only the registered PM's actual nonzero exit can bypass the generic
        // Agent recovery latch; a read error or replacement grants no such cause.
        let pm_crash_candidate = exit_confirmed
            && raw_status == crate::WindowProcessStatus::Error
            && self.runtimes.get(&id).is_some_and(|runtime| {
                runtime.current.load(Ordering::Acquire)
                    && runtime
                        .pty
                        .try_wait()
                        .ok()
                        .flatten()
                        .is_some_and(|exit| exit.exit_code() != 0)
                    && runtime.agent.as_ref().is_some_and(|agent| {
                        crate::pm_registry::registered_pm_worktree_authority(
                            &runtime.project_root,
                            &agent.snapshot.id,
                            &agent.snapshot.worktree_path,
                        )
                    })
            });
        if matches!(
            raw_status,
            crate::WindowProcessStatus::Error | crate::WindowProcessStatus::Stopped
        ) {
            self.cancel_policy_capacity_resume(&id);
            let entry = {
                self.pty_writers
                    .read()
                    .ok()
                    .and_then(|writers| writers.get(&id).cloned())
            };
            if let Some(entry) = entry {
                if let Err(error) =
                    super::agent_dispatch::invalidate_current_writer(&self.pty_writers, &id, &entry)
                {
                    tracing::warn!(%error, window_id = %id, "terminal owner writer invalidation failed");
                }
            }
        }
        if exit_confirmed {
            self.persist_owner_exit_receipt(&id);
        }
        let runtime = self.runtimes.get_mut(&id).expect("exact owner status");
        let screen = runtime
            .pane
            .lock()
            .ok()
            .map(|pane| pane.screen().contents());
        let agent_id = runtime
            .agent
            .as_ref()
            .map(|agent| agent.active.agent_id.clone());
        // Preserve explicit recovery policy and provider prompts before the
        // shared classifier clears approval/hook state on terminal output.
        let pm_generic_crash = pm_crash_candidate
            && runtime.policy.approval.is_none()
            && runtime.policy.api_error.is_none()
            && screen.as_deref().is_some_and(|screen| {
                let evidence = format!("{}\n{screen}", detail.as_deref().unwrap_or_default());
                let provider = super::runtime_events::approval_provider(agent_id.as_deref());
                crate::window_state::approval_prompt_fingerprint(provider, &evidence).is_none()
                    && crate::window_state::directory_trust_prompt_fingerprint(provider, &evidence)
                        .is_none()
                    && gwt_core::usage::limit_notice::detect_provider_auth_notice(&evidence)
                        .is_none()
                    && gwt_core::usage::detect_provider_api_error(&evidence).is_none()
            });
        let preset = Self::policy_preset(runtime);
        let outcome = super::runtime_events::classify_exit(
            &mut runtime.policy,
            super::runtime_events::ExitPolicyInput {
                window_id: &id,
                preset,
                agent_id: agent_id.as_deref(),
                has_agent_session: runtime.agent.is_some(),
                raw_status,
                hook_state: runtime.hook_status,
                exit_confirmed,
                exact_incarnation: true,
                detail,
                final_screen: screen.as_deref(),
                accounts: &self.policy_usage_accounts,
                now: chrono::Utc::now(),
            },
        );
        runtime.pty_status = raw_status;
        if outcome.clear_hook {
            runtime.hook_status = None;
        }
        if exit_confirmed {
            runtime._initial_prompt_file = None;
        }
        runtime.detail = outcome.detail.clone();
        let pm_generic_crash = pm_generic_crash && outcome.quota_hold.is_none();
        let retire = pm_generic_crash
            || outcome.auto_close
            || (exit_confirmed
                && outcome.quota_hold.is_some()
                && matches!(
                    raw_status,
                    crate::WindowProcessStatus::Error | crate::WindowProcessStatus::Stopped
                ))
            || (exit_confirmed
                && !outcome.keep_recovery
                && matches!(
                    raw_status,
                    crate::WindowProcessStatus::Error | crate::WindowProcessStatus::Stopped
                ));
        if exit_confirmed && runtime.agent.is_some() {
            let failure = outcome
                .quota_hold
                .clone()
                .or_else(|| {
                    super::runtime_events::classify_issue_monitor_failure(
                        outcome.detail.as_deref().unwrap_or_default(),
                        runtime.agent.as_ref().expect("agent").snapshot.session_mode,
                    )
                })
                .or_else(|| {
                    Some(crate::IssueMonitorFailure::Termination {
                        classification: self.classify_owner_termination(&id, true),
                    })
                });
            let detail = outcome
                .detail
                .as_deref()
                .unwrap_or("Agent exited without completing the work");
            if retire {
                self.runtimes
                    .get_mut(&id)
                    .and_then(|runtime| runtime.agent.as_mut())
                    .expect("exact terminating agent")
                    .monitor_terminal_failure = Some((detail.into(), failure));
            } else {
                self.runtime_monitor_failure(&id, detail, failure, true);
            }
        }
        if exit_confirmed && outcome.status == crate::WindowProcessStatus::Error {
            if let Some(launch) = self.monitor_launch(&id) {
                let _ = self.events.send(OwnerEvent::Notice {
                    level: "error".into(),
                    message: outcome
                        .detail
                        .clone()
                        .unwrap_or_else(|| "Agent exited without completing the work".into()),
                    issue_number: launch.feedback.issue_monitor_launch_binding_issue_number(),
                });
            }
        }
        if retire {
            let runtime = self
                .runtimes
                .remove(&id)
                .expect("exact terminal owner runtime");
            let (reply, _) = oneshot::channel();
            let cause = if pm_generic_crash {
                CloseCause::PmGenericCrash
            } else if outcome.auto_close {
                CloseCause::NaturalExit
            } else {
                CloseCause::DiagnosticExit
            };
            self.start_close(id, runtime, cause, reply);
        } else {
            self.publish_status(
                id,
                incarnation,
                outcome.status,
                outcome.detail,
                exit_confirmed,
            );
        }
    }

    fn runtime_status(runtime: &WindowRuntime) -> crate::WindowProcessStatus {
        super::runtime_events::composed_policy_status(
            &runtime.policy,
            runtime.pty_status,
            Self::policy_preset(runtime),
            runtime.hook_status,
            runtime.agent.is_some(),
        )
    }
    fn dispatch_project(&self, event: crate::BackendEvent, seq: Option<u64>) {
        if let Ok(key) = gwt_core::repo_hash::ProjectKey::parse(&self.project_key) {
            self.clients
                .dispatch(vec![crate::project_transport::OutboundEvent::project(
                    key, event,
                )
                .with_terminal_stream_seq(seq)]);
        }
    }
    fn persist_registry_status(
        &mut self,
        id: &str,
        expected_session: Option<&str>,
        status: crate::WindowProcessStatus,
    ) -> Result<(), String> {
        let Some(raw) = id.strip_prefix(&format!("{}::", self.project_key)) else {
            return Ok(());
        };
        let registry = self.agent_registry.as_ref().map_err(Clone::clone)?;
        let mut registry = registry.lock().map_err(|error| error.to_string())?;
        let Some(row) = registry.window(raw) else {
            return Ok(());
        };
        if row.session_id.as_deref() != expected_session {
            return Err("owner registry Session changed before terminal status".into());
        }
        registry.set_status(raw, status);
        crate::persistence::save_workspace_state_durable(
            &crate::persistence::workspace_state_path(&self.root),
            &registry.persistable_state(),
        )
        .map_err(|error| error.to_string())?;
        self.registry_projection = registry.persisted().clone();
        Ok(())
    }

    fn publish_status(
        &mut self,
        id: String,
        incarnation: u64,
        status: crate::WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    ) {
        if let Some(runtime) = self
            .runtimes
            .get_mut(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
        {
            if detail.is_some() {
                runtime.detail = detail.clone();
            }
        }
        let expected_session = self
            .runtimes
            .get(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
            .and_then(|runtime| runtime.agent.as_ref())
            .map(|agent| agent.snapshot.id.clone());
        if let Err(error) = self.persist_registry_status(&id, expected_session.as_deref(), status) {
            tracing::warn!(%error, window_id = %id, "owner status registry persistence failed");
        }
        self.dispatch_project(
            crate::BackendEvent::TerminalStatus {
                id: id.clone(),
                status,
                detail: detail.clone(),
                error_code: None,
                retryable: None,
            },
            None,
        );
        let (raw_status, exit_code) = self
            .runtimes
            .get(&id)
            .filter(|runtime| runtime.incarnation == incarnation)
            .map(|runtime| {
                (
                    runtime.pty_status,
                    if exit_confirmed {
                        runtime
                            .pane
                            .lock()
                            .ok()
                            .and_then(|pane| pane.last_exit().map(|exit| exit.exit_code))
                    } else {
                        None
                    },
                )
            })
            .unwrap_or((status, None));
        let _ = self.events.send(OwnerEvent::Status {
            id,
            incarnation,
            status,
            detail,
            exit_confirmed,
            raw_status,
            exit_code,
        });
    }
    fn pane_event(&mut self, event: PaneRuntimeEvent) {
        let projected = match event {
            PaneRuntimeEvent::Output {
                id,
                incarnation,
                data,
                seq,
                screen,
                screen_ansi,
            } if self.exact(&id, incarnation).is_ok() => {
                self.runtimes
                    .get_mut(&id)
                    .expect("exact output pane")
                    .output_bytes = self.runtimes[&id]
                    .output_bytes
                    .saturating_add(data.len() as u64);
                self.runtimes.get_mut(&id).unwrap().last_output_at =
                    Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                use base64::Engine;
                self.dispatch_project(
                    crate::BackendEvent::TerminalOutput {
                        id: id.clone(),
                        data_base64: base64::engine::general_purpose::STANDARD.encode(&data),
                    },
                    Some(seq),
                );
                // The reader captured these together while parsing this seq.
                // Re-locking its hot Pane here can starve the owner mailbox.
                let (screen, screen_ansi, screen_seq) = match (screen, screen_ansi) {
                    (Some(screen), Some(screen_ansi)) => (screen, screen_ansi, seq),
                    _ => {
                        let pane = self.runtimes[&id]
                            .pane
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        (
                            pane.screen().contents(),
                            pane.screen().contents_formatted(),
                            pane.output_seq(),
                        )
                    }
                };
                self.observe_owner_screen_policy(
                    &id,
                    chrono::Utc::now(),
                    std::time::Instant::now(),
                    Some(screen.clone()),
                );
                OwnerEvent::Output {
                    id,
                    incarnation,
                    data,
                    seq,
                    screen,
                    screen_ansi,
                    screen_seq,
                }
            }
            PaneRuntimeEvent::Status {
                id,
                incarnation,
                status,
                detail,
                exit_confirmed,
            } if self.exact(&id, incarnation).is_ok() => {
                self.owner_status_event(id, incarnation, status, detail, exit_confirmed);
                return;
            }
            _ => return,
        };
        let _ = self.events.send(projected);
    }
}
#[allow(clippy::too_many_arguments)]
impl OwnerState {
    fn monitor_launch(&self, id: &str) -> Option<super::monitor::MonitorLaunch> {
        let runtime = self.runtimes.get(id)?;
        let agent = runtime.agent.as_ref()?;
        monitor_launch_for_agent(id, runtime.incarnation, agent)
    }

    fn is_failed_monitor_runtime(runtime: &WindowRuntime, issue_number: u64) -> bool {
        runtime.agent.as_ref().is_some_and(|agent| {
            agent.snapshot.id == agent.active.session_id
                && agent.snapshot.linked_issue_number == Some(issue_number)
                && !agent.snapshot.review_dispatch
                && agent.feedback.as_ref().is_some_and(|feedback| {
                    feedback.issue_monitor_launch_binding_issue_number() == Some(issue_number)
                })
                && matches!(
                    Self::runtime_status(runtime),
                    crate::WindowProcessStatus::Error | crate::WindowProcessStatus::Stopped
                )
        })
    }

    fn deliver_owner_answer(
        &mut self,
        id: String,
        incarnation: u64,
        issue_number: u64,
        delivery_id: Option<String>,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let capture = (|| {
            let runtime = self.exact(&id, incarnation)?;
            let agent = runtime
                .agent
                .as_ref()
                .ok_or("autonomous answer requires an actual owner Agent Session")?;
            if !agent.current.load(Ordering::Acquire)
                || agent.snapshot.linked_issue_number != Some(issue_number)
            {
                return Err("autonomous answer Session/Issue ownership changed".to_string());
            }
            if agent.monitor_answer_inflight {
                return Err("autonomous answer delivery is already in flight".into());
            }
            let writer = self
                .pty_writers
                .read()
                .map_err(|error| error.to_string())?
                .get(&id)
                .filter(|entry| {
                    Arc::ptr_eq(&entry.current, &runtime.current)
                        && Arc::ptr_eq(&entry.handle, &runtime.pty)
                })
                .cloned()
                .ok_or("autonomous answer has no current physical writer")?;
            let mut feedback =
                agent
                    .feedback
                    .clone()
                    .unwrap_or(super::launch_payload::LaunchFeedbackContext {
                        client_id: "__issue_monitor__".into(),
                        title: "Issue Monitor".into(),
                        issue_monitor_issue_number: Some(issue_number),
                        issue_monitor_delivery_id: delivery_id.clone(),
                        issue_monitor_project_root: Some(self.root.clone()),
                        issue_monitor_session_mode: Some(agent.snapshot.session_mode),
                        issue_monitor_autonomous_handoff: None,
                        issue_monitor_autonomous_submit_started: false,
                        issue_monitor_review_dispatch: false,
                    });
            if feedback.issue_monitor_review_dispatch
                || feedback.issue_monitor_launch_binding_issue_number() != Some(issue_number)
            {
                return Err("autonomous answer cannot target a review or another Issue".into());
            }
            feedback.issue_monitor_delivery_id = delivery_id;
            let source = agent.monitor_source.clone().filter(|source| {
                    matches!(source,
                        crate::runtime_daemon_events::IssueMonitorAgentFailureSource::Bound { identity, .. }
                        if identity.delivery_id == feedback.issue_monitor_delivery_id)
                });
            Ok((
                super::monitor::MonitorLaunch {
                    root: self.root.clone(),
                    id: id.clone(),
                    incarnation,
                    session_id: agent.active.session_id.clone(),
                    agent_id: agent.active.agent_id.clone(),
                    materializer_id: self.host_instance_id.clone(),
                    feedback,
                    current: agent.current.clone(),
                    source,
                },
                agent.snapshot.clone(),
                agent.issuer.clone(),
                agent.capability_token.clone(),
                writer,
            ))
        })();
        let (launch, session, issuer, token, writer) = match capture {
            Ok(capture) => capture,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let snapshot = self.owner_monitor_window_snapshot(&id);
        self.begin_policy_input(&id, b"\r");
        let agent = self.runtimes.get_mut(&id).unwrap().agent.as_mut().unwrap();
        agent.feedback = Some(launch.feedback.clone());
        agent.monitor_owner = Some((launch.root.clone(), launch.materializer_id.clone()));
        agent.monitor_source = launch.source.clone();
        agent.monitor_answer_inflight = true;
        self.input_jobs += 1;
        let writers = self.pty_writers.clone();
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let outcome = super::monitor::deliver_existing_answer(
                launch, session, issuer, token, writers, writer, snapshot,
            );
            let _ = sender.send(OwnerMessage::AutonomousAnswerFinished {
                id,
                incarnation,
                outcome,
                reply,
            });
        });
    }

    fn acknowledge_owner_monitor(
        &mut self,
        command: OwnerCommand,
        reply: oneshot::Sender<OwnerReply>,
    ) {
        let (id, incarnation, issue_number, delivery_id, adopt) = match command {
            OwnerCommand::AcknowledgeMonitorLaunch {
                id,
                incarnation,
                issue_number,
                delivery_id,
            } => (id, incarnation, issue_number, delivery_id, false),
            OwnerCommand::AdoptMonitorLaunch {
                id,
                incarnation,
                issue_number,
                delivery_id,
            } => (id, incarnation, issue_number, delivery_id, true),
            _ => unreachable!(),
        };
        let valid = self.exact(&id, incarnation).and_then(|runtime| {
            let agent = runtime
                .agent
                .as_ref()
                .ok_or("Monitor ACK requires an actual owner Agent Session")?;
            if !agent.current.load(Ordering::Acquire)
                || agent.snapshot.linked_issue_number != Some(issue_number)
            {
                return Err("Monitor ACK Session/Issue ownership changed".into());
            }
            if let Some(feedback) = &agent.feedback {
                if feedback.issue_monitor_launch_binding_issue_number() != Some(issue_number)
                    || feedback.issue_monitor_delivery_id != delivery_id
                {
                    return Err("Monitor ACK delivery identity changed".into());
                }
            } else if !adopt {
                return Err("Monitor ACK has no owner launch provenance".into());
            }
            Ok(())
        });
        if let Err(error) = valid {
            let _ = reply.send(Err(error));
            return;
        }
        let agent = self.runtimes.get_mut(&id).unwrap().agent.as_mut().unwrap();
        if agent.feedback.is_none() {
            agent.monitor_owner = Some((self.root.clone(), self.host_instance_id.clone()));
            agent.feedback = Some(super::launch_payload::LaunchFeedbackContext {
                client_id: "__issue_monitor__".into(),
                title: "Issue Monitor".into(),
                issue_monitor_issue_number: Some(issue_number),
                issue_monitor_delivery_id: delivery_id,
                issue_monitor_project_root: Some(self.root.clone()),
                issue_monitor_session_mode: Some(agent.snapshot.session_mode),
                issue_monitor_autonomous_handoff: None,
                issue_monitor_autonomous_submit_started: true,
                issue_monitor_review_dispatch: false,
            });
            agent.monitor_adopt_claim = true;
        }
        if agent.monitor_source.is_some() {
            let _ = reply.send(Ok(
                json!({"acknowledged":true,"id":id,"incarnation":incarnation}),
            ));
            return;
        }
        agent.monitor_ack_replies.push(reply);
        self.commit_monitor_launch(&id);
    }

    fn commit_monitor_launch(&mut self, id: &str) {
        let Some(launch) = self.monitor_launch(id) else {
            return;
        };
        let agent = self.runtimes.get_mut(id).unwrap().agent.as_mut().unwrap();
        if !agent.current.load(Ordering::Acquire)
            || agent.monitor_commit_inflight
            || agent.monitor_source.is_some()
        {
            return;
        }
        let ready = agent.monitor_ready;
        let adopt_claim = agent.monitor_adopt_claim;
        agent.monitor_commit_inflight = true;
        agent.monitor_retry_at = None;
        let stale_runtime = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&self.root),
        )
        .ok()
        .and_then(|prefs| {
            prefs
                .failed_issues
                .into_iter()
                .find(|failed| {
                    Some(failed.issue_number)
                        == launch.feedback.issue_monitor_launch_binding_issue_number()
                })
                .and_then(|failed| failed.window_id)
        })
        .filter(|stale| stale != id)
        .and_then(|stale| {
            let issue_number = launch
                .feedback
                .issue_monitor_launch_binding_issue_number()?;
            self.runtimes
                .get(&stale)
                .filter(|runtime| Self::is_failed_monitor_runtime(runtime, issue_number))
                .map(|runtime| (stale, runtime.incarnation, issue_number))
        });
        self.input_jobs += 1;
        let sender = self.sender.clone();
        spawn_owner_worker(move || {
            let result = (|| {
                if adopt_claim {
                    if let Some(delivery_id) = &launch.feedback.issue_monitor_delivery_id {
                        let claim = super::installation::IssueMonitorLaunchClaimInput {
                            project_root: launch.root.clone(),
                            issue_number: launch
                                .feedback
                                .issue_monitor_launch_binding_issue_number()
                                .expect("owner issue"),
                            delivery_id: delivery_id.clone(),
                            materializer_window_id: launch.id.clone(),
                            materializer_id: launch.materializer_id.clone(),
                            materializer_pid: std::process::id(),
                            fallback_timeout:
                                super::installation::local_issue_monitor_fallback_projection_timeout(
                                ),
                            snapshot: None,
                            snapshot_tabs: Default::default(),
                        };
                        if !claim
                            .claim_from_worker()
                            .map_err(|error| error.to_string())?
                        {
                            return Err(
                                "Monitor adoption claim belongs to another exact materializer"
                                    .into(),
                            );
                        }
                    }
                }
                launch.commit_launch(ready)
            })();
            let expired = result.is_err() && launch.expired_delivery().unwrap_or(false);
            let _ = sender.send(OwnerMessage::MonitorCommitted {
                id: launch.id,
                incarnation: launch.incarnation,
                ready,
                stale_runtime,
                expired,
                result,
            });
        });
    }

    fn runtime_monitor_heartbeat(
        &mut self,
        id: &str,
        source_event: Option<&str>,
        occurred_at: Option<&str>,
    ) {
        let Some(launch) = self.monitor_launch(id) else {
            return;
        };
        self.input_jobs += 1;
        let sender = self.sender.clone();
        let source_event = source_event.map(str::to_string);
        let occurred_at = occurred_at.map(str::to_string);
        spawn_owner_worker(move || {
            let result = launch.heartbeat(occurred_at.as_deref(), source_event.as_deref());
            let _ = sender.send(OwnerMessage::MonitorNotificationFinished { result });
        });
    }

    fn runtime_monitor_failure(
        &mut self,
        id: &str,
        detail: &str,
        failure: Option<crate::IssueMonitorFailure>,
        terminal: bool,
    ) {
        let Some(launch) = self.monitor_launch(id) else {
            return;
        };
        self.input_jobs += 1;
        let sender = self.sender.clone();
        let detail = detail.to_string();
        spawn_owner_worker(move || {
            let result = launch.failure(&detail, failure.as_ref(), terminal);
            let _ = sender.send(OwnerMessage::MonitorNotificationFinished { result });
        });
    }

    fn next_monitor_deadline(&self) -> Option<std::time::Instant> {
        if self.stopping {
            return None;
        }
        self.runtimes
            .values()
            .filter_map(|runtime| runtime.agent.as_ref())
            .flat_map(|agent| [agent.monitor_retry_at, agent.monitor_startup_at])
            .flatten()
            .min()
    }

    fn check_monitor_deadlines(&mut self, now: std::time::Instant) {
        let activated: Vec<_> = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                let agent = runtime.agent.as_ref()?;
                let pending = agent.pending_fresh.as_ref()?;
                (agent.feedback.is_some()
                    && !self.readiness_inflight(id, runtime.incarnation)
                    && super::launch_finalization::pending_fresh_execution_activation_status(
                        pending,
                    ) == Some(true))
                .then(|| id.clone())
            })
            .collect();
        for id in activated {
            self.begin_readiness(&id, None, true, None);
        }
        let due: Vec<_> = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                let agent = runtime.agent.as_ref()?;
                let retry = agent.monitor_retry_at.is_some_and(|at| at <= now);
                let startup = agent.monitor_startup_at.is_some_and(|at| at <= now);
                (retry || startup).then_some((id.clone(), retry, startup))
            })
            .collect();
        for (id, retry, startup) in due {
            if retry {
                self.commit_monitor_launch(&id);
            }
            if startup {
                let Some(launch) = self.monitor_launch(&id) else {
                    continue;
                };
                let issue = launch
                    .feedback
                    .issue_monitor_launch_binding_issue_number()
                    .unwrap();
                let screen = self.runtimes[&id]
                    .pane
                    .lock()
                    .ok()
                    .map(|pane| pane.screen().contents());
                let detail =
                    super::monitor::monitor_startup_failure_detail(issue, screen.as_deref());
                self.runtimes
                    .get_mut(&id)
                    .unwrap()
                    .agent
                    .as_mut()
                    .unwrap()
                    .monitor_startup_at = Some(now + std::time::Duration::from_secs(60));
                if !self.begin_readiness(
                    &id,
                    None,
                    false,
                    Some((
                        detail.clone(),
                        super::launch_readiness::LaunchPaneDisposition::Teardown,
                    )),
                ) {
                    self.input_jobs += 1;
                    let sender = self.sender.clone();
                    let watch_at = self.runtimes[&id]
                        .agent
                        .as_ref()
                        .unwrap()
                        .monitor_startup_at
                        .unwrap();
                    spawn_owner_worker(move || {
                        let result = launch.commit_startup_failure(&detail);
                        let _ = sender.send(OwnerMessage::MonitorStartupFailed {
                            id: launch.id,
                            incarnation: launch.incarnation,
                            watch_at,
                            result,
                        });
                    });
                }
            }
        }
    }
}

fn repair_fresh_readiness(
    pending: &super::launch_payload::PendingFreshExecutionLaunch,
    active: &ActiveAgentSession,
    fence: &ReadinessFence,
    issuer: &AgentCapabilityIssuer,
    sessions: &std::path::Path,
    cache: &std::path::Path,
    current: &AtomicBool,
) -> super::continuation::FreshExecutionFinalizationOutcome {
    use super::continuation::FreshExecutionFinalizationOutcome::{Committed, Retained};

    let exact = current.load(Ordering::Acquire)
        && fence
            .token
            .as_deref()
            .is_some_and(|token| issuer.active_token_is_current(token, &pending.binding));
    let repaired = exact
        && super::continuation::resolve_activated_fresh_execution_commit(
            &pending.project_root,
            &pending.worktree_path,
            pending.owner,
            &pending.operation_id,
            &pending.request,
            sessions,
            &pending.session_identity,
        )
        .is_ok_and(|committed| committed);
    if !repaired {
        return Retained;
    }
    let probe = crate::probe_authenticated_execution_binding(
        &pending.project_root,
        &pending.binding.session_id,
        &pending.binding,
        "fresh-linked-owner-launch-coordinator",
        crate::AgentExecutionBindingProbeRequest {
            schema_version: crate::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: pending.operation_id.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    );
    if probe.is_ok_and(|receipt| {
        receipt.execution_binding == pending.binding.identity
            && receipt.capability_generation == pending.binding.capability_generation
    }) && current.load(Ordering::Acquire)
        && fence
            .token
            .as_deref()
            .is_some_and(|token| issuer.active_token_is_current(token, &pending.binding))
        && super::continuation::prepare_fresh_execution_launch_publication(
            pending, active, sessions, cache,
        )
    {
        Committed
    } else {
        Retained
    }
}

fn abandon_agent_launch(
    success: &PreparedAgentLaunchSuccess,
    sessions_dir: &std::path::Path,
    issuer: &AgentCapabilityIssuer,
    durable_candidate_removed: bool,
    registry_cleanup: Option<&super::agent_dispatch::OwnedWindowRecordCleanup>,
) -> Result<(), String> {
    let id = &success.active_session.window_id;
    if !crate::session_finalizer::kill_and_reap(&success.pane.shared_pty(), id)
        .map_err(|error| error.to_string())?
    {
        return Err("cancelled agent process tree has not exited".into());
    }
    success.pane.pty().invalidate_input_generation();
    let mut handled = durable_candidate_removed;
    if !durable_candidate_removed {
        if let Some(pending) = &success.pending_continue_work {
            handled =
                super::launch_finalization::rollback_worker_continue_work(sessions_dir, pending)?;
        }
        if let Some(pending) = &success.pending_fresh_execution {
            handled =
                super::launch_finalization::rollback_worker_fresh_execution(sessions_dir, pending)?
                    || handled;
        }
    }
    if !handled {
        if let Some(genesis) = &success.materialized_genesis {
            super::launch_finalization::rollback_worker_genesis(
                sessions_dir,
                &success.project_root,
                &success.active_session,
                genesis,
                "owner launch cancelled before pane installation",
                &mut None,
            )?;
        } else {
            let snapshot = success
                .session_snapshot
                .as_ref()
                .ok_or("cancelled launch has no exact durable Session snapshot")?;
            finish_agent_session(
                sessions_dir,
                snapshot,
                success.incarnation,
                &success.pane.shared_pty(),
                id,
                Some(false),
                None,
            )?;
        }
    }
    if let Some(token) = &success.capability_token {
        issuer.revoke_token(token);
    }
    if let Some(cleanup) = registry_cleanup {
        cleanup.finish()?;
    }
    if let (Some(snapshot), Some(reservation)) = (
        success
            .session_snapshot
            .as_ref()
            .or(success.cached_session.as_ref()),
        success.member_reservation.as_deref(),
    ) {
        crate::agent_names::release_reaped_project_member(
            &success.project_root,
            sessions_dir,
            snapshot,
            reservation,
            &success.pane.shared_pty(),
        )?;
    }
    Ok(())
}

fn retire_unspawned_launch_runtime(
    sessions_dir: &std::path::Path,
    expected: &gwt_agent::Session,
    captured_runtime: Option<&Result<Option<Vec<u8>>, String>>,
) -> Result<gwt_agent::Session, String> {
    let outcome = gwt_agent::update_session_if_matches_with(
        sessions_dir,
        expected,
        |current, expected| Ok(serde_json::to_value(current)? == serde_json::to_value(expected)?),
        |current| {
            let path = gwt_agent::runtime_state_path(sessions_dir, &current.id);
            let _writer = gwt_agent::session::runtime_writer_lease(&path)?;
            let _publication = gwt_agent::session::runtime_publication_lease(&path)?;
            match captured_runtime {
                Some(Ok(Some(expected))) => {
                    let captured: gwt_agent::SessionRuntimeState =
                        serde_json::from_slice(expected)?;
                    if captured.execution_identity.is_some()
                        || captured.runtime_incarnation.is_some()
                        || captured.host_started_at.is_some()
                        || captured.child_pid.is_some()
                        || captured.child_started_at.is_some()
                        || captured.source_event.is_some()
                        || captured.active_tool_calls.is_some()
                        || captured.last_completed_hook_event_at.is_some()
                        || captured.pending_discussion.is_some()
                        || std::fs::read(&path)? != *expected
                    {
                        return Err(std::io::Error::other(
                            "failed launch runtime changed or carries physical evidence",
                        ));
                    }
                    std::fs::remove_file(&path)?;
                    #[cfg(unix)]
                    std::fs::File::open(path.parent().expect("runtime namespace"))?.sync_all()?;
                }
                Some(Ok(None)) if !path.try_exists()? => {}
                _ => {
                    return Err(std::io::Error::other(
                        "failed launch provisional runtime is unconfirmed",
                    ))
                }
            }
            current.update_status(gwt_agent::AgentStatus::Stopped);
            Ok(current.clone())
        },
    )
    .map_err(|error| error.to_string())?;
    match outcome {
        gwt_agent::SessionSnapshotUpdateOutcome::Updated(stopped) => Ok(stopped),
        _ => Err("failed launch Session changed before provisional runtime retirement".into()),
    }
}

fn finish_agent_session(
    sessions_dir: &std::path::Path,
    snapshot: &gwt_agent::Session,
    incarnation: u64,
    pty: &PtyHandle,
    id: &str,
    desired_restore_on_startup: Option<bool>,
    exit_receipt: Option<&gwt_agent::SessionExitReceipt>,
) -> Result<(), String> {
    if let Some(identity) = gwt_agent::SessionExecutionIdentity::from_session(snapshot)? {
        crate::session_finalizer::finalize_exact_close_with_restore_policy(
            sessions_dir,
            &identity,
            incarnation,
            pty,
            id,
            desired_restore_on_startup,
            exit_receipt,
        )
        .map_err(|error| error.to_string())?
        .ok_or("bound Session runtime changed before close")?;
    } else {
        let outcome = gwt_agent::update_session_if_matches_with(
            sessions_dir,
            snapshot,
            |current, expected| {
                Ok(current.id == expected.id
                    && current.created_at == expected.created_at
                    && current.worktree_path == expected.worktree_path
                    && current.branch == expected.branch
                    && current.agent_id == expected.agent_id
                    && current.linked_issue_number == expected.linked_issue_number
                    && current.execution_binding.is_none())
            },
            |session| {
                let path = gwt_agent::runtime_state_path(sessions_dir, &session.id);
                let mut runtime = gwt_agent::SessionRuntimeState::load(&path)?;
                let child_pid = pty
                    .process_id()
                    .ok_or_else(|| std::io::Error::other("captured child PID unavailable"))?;
                let running = pty
                    .try_wait()
                    .map_err(|error| std::io::Error::other(error.to_string()))?
                    .is_none();
                if runtime.runtime_incarnation != Some(incarnation)
                    || runtime.execution_identity.is_some()
                    || runtime.host_started_at
                        != crate::process::host_process_start_time(std::process::id())
                    || runtime.host_started_at.is_none()
                    || runtime.child_pid != Some(child_pid)
                    || runtime.child_started_at.is_none()
                    || (running
                        && runtime.child_started_at
                            != crate::process::host_process_start_time(child_pid))
                {
                    return Err(std::io::Error::other(
                        "unbound Session physical proof changed before close",
                    ));
                }
                if !crate::session_finalizer::kill_and_reap(pty, id)
                    .map_err(|error| std::io::Error::other(error.to_string()))?
                {
                    return Err(std::io::Error::other("agent process tree did not exit"));
                }
                runtime.status = gwt_agent::AgentStatus::Stopped;
                runtime.updated_at = chrono::Utc::now();
                runtime.save(&path)?;
                session.update_status(gwt_agent::AgentStatus::Stopped);
                if let Some(receipt) = exit_receipt {
                    session.record_process_exit(receipt);
                }
                if let Some(restore) = desired_restore_on_startup {
                    session.restore_window_on_startup = restore;
                }
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
        match outcome {
            gwt_agent::SessionSnapshotUpdateOutcome::Updated(()) => {}
            gwt_agent::SessionSnapshotUpdateOutcome::MutationFailed(error)
            | gwt_agent::SessionSnapshotUpdateOutcome::SnapshotUnreadable(error) => {
                return Err(error.to_string())
            }
            gwt_agent::SessionSnapshotUpdateOutcome::SnapshotChanged => {
                return Err("unbound Session identity changed before close".into())
            }
        }
    }
    Ok(())
}

fn monitor_launch_for_agent(
    id: &str,
    incarnation: u64,
    agent: &AgentRuntimeRecord,
) -> Option<super::monitor::MonitorLaunch> {
    let feedback = agent.feedback.as_ref()?;
    feedback.issue_monitor_launch_binding_issue_number()?;
    if feedback.issue_monitor_review_dispatch {
        return None;
    }
    let (root, materializer) = agent.monitor_owner.as_ref()?;
    Some(super::monitor::MonitorLaunch {
        root: root.clone(),
        id: id.into(),
        incarnation,
        session_id: agent.active.session_id.clone(),
        agent_id: agent.active.agent_id.clone(),
        materializer_id: materializer.clone(),
        feedback: feedback.clone(),
        current: agent.current.clone(),
        source: agent.monitor_source.clone(),
    })
}

fn same_legacy_session_source(left: &gwt_agent::Session, right: &gwt_agent::Session) -> bool {
    left.id == right.id
        && left.created_at == right.created_at
        && left.worktree_path == right.worktree_path
        && left.branch == right.branch
        && left.agent_id == right.agent_id
        && left.repo_hash == right.repo_hash
        && left.linked_issue_number == right.linked_issue_number
}

fn publish_repaired_inspection_runtime(
    source: &gwt_agent::Session,
    incarnation: u64,
    current: &Arc<AtomicBool>,
    pty: &Arc<PtyHandle>,
    child_started_at: Option<u64>,
    issuer: &AgentCapabilityIssuer,
    token: &str,
) -> Result<gwt_agent::Session, String> {
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let binding = issuer
        .active_execution_binding_for_token(token)
        .ok_or("legacy repair has no exact Active bearer")?;
    let host_started_at = crate::process::host_process_start_time(std::process::id())
        .ok_or("owning Host birth unavailable")?;
    let child_pid = pty.process_id().ok_or("owned child PID unavailable")?;
    let child_started_at = child_started_at.ok_or("captured child birth unavailable")?;
    let published = crate::cli::execution_state::with_current_active_execution_binding_lease(
        &sessions,
        &binding,
        || -> Result<gwt_agent::Session, String> {
            let snapshot = gwt_agent::Session::load(&sessions.join(format!("{}.toml", source.id)))
                .map_err(|error| error.to_string())?;
            if !same_legacy_session_source(&snapshot, source)
                || snapshot.execution_binding.as_ref() != Some(&binding)
            {
                return Err(
                    "legacy repair durable Session changed before runtime publication".into(),
                );
            }
            let source_root = crate::validated_project_state_root_for_session_recovery(source)
                .map_err(|error| error.to_string())?;
            let repaired_root = crate::validated_project_state_root_for_session_recovery(&snapshot)
                .map_err(|error| error.to_string())?;
            if gwt_core::paths::project_scope_hash(&source_root)
                != gwt_core::paths::project_scope_hash(&repaired_root)
            {
                return Err(
                    "legacy repair project scope changed before runtime publication".into(),
                );
            }
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&snapshot)
                .map_err(|error| error.to_string())?
                .ok_or("legacy repair Session is unbound")?;
            issuer
                .with_current_active_token(token, &binding, || -> Result<(), String> {
                    let path = gwt_agent::runtime_state_path_for_pid(
                        &sessions,
                        std::process::id(),
                        &source.id,
                    );
                    let _writer = gwt_agent::session::runtime_writer_lease(&path)
                        .map_err(|error| error.to_string())?;
                    let _publication = gwt_agent::session::runtime_publication_lease(&path)
                        .map_err(|error| error.to_string())?;
                    let mut proof = gwt_agent::SessionRuntimeState::load(&path)
                        .map_err(|error| error.to_string())?;
                    if !current.load(Ordering::Acquire)
                        || pty.try_wait().map_err(|error| error.to_string())?.is_some()
                        || crate::process::host_process_start_time(child_pid)
                            != Some(child_started_at)
                        || proof.runtime_incarnation != Some(incarnation)
                        || proof.host_started_at != Some(host_started_at)
                        || proof.child_pid != Some(child_pid)
                        || proof.child_started_at != Some(child_started_at)
                        || proof
                            .execution_identity
                            .as_ref()
                            .is_some_and(|previous| previous != &identity)
                        || matches!(
                            proof.status,
                            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                        )
                    {
                        return Err(
                            "legacy inspection runtime changed before binding publication".into(),
                        );
                    }
                    proof.execution_identity = Some(identity);
                    proof.save(&path).map_err(|error| error.to_string())
                })
                .ok_or("legacy repair Active bearer changed before runtime publication")??;
            Ok(snapshot)
        },
    )
    .map_err(|error| error.to_string())?
    .ok_or("legacy repair binding changed before runtime publication")??;
    Ok(published)
}

fn retire_reaped_missing_session_runtime(
    sessions_dir: &std::path::Path,
    snapshot: &gwt_agent::Session,
    incarnation: u64,
    pty: &PtyHandle,
    child_started_at: Option<u64>,
) -> Result<(), String> {
    if !pty.tree_exit_confirmed() {
        return Err("missing Session cleanup requires the captured process tree exit".into());
    }
    gwt_agent::with_session_path_lease(sessions_dir, &snapshot.id, |state| {
        match state {
            gwt_agent::SessionPathState::Present(_) => return Ok(()),
            gwt_agent::SessionPathState::Error(error) => return Err(error),
            gwt_agent::SessionPathState::Missing => {}
        }
        // This is exclusively the current owner's namespace. A missing
        // Session grants no right to sweep another Host's sidecars.
        let path =
            gwt_agent::runtime_state_path_for_pid(sessions_dir, std::process::id(), &snapshot.id);
        let _writer = gwt_agent::session::runtime_writer_lease(&path)?;
        let _publication = gwt_agent::session::runtime_publication_lease(&path)?;
        let runtime = match gwt_agent::SessionRuntimeState::load(&path) {
            Ok(runtime) => runtime,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let expected = gwt_agent::SessionExecutionIdentity::from_session(snapshot)
            .map_err(std::io::Error::other)?
            .ok_or_else(|| {
                std::io::Error::other("missing Session has no captured bound identity")
            })?;
        let host_started_at = crate::process::host_process_start_time(std::process::id())
            .filter(|started| *started > 0);
        let child_pid = pty.process_id().filter(|pid| *pid > 0);
        let child_started_at = child_started_at.filter(|started| *started > 0);
        if runtime.execution_identity.as_ref() != Some(&expected)
            || incarnation == 0
            || runtime.runtime_incarnation != Some(incarnation)
            || host_started_at.is_none()
            || runtime.host_started_at != host_started_at
            || child_pid.is_none()
            || runtime.child_pid != child_pid
            || child_started_at.is_none()
            || runtime.child_started_at != child_started_at
        {
            return Err(std::io::Error::other(
                "missing Session runtime changed before exact cleanup",
            ));
        }
        std::fs::remove_file(&path)?;
        #[cfg(unix)]
        std::fs::File::open(path.parent().expect("captured runtime namespace"))?.sync_all()?;
        Ok(())
    })
    .map_err(|error| error.to_string())
}

fn close_agent_runtime(
    id: &str,
    runtime: &WindowRuntime,
    agent: &AgentRuntimeRecord,
    desired_restore_on_startup: Option<bool>,
) -> Result<bool, String> {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let mut handled = false;
    let mut fresh_rolled_back = false;
    if agent.pending_continue.is_some() || agent.pending_fresh.is_some() {
        if !crate::session_finalizer::kill_and_reap(&runtime.pty, id)
            .map_err(|error| error.to_string())?
        {
            return Err("pending agent process tree did not exit".into());
        }
        if let Some(pending) = &agent.pending_continue {
            handled =
                super::launch_finalization::rollback_worker_continue_work(&sessions_dir, pending)?;
        }
        if let Some(pending) = &agent.pending_fresh {
            fresh_rolled_back = super::launch_finalization::rollback_worker_fresh_execution(
                &sessions_dir,
                pending,
            )?;
            handled |= fresh_rolled_back;
        }
    }
    if handled {
        retire_reaped_missing_session_runtime(
            &sessions_dir,
            &agent.snapshot,
            runtime.incarnation,
            &runtime.pty,
            runtime.child_started_at,
        )?;
    }
    if !handled {
        let exit_receipt = runtime
            .pane
            .lock()
            .map_err(|error| error.to_string())?
            .last_exit()
            .map(|exit| gwt_agent::SessionExitReceipt {
                exit_code: exit.exit_code,
                signal: exit.signal.clone(),
                exited_at: exit.observed_at.into(),
            });
        finish_agent_session(
            &sessions_dir,
            &agent.snapshot,
            runtime.incarnation,
            &runtime.pty,
            id,
            desired_restore_on_startup,
            exit_receipt.as_ref(),
        )?;
        // The physical owner keeps its captured identity after directory loss.
        // Re-deriving project paths then would write into a new fallback store
        // (or a replacement repository), rather than this owner's Work.
        let project_is_current = runtime.project_root.is_dir()
            && id.split_once("::").is_some_and(|(project, _)| {
                gwt_core::paths::project_scope_hash(&runtime.project_root).as_str() == project
            });
        if !project_is_current {
            tracing::warn!(
                window_id = id,
                "skipping Work cleanup after project identity changed"
            );
        } else if runtime.policy.quota_hold.is_none()
            && super::workspace::session_uses_ephemeral_worktree_for_project(
                Some(&runtime.project_root),
                &agent.active,
            )
        {
            super::workspace::finalize_ephemeral_worktree_for_project(
                Some(&runtime.project_root),
                &agent.active,
            );
        } else {
            if let Some(task) = super::workspace::paused_work_record_task(
                &runtime.project_root,
                &agent.active,
                chrono::Utc::now(),
            ) {
                task();
            }
            gwt_core::workspace_projection::mark_workspace_agent_stopped(
                &runtime.project_root,
                &agent.active.session_id,
                Some(&agent.active.window_id),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    if let Some(token) = &agent.capability_token {
        agent.issuer.revoke_token(token);
    }
    if desired_restore_on_startup == Some(false) {
        if let Some(launch) = monitor_launch_for_agent(id, runtime.incarnation, agent) {
            launch.closed(agent.monitor_terminal_failure.as_ref())?;
        }
    }
    Ok(fresh_rolled_back)
}

pub(crate) fn close_runtime(
    id: &str,
    runtime: &mut WindowRuntime,
    cause: CloseCause,
) -> Result<(), String> {
    if runtime.close_handoff.is_none() {
        if let Some(agent) = runtime.agent.as_ref() {
            if let Some(binding) = agent.snapshot.execution_binding.as_ref() {
                let active_token = agent.capability_token.as_deref().filter(|token| {
                    agent
                        .issuer
                        .active_execution_binding_for_token(token)
                        .as_ref()
                        == Some(binding)
                });
                if active_token.is_some()
                    || (agent.pending_continue.is_none() && agent.pending_fresh.is_none())
                {
                    let reservation = match active_token {
                        Some(token) => agent.issuer.begin_manual_execution_handoff(token, binding),
                        None => agent.issuer.reserve_manual_execution_handoff(binding),
                    }?;
                    runtime.close_handoff =
                        Some(crate::session_finalizer::CloseHandoffReservation::new(
                            agent.issuer.clone(),
                            reservation,
                        ));
                }
            }
        }
    }
    let stopped = if let Some(agent) = &runtime.agent {
        close_agent_runtime(id, runtime, agent, cause.restore_policy())
    } else if !crate::session_finalizer::kill_and_reap(&runtime.pty, id)
        .map_err(|e| e.to_string())?
    {
        Err("owned process tree did not exit".into())
    } else {
        Ok(false)
    };
    if let Some(handoff) = runtime.close_handoff.as_mut() {
        if runtime.pty.tree_exit_confirmed() {
            if !handoff.is_committed() && !handoff.commit_and_hold() {
                return Err("exact close capability fence could not be committed".into());
            }
        } else if stopped.is_err() {
            if !handoff.rollback() {
                return Err("exact close capability fence could not be rolled back".into());
            }
            runtime.close_handoff = None;
        }
    }
    let fresh_rolled_back = stopped?;
    if let Some((launch, detail)) = runtime
        .rollback_monitor_failure
        .as_ref()
        .filter(|_| cause.restore_policy() == Some(false))
    {
        launch.closed(Some(&(detail.clone(), None)))?;
        crate::session_launch::recovery::clear_durable_launch_recovery(
            &gwt_core::paths::gwt_sessions_dir(),
            &launch.session_id,
        )
        .map_err(|error| error.to_string())?;
        runtime.rollback_monitor_failure = None;
    }

    // The child tree is proven gone above. On Windows, releasing the
    // ConPTY descriptors lets the reader reach EOF before it is joined.
    runtime.pty.release_descriptors();
    if let Some(worker) = runtime.output_thread.take() {
        worker
            .join()
            .map_err(|_| "output worker panicked".to_string())?;
    }
    if let Some(worker) = runtime.status_thread.take() {
        worker
            .join()
            .map_err(|_| "status worker panicked".to_string())?;
    }
    if matches!(cause, CloseCause::ExplicitClose | CloseCause::NaturalExit) {
        if let Some(cleanup) = runtime.registry_cleanup.as_ref() {
            cleanup.finish()?;
            runtime.registry_cleanup = None;
        }
    } else if matches!(cause, CloseCause::GuiLifetime) && fresh_rolled_back {
        if let Some(cleanup) = runtime.registry_cleanup.as_ref() {
            let pending = runtime
                .agent
                .as_ref()
                .and_then(|agent| agent.pending_fresh.as_ref())
                .ok_or("compensated canvas cleanup has no captured fresh attempt")?;
            if cleanup.canonical_id != id
                || cleanup.expected_session_id.as_deref()
                    != Some(pending.binding.session_id.as_str())
            {
                return Err("compensated canvas cleanup identity changed".into());
            }
            // Only the exact rollback above grants this cleanup. Keep the
            // candidate absent through row CAS; a same-SID replacement must
            // not inherit the retired candidate's canvas-removal authority.
            let removed = crate::cli::execution_state::commit_if_session_missing_with_owner_lease(
                &pending.worktree_path,
                pending.owner,
                &gwt_core::paths::gwt_sessions_dir(),
                &pending.binding.session_id,
                || cleanup.finish().map_err(std::io::Error::other),
            )
            .map_err(|error| error.to_string())?;
            if !removed {
                return Err("compensated fresh Session changed before exact canvas cleanup".into());
            }
            runtime.registry_cleanup = None;
        }
    }
    if let Some(snapshot) = runtime.close_monitor_snapshot.as_ref() {
        let publication =
            crate::daemon_publisher::publish_issue_monitor_window_snapshot_with_timeout(
                &runtime.project_root,
                snapshot,
                &std::collections::BTreeSet::from([snapshot.project_tab_id.clone()]),
                std::time::Duration::from_secs(5),
            );
        match publication {
            Err(error) if error.allows_local_fallback() => {
                super::installation::try_commit_local_issue_monitor_control_with_timeout(
                    &runtime.project_root,
                    super::installation::local_issue_monitor_fallback_projection_timeout(),
                    |monitor| {
                        if !runtime.pty.tree_exit_confirmed()
                            || monitor.launched_window_issue(id).is_some()
                            || monitor
                                .prefs()
                                .pending_launch_deliveries
                                .iter()
                                .any(|pending| {
                                    pending.materializer_window_id.as_deref() == Some(id)
                                })
                        {
                            return Err(std::io::Error::other(
                                "closed runtime or current Monitor source changed before snapshot",
                            ));
                        }
                        let registry = crate::persistence::load_workspace_state(
                            &crate::persistence::workspace_state_path(&runtime.project_root),
                        )?;
                        let same_row =
                            |window: &crate::IssueMonitorWindowObservation,
                             row: &crate::PersistedWindowState| {
                                window
                                    .window_id
                                    .split_once("::")
                                    .is_some_and(|(project, raw)| {
                                        project == snapshot.project_tab_id && raw == row.id
                                    })
                                    && window.session_id == row.session_id
                            };
                        let rows = registry
                            .windows
                            .iter()
                            .filter(|row| {
                                row.preset == crate::WindowPreset::Agent && row.session_id.is_some()
                            })
                            .collect::<Vec<_>>();
                        if registry
                            .windows
                            .iter()
                            .any(|row| id.split_once("::").is_some_and(|(_, raw)| raw == row.id))
                            || rows.iter().any(|row| {
                                !snapshot.windows.iter().any(|window| same_row(window, row))
                            })
                            || snapshot
                                .windows
                                .iter()
                                .filter(|window| window.session_id.is_some())
                                .any(|window| !rows.iter().any(|row| same_row(window, row)))
                        {
                            return Err(std::io::Error::other(
                                "owner canvas changed after the captured close snapshot",
                            ));
                        }
                        let tabs =
                            std::collections::BTreeSet::from([snapshot.project_tab_id.clone()]);
                        if let Some(started) =
                            crate::process::host_process_start_time(std::process::id())
                        {
                            monitor.record_window_snapshot_from_host(
                                snapshot.clone(),
                                std::process::id(),
                                started,
                                tabs,
                            );
                        } else {
                            monitor.record_window_snapshot_for_tabs(snapshot.clone(), tabs);
                        }
                        Ok(())
                    },
                )
                .map_err(|error| error.to_string())?;
            }
            result => result.map_err(|error| error.to_string())?,
        }
        runtime.close_monitor_snapshot = None;
    }
    if let Some(handoff) = runtime.close_handoff.as_mut() {
        if !handoff.release_committed() {
            return Err("exact close capability fence could not be released".into());
        }
        runtime.close_handoff = None;
    }
    if let Some((snapshot, reservation)) = runtime.member_cleanup.as_ref() {
        crate::agent_names::release_reaped_project_member(
            &runtime.project_root,
            &gwt_core::paths::gwt_sessions_dir(),
            snapshot,
            reservation,
            &runtime.pty,
        )?;
        runtime.member_cleanup = None;
    }
    if let Some(agent) = runtime.agent.as_ref() {
        let pm_cause = match cause {
            CloseCause::PmGenericCrash => super::pm_delivery::PmLifecycleCause::Crash,
            CloseCause::ExplicitClose | CloseCause::NaturalExit => {
                super::pm_delivery::PmLifecycleCause::Deregister
            }
            CloseCause::DiagnosticExit
                if !runtime.policy.recoverable_error && runtime.policy.quota_hold.is_none() =>
            {
                super::pm_delivery::PmLifecycleCause::Crash
            }
            CloseCause::DiagnosticExit | CloseCause::GuiLifetime => {
                super::pm_delivery::PmLifecycleCause::Preserve
            }
        };
        super::pm_delivery::finish_pm_lifecycle(
            &runtime.project_root,
            &agent.snapshot.id,
            pm_cause,
            &mut runtime.pm_lifecycle,
        )?;
    }
    tracing::info!(target: "gwt.pane.teardown", window_id = %id, incarnation = runtime.incarnation, stage = "close_finalizer", outcome = "completed", "pane close finalizer completed");
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::agent_capability::AgentCapabilityRegistry;

    fn run_fixture_owner(state: OwnerState, receiver: mpsc::Receiver<OwnerMessage>) {
        // These direct actor units install no lifetime producers. Complete the
        // same returned shutdown receipts after their actual actor has drained.
        for reply in state.run(receiver) {
            let _ = reply.send(());
        }
    }

    fn owner_fixture(
        root: &std::path::Path,
    ) -> (OwnerState, mpsc::Receiver<OwnerMessage>, OwnerHandle) {
        let capabilities = AgentCapabilityRegistry::default();
        let (sender, receiver) = mpsc::channel();
        let (events, _) = broadcast::channel(16);
        let handle = OwnerHandle {
            sender: sender.clone(),
            events: events.clone(),
            residency: tokio::sync::watch::channel(ResidencySnapshot::default()).1,
        };
        let state = OwnerState {
            issue_client_factory: super::super::launch::default_issue_client_factory(),
            test_dispatch_barriers: Vec::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_operation_clock: None,
            policy_usage_accounts: Vec::new(),
            policy_capacity_resumes: HashMap::new(),
            agent_registry: super::super::agent_dispatch::load_owner_registry(root),
            shared_registry_initialization: None,
            registry_projection: crate::persistence::empty_workspace_state(),
            pty_writers: Arc::new(std::sync::RwLock::new(HashMap::new())),
            agent_dispatch_jobs: 0,
            deferred_agent_frontend: HashMap::new(),
            pending_agent_self_closes: HashMap::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_capacity_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_launch_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_pty_start_gate: None,
            root: root.into(),
            project_key: "0123456789abcdef".into(),
            runtimes: HashMap::new(),
            clients: ClientHub::default(),
            issuer: AgentCapabilityIssuer::new(
                "http://127.0.0.1/hook".into(),
                "ws://127.0.0.1/pane".into(),
                "ws://127.0.0.1/pane".into(),
                capabilities.clone(),
            ),
            host_instance_id: "host".into(),
            sender,
            events,
            pending: HashMap::new(),
            closing: HashMap::new(),
            next_incarnation: 1,
            pending_transport: VecDeque::new(),
            stopping: false,
            gui_presence: GuiPresence::default(),
            next_gui_presence_id: 0,
            residency: tokio::sync::watch::channel(ResidencySnapshot::default()).0,
            shutdown_replies: Vec::new(),
            retiring: 0,
            input_jobs: 0,
            switch_job_inflight: false,
            pm_wake_permits: Default::default(),
            agent_operations: HashMap::new(),
            retirements: HashMap::new(),
            readiness: HashMap::new(),
            close_replies: HashMap::new(),
            continue_work_outcomes: HashMap::new(),
            continue_work_waiters: HashMap::new(),
            deferred_readiness_closes: HashMap::new(),
        };
        (state, receiver, handle)
    }
    fn unbound_fixture(root: &std::path::Path) -> (gwt_agent::Session, Pane) {
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let session = gwt_agent::Session::new(
            root,
            "main",
            gwt_agent::AgentId::Custom("owner-fixture".into()),
        );
        session.save(&sessions).unwrap();
        let pane = super::super::spawn_unbound_pane(
            "pane",
            SpawnConfig {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "read line".into()],
                cols: 80,
                rows: 24,
                env: Default::default(),
                remove_env: Vec::new(),
                cwd: Some(root.into()),
            },
            None,
            1,
            Some((&sessions, &session.id)),
            true,
        )
        .unwrap();
        (session, pane)
    }
    fn active_fixture(root: &std::path::Path, session: &gwt_agent::Session) -> ActiveAgentSession {
        ActiveAgentSession {
            window_id: "pane".into(),
            session_id: session.id.clone(),
            agent_id: "owner-fixture".into(),
            branch_name: "main".into(),
            display_name: "fixture".into(),
            worktree_path: root.into(),
            agent_project_root: root.to_string_lossy().into_owned(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "project".into(),
        }
    }
    fn set_fixture_incarnation(session: &gwt_agent::Session, incarnation: u64) {
        let path = gwt_agent::runtime_state_path(&gwt_core::paths::gwt_sessions_dir(), &session.id);
        let mut runtime = gwt_agent::SessionRuntimeState::load(&path).unwrap();
        runtime.runtime_incarnation = Some(incarnation);
        runtime.save(&path).unwrap();
    }
    fn request(command: OwnerCommand) -> OwnerRequest {
        OwnerRequest {
            project_key: "0123456789abcdef".into(),
            command,
        }
    }

    #[tokio::test]
    async fn shared_registry_initialization_recovers_on_the_same_owner_before_admission() {
        use futures_util::FutureExt as _;

        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let root = home.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let project_key = gwt_core::paths::project_scope_hash(&root).to_string();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
        session.agent_name = Some("Retained Cedar".into());
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.save(&sessions).unwrap();
        let session_path = sessions.join(format!("{}.toml", session.id));
        let original_session = std::fs::read(&session_path).unwrap();
        let mut registry =
            crate::WindowRegistry::from_persisted(crate::persistence::empty_workspace_state());
        let row = registry.add_window(crate::preset::WindowPreset::Agent);
        registry.set_session_id(&row.id, Some(session.id.clone()));
        let workspace_path = crate::persistence::workspace_state_path(&root);
        crate::persistence::save_workspace_state_durable(
            &workspace_path,
            &registry.persistable_state(),
        )
        .unwrap();
        let original_workspace = std::fs::read(&workspace_path).unwrap();
        let registry_path = crate::agent_registry::path();
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        std::fs::write(&registry_path, b"{").unwrap();
        let owner = OwnerState::start(
            root.clone(),
            project_key.clone(),
            ClientHub::default(),
            AgentCapabilityIssuer::new(
                "http://127.0.0.1/hook".into(),
                "ws://127.0.0.1/pane".into(),
                "ws://127.0.0.1/pane".into(),
                AgentCapabilityRegistry::default(),
            ),
            "shared-registry-preflight".into(),
            gwt_core::daemon::DaemonStartIntent::Standalone,
        );
        let request = |command| OwnerRequest {
            project_key: project_key.clone(),
            command,
        };
        let verified =
            std::panic::AssertUnwindSafe(async {
                let health = owner.request(request(OwnerCommand::Health)).await.unwrap();
                assert_eq!(health["reconciliation_required"], true);
                assert!(health["agent_registry_initialization_error"]
                    .as_str()
                    .is_some_and(
                        |error| error.contains("Shared Agent registry initialization failed")
                    ));
                assert_eq!(std::fs::read(&session_path).unwrap(), original_session);
                assert_eq!(std::fs::read(&workspace_path).unwrap(), original_workspace);
                let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex).build();
                let refused = owner
                    .request(request(OwnerCommand::LaunchAgent {
                        id: format!("{project_key}::not-admitted"),
                        operation_id: "registry-not-admitted".into(),
                        tab_id: project_key.clone(),
                        config: Box::new(config),
                        metadata: Box::default(),
                        cols: 80,
                        rows: 24,
                    }))
                    .await;
                assert!(refused.is_err_and(|error| {
                    error.contains("Shared Agent registry initialization failed")
                }));
                let panes = owner
                    .request(request(OwnerCommand::ListPanes))
                    .await
                    .unwrap();
                assert_eq!(panes["panes"], json!([]));
                assert_eq!(panes["pending"], json!([]));
                assert_eq!(std::fs::read(&registry_path).unwrap(), b"{");

                // External correction makes the original owner retryable; no actor
                // or daemon replacement is needed to resume its skipped startup.
                std::fs::remove_file(&registry_path).unwrap();
                let ready = owner.request(request(OwnerCommand::Health)).await.unwrap();
                assert_eq!(ready["reconciliation_required"], false);
                assert!(ready["agent_registry_initialization_error"].is_null());
                assert_eq!(
                    crate::agent_registry::status(None).unwrap().phase,
                    crate::agent_registry::RegistryPhase::Active
                );
                let panes = owner
                    .request(request(OwnerCommand::ListPanes))
                    .await
                    .unwrap();
                let retained = panes["canonical_rows"]["windows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|window| window["id"] == row.id)
                    .unwrap();
                assert_eq!(retained["agent_name"], "Retained Cedar");

                // A ready owner must not rerun startup backfill on later health reads.
                let ready_workspace = std::fs::read(&workspace_path).unwrap();
                session.agent_name = Some("External later name".into());
                session.save(&sessions).unwrap();
                let ready = owner.request(request(OwnerCommand::Health)).await.unwrap();
                assert_eq!(ready["reconciliation_required"], false);
                assert_eq!(std::fs::read(&workspace_path).unwrap(), ready_workspace);
            })
            .catch_unwind()
            .await;
        owner.shutdown().await;
        if let Err(panic) = verified {
            std::panic::resume_unwind(panic);
        }
    }

    #[tokio::test]
    async fn permission_audit_replays_unknown_ack_without_launching_or_trusting_a_claimed_decision()
    {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init", "-q", "-b", "work/issue-4544"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/audit/project.git",
            ],
        ] {
            assert!(gwt_core::process::hidden_command("git")
                .current_dir(&root)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        let project_key = gwt_core::paths::project_scope_hash(&root).to_string();
        let (mut state, receiver, handle) = owner_fixture(&root);
        state.project_key = project_key.clone();
        let home = temp.path().to_path_buf();
        let actor = thread::spawn(move || {
            let _home = gwt_core::test_support::ScopedGwtHome::set(home);
            run_fixture_owner(state, receiver);
        });
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::OpenClaw)
            .working_dir(&root)
            .branch("work/issue-4544")
            .linked_issue_number(4544)
            .build();
        // A caller's supported verdict cannot change the actual provider mapping.
        config.permission_decision = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .build()
            .permission_decision;
        let operation = uuid::Uuid::new_v4().to_string();
        let audit = |operation_id: String, config: gwt_agent::LaunchConfig| OwnerRequest {
            project_key: project_key.clone(),
            command: OwnerCommand::CheckLaunchPermission {
                operation_id,
                config: Box::new(config),
            },
        };
        let (reply, unknown_ack) = oneshot::channel();
        handle
            .sender
            .send(OwnerMessage::Request(
                audit(operation.clone(), config.clone()),
                reply,
            ))
            .unwrap();
        // Lose only this audit's reply; its actual accepted worker still finishes.
        drop(unknown_ack);
        let replay = handle
            .request(audit(operation.clone(), config.clone()))
            .await
            .unwrap();
        assert_eq!(replay["blocked"], true);
        let record = crate::cli::permission_readiness::load(&root)
            .unwrap()
            .unwrap();
        assert_eq!(record.provider, "openclaw");
        assert_eq!(record.session_id, format!("permission-check:{operation}"));
        assert_eq!(replay["record"], serde_json::to_value(&record).unwrap());
        let mut changed = config.clone();
        changed.args.push("changed immutable request".into());
        assert!(handle
            .request(audit(operation, changed))
            .await
            .unwrap_err()
            .contains("operation changed"));
        let mut wrong_owner = config.clone();
        wrong_owner.linked_issue_number = Some(4545);
        assert!(handle
            .request(audit(uuid::Uuid::new_v4().to_string(), wrong_owner))
            .await
            .unwrap_err()
            .contains("owner changed"));
        let foreign = tempfile::tempdir().unwrap();
        let mut wrong_project = config.clone();
        wrong_project.working_dir = Some(foreign.path().into());
        assert!(handle
            .request(audit(uuid::Uuid::new_v4().to_string(), wrong_project))
            .await
            .unwrap_err()
            .contains("another project"));
        let mut wrong_envelope = audit(uuid::Uuid::new_v4().to_string(), config.clone());
        wrong_envelope.project_key = "another project".into();
        assert!(handle
            .request(wrong_envelope)
            .await
            .unwrap_err()
            .contains("project key mismatch"));
        let mut allowed = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(&root)
            .branch("work/issue-4544")
            .linked_issue_number(4544)
            .build();
        // A stale refusal cannot force the audit to launch, or invent a new block.
        allowed.permission_decision =
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::OpenClaw)
                .linked_issue_number(4544)
                .build()
                .permission_decision;
        for allowed in [
            allowed,
            {
                let mut review = config.clone();
                review.suppress_execution_control = true;
                review
            },
            {
                let mut continuation = config;
                continuation.session_mode = gwt_agent::SessionMode::Resume;
                continuation
            },
        ] {
            let checked = handle
                .request(audit(uuid::Uuid::new_v4().to_string(), allowed))
                .await
                .unwrap();
            assert_eq!(checked["blocked"], false);
            assert!(checked["record"].is_null());
        }
        let health = handle
            .request(OwnerRequest {
                project_key: project_key.clone(),
                command: OwnerCommand::Health,
            })
            .await
            .unwrap();
        assert_eq!(health["panes"], 0);
        assert_eq!(health["test_pty_writers"], 0);
        let inventory = handle
            .request(OwnerRequest {
                project_key,
                command: OwnerCommand::ListPanes,
            })
            .await
            .unwrap();
        assert!(inventory["panes"].as_array().unwrap().is_empty());
        assert!(inventory["pending"].as_array().unwrap().is_empty());
        assert!(inventory["canonical_rows"]["windows"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", record.session_id))
        )
        .is_err());
        handle.shutdown().await;
        actor.join().unwrap();
    }

    #[test]
    fn session_start_waits_for_its_materialized_launch_and_never_exposes_nonce() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, _, handle) = owner_fixture(root.path());
        state.project_key = gwt_core::paths::resolve_project_scope(root.path())
            .hash
            .to_string();
        let mut events = handle.subscribe();
        state.agent_operations.insert(
            "launch".into(),
            AgentOperation {
                session_id: Some("materialized-session".into()),
                early_events: VecDeque::new(),
                request: json!({}),
                id: "pane".into(),
                incarnation: 1,
                current: Arc::new(AtomicBool::new(true)),
                replies: Vec::new(),
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: None,
            },
        );
        let event = crate::RuntimeHookEvent {
            kind: crate::RuntimeHookEventKind::RuntimeState,
            source_event: Some("SessionStart".into()),
            gwt_session_id: Some("materialized-session".into()),
            continuation_readiness_nonce: Some("private-challenge".into()),
            agent_session_id: None,
            project_root: Some(root.path().display().to_string()),
            branch: None,
            status: None,
            tool_name: None,
            message: None,
            occurred_at: chrono::Utc::now().to_rfc3339(),
        };
        state.handle_transport_event(TransportEvent::RuntimeHook(event.clone()));
        assert_eq!(
            state.agent_operations["launch"].early_events.len(),
            1,
            "a materialized SessionStart must survive the physical installation gap"
        );
        assert!(
            events.try_recv().is_err(),
            "private readiness cannot escape before installation"
        );
        let mut legacy = event;
        legacy.agent_session_id = legacy.gwt_session_id.take();
        state.handle_transport_event(TransportEvent::RuntimeHook(legacy));
        assert_eq!(
            state.agent_operations["launch"].early_events.len(),
            1,
            "SessionStart must identify the exact gwt Session rather than a provider id"
        );
        assert!(
            state.pending_transport.is_empty(),
            "hook traffic must be consumed by the fixed owner"
        );
    }

    #[test]
    fn rolled_back_readiness_close_is_physical_only_after_session_removal() {
        let root = tempfile::tempdir().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let (snapshot, pane) = unbound_fixture(root.path());
        let (mut state, receiver, _) = owner_fixture(root.path());
        let issuer = state.issuer.clone();
        let token = issuer.issue(root.path(), &snapshot.id).unwrap().token;
        let identity = gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation".into(),
            binding_id: "binding".into(),
            ledger_head_hash: "head".into(),
        };
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: snapshot.id.clone(),
            repo_hash: "repo".into(),
            owner_kind: "issue".into(),
            owner_number: 4735,
            identity: identity.clone(),
            capability_generation: 1,
        };
        let pending = super::super::launch_payload::PendingFreshExecutionLaunch {
            operation_id: "operation".into(),
            project_root: root.path().into(),
            worktree_path: root.path().into(),
            owner: crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 4735,
            },
            request: crate::cli::execution_state::SuccessorRequest {
                operation_id: "operation".into(),
                principal_id: "principal".into(),
                work_id: None,
                source: "fixture".into(),
                session_binding_id: "binding".into(),
                initial_session_id: snapshot.id.clone(),
                entrypoint: "fixture".into(),
                requested_at: chrono::Utc::now(),
            },
            binding: binding.clone(),
            session_identity: gwt_agent::SessionExecutionIdentity {
                session_id: snapshot.id.clone(),
                worktree_path: snapshot.worktree_path.clone(),
                project_state_root: snapshot.project_state_root.clone(),
                repo_hash: snapshot.repo_hash.clone(),
                branch: snapshot.branch.clone(),
                agent_id: snapshot.agent_id.clone(),
                linked_issue_number: snapshot.linked_issue_number,
                execution_binding: binding.clone(),
            },
            readiness_nonce: "nonce".into(),
            predecessor_binding: identity,
            base_branch: None,
            linked_issue_number: None,
            resume_context: None,
            launch_feedback_context: None,
        };
        let current = Arc::new(AtomicBool::new(true));
        let fence = ReadinessFence {
            id: "pane".into(),
            incarnation: 1,
            operation_id: "operation".into(),
            binding,
            token: Some(token.clone()),
        };
        state
            .install_pane("pane".into(), 1, pane, current.clone())
            .unwrap();
        state.runtimes.get_mut("pane").unwrap().agent = Some(AgentRuntimeRecord {
            feedback: None,
            monitor_owner: None,
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: false,
            monitor_retry_at: None,
            monitor_startup_at: None,
            current: current.clone(),
            active: active_fixture(root.path(), &snapshot),
            snapshot: snapshot.clone(),
            capability_token: Some(token.clone()),
            issuer: issuer.clone(),
            pending_fresh: Some(pending.clone()),
            pending_continue: None,
            purpose_title: None,
            readiness_deadline: None,
        });
        state.readiness.insert(
            "pane".into(),
            ReadinessJob {
                fence: fence.clone(),
                pending: ReadinessPending::Fresh(Box::new(pending)),
                current: current.clone(),
                replies: Vec::new(),
                teardown_after_rollback: true,
            },
        );
        let (reply, mut completion) = oneshot::channel();
        let runtime = state.runtimes.remove("pane").unwrap();
        state.start_close("pane".into(), runtime, CloseCause::ExplicitClose, reply);
        assert!(!current.load(Ordering::Acquire));
        assert!(
            matches!(
                completion.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "close must wait for its accepted readiness worker"
        );
        // The canonical finalizer already removed this exact Session before
        // returning RolledBack. The owner must now perform physical teardown only.
        std::fs::remove_file(sessions.join(format!("{}.toml", snapshot.id))).unwrap();
        state.readiness_finalized(
            fence,
            ReadinessOutcome::Fresh(
                super::super::continuation::FreshExecutionFinalizationOutcome::RolledBack(
                    "fixture rollback".into(),
                ),
            ),
            None,
        );
        let (mut runtime, result) = loop {
            if let OwnerMessage::Closed {
                runtime, result, ..
            } = receiver
                .recv_timeout(std::time::Duration::from_secs(10)) // test-hygiene: allow-wall-clock-deadline Bounds the actual rollback worker completion before inspecting exact teardown.
                .unwrap()
            {
                break (runtime, result);
            }
        };
        let _ = crate::session_finalizer::kill_and_reap(&runtime.pty, "pane");
        if let Some(worker) = runtime.output_thread.take() {
            worker.join().unwrap();
        }
        if let Some(worker) = runtime.status_thread.take() {
            worker.join().unwrap();
        }
        assert!(
            result.is_ok(),
            "RolledBack must not repeat durable rollback or reread the deleted Session: {result:?}"
        );
        assert!(
            !issuer.authenticates_token(&token),
            "exact rolled-back capability must be revoked"
        );
    }

    #[tokio::test]
    async fn project_transport_invalidates_only_its_fixed_project() {
        let root = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let project_key = gwt_core::paths::resolve_project_scope(root.path())
            .hash
            .to_string();
        state.project_key = project_key.clone();
        let mut events = handle.subscribe();
        let actor = thread::spawn(move || run_fixture_owner(state, receiver));
        handle.transport_event(TransportEvent::WorkspaceProjectionChanged {
            project_root: root.path().into(),
        });
        handle
            .request(OwnerRequest {
                project_key: project_key.clone(),
                command: OwnerCommand::Health,
            })
            .await
            .unwrap();
        let own_changed = matches!(events.try_recv(), Ok(OwnerEvent::Invalidated));
        handle.transport_event(TransportEvent::WorkspaceProjectionChanged {
            project_root: foreign.path().into(),
        });
        handle
            .request(OwnerRequest {
                project_key,
                command: OwnerCommand::Health,
            })
            .await
            .unwrap();
        let foreign_changed = events.try_recv().is_ok();
        handle.shutdown().await;
        actor.join().unwrap();
        assert!(
            own_changed,
            "authenticated project transport must dispatch to its owner"
        );
        assert!(
            !foreign_changed,
            "a foreign projection must not invalidate this owner"
        );
    }

    #[tokio::test]
    async fn readiness_transport_refuses_unbound_grant_before_reply_barrier() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let project_key = gwt_core::paths::resolve_project_scope(root.path())
            .hash
            .to_string();
        state.project_key = project_key.clone();
        let issued = state.issuer.issue(root.path(), "unbound-session").unwrap();
        let grant = state.issuer.grant_for_test(&issued.token).unwrap();
        let actor = thread::spawn(move || run_fixture_owner(state, receiver));
        let (reply, response) = mpsc::channel();
        handle.transport_event(TransportEvent::FreshExecutionReadyResend {
            grant,
            request: crate::AgentExecutionContinuationRequest {
                schema_version: crate::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "unbound-readiness".into(),
                readiness_nonce: None,
            },
            reply,
        });
        handle
            .request(OwnerRequest {
                project_key,
                command: OwnerCommand::Health,
            })
            .await
            .unwrap();
        let result = response.try_recv();
        handle.shutdown().await;
        actor.join().unwrap();
        assert!(
            matches!(result, Ok(Err(error)) if error.code == crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch),
            "an unbound readiness request must receive an explicit owner refusal"
        );
    }

    #[test]
    fn unbound_close_preserves_metadata_and_rejects_replacement_proof() {
        let root = tempfile::tempdir().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let (snapshot, pane) = unbound_fixture(root.path());
        let mut edited = snapshot.clone();
        edited.agent_name = Some("renamed while running".into());
        edited.save(&sessions).unwrap();
        set_fixture_incarnation(&snapshot, 2);
        assert!(
            finish_agent_session(
                &sessions,
                &snapshot,
                1,
                &pane.shared_pty(),
                "pane",
                Some(false),
                None
            )
            .is_err(),
            "replacement runtime proof must refuse close"
        );
        set_fixture_incarnation(&snapshot, 1);
        let result = finish_agent_session(
            &sessions,
            &snapshot,
            1,
            &pane.shared_pty(),
            "pane",
            Some(false),
            None,
        );
        let _ = crate::session_finalizer::kill_and_reap(&pane.shared_pty(), "pane");
        assert!(
            result.is_ok(),
            "metadata changes must not permanently block exact close: {result:?}"
        );
        let closed =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", snapshot.id))).unwrap();
        assert_eq!(closed.agent_name, edited.agent_name);
        assert_eq!(closed.status, gwt_agent::AgentStatus::Stopped);
        assert!(!closed.restore_window_on_startup);
    }

    #[tokio::test]
    async fn shutdown_retains_failed_close_and_accepts_exact_retry() {
        let root = tempfile::tempdir().unwrap();
        let (snapshot, pane) = unbound_fixture(root.path());
        let pty = pane.shared_pty();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let current = Arc::new(AtomicBool::new(true));
        state
            .install_pane("pane".into(), 1, pane, current.clone())
            .unwrap();
        state.runtimes.get_mut("pane").unwrap().agent = Some(AgentRuntimeRecord {
            feedback: None,
            monitor_owner: None,
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: false,
            monitor_retry_at: None,
            monitor_startup_at: None,
            current,
            active: active_fixture(root.path(), &snapshot),
            snapshot: snapshot.clone(),
            capability_token: None,
            issuer: state.issuer.clone(),
            pending_fresh: None,
            pending_continue: None,
            purpose_title: None,
            readiness_deadline: None,
        });
        set_fixture_incarnation(&snapshot, 2);
        let owner = thread::spawn(move || run_fixture_owner(state, receiver));
        let (shutdown, mut done) = oneshot::channel();
        handle
            .sender
            .send(OwnerMessage::Shutdown(shutdown))
            .ok()
            .unwrap();
        // test-hygiene: allow-wall-clock-deadline Bounds actual owner reconciliation progress during shutdown; it does not assert elapsed latency.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let health = handle.request(request(OwnerCommand::Health)).await;
            if health.is_err() {
                let _ = crate::session_finalizer::kill_and_reap(&pty, "pane");
                owner.join().unwrap();
                panic!("draining owner must retain failed close and accept reconciliation requests: {health:?}");
            }
            let health = health.unwrap();
            assert_eq!(health["draining"], true);
            if health["reconciliation_required"] == true {
                break;
            }
            // test-hygiene: allow-wall-clock-deadline Bounds actual owner reconciliation progress during shutdown.
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            matches!(done.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "failed close cannot acknowledge shutdown"
        );
        set_fixture_incarnation(&snapshot, 1);
        let result = handle
            .request(request(OwnerCommand::Close {
                id: "pane".into(),
                incarnation: 1,
            }))
            .await;
        assert!(
            result.is_ok(),
            "exact close must remain retryable during drain: {result:?}"
        );
        done.await.unwrap();
        owner.join().unwrap();
    }

    #[tokio::test]
    async fn failed_agent_retirement_retains_capability_and_exact_retry() {
        let root = tempfile::tempdir().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let (snapshot, pane) = unbound_fixture(root.path());
        let pty = pane.shared_pty();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let issuer = state.issuer.clone();
        let token = issuer.issue(root.path(), &snapshot.id).unwrap().token;
        let (reply, completion) = oneshot::channel();
        state.pending.insert("pane".into(), 1);
        state.agent_operations.insert(
            "operation".into(),
            AgentOperation {
                session_id: None,
                early_events: VecDeque::new(),
                request: json!(null),
                id: "pane".into(),
                incarnation: 1,
                current: Arc::new(AtomicBool::new(false)),
                replies: vec![reply],
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: None,
            },
        );
        set_fixture_incarnation(&snapshot, 2);
        let input = InstallAgentLaunchInput {
            auto_resume_source_session_id: None,
            auto_resume_source_snapshot: None,
            current: Arc::new(AtomicBool::new(false)),
            failure_session: None,
            physical_spawn_started: false,
            failed_launch: None,
            had_window: true,
            is_continue_work: false,
            issue_link_cache_dir: root.path().into(),
            launch_feedback_context: None,
            live_pm_session_ids: Default::default(),
            materializer_id: "host".into(),
            origin: gwt_agent::SessionLaunchOrigin::Launch,
            pending_continue_work: None,
            pending_fresh_execution: None,
            pending_pm_project_root: None,
            project_root: root.path().into(),
            removed_genesis_session: None,
            sessions_dir: sessions.clone(),
            stage_id: 1,
            tab_id: "project".into(),
            window_id: "pane".into(),
            workspace_resume_context: None,
            incarnation: 1,
        };
        let success = PreparedAgentLaunchSuccess {
            project_root: root.path().into(),
            active_session: active_fixture(root.path(), &snapshot),
            pane,
            incarnation: 1,
            initial_prompt_file: None,
            capability_token: Some(token.clone()),
            member_reservation: None,
            pending_fresh_execution: None,
            pending_continue_work: None,
            workspace_projection_updated: false,
            pm_registration: None,
            cached_session: Some(snapshot.clone()),
            session_snapshot: Some(snapshot.clone()),
            predecessor_session_id: None,
            materialized_genesis: None,
        };
        let owner = thread::spawn(move || run_fixture_owner(state, receiver));
        handle
            .sender
            .send(OwnerMessage::AgentPrepared(Box::new(AgentPrepared {
                monitor_failure_committed: false,
                launch_failure: None,
                restore: None,
                registry_cleanup: None,
                retirement_reason: None,
                cancelled_cleanup: false,
                durable_candidate_removed: false,
                purpose_title: None,
                operation_id: "operation".into(),
                input,
                result: Ok(success),
            })))
            .ok()
            .unwrap();
        assert!(completion.await.unwrap().is_err());
        assert!(
            issuer.authenticates_token(&token),
            "failed cleanup must retain exact capability evidence"
        );
        set_fixture_incarnation(&snapshot, 1);
        let result = handle
            .request(request(OwnerCommand::Close {
                id: "pane".into(),
                incarnation: 1,
            }))
            .await;
        let _ = crate::session_finalizer::kill_and_reap(&pty, "pane");
        handle.shutdown().await;
        owner.join().unwrap();
        assert!(
            result.is_ok(),
            "failed retirement lost captured launch instead of allowing exact retry: {result:?}"
        );
        assert!(!issuer.authenticates_token(&token));
    }
    #[tokio::test]
    async fn shutdown_waits_for_an_already_running_close_finalizer() {
        let root = tempfile::tempdir().unwrap();
        let capabilities = AgentCapabilityRegistry::default();
        let (sender, receiver) = mpsc::channel();
        let (events, _) = broadcast::channel(16);
        let handle = OwnerHandle {
            sender: sender.clone(),
            events: events.clone(),
            residency: tokio::sync::watch::channel(ResidencySnapshot::default()).1,
        };
        let pane = super::super::spawn_unbound_pane(
            "pane",
            SpawnConfig {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "read line".into()],
                cols: 80,
                rows: 24,
                env: Default::default(),
                remove_env: Vec::new(),
                cwd: Some(root.path().to_path_buf()),
            },
            None,
            1,
            None,
            false,
        )
        .unwrap();
        let child_pid = pane.pty().process_id().unwrap();
        let (entered, wait_entered) = oneshot::channel();
        let (release, wait_release) = mpsc::channel();
        let finalizer = thread::spawn(move || {
            let _ = entered.send(());
            wait_release.recv().unwrap();
        });
        let runtime = WindowRuntime {
            project_root: root.path().into(),
            policy: Default::default(),
            _initial_prompt_file: None,
            registry_cleanup: None,
            close_handoff: None,
            close_monitor_snapshot: None,
            pm_lifecycle: None,
            rollback_monitor_failure: None,
            member_cleanup: None,
            started_at_ms: 0,
            child_started_at: None,
            current: Arc::new(AtomicBool::new(true)),
            detail: None,
            last_output_at: None,
            pty: pane.shared_pty(),
            incarnation: 1,
            pane: Arc::new(Mutex::new(pane)),
            output_thread: None,
            status_thread: Some(finalizer),
            pty_status: crate::WindowProcessStatus::Running,
            hook_status: None,
            output_bytes: 0,
            agent: None,
        };
        let state = OwnerState {
            issue_client_factory: super::super::launch::default_issue_client_factory(),
            test_dispatch_barriers: Vec::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_operation_clock: None,
            policy_usage_accounts: Vec::new(),
            policy_capacity_resumes: HashMap::new(),
            agent_registry: super::super::agent_dispatch::load_owner_registry(root.path()),
            shared_registry_initialization: None,
            registry_projection: crate::persistence::empty_workspace_state(),
            pty_writers: Arc::new(std::sync::RwLock::new(HashMap::new())),
            agent_dispatch_jobs: 0,
            deferred_agent_frontend: HashMap::new(),
            pending_agent_self_closes: HashMap::new(),
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_capacity_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            next_launch_checkpoint: None,
            #[cfg(any(test, feature = "test-gh-guard"))]
            test_pty_start_gate: None,
            root: root.path().to_path_buf(),
            project_key: "0123456789abcdef".into(),
            runtimes: HashMap::from([("pane".into(), runtime)]),
            clients: ClientHub::default(),
            issuer: AgentCapabilityIssuer::new(
                "http://127.0.0.1/hook".into(),
                "ws://127.0.0.1/pane".into(),
                "ws://127.0.0.1/pane".into(),
                capabilities.clone(),
            ),
            host_instance_id: "host".into(),
            sender,
            events,
            pending: HashMap::new(),
            closing: HashMap::new(),
            next_incarnation: 1,
            pending_transport: VecDeque::new(),
            stopping: false,
            gui_presence: GuiPresence::default(),
            next_gui_presence_id: 0,
            residency: tokio::sync::watch::channel(ResidencySnapshot::default()).0,
            shutdown_replies: Vec::new(),
            retiring: 0,
            input_jobs: 0,
            switch_job_inflight: false,
            pm_wake_permits: Default::default(),
            agent_operations: HashMap::new(),
            retirements: HashMap::new(),
            readiness: HashMap::new(),
            close_replies: HashMap::new(),
            continue_work_outcomes: HashMap::new(),
            continue_work_waiters: HashMap::new(),
            deferred_readiness_closes: HashMap::new(),
        };
        let owner_thread = thread::spawn(move || run_fixture_owner(state, receiver));
        let (close_reply, close) = oneshot::channel();
        assert!(handle
            .sender
            .send(OwnerMessage::Request(
                OwnerRequest {
                    project_key: "0123456789abcdef".into(),
                    command: OwnerCommand::Close {
                        id: "pane".into(),
                        incarnation: 1
                    },
                },
                close_reply
            ))
            .is_ok());
        assert!(handle
            .request(OwnerRequest {
                project_key: "0123456789abcdef".into(),
                command: OwnerCommand::Health
            })
            .await
            .is_ok());
        wait_entered.await.unwrap();
        let (shutdown, mut shutdown_done) = oneshot::channel();
        assert!(handle.sender.send(OwnerMessage::Shutdown(shutdown)).is_ok());
        // A rejected launch is a mailbox barrier after shutdown was received.
        let health = handle
            .request(OwnerRequest {
                project_key: "0123456789abcdef".into(),
                command: OwnerCommand::LaunchShell {
                    id: "rejected".into(),
                    command: "/bin/sh".into(),
                    args: vec![],
                    cols: 80,
                    rows: 24,
                },
            })
            .await;
        assert!(health.is_err(), "a draining owner must reject new requests");
        let premature = shutdown_done.try_recv().is_ok();
        release.send(()).unwrap();
        assert!(
            !premature,
            "shutdown acknowledged before the captured reader/status workers joined"
        );
        shutdown_done.await.unwrap();
        assert!(close.await.unwrap().is_ok());
        owner_thread.join().unwrap();
        assert!(
            !crate::process::is_process_alive(child_pid),
            "shutdown must reap the captured PTY child"
        );
    }
    #[tokio::test]
    async fn natural_exit_commits_session_receipt_and_stopped_state_before_terminal_projection() {
        let root = tempfile::tempdir().unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let (mut snapshot, pane) = unbound_fixture(root.path());
        snapshot.update_status(gwt_agent::AgentStatus::Running);
        snapshot.restore_window_on_startup = true;
        snapshot.save(&sessions).unwrap();
        let child_pid = pane.pty().process_id().unwrap();
        let issued = state.issuer.issue(root.path(), &snapshot.id).unwrap();
        let current = Arc::new(AtomicBool::new(true));
        state
            .install_pane("pane".into(), 1, pane, current.clone())
            .unwrap();
        state.runtimes.get_mut("pane").unwrap().agent = Some(AgentRuntimeRecord {
            feedback: None,
            monitor_owner: None,
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: false,
            monitor_retry_at: None,
            monitor_startup_at: None,
            current,
            active: active_fixture(root.path(), &snapshot),
            snapshot: snapshot.clone(),
            capability_token: Some(issued.token),
            issuer: state.issuer.clone(),
            pending_fresh: None,
            pending_continue: None,
            purpose_title: None,
            readiness_deadline: None,
        });
        let mut events = handle.subscribe();
        let actor = thread::spawn(move || run_fixture_owner(state, receiver));
        handle
            .request(request(OwnerCommand::Input {
                id: "pane".into(),
                incarnation: 1,
                data: b"finish\n".to_vec(),
            }))
            .await
            .unwrap();
        // test-hygiene: allow-wall-clock-deadline Bounds the actual terminal owner event; durable/process assertions determine success.
        let terminal = tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                if matches!(events.recv().await.unwrap(), OwnerEvent::Status { ref id, incarnation: 1, exit_confirmed: true, .. } if id == "pane") { break; }
            }
        }).await;
        handle.request(request(OwnerCommand::Health)).await.unwrap();
        let observed =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", snapshot.id))).unwrap();
        let child_gone = !crate::process::is_process_alive(child_pid);
        handle.shutdown().await;
        actor.join().unwrap();
        assert!(
            terminal.is_ok(),
            "the actual owned PTY must report its exact exit"
        );
        assert_eq!(
            observed.status,
            gwt_agent::AgentStatus::Stopped,
            "natural exit must be finalized by its physical project owner"
        );
        assert_eq!(
            observed.last_exit_code,
            Some(0),
            "Session must retain the physical exit receipt"
        );
        assert!(
            !observed.restore_window_on_startup,
            "a clean Agent exit closes its saved window"
        );
        assert!(
            child_gone,
            "terminal projection must follow owned child exit"
        );
    }
    #[tokio::test]
    async fn cached_completed_launch_does_not_block_exact_stopped_husk_close() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let root = home.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut snapshot = gwt_agent::Session::new(
            &root,
            "main",
            gwt_agent::AgentId::Custom("owner-fixture".into()),
        );
        snapshot.status = gwt_agent::AgentStatus::Stopped;
        snapshot.restore_window_on_startup = true;
        snapshot.save(&sessions).unwrap();
        let (mut state, receiver, handle) = owner_fixture(&root);
        state.project_key = gwt_core::paths::project_scope_hash(&root).to_string();
        let key = state.project_key.clone();
        let id = format!("{key}::stopped");
        let registry = state.agent_registry.as_ref().unwrap().clone();
        let row = super::super::agent_dispatch::owned_process_window_state(
            &root,
            &id,
            Some(&snapshot),
            None,
            None,
            None,
        )
        .unwrap();
        super::super::agent_dispatch::publish_owned_process_window(
            &registry,
            &root,
            &id,
            None,
            row,
            &AtomicBool::new(true),
        )
        .map_err(|error| error.detail)
        .unwrap();
        state.refresh_registry_projection();
        state.agent_operations.insert(
            "completed".into(),
            AgentOperation {
                session_id: Some(snapshot.id.clone()),
                early_events: VecDeque::new(),
                request: json!(null),
                id: id.clone(),
                incarnation: 1,
                current: Arc::new(AtomicBool::new(false)),
                replies: Vec::new(),
                cancel_replies: Vec::new(),
                cancellation_cleaned: false,
                manual_predecessor_stopped: None,
                retained_manual_failure: None,
                pending_continue: None,
                failed_session: None,
                result: Some(Ok(json!({"id":id}))),
            },
        );
        let owner_home = home.path().to_path_buf();
        let actor = thread::spawn(move || {
            let _home = gwt_core::test_support::ScopedGwtHome::set(owner_home);
            run_fixture_owner(state, receiver);
        });
        let result = handle
            .request(OwnerRequest {
                project_key: key,
                command: OwnerCommand::CloseStoppedSession {
                    session_id: snapshot.id.clone(),
                    expected: Box::new(snapshot.clone()),
                },
            })
            .await;
        // test-hygiene: allow-wall-clock-deadline Hang guard for the actual owner mailbox drain, not an elapsed-time assertion.
        tokio::time::timeout(std::time::Duration::from_secs(10), handle.shutdown())
            .await
            .expect("stopped husk cleanup must let the owner drain");
        actor.join().unwrap();
        assert!(
            result.is_ok(),
            "cached completed receipt is not an active/pending Session: {result:?}"
        );
        assert!(
            !gwt_agent::Session::load(&sessions.join(format!("{}.toml", snapshot.id)))
                .unwrap()
                .restore_window_on_startup
        );
        assert!(!crate::persistence::load_workspace_state(
            &crate::persistence::workspace_state_path(&root)
        )
        .unwrap()
        .windows
        .iter()
        .any(|row| row.id == "stopped"));
    }
    #[tokio::test]
    async fn shutdown_keeps_accepted_self_close_explicit_before_origin_commit() {
        let root = tempfile::tempdir().unwrap();
        let (mut snapshot, pane) = unbound_fixture(root.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        snapshot.restore_window_on_startup = true;
        snapshot.save(&sessions).unwrap();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        state.project_key = gwt_core::paths::project_scope_hash(root.path()).to_string();
        let id = format!("{}::pane", state.project_key);
        let registry = state.agent_registry.as_ref().unwrap().clone();
        let row = super::super::agent_dispatch::owned_process_window_state(
            root.path(),
            &id,
            Some(&snapshot),
            None,
            None,
            None,
        )
        .unwrap();
        super::super::agent_dispatch::publish_owned_process_window(
            &registry,
            root.path(),
            &id,
            None,
            row,
            &AtomicBool::new(true),
        )
        .map_err(|error| error.detail)
        .unwrap();
        state.refresh_registry_projection();
        state
            .install_pane(id.clone(), 1, pane, Arc::new(AtomicBool::new(true)))
            .unwrap();
        let token = state.issuer.issue(root.path(), &snapshot.id).unwrap().token;
        let grant = state.issuer.grant_for_test(&token).unwrap();
        let ticket = state.issuer.begin_self_close_if_current(&grant).unwrap();
        let current = state.runtimes[&id].current.clone();
        let mut active = active_fixture(root.path(), &snapshot);
        active.window_id = id.clone();
        state.runtimes.get_mut(&id).unwrap().agent = Some(AgentRuntimeRecord {
            feedback: None,
            monitor_owner: None,
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: false,
            monitor_retry_at: None,
            monitor_startup_at: None,
            current,
            active,
            snapshot: snapshot.clone(),
            capability_token: Some(token),
            issuer: state.issuer.clone(),
            pending_fresh: None,
            pending_continue: None,
            purpose_title: None,
            readiness_deadline: None,
        });
        let (responder, mut direct_response) =
            crate::agent_capability::AgentSelfCloseResponder::channel();
        state.agent_dispatch_jobs = 1;
        state.agent_dispatch_finished(super::super::agent_dispatch::AgentDispatchCompletion {
            client_id: "origin".into(),
            outcome: super::super::agent_frontend::AgentFrontendDispatchOutcome::Dispatched(
                Vec::new(),
            ),
            close: None,
            acceptance: Some(super::super::agent_dispatch::AgentSelfCloseAcceptance {
                pending: super::super::agent_dispatch::PendingAgentSelfClose {
                    ticket,
                    window_id: id.clone(),
                    tab_id: state.project_key.clone(),
                    raw_id: "pane".into(),
                    session_id: snapshot.id.clone(),
                    window_lifecycle_generation: 1,
                },
                responder,
                request_id: uuid::Uuid::new_v4().to_string(),
            }),
            invalidated: false,
        });
        let accepted = direct_response.try_recv().unwrap().unwrap();
        assert!(!state.issuer.grant_is_current(&grant));
        assert!(state.runtimes.contains_key(&id));
        // The direct response attempt has not completed. Its completion must
        // choose the exact explicit close even if shutdown is already queued.
        state.begin_shutdown(None);
        drop(accepted);
        let actor = thread::spawn(move || run_fixture_owner(state, receiver));
        // test-hygiene: allow-wall-clock-deadline Hang guard for actual physical owner drain, not an elapsed-time assertion.
        tokio::time::timeout(std::time::Duration::from_secs(10), handle.shutdown())
            .await
            .unwrap();
        actor.join().unwrap();
        let stopped =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", snapshot.id))).unwrap();
        assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
        assert!(
            !stopped.restore_window_on_startup,
            "accepted explicit self-close must survive a lifetime drain before origin commit"
        );
        assert!(
            !crate::persistence::load_workspace_state(&crate::persistence::workspace_state_path(
                root.path()
            ))
            .unwrap()
            .windows
            .iter()
            .any(|row| row.id == "pane"),
            "accepted self-close must durably retire its exact row"
        );
    }

    #[tokio::test]
    async fn self_close_origin_disconnect_before_acceptance_keeps_lifetime_restore_intent() {
        let root = tempfile::tempdir().unwrap();
        let (mut snapshot, pane) = unbound_fixture(root.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        snapshot.restore_window_on_startup = true;
        snapshot.save(&sessions).unwrap();
        let (mut state, receiver, handle) = owner_fixture(root.path());
        let mut events = handle.subscribe();
        let id = "pane".to_string();
        state
            .install_pane(id.clone(), 1, pane, Arc::new(AtomicBool::new(true)))
            .unwrap();
        let token = state.issuer.issue(root.path(), &snapshot.id).unwrap().token;
        let grant = state.issuer.grant_for_test(&token).unwrap();
        let ticket = state.issuer.begin_self_close_if_current(&grant).unwrap();
        let current = state.runtimes[&id].current.clone();
        state.runtimes.get_mut(&id).unwrap().agent = Some(AgentRuntimeRecord {
            feedback: None,
            monitor_owner: None,
            monitor_terminal_failure: None,
            monitor_answer_inflight: false,
            binding_repair_inflight: false,
            monitor_source: None,
            monitor_ack_replies: Vec::new(),
            monitor_adopt_claim: false,
            monitor_commit_inflight: false,
            monitor_ready: false,
            monitor_retry_at: None,
            monitor_startup_at: None,
            current,
            active: active_fixture(root.path(), &snapshot),
            snapshot: snapshot.clone(),
            capability_token: Some(token),
            issuer: state.issuer.clone(),
            pending_fresh: None,
            pending_continue: None,
            purpose_title: None,
            readiness_deadline: None,
        });
        let (responder, direct_response) =
            crate::agent_capability::AgentSelfCloseResponder::channel();
        drop(direct_response);
        state.agent_dispatch_jobs = 1;
        state.agent_dispatch_finished(super::super::agent_dispatch::AgentDispatchCompletion {
            client_id: "disconnected-origin".into(),
            outcome: super::super::agent_frontend::AgentFrontendDispatchOutcome::Dispatched(
                Vec::new(),
            ),
            close: None,
            acceptance: Some(super::super::agent_dispatch::AgentSelfCloseAcceptance {
                pending: super::super::agent_dispatch::PendingAgentSelfClose {
                    ticket,
                    window_id: id.clone(),
                    tab_id: state.project_key.clone(),
                    raw_id: id.clone(),
                    session_id: snapshot.id.clone(),
                    window_lifecycle_generation: 1,
                },
                responder,
                request_id: uuid::Uuid::new_v4().to_string(),
            }),
            invalidated: false,
        });
        assert!(state.issuer.grant_is_current(&grant));
        assert!(state.runtimes.contains_key(&id));
        assert!(state.pending_agent_self_closes.is_empty());
        assert!(events.try_recv().is_err());
        let actor = thread::spawn(move || run_fixture_owner(state, receiver));
        // test-hygiene: allow-wall-clock-deadline Hang guard for actual physical owner drain, not an elapsed-time assertion.
        tokio::time::timeout(std::time::Duration::from_secs(10), handle.shutdown())
            .await
            .unwrap();
        actor.join().unwrap();
        let stopped =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", snapshot.id))).unwrap();
        assert_eq!(stopped.status, gwt_agent::AgentStatus::Stopped);
        assert!(stopped.restore_window_on_startup);
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;
    use std::time::Duration;

    struct TestPtyGuard(Arc<PtyHandle>);
    impl Drop for TestPtyGuard {
        fn drop(&mut self) {
            let _ = crate::session_finalizer::kill_and_reap(&self.0, "drop-runtime");
            self.0.release_descriptors();
        }
    }

    #[tokio::test]
    async fn cancelled_preinstall_pane_retires_its_captured_writer_generation() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let root = home.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let project_key = gwt_core::repo_hash::compute_path_hash(&root).to_string();
        let id = format!("{project_key}::cancelled-pane");
        let (command, args) = if cfg!(windows) {
            ("cmd", vec!["/d", "/s", "/c", "set /p input="])
        } else {
            ("/bin/cat", vec![])
        };
        let mut pane = Pane::new(
            id.clone(),
            command.into(),
            args.into_iter().map(str::to_owned).collect(),
            80,
            24,
            HashMap::new(),
            Some(root.clone()),
        )
        .unwrap();
        // Source-level writer reconciliation: this parser-fed report is not
        // evidence of a native startup-query ordering or counterexample.
        let report = pane.process_owner_output(b"\x1b[6n").cursor_reports[0];
        let pty = pane.shared_pty();
        let _cleanup = TestPtyGuard(pty.clone());
        let owner = OwnerState::start(
            root,
            project_key,
            ClientHub::default(),
            AgentCapabilityIssuer::new(
                "http://127.0.0.1/hook".into(),
                "ws://127.0.0.1/pane".into(),
                "ws://127.0.0.1/pane".into(),
                crate::agent_capability::AgentCapabilityRegistry::default(),
            ),
            "retirement-proof".into(),
            gwt_core::daemon::DaemonStartIntent::Standalone,
        );
        let (reply, completion) = oneshot::channel();
        // The physical worker completed after its pending launch disappeared.
        // Let the production owner perform its normal captured retirement.
        owner
            .sender
            .send(OwnerMessage::Prepared {
                id,
                incarnation: 1,
                pane: Ok(Box::new(pane)),
                reply: Some(reply),
                operation_id: None,
                initial_prompt_file: None,
            })
            .ok()
            .unwrap();
        let result = completion.await.unwrap();
        // Exercise the physical generation fence independently of the expired
        // operation's logical current flag; no runtime authority is restored.
        let response = pty.write_cursor_position_report(report, &AtomicBool::new(true));
        owner.shutdown().await;
        assert!(result.is_err_and(|error| error.contains("cancelled before installation")));
        assert!(pty.tree_exit_confirmed());
        assert!(
            response.is_err_and(|error| error.to_string().contains("generation")),
            "successful captured retirement must invalidate its physical writer"
        );

        // Keep logical cancellation independent from that physical fence. A
        // real configured launch substitutes its operation's current flag
        // after installation; its output reader must observe the same flag.
        use futures_util::FutureExt as _;
        use tracing_subscriber::layer::SubscriberExt as _;
        struct CursorLayer {
            id: String,
            entered: mpsc::Sender<u64>,
            release: Mutex<mpsc::Receiver<()>>,
            reader: Mutex<Option<thread::ThreadId>>,
            writes: Arc<std::sync::atomic::AtomicUsize>,
            all_writes: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CursorLayer {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _context: tracing_subscriber::layer::Context<'_, S>,
            ) {
                #[derive(Default)]
                struct Fields {
                    stage: String,
                    id: String,
                    seq: u64,
                    reports: u64,
                }
                impl tracing::field::Visit for Fields {
                    fn record_debug(
                        &mut self,
                        _field: &tracing::field::Field,
                        _value: &dyn std::fmt::Debug,
                    ) {
                    }
                    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                        match field.name() {
                            "stage" => self.stage = value.into(),
                            "window_id" => self.id = value.into(),
                            _ => {}
                        }
                    }
                    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                        match field.name() {
                            "seq" => self.seq = value,
                            "report_count" => self.reports = value,
                            _ => {}
                        }
                    }
                }
                if !matches!(
                    event.metadata().target(),
                    "gwt_input_trace" | "gwt_terminal_protocol"
                ) {
                    return;
                }
                let mut fields = Fields::default();
                event.record(&mut fields);
                if fields.stage == "owner_cursor_response"
                    && fields.id == self.id
                    && fields.reports > 0
                {
                    let first = {
                        let mut reader = self.reader.lock().unwrap();
                        if reader.is_some() {
                            false
                        } else {
                            *reader = Some(thread::current().id());
                            true
                        }
                    };
                    if first {
                        self.entered.send(fields.seq).unwrap();
                        self.release.lock().unwrap().recv().unwrap();
                    }
                } else if fields.stage == "pty_writer" {
                    self.all_writes.fetch_add(1, Ordering::AcqRel);
                    if self.reader.lock().unwrap().as_ref() == Some(&thread::current().id()) {
                        self.writes.fetch_add(1, Ordering::AcqRel);
                    }
                }
            }
        }
        let root = home.path().join("repo");
        let project_key = gwt_core::paths::project_scope_hash(&root).to_string();
        let id = format!("{project_key}::logical-cursor-fence");
        let (entered, observed_query) = mpsc::channel();
        let (release, resume_query) = mpsc::channel();
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let all_writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(CursorLayer {
            id: id.clone(),
            entered,
            release: Mutex::new(resume_query),
            reader: Mutex::new(None),
            writes: writes.clone(),
            all_writes: all_writes.clone(),
        }));
        let owner = tracing::dispatcher::with_default(&dispatch, || {
            OwnerState::start(
                root.clone(),
                project_key.clone(),
                ClientHub::default(),
                AgentCapabilityIssuer::new(
                    "http://127.0.0.1/hook".into(),
                    "ws://127.0.0.1/pane".into(),
                    "ws://127.0.0.1/pane".into(),
                    crate::agent_capability::AgentCapabilityRegistry::default(),
                ),
                "logical-cursor-proof".into(),
                gwt_core::daemon::DaemonStartIntent::Standalone,
            )
        });
        let mut events = owner.subscribe();
        let verified = std::panic::AssertUnwindSafe(async {
            let (command, args) = if cfg!(windows) {
                // ConPTY emits its real initial query before this direct Shell
                // reaches its input loop; no gate preanswer is involved.
                ("cmd", vec!["/d", "/s", "/c", "set /p input="])
            } else {
                ("/bin/sh", vec!["-c", "printf '\\033[6n'; read line"])
            };
            let receipt = owner.request(OwnerRequest {
                project_key: project_key.clone(),
                command: OwnerCommand::LaunchConfiguredShell {
                    id: id.clone(), operation_id: "logical-cursor-operation".into(),
                    config: Box::new(crate::launch_wizard::ShellLaunchConfig {
                        working_dir: Some(root.clone()), branch: None, base_branch: None,
                        display_name: "Shell".into(), runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                        docker_service: None, docker_lifecycle_intent: Default::default(),
                        windows_shell: None, env_vars: Default::default(), remove_env: Vec::new(),
                        command_override: Some(command.into()),
                        command_args_override: Some(args.into_iter().map(str::to_owned).collect()),
                    }), cols: 80, rows: 24,
                },
            }).await.unwrap();
            // test-hygiene: allow-wall-clock-deadline The channel proves that the real reader parsed a query outside the Pane lock before logical cancellation.
            let query_seq = observed_query.recv_timeout(gwt_core::deadline_budget::HANG_GUARD).unwrap();
            let pane = owner.test_probe_runtime(&id, receipt["incarnation"].as_u64().unwrap()).await.unwrap();
            pane.current.store(false, Ordering::Release);
            // A scoped positive control proves physical-writer events reach
            // this collector and that physical generation is still active.
            tracing::dispatcher::with_default(&dispatch, || pane.pty.write_machine_input(b"")).unwrap();
            assert!(all_writes.load(Ordering::Acquire) > 0);
            let reservation = pane.pty.reserve_input_transaction().unwrap();
            let transaction = reservation.transaction_revision();
            let ordinary = pane.pty.ordinary_input_observation();
            release.send(()).unwrap();
            // test-hygiene: allow-wall-clock-deadline The exact Output seq follows all responses for this parsed query; elapsed time is not the ordering proof.
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                loop {
                    if matches!(events.recv().await.unwrap(), OwnerEvent::Output { id: ref observed, incarnation, seq, .. }
                        if observed == &id && incarnation == pane.incarnation && seq == query_seq) { break; }
                }
            }).await.unwrap();
            assert_eq!(writes.load(Ordering::Acquire), 0,
                "the actual reader must reject CPR after its operation loses current, even while physical generation remains active");
            assert_eq!(pane.pty.ordinary_input_observation(), ordinary);
            assert!(pane.pty.reserve_input_transaction().is_err());
            drop(reservation);
            let next = pane.pty.reserve_input_transaction().unwrap();
            assert_eq!(next.previous_transaction_revision(), transaction);
        }).catch_unwind().await;
        // Always release the local observation gate before the genuine owner
        // drains, including a RED assertion or a fixture-stage failure.
        let _ = release.send(());
        owner.shutdown().await;
        if let Err(error) = verified {
            std::panic::resume_unwind(error);
        }
    }

    #[test]
    fn dropping_window_runtime_reclaims_worker_owned_pty_without_the_pane_lock() {
        let _lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let (command, args) = if cfg!(windows) {
            ("cmd", vec!["/d", "/s", "/c", "exit /b 0"])
        } else {
            ("/bin/cat", vec![])
        };
        // No frontend answers ConPTY's startup query: even this exit command
        // remains alive until its owning runtime explicitly tears it down.
        let pane = Arc::new(Mutex::new(
            Pane::new(
                "drop-runtime".into(),
                command.into(),
                args.into_iter().map(str::to_owned).collect(),
                80,
                24,
                HashMap::new(),
                Some(temp.path().to_owned()),
            )
            .unwrap(),
        ));
        let pty = pane.lock().unwrap().shared_pty();
        let _cleanup = TestPtyGuard(pty.clone());
        let (exit_tx, exit_rx) = mpsc::channel();
        let output_tx = exit_tx.clone();
        let mut runtime = WindowRuntime {
            project_root: temp.path().into(),
            policy: Default::default(),
            _initial_prompt_file: None,
            started_at_ms: 0,
            child_started_at: None,
            current: Arc::new(AtomicBool::new(true)),
            registry_cleanup: None,
            close_handoff: None,
            close_monitor_snapshot: None,
            pm_lifecycle: None,
            rollback_monitor_failure: None,
            member_cleanup: None,
            detail: None,
            last_output_at: None,
            pty: pty.clone(),
            incarnation: 1,
            pane: pane.clone(),
            output_thread: None,
            status_thread: None,
            pty_status: crate::WindowProcessStatus::Running,
            hook_status: None,
            output_bytes: 0,
            agent: None,
        };
        runtime.output_thread = Some(crate::pane_runtime::spawn_output_thread(
            "drop-runtime".into(),
            1,
            pane.clone(),
            runtime.current.clone(),
            move |event| {
                if matches!(event, crate::pane_runtime::PaneRuntimeEvent::Status { .. }) {
                    let _ = output_tx.send("output");
                }
            },
        ));
        runtime.status_thread = Some(crate::pane_runtime::spawn_status_thread(
            "drop-runtime".into(),
            1,
            pane.clone(),
            move |_| {
                let _ = exit_tx.send("status");
            },
        ));
        let pane_lock = pane.lock().unwrap();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(runtime);
            let _ = dropped_tx.send(());
        });
        // test-hygiene: allow-wall-clock-deadline Bound a regressed pane mutex wait; the channel establishes Drop completion.
        let dropped_without_pane_lock = dropped_rx.recv_timeout(Duration::from_secs(5)).is_ok();
        drop(pane_lock);
        dropper.join().unwrap();
        assert!(
            dropped_without_pane_lock,
            "runtime Drop must dispatch cleanup without taking the pane lock"
        );
        let mut exited = [
            exit_rx
                // test-hygiene: allow-wall-clock-deadline Bound actual OS child exit; this event establishes completion.
                .recv_timeout(Duration::from_secs(5))
                .expect("runtime Drop must stop its owned child"),
            exit_rx
                // test-hygiene: allow-wall-clock-deadline Bound actual PTY reader exit; this event establishes completion.
                .recv_timeout(Duration::from_secs(5))
                .expect("runtime Drop must release its PTY reader"),
        ];
        exited.sort_unstable();
        assert_eq!(exited, ["output", "status"]);
        assert!(
            pty.try_wait().unwrap().is_some(),
            "owned child must be reaped before verification ends"
        );
    }
}
