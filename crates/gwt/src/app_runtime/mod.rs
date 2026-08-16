use super::*;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone)]
pub enum AppEventProxy {
    Real(EventLoopProxy<UserEvent>),
    #[cfg(test)]
    Stub(Arc<Mutex<Vec<UserEvent>>>),
}

impl AppEventProxy {
    pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self::Real(proxy)
    }

    pub(crate) fn send(&self, event: UserEvent) {
        match self {
            Self::Real(proxy) => {
                let _ = proxy.send_event(event);
            }
            #[cfg(test)]
            Self::Stub(events) => {
                if let Ok(mut events) = events.lock() {
                    events.push(event);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn stub() -> (Self, Arc<Mutex<Vec<UserEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Self::Stub(events.clone()), events)
    }
}

#[cfg(test)]
pub(crate) type BlockingTestTask = Box<dyn FnOnce() + Send + 'static>;
#[cfg(test)]
pub(crate) type BlockingTestTaskQueue = Arc<Mutex<Vec<BlockingTestTask>>>;

#[derive(Clone)]
pub enum BlockingTaskSpawner {
    Tokio(tokio::runtime::Handle),
    #[cfg(test)]
    Thread,
    #[cfg(test)]
    Failing(String),
    #[cfg(test)]
    Queued(BlockingTestTaskQueue),
}

impl BlockingTaskSpawner {
    pub(crate) fn tokio(handle: tokio::runtime::Handle) -> Self {
        Self::Tokio(handle)
    }

    #[cfg(test)]
    pub(crate) fn thread() -> Self {
        Self::Thread
    }

    #[cfg(test)]
    pub(crate) fn failing(message: impl Into<String>) -> Self {
        Self::Failing(message.into())
    }

    #[cfg(test)]
    pub(crate) fn queued() -> (Self, BlockingTestTaskQueue) {
        let tasks = Arc::new(Mutex::new(Vec::new()));
        (Self::Queued(tasks.clone()), tasks)
    }

    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.try_spawn(task).expect("spawn blocking task");
    }

    pub(crate) fn try_spawn<F>(&self, task: F) -> Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        match self {
            Self::Tokio(handle) => {
                drop(handle.spawn_blocking(task));
                Ok(())
            }
            #[cfg(test)]
            Self::Thread => {
                let gwt_home = gwt_core::test_support::gwt_home_override();
                thread::Builder::new()
                    .name("gwt-blocking-task".to_string())
                    .spawn(move || {
                        let _gwt_home = gwt_home
                            .as_ref()
                            .map(gwt_core::test_support::ScopedGwtHome::set);
                        task();
                    })
                    .map(drop)
                    .map_err(|error| error.to_string())
            }
            #[cfg(test)]
            Self::Failing(message) => Err(message.clone()),
            #[cfg(test)]
            Self::Queued(tasks) => {
                tasks
                    .lock()
                    .map_err(|error| error.to_string())?
                    .push(Box::new(task));
                Ok(())
            }
        }
    }
}

static NEXT_WINDOW_RUNTIME_INCARNATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_window_runtime_incarnation() -> u64 {
    NEXT_WINDOW_RUNTIME_INCARNATION
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |incarnation| incarnation.checked_add(1),
        )
        .expect("window runtime incarnation space exhausted")
}

pub struct WindowRuntime {
    /// Process-local identity of this exact PTY runtime. A window id may be
    /// reused by a successor, so background events must carry this value and
    /// prove they still belong to the runtime currently stored for the id.
    incarnation: u64,
    pane: Arc<Mutex<Pane>>,
    /// Handle to the background reader thread that forwards PTY output.
    /// Taken and joined during `stop_window_runtime` so the reader releases
    /// its Arc clone of `pane` before the runtime is fully torn down.
    output_thread: Option<JoinHandle<()>>,
    /// Handle to the process status watcher. It is independent from PTY EOF
    /// because some agent exits can leave the terminal reader waiting even
    /// after the direct child has finished.
    status_thread: Option<JoinHandle<()>>,
}

struct RuntimeStopThreads {
    output_thread: Option<JoinHandle<()>>,
    status_thread: Option<JoinHandle<()>>,
}

mod attachments;
mod board;
pub(crate) mod continuation;
mod file_windows;
mod frontend_action_log;
mod knowledge;
mod launch;
mod launch_errors;
mod launch_output_mirror;
mod loaders;
mod migration;
pub(crate) mod persist_dispatcher;
mod pm;
mod profile;
mod project_tabs;
mod pty_io;
mod runtime_events;
mod settings_update;
mod startup;
mod title_sync;
mod ui_trace;
mod window;
mod wizard;
mod workspace;
mod workspace_views;
use attachments::UploadedImagePasteOperation;
#[cfg(test)]
use attachments::{
    format_file_attachment_prompt, prepare_file_attachment, prepare_image_paste_file,
    save_file_attachment_with_progress, FileAttachmentError, ImagePasteError,
    PreparedFileAttachment,
};
pub use board::BoardPostRequest;
#[cfg(test)]
use frontend_action_log::frontend_user_action_log;
use frontend_action_log::log_frontend_user_action;
use knowledge::knowledge_error_event;
#[cfg(test)]
use knowledge::KnowledgeRefreshTask;
pub use knowledge::{KnowledgeLoadRequest, KnowledgeSearchRequest, ProjectIndexSearchRequest};
#[cfg(test)]
pub(crate) use launch::AgentLaunchCompletion;
#[cfg(test)]
pub(crate) use launch::AgentLaunchRuntimeContext;
#[cfg(test)]
use launch::{
    codex_hook_discovery_mode_for_launch_config,
    codex_hook_discovery_mode_from_codex_version_output,
    codex_hook_discovery_mode_from_selected_codex_version, dispatch_agent_launch_success,
    maybe_register_codex_managed_hook_trust_for_launch,
};
pub(crate) use launch::{continue_work_readiness_decision, ReadinessDeadlineDecision};
use launch::{launch_config_from_persisted_session, IssueBranchLinkStore};
pub use launch::{
    AgentLaunchResult, ContinueWorkReadinessWatch, LaunchWizardMemoryCache, ProcessLaunch,
};
#[cfg(test)]
use loaders::{load_log_entries_from_dir, skipped_lines_warning};
use profile::ProfileSaveRequest;
#[cfg(test)]
use project_tabs::parse_github_repository_search_results;
use project_tabs::recovery_state_label;
#[cfg(test)]
use settings_update::{os_url_open_command, validate_server_url, validate_update_log_path};
use startup::mark_auto_resume_source_completed;
use ui_trace::save_ui_trace_to_log_dir;
use workspace::{
    active_agent_summary_from_session, merge_active_sessions_into_projection,
    retain_live_workspace_agents, save_shell_work_projection, save_workspace_launch_projection,
    workspace_cleanup_candidate_for_projection, workspace_projection_for_current_resume,
    workspace_projection_owner_title, WorkspaceLaunchProjectionKind,
};
use workspace_views::{
    active_agent_session_matches_work, active_work_cleanup_candidate_view_from_candidate,
    active_work_projection_from_saved_with_journal, agent_launch_purpose_title,
    linked_issue_workspace_context, non_empty_workspace_text, save_resumed_workspace_projection,
    save_start_work_workspace_projection, session_exact_resume_materializable, work_session_index,
    workspace_journal_entry_view_from_entry, workspace_resume_branch_exists,
    workspace_resume_branch_from_journal_project_root, workspace_resume_context_for_work_item,
    workspace_resume_context_from_journal, workspace_resume_context_from_projection,
    workspace_resume_owner_issue_number, workspace_work_item_view_from_item,
    WORKSPACE_CLEANUP_EVENT_ID, WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};
#[cfg(test)]
use workspace_views::{
    active_work_projection_from_saved, apply_work_summary_external_sources,
    assign_and_merge_workspace_groups, attach_registry_sessions_to_active_works,
    derive_work_summary, is_identifier_like_title, mark_merged_active_works,
    mark_remote_only_active_works, mark_workspace_cleanup_candidates,
    workspace_execution_diagnosis_view, workspace_work_agent_view_from_ref,
    workspace_work_event_kind_wire,
};

#[derive(Debug, Clone)]
pub struct ActiveAgentSession {
    pub(crate) window_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) branch_name: String,
    pub(crate) display_name: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) agent_project_root: String,
    pub(crate) runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub(crate) tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceResumeContext {
    pub(crate) title: Option<String>,
    pub(crate) owner: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) next_action: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingContinueWork {
    pub(crate) client_id: ClientId,
    pub(crate) operation_id: String,
    pub(crate) work_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) worktree_path: PathBuf,
    pub(crate) owner: gwt::cli::execution_state::ExecutionOwnerKey,
    pub(crate) work_branch: String,
    pub(crate) work_agent_id: gwt_agent::AgentId,
    pub(crate) work_agent_session_id: Option<String>,
    pub(crate) execution: PendingContinueWorkExecution,
    pub(crate) binding: gwt_agent::SessionExecutionBinding,
    pub(crate) readiness_nonce: String,
    pub(crate) outcome: gwt::ContinueWorkOutcomeKind,
    pub(crate) resume_context: WorkspaceResumeContext,
    pub(crate) predecessor_session_id: String,
    pub(crate) predecessor_binding: gwt_agent::ExecutionBindingIdentity,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingContinueWorkExecution {
    Successor(gwt::cli::execution_state::SuccessorRequest),
    #[allow(dead_code)] // Retained for reconciliation of legacy in-memory fixtures.
    Takeover(gwt::cli::execution_state::GenerationTakeoverRequest),
}

/// Explicit linked-owner launch that is waiting for an authenticated
/// SessionStart before replacing an integrity-valid Blocked generation.
#[derive(Debug, Clone)]
pub(crate) struct PendingFreshExecutionLaunch {
    pub(crate) operation_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) worktree_path: PathBuf,
    pub(crate) owner: gwt::cli::execution_state::ExecutionOwnerKey,
    pub(crate) request: gwt::cli::execution_state::SuccessorRequest,
    pub(crate) binding: gwt_agent::SessionExecutionBinding,
    pub(crate) session_identity: gwt_agent::SessionExecutionIdentity,
    pub(crate) readiness_nonce: String,
    pub(crate) predecessor_binding: gwt_agent::ExecutionBindingIdentity,
    pub(crate) base_branch: Option<String>,
    pub(crate) linked_issue_number: Option<u64>,
    pub(crate) resume_context: Option<WorkspaceResumeContext>,
    pub(crate) launch_feedback_context: Option<LaunchFeedbackContext>,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedContinueWorkOutcome {
    pub(crate) work_id: String,
    pub(crate) outcome: gwt::ContinueWorkOutcomeKind,
    pub(crate) message: Option<String>,
    pub(crate) error_code: Option<String>,
    pub(crate) retryable: bool,
}

impl WorkspaceResumeContext {
    fn purpose_title(&self) -> Option<String> {
        self.title
            .as_deref()
            .or(self.summary.as_deref())
            .or(self.owner.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStartupAutoResumeSession {
    pub(crate) tab_id: String,
    pub(crate) session: gwt_agent::Session,
    pub(crate) workspace_resume_context: Option<WorkspaceResumeContext>,
}

#[derive(Debug, Clone)]
pub enum DispatchTarget {
    Broadcast,
    Client(ClientId),
}

#[derive(Debug, Clone)]
pub(crate) enum KnowledgeWireMetadata {
    SemanticRetry(gwt::KnowledgeSemanticRetry),
    NonSemanticError,
}

#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub(crate) target: DispatchTarget,
    pub(crate) event: BackendEvent,
    /// SPEC #1939 FR-407 / SPEC #3170 FR-098: private wire-only metadata for
    /// semantic retry directives and explicitly non-semantic search errors.
    /// Keeping it outside the public `BackendEvent` preserves the baseline
    /// Rust construction/destructuring shape.
    pub(crate) knowledge_wire_metadata: Option<KnowledgeWireMetadata>,
}

pub(crate) enum AgentFrontendDispatchOutcome {
    Dispatched(Vec<OutboundEvent>),
    StaleCapability,
    ExecutionAuthorityUnavailable,
}

#[cfg(test)]
type AgentDispatchTestHook = std::cell::RefCell<Option<Box<dyn FnOnce()>>>;

#[cfg(test)]
std::thread_local! {
    static AGENT_AFTER_DURABLE_CHECK_TEST_HOOK: AgentDispatchTestHook =
        const { AgentDispatchTestHook::new(None) };
    static AGENT_LEASED_MUTATION_TEST_HOOK: AgentDispatchTestHook =
        const { AgentDispatchTestHook::new(None) };
}

#[cfg(test)]
fn set_agent_after_durable_check_test_hook(hook: impl FnOnce() + 'static) {
    AGENT_AFTER_DURABLE_CHECK_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn set_agent_leased_mutation_test_hook(hook: impl FnOnce() + 'static) {
    AGENT_LEASED_MUTATION_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_agent_dispatch_test_hook(slot: &'static std::thread::LocalKey<AgentDispatchTestHook>) {
    slot.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

impl OutboundEvent {
    pub(crate) fn broadcast(event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Broadcast,
            event,
            knowledge_wire_metadata: None,
        }
    }

    pub(crate) fn reply(client_id: impl Into<ClientId>, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: None,
        }
    }

    pub(crate) fn reply_with_knowledge_semantic_retry(
        client_id: impl Into<ClientId>,
        event: BackendEvent,
        semantic_retry: Option<gwt::KnowledgeSemanticRetry>,
    ) -> Self {
        assert!(
            matches!(event, BackendEvent::KnowledgeSearchResults { .. }),
            "knowledge semantic retry metadata requires KnowledgeSearchResults"
        );
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: semantic_retry.map(KnowledgeWireMetadata::SemanticRetry),
        }
    }

    pub(crate) fn reply_with_nonsemantic_knowledge_error(
        client_id: impl Into<ClientId>,
        event: BackendEvent,
    ) -> Self {
        assert!(
            matches!(
                event,
                BackendEvent::KnowledgeError {
                    request_id: Some(_),
                    query: Some(_),
                    ..
                }
            ),
            "non-semantic knowledge error metadata requires a correlated KnowledgeError"
        );
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
            knowledge_wire_metadata: Some(KnowledgeWireMetadata::NonSemanticError),
        }
    }
}

pub fn build_frontend_sync_events(
    client_id: &str,
    workspace: gwt::AppStateView,
    terminal_statuses: Vec<(String, WindowProcessStatus, String)>,
    terminal_snapshots: Vec<(String, Vec<u8>)>,
    launch_wizard: Option<gwt::LaunchWizardView>,
    pending_update: Option<gwt_core::update::UpdateState>,
) -> Vec<OutboundEvent> {
    let mut events = vec![OutboundEvent::reply(
        client_id,
        BackendEvent::WindowCanvasState { workspace },
    )];

    for (id, status, detail) in terminal_statuses {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::TerminalStatus {
                id,
                status,
                detail: Some(detail),
            },
        ));
    }

    events.push(OutboundEvent::reply(
        client_id,
        BackendEvent::LaunchWizardState {
            wizard: launch_wizard.map(Box::new),
        },
    ));

    if let Some(state) = pending_update {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::UpdateState(state),
        ));
    }

    // SPEC-2359 W-17 (FR-397): bulky terminal snapshots go last so a
    // reconnect replay delivers lightweight state (wizard, statuses, update)
    // before scrollback payloads, instead of burying it behind them.
    for (id, snapshot) in terminal_snapshots {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::TerminalSnapshot {
                id,
                data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
            },
        ));
    }

    events
}

#[derive(Debug, Clone)]
pub struct ProjectTabRuntime {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) project_root: PathBuf,
    pub(crate) kind: gwt::ProjectKind,
    pub(crate) workspace: WindowCanvasState,
    /// SPEC-1934 US-6: in-memory flag set when the tab was opened on a Normal
    /// Git layout that we want to migrate. The frontend sees a
    /// [`BackendEvent::MigrationDetected`] until the user picks Migrate /
    /// Skip / Quit. Not persisted: re-detected on every launch.
    pub(crate) migration_pending: bool,
    /// SPEC-2014 FR-PERF-003: cached `git rev-parse --git-common-dir`
    /// resolution for this tab. `gwt_git::worktree::main_worktree_root`
    /// spawns `git.exe`; on Windows every spawn costs several hundred
    /// milliseconds (`CreateProcess` + Defender real-time scan). The Launch
    /// Wizard / Start Work / Add Agent / Resume Workspace paths used to call
    /// it on every open, accounting for the bulk of the cold-open delay.
    /// We resolve the value on first access and reuse it for the lifetime
    /// of the tab; the [`Arc`] wrapper keeps `ProjectTabRuntime: Clone`.
    pub(crate) main_worktree_root_cache: std::sync::Arc<std::sync::OnceLock<PathBuf>>,
}

impl ProjectTabRuntime {
    /// Return the cached primary repository root for this tab, lazily
    /// resolving it on first access (FR-PERF-003). Falls back to
    /// `project_root` when `git rev-parse --git-common-dir` fails so the
    /// caller never has to deal with `Result`.
    pub(crate) fn main_worktree_root(&self) -> PathBuf {
        self.main_worktree_root_cache
            .get_or_init(|| {
                gwt_git::worktree::main_worktree_root(&self.project_root)
                    .unwrap_or_else(|_| self.project_root.clone())
            })
            .clone()
    }
}

#[derive(Debug, Clone)]
pub struct WindowAddress {
    pub(crate) tab_id: String,
    pub(crate) raw_id: String,
}

pub(crate) struct PendingAgentSelfClose {
    ticket: AgentSelfCloseCapabilityTicket,
    window_id: String,
    address: WindowAddress,
    session_id: String,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardSession {
    pub(crate) tab_id: String,
    pub(crate) wizard_id: String,
    pub(crate) wizard: LaunchWizardState,
    pub(crate) workspace_resume_context: Option<WorkspaceResumeContext>,
    pub(crate) agent_kanban_target: Option<AgentKanbanLaunchTarget>,
    pub(crate) auto_submit_after_runtime_resolution: Option<WindowGeometry>,
    pub(crate) issue_monitor_profile_save: Option<IssueMonitorProfileSaveContext>,
    pub(crate) issue_monitor_launch_issue_number: Option<u64>,
    /// Typed front door for the wizard. Manual holder arbitration is allowed
    /// only for an explicit Launch Agent invocation; it must never be inferred
    /// from a linked Issue on Start Work, Knowledge, Monitor, or resume flows.
    pub(crate) origin: LaunchWizardOrigin,
    /// Exact manual-holder arbitration snapshot. `None` on autonomous,
    /// startup, Resume, Continue Work, and Issue Monitor launch adapters.
    pub(crate) manual_holder_intent: Option<ManualLaunchHolderIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualLaunchHolderIntent {
    pub(crate) fingerprint: String,
    pub(crate) owner: gwt::cli::execution_state::ExecutionOwnerKey,
    pub(crate) predecessor: gwt_agent::SessionExecutionIdentity,
    pub(crate) predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor,
    pub(crate) local_window_id: Option<String>,
    pub(crate) local_runtime_incarnation: Option<u64>,
    pub(crate) runtime_proof: Option<gwt_agent::ManualLaunchRuntimeProof>,
    pub(crate) operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManualLaunchGenerationDisposition {
    NotApplicable,
    Genesis,
    Prepare(ManualLaunchPreparation),
    ExistingSuccessorWindow(String),
    ConfirmLive(ManualLaunchHolderIntent),
    Conflict(String),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchWizardOrigin {
    ManualLaunchAgent,
    Knowledge,
    StartWork,
    IssueMonitor,
    WorkspaceResume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualLaunchPreparation {
    pub(crate) owner: gwt::cli::execution_state::ExecutionOwnerKey,
    pub(crate) expected_binding: gwt_agent::ExecutionBindingIdentity,
    pub(crate) expected_session: Option<gwt_agent::SessionExecutionIdentity>,
    pub(crate) expected_runtime: Option<gwt_agent::ManualLaunchRuntimeProof>,
    pub(crate) predecessor_kind: gwt_agent::ManualLaunchSuccessorPredecessor,
    pub(crate) operation_id: String,
}

impl ManualLaunchHolderIntent {
    fn preparation(&self) -> ManualLaunchPreparation {
        ManualLaunchPreparation {
            owner: self.owner,
            expected_binding: self.predecessor.execution_binding.identity.clone(),
            expected_session: Some(self.predecessor.clone()),
            expected_runtime: self.runtime_proof,
            predecessor_kind: self.predecessor_kind,
            operation_id: self.operation_id.clone(),
        }
    }

    fn decision_view(&self, holder_summary: String) -> gwt::LaunchWizardHolderDecisionView {
        let local = self.local_window_id.is_some();
        gwt::LaunchWizardHolderDecisionView {
            fingerprint: self.fingerprint.clone(),
            holder_session_id: self.predecessor.session_id.clone(),
            holder_window_id: self.local_window_id.clone(),
            holder_summary,
            stop_available: local,
            stop_unavailable_reason: (!local)
                .then(|| "The current holder is not controlled by this gwt window.".to_string()),
            move_available: local,
            move_unavailable_reason: (!local)
                .then(|| "The current holder pane is not available in this window.".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IssueMonitorProfileSaveContext {
    pub(crate) client_id: ClientId,
    pub(crate) issue_number: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AgentKanbanLaunchTarget {
    pub(crate) board_id: String,
    pub(crate) lane_id: gwt::AgentKanbanLane,
}

#[derive(Debug, Clone)]
pub struct LaunchFeedbackContext {
    pub(crate) client_id: ClientId,
    pub(crate) title: String,
    pub(crate) issue_monitor_issue_number: Option<u64>,
    pub(crate) issue_monitor_delivery_id: Option<String>,
    pub(crate) issue_monitor_project_root: Option<PathBuf>,
    pub(crate) issue_monitor_session_mode: Option<gwt_agent::SessionMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IssueMonitorLaunchDeliveryState {
    Materializing {
        window_id: String,
        started_at: std::time::Instant,
    },
    LaunchedPendingAck {
        window_id: String,
    },
    Launched {
        window_id: String,
    },
    LaunchFailed {
        message: String,
        session_mode: gwt_agent::SessionMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorFailureCommit {
    Committed(Option<u64>),
    Rejected,
    AuthorityExhausted,
}

fn issue_monitor_writer_conflict_commit(
    outcome: gwt::IssueMonitorResumeWriterConflictOutcome,
    issue_number: u64,
) -> IssueMonitorFailureCommit {
    match outcome {
        gwt::IssueMonitorResumeWriterConflictOutcome::Requeued => {
            IssueMonitorFailureCommit::Committed(Some(issue_number))
        }
        gwt::IssueMonitorResumeWriterConflictOutcome::Rejected => {
            IssueMonitorFailureCommit::Rejected
        }
        gwt::IssueMonitorResumeWriterConflictOutcome::AuthorityExhausted => {
            IssueMonitorFailureCommit::AuthorityExhausted
        }
    }
}

const ISSUE_MONITOR_MATERIALIZING_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct IssueLaunchWizardPrepared {
    pub(crate) client_id: ClientId,
    pub(crate) id: String,
    pub(crate) knowledge_kind: KnowledgeKind,
    pub(crate) tab_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) issue_number: u64,
    pub(crate) result: Result<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProjectOpenTarget {
    pub(crate) project_root: PathBuf,
    pub(crate) title: String,
    pub(crate) kind: gwt::ProjectKind,
    /// `true` when the resolved layout is a Normal Git checkout that gwt would
    /// like to migrate to its Nested Bare+Worktree convention (SPEC-1934 US-6).
    pub(crate) needs_migration: bool,
}

/// SPEC-3075 FR-006: at most this many Workspaces get an AI-polished summary per
/// scan, bounding both the git calls and the AI prompt size for large repos.
const AI_SUMMARY_BRANCH_CAP: usize = 40;

/// Issue #3604: minimum spacing between two repository-wide PR-title
/// enumerations for the same project.
///
/// Each enumeration is a `gh pr list --state all --limit 999` — up to ten
/// GraphQL requests out of a 5000-point hourly budget — and it used to ride the
/// 30s ingest throttle, so a GUI in ordinary use re-enumerated every PR in the
/// repository twice a minute. Bounded staleness is the right trade here: the
/// rail falls back to the branch's tip commit subject until the title arrives,
/// and a project-open refresh bypasses the window entirely
/// ([`AppRuntime::spawn_work_events_ingest`]).
pub(crate) const WORK_PR_TITLES_SCAN_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// SPEC-3075 FR-006: a tip commit subject that carries no real purpose — merge
/// commits and release-version bumps. These are the cases the AI polish targets
/// (it reads the underlying feature commits instead).
fn is_summary_noise(subject: &str) -> bool {
    let s = subject.trim();
    s.is_empty()
        || s.starts_with("Merge pull request")
        || s.starts_with("Merge branch")
        || s.starts_with("Merge remote-tracking")
        || s.starts_with("Merge tag")
        || s.starts_with("merge:")
        || s.starts_with("chore: merge")
        || s.starts_with("chore(release):")
        || s.starts_with("chore(deps):")
}

/// SPEC-3075 FR-006: build the AI-summary inputs for a project. For every
/// non-terminal Workspace whose branch tip is merge/release noise, gather the
/// recent non-merge commit subjects (the real work) plus the owner. Only the
/// noisy Workspaces are included (PR titles / clean commit subjects need no
/// polish), and the count is capped. Pure structured meta — no transcript.
fn build_ai_summary_inputs(project_root: &Path, cap: usize) -> Vec<gwt_ai::WorkSummaryInput> {
    let Ok(projection) =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(project_root)
    else {
        return Vec::new();
    };
    let tip_subjects = gwt_git::refs::branch_tip_subjects(project_root).unwrap_or_default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut inputs = Vec::new();
    for item in &projection.work_items {
        if item.is_terminal() || inputs.len() >= cap {
            continue;
        }
        let Some(branch) = item
            .execution_containers
            .iter()
            .find_map(|container| container.branch.as_deref())
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
        else {
            continue;
        };
        if !seen.insert(branch.clone()) {
            continue;
        }
        let tip = tip_subjects
            .get(&branch)
            .or_else(|| tip_subjects.get(&format!("origin/{branch}")))
            .map(String::as_str)
            .unwrap_or("");
        // Only polish the noisy ones — a clean tip subject is already a usable
        // summary, and a missing branch has nothing to read.
        if !is_summary_noise(tip) {
            continue;
        }
        let mut signals =
            gwt_git::commit::branch_recent_subjects(project_root, &branch, 5).unwrap_or_default();
        if signals.is_empty() {
            signals = gwt_git::commit::branch_recent_subjects(
                project_root,
                &format!("origin/{branch}"),
                5,
            )
            .unwrap_or_default();
        }
        signals.retain(|subject| !is_summary_noise(subject));
        if signals.is_empty() {
            continue;
        }
        inputs.push(gwt_ai::WorkSummaryInput {
            branch,
            owner: item.owner.clone(),
            signals,
        });
    }
    inputs
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ApprovalPromptLatch {
    pub(crate) active_fingerprint: Option<u64>,
    pub(crate) resolving_fingerprint: Option<u64>,
    pub(crate) resolution_started: bool,
    pub(crate) pending_settle_token: Option<u64>,
}

pub struct AppRuntime {
    pub(crate) tabs: Vec<ProjectTabRuntime>,
    pub(crate) active_tab_id: Option<String>,
    pub(crate) recent_projects: Vec<gwt::RecentProjectEntry>,
    pub(crate) profile_selections: HashMap<String, String>,
    pub(crate) profile_config_path: Option<PathBuf>,
    pub(crate) runtimes: HashMap<String, WindowRuntime>,
    pub(crate) window_details: HashMap<String, String>,
    pub(crate) launch_error_terminal_details: HashMap<String, String>,
    pub(crate) window_lookup: HashMap<String, WindowAddress>,
    pub(crate) board_all_view_windows: HashSet<String>,
    pub(crate) session_state_path: PathBuf,
    pub(crate) log_dir: PathBuf,
    pub(crate) proxy: AppEventProxy,
    pub(crate) blocking_tasks: BlockingTaskSpawner,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) launch_wizard_cache: LaunchWizardMemoryCache,
    pub(crate) launch_wizard: Option<LaunchWizardSession>,
    /// Single-use launch requests keyed by the exact wizard that produced
    /// them. The visible modal may be replaced before the queued event runs;
    /// materialization ownership must not move with that global UI slot.
    pub(crate) pending_launch_wizard_materializations: HashMap<String, LaunchWizardSession>,
    pub(crate) pending_workspace_resume_contexts: HashMap<String, WorkspaceResumeContext>,
    pub(crate) pending_launch_feedback_contexts: HashMap<String, LaunchFeedbackContext>,
    /// SPEC #3200 FR-052: daemon launch requests are at-least-once deliveries.
    /// Remember materialization and terminal ACK state by delivery id so a
    /// replay never creates a second agent window and can re-ACK after a
    /// transient daemon disconnect.
    pub(crate) issue_monitor_launch_deliveries: HashMap<String, IssueMonitorLaunchDeliveryState>,
    pub(crate) issue_monitor_materializer_id: String,
    /// Issue #3505: prefs-path scoped scheduled scans currently running in a
    /// blocking worker. Duplicate ticks are coalesced by dropping them while
    /// the same canonical project scope is in flight.
    pub(crate) issue_monitor_scheduled_scans_in_flight: HashSet<PathBuf>,
    /// Prepared producing continuations keyed by their pending/active window.
    /// The entry remains until an authenticated SessionStart finalizes the
    /// generation + Work transaction and promotes the same bearer.
    pub(crate) pending_continue_work: HashMap<String, PendingContinueWork>,
    /// Fresh linked-owner launches prepared from legacy Blocked authority.
    /// Like Continue work, these remain non-producing until SessionStart
    /// proves the exact candidate binding and the successor CAS commits.
    pub(crate) pending_fresh_execution_launches: HashMap<String, PendingFreshExecutionLaunch>,
    /// Process-local fast replay for a lost client response. Durable
    /// reconciliation still uses the owner ledger + Work commit receipt.
    pub(crate) continue_work_outcomes: HashMap<String, CachedContinueWorkOutcome>,
    /// Additional WebSocket clients waiting on an in-flight operation after
    /// reconnect/retry. The original requester remains on PendingContinueWork.
    pub(crate) continue_work_waiters: HashMap<String, HashSet<ClientId>>,
    /// SPEC-2359 W-17 (FR-398, Issue #3034): launches whose window is
    /// registered but whose agent session is not live yet, keyed by
    /// (tab, branch, working dir). A re-click in this window focuses the
    /// pending window instead of spawning a duplicate. Entries clear on
    /// launch completion/failure or after a TTL.
    pub(crate) inflight_launches: HashMap<String, (String, std::time::Instant)>,
    /// SPEC-3431 FR-001: window ids of in-flight PM launches, mapped to the
    /// project root whose `pm.json` must record the resulting session. The
    /// entry is consumed by `handle_launch_complete`, which writes the PM
    /// registration once the session id exists.
    pub(crate) pending_pm_launches: HashMap<String, PathBuf>,
    /// SPEC-3431 FR-020/FR-021: project root -> registered PM session id.
    /// A read-through cache of `pm.json` so the per-broadcast window view can
    /// mark the PM window without touching disk on every render. Refreshed
    /// wherever the registration is read or written.
    pub(crate) pm_sessions: HashMap<PathBuf, String>,
    /// SPEC-3431 T-093 (FR-012): per project, the monitor signal set the wake
    /// path has already seen. The first snapshot is a baseline; only signals
    /// beyond it can wake a quiet PM, so one event wakes at most once.
    pub(crate) pm_wake_seen: HashMap<PathBuf, std::collections::BTreeSet<String>>,
    /// SPEC-3431 FR-002: tabs whose PM ensure was queued at bootstrap and
    /// runs once the frontend reports canvas bounds (same deferral rule as
    /// startup auto-resume — agent panes never spawn before the canvas is
    /// ready).
    pub(crate) pending_startup_pm_tabs: Vec<String>,
    pub(crate) pending_auto_resume_sources: HashMap<String, String>,
    /// Legacy official-provider provenance is staged during preparation and
    /// committed only after the exact launched Session emits authenticated
    /// SessionStart. Any earlier route failure leaves the source Session bytes
    /// unchanged and retryable.
    pub(crate) pending_tool_runtime_migrations:
        HashMap<String, launch::PendingToolRuntimeMigration>,
    pub(crate) pending_startup_auto_resume_sessions: Vec<PendingStartupAutoResumeSession>,
    pub(crate) active_agent_sessions: HashMap<String, ActiveAgentSession>,
    /// SPEC-2359 W-15 (FR-386): per-project set of branches (canonical names)
    /// fully merged into a base on origin, filled by the background merge
    /// scan. Runtime-only; never persisted.
    /// SPEC-2359 W-15/W16-4 (FR-386/FR-391): merged branches per project →
    /// merge reference time (branch tip committer time proxy). Drives the
    /// "safe to delete" badge and the derived Done-equivalent classification.
    pub(crate) work_merged_branches:
        HashMap<PathBuf, HashMap<String, chrono::DateTime<chrono::Utc>>>,
    /// SPEC-3170 FR-075: normalized branches whose materialized worktree was
    /// dirty during the latest background merge scan. Projection rendering
    /// consumes this cache instead of probing every worktree on the event
    /// loop.
    pub(crate) work_dirty_branches: HashMap<PathBuf, HashSet<String>>,
    /// SPEC-3170 FR-076: normalized branches whose worktrees contain a live
    /// process cwd according to the latest background merge scan. Projection
    /// consumes only this cache and never enumerates OS processes.
    pub(crate) work_live_process_branches: HashMap<PathBuf, HashSet<String>>,
    /// SPEC-2359 US-84: per-project cleanup-ready branches and their reason.
    /// This includes merged branches and branches with no effective tree diff
    /// from the canonical base. Runtime-only; never persisted.
    pub(crate) work_cleanup_ready_branches: HashMap<PathBuf, HashMap<String, String>>,
    /// SPEC-3075: per-project `branch short name -> tip commit subject`, resolved
    /// off the hot path by [`AppRuntime::spawn_work_tip_subjects_scan`] (one
    /// `for-each-ref` spawn). Fills the Workspace rail summary for historical
    /// Works that never recorded a `title-summary` purpose.
    pub(crate) work_tip_subjects: HashMap<PathBuf, HashMap<String, String>>,
    /// SPEC-3075: per-project `branch (PR head ref) -> PR title`, resolved off
    /// the hot path by [`AppRuntime::spawn_work_pr_titles_scan`] (one `gh pr
    /// list` call). The PR title is the human-written purpose, so it is the
    /// top-priority Workspace rail summary. Empty when offline / `gh` absent.
    pub(crate) work_pr_titles: HashMap<PathBuf, HashMap<String, String>>,
    /// SPEC-3075 FR-006: per-project `branch -> AI-polished summary`, generated
    /// off the hot path by [`AppRuntime::spawn_work_ai_summaries_scan`] only when
    /// AI is enabled (`summary_enabled` + valid endpoint/model). The AI cleans
    /// merge/release commit noise into a human purpose; it fills the gap above
    /// the raw commit subject but below PR title / agent title-summary. Empty
    /// when AI is disabled — the non-AI chain then stands unchanged.
    pub(crate) work_ai_summaries: HashMap<PathBuf, HashMap<String, String>>,
    /// Incremental loader for the machine-local session ledger; keeps
    /// projection rebuilds from re-parsing thousands of unchanged TOMLs
    /// (window-close latency fix, 2026-06-11). RefCell: the runtime lives on
    /// the single event-loop thread and the projection builder takes `&self`.
    pub(crate) session_ledger_cache:
        std::cell::RefCell<crate::session_ledger_cache::SessionLedgerCache>,
    /// Same root fix for the home works.json (megabytes of Work items +
    /// events): cache hit clones instead of re-parsing per projection event.
    pub(crate) work_items_cache: std::cell::RefCell<gwt_core::workspace_projection::WorkItemsCache>,
    /// SPEC-3170 FR-076: latest fully built projection per tab. FrontendReady
    /// replays this snapshot (or a live-session-only fallback) without
    /// entering disk-backed projection loading on the GUI event loop.
    pub(crate) active_work_projection_cache:
        std::cell::RefCell<HashMap<String, gwt::ActiveWorkProjectionView>>,
    /// SPEC-2359 W-16 (FR-387): last work-events ingest per project — the
    /// 30s throttle for tab-change / post-launch triggers.
    pub(crate) last_work_events_ingest: std::cell::RefCell<HashMap<PathBuf, std::time::Instant>>,
    /// Issue #3604: last full PR-title enumeration per project. Kept separate
    /// from `last_work_events_ingest` because the local part of that ingest is
    /// cheap while this one costs a repository-wide `gh pr list`.
    pub(crate) last_work_pr_titles_scan: std::cell::RefCell<HashMap<PathBuf, std::time::Instant>>,
    /// SPEC-2359 W16-3 (FR-390): normalized branch names that currently have
    /// a LOCAL worktree, per project — refreshed by the worktree reconcile.
    /// The view marks `remote_only` by cache lookup alone (no git spawn on
    /// the projection build path).
    pub(crate) local_worktree_branches:
        std::cell::RefCell<HashMap<PathBuf, std::collections::HashSet<String>>>,
    pub(crate) window_pty_statuses: HashMap<String, WindowProcessStatus>,
    /// Issue #3475: PTY output bytes seen per window, counted monotonically
    /// for as long as the window keeps its runtime state tracking. The
    /// authenticated SessionStart readiness deadline compares it against the
    /// count captured when the deadline was armed, so it only ever needs the
    /// delta — an in-place agent restart reusing the same window is fine.
    /// Runtime-only; never persisted.
    pub(crate) window_output_bytes: HashMap<String, u64>,
    pub(crate) window_hook_states: HashMap<String, WindowProcessStatus>,
    /// Live Agent panes whose rendered provider UI is blocked on a human tool
    /// approval. Runtime-only and never persisted. A remote daemon overlay has
    /// no fingerprint; a locally classified prompt records only its u64
    /// identity, never screen text.
    pub(crate) window_approval_waiting: HashMap<String, ApprovalPromptLatch>,
    pub(crate) approval_settle_epoch: u64,
    pub(crate) recoverable_agent_error_windows: HashSet<String>,
    /// SPEC-3431 FR-068: when each agent window last showed activity, so the
    /// heartbeat published to the Issue Monitor can be throttled instead of
    /// firing a daemon control on every hook. Keyed by combined window id.
    pub(crate) last_agent_activity: HashMap<String, chrono::DateTime<chrono::Utc>>,
    pub(crate) agent_capability_issuer: Option<AgentCapabilityIssuer>,
    /// Issue-time opaque agent capability keyed by combined window id.
    ///
    /// Kept separately from [`ActiveAgentSession`] so stop/failure cleanup
    /// never needs to reconstruct authority from a mutable filesystem path.
    pub(crate) agent_capability_tokens: HashMap<String, String>,
    /// Accepted correlated self-closes waiting for the origin WebSocket task
    /// to finish its bounded direct ACK attempt. Entries are keyed by an
    /// unguessable process-local ticket, never by wire correlation data.
    pub(crate) pending_agent_self_closes: HashMap<String, PendingAgentSelfClose>,
    pub(crate) issue_link_cache_dir: PathBuf,
    /// SPEC #3170 FR-102: latest related-work snapshot per project root,
    /// captured by full load/search/refresh augmentation and reused verbatim
    /// by the detail-only selection path.
    pub(crate) knowledge_related_snapshot: knowledge::KnowledgeRelatedSnapshot,
    /// SPEC #3214 Phase 15: latest complete Issue Monitor projection per
    /// project root, joined into cache-backed Knowledge rows by issue number.
    pub(crate) knowledge_monitor_snapshot: knowledge::KnowledgeMonitorSnapshot,
    pub(crate) issue_client_factory: RuntimeIssueClientFactory,
    /// Cached update state so late-connecting WebView clients get the toast.
    pub(crate) pending_update: Option<gwt_core::update::UpdateState>,
    /// Shared PTY writer registry published to the WebSocket fast-path.
    pub(crate) pty_writers: PtyWriterRegistry,
    /// Browser-uploaded attachment temp files waiting to be staged under the
    /// active worktree.
    pub(crate) attachment_uploads: AttachmentUploadStore,
    /// Async writer that flushes session/workspace snapshots off the event
    /// loop thread (Issue #2694 Phase B).
    pub(crate) persist_dispatcher: persist_dispatcher::PersistDispatcher,
    /// SPEC-2009 amendment: per-window selected worktree root for File Tree
    /// windows. Reset every time the user reopens the picker, so this is a
    /// transient in-memory map and is not persisted with the session state.
    pub(crate) file_tree_worktree_roots: HashMap<String, PathBuf>,
    /// SPEC-2785 FR-E: embedded server URL captured after the axum bind so
    /// `open_server_url_events` can reject requests whose origin differs from
    /// the bound URL. `None` before the server is started (e.g. during early
    /// AppRuntime construction or unit tests that never call
    /// `set_server_url`).
    pub(crate) server_url: Option<String>,
    /// SPEC-2970: notifies the background usage poller to refresh immediately
    /// (e.g. after the Claude opt-in toggle changes). `None` in unit tests and
    /// before `set_usage_refresh` is called during startup wiring.
    pub(crate) usage_refresh: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// SPEC-3064 FR-002: monotonic sequence feeding image paste / attachment
    /// unique tokens (formerly the `IMAGE_PASTE_SEQUENCE` module static in
    /// `attachments.rs`). Per-runtime-instance; `AppRuntime` is constructed
    /// once per process in production, so observable behavior is unchanged.
    pub(crate) image_paste_sequence: std::sync::atomic::AtomicU64,
    /// SPEC-3064 FR-002: per-spawn correlation id source for the SPEC-2809
    /// launch stage banners (formerly the `AGENT_LAUNCH_STAGE_COUNTER`
    /// module static).
    pub(crate) agent_launch_stage_counter: std::sync::atomic::AtomicU64,
    /// Latest requested Improvement Inbox snapshot per project. Loads run on
    /// blocking workers; the event loop only accepts the newest epoch so a
    /// delayed read cannot roll the UI back.
    pub(crate) improvement_refresh_epoch: u64,
    pub(crate) improvement_latest_refresh_epochs: HashMap<PathBuf, u64>,
    /// Latest asynchronous backend connection probe per `(client, agent)`.
    /// Completion events are accepted only when their generation still
    /// matches this map, preventing a slower older request from rolling back
    /// the Settings UI after a newer request has completed.
    pub(crate) agent_backend_probe_generation: u64,
    pub(crate) agent_backend_latest_probe_generations:
        HashMap<(ClientId, gwt_agent::BuiltinAgentId), u64>,
}

impl ProjectTabRuntime {
    pub(crate) fn from_persisted(
        tab: gwt::PersistedSessionTabState,
        workspace: gwt::PersistedWindowCanvasState,
    ) -> Self {
        Self {
            id: tab.id,
            title: tab.title,
            project_root: tab.project_root,
            kind: tab.kind,
            workspace: WindowCanvasState::from_persisted(workspace),
            // Re-detected at startup via resolve_project_target; persistence
            // does not carry the flag.
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }
}

/// Whether a GUI-local Issue Monitor pass may refresh the read-only cache.
/// Successful daemon controls do not run a local pass at all: the daemon is the
/// sole writer and broadcasts the committed projection. When publication is
/// unavailable, `Scan` preserves the GUI's read model without granting Unix
/// GUI code authority to execute remote effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorScanPolicy {
    Scan,
    CacheOnly,
}

fn local_issue_monitor_fallback_projection_timeout() -> std::time::Duration {
    #[cfg(test)]
    if let Some(timeout) = std::env::var_os("GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS")
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .filter(|timeout| *timeout <= 60_000)
    {
        return std::time::Duration::from_millis(timeout);
    }
    std::time::Duration::from_secs(1)
}

#[cfg(test)]
thread_local! {
    static LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCAL_ISSUE_MONITOR_REMOTE_SCANS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
type ScheduledScanCommitTestHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
fn scheduled_scan_commit_test_hook() -> &'static Mutex<Option<ScheduledScanCommitTestHook>> {
    static HOOK: std::sync::OnceLock<Mutex<Option<ScheduledScanCommitTestHook>>> =
        std::sync::OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn set_scheduled_scan_after_lease_before_commit_test_hook(hook: impl FnOnce() + Send + 'static) {
    let mut slot = scheduled_scan_commit_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(slot.replace(Box::new(hook)).is_none());
}

#[cfg(test)]
fn run_scheduled_scan_after_lease_before_commit_test_hook() {
    let hook = scheduled_scan_commit_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
fn reset_local_issue_monitor_fallback_commit_count() {
    LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.set(0);
}

#[cfg(test)]
fn local_issue_monitor_fallback_commit_count() -> usize {
    LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.get()
}

#[cfg(test)]
fn reset_local_issue_monitor_remote_scan_count() {
    LOCAL_ISSUE_MONITOR_REMOTE_SCANS.set(0);
}

#[cfg(test)]
fn local_issue_monitor_remote_scan_count() -> usize {
    LOCAL_ISSUE_MONITOR_REMOTE_SCANS.get()
}

fn load_mutate_and_persist_issue_monitor_state<T: Default>(
    prefs_path: &Path,
    mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> T,
) -> (gwt::IssueMonitorState, T) {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + std::time::Duration::from_millis(250),
    );
    let mut mutation = Some(mutation);
    let mut monitor = None;
    let mut result = None;
    let recovery_baseline = gwt::IssueMonitorPrefs::recovery_default();
    let transaction =
        gwt::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |prefs| {
            let mut latest = gwt::IssueMonitorState::with_prefs(
                gwt::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            let apply = mutation
                .take()
                .expect("issue monitor prefs mutation runs once");
            result = Some(apply(&mut latest));
            *prefs = latest.prefs();
            monitor = Some(latest);
        });
    if let Err(error) = transaction {
        tracing::warn!(
            error = %error,
            "issue monitor GUI prefs transaction failed"
        );
    }
    let monitor = monitor.unwrap_or_else(|| {
        let prefs = gwt::load_issue_monitor_prefs(prefs_path)
            .unwrap_or_else(|_| gwt::IssueMonitorPrefs::recovery_default());
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs)
    });
    // A timed-out/failed transaction did not commit authority. Never render a
    // volatile mutation as if it had won: callers receive canonical disk state
    // and a neutral result while the original closure is dropped unapplied.
    (monitor, result.unwrap_or_default())
}

fn rebase_mutate_and_persist_issue_monitor_state<T: Default>(
    prefs_path: &Path,
    monitor: &mut gwt::IssueMonitorState,
    mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> T,
) -> T {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + std::time::Duration::from_millis(250),
    );
    let mut mutation = Some(mutation);
    let mut result = None;
    let recovery_baseline = monitor.prefs();
    let transaction =
        gwt::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            monitor.rebase_gui_observer_prefs(disk);
            let apply = mutation
                .take()
                .expect("issue monitor prefs mutation runs once");
            result = Some(apply(monitor));
            *disk = monitor.prefs();
        });
    if let Err(error) = transaction {
        tracing::warn!(
            error = %error,
            "issue monitor GUI prefs transaction failed"
        );
    }
    if result.is_none() {
        let prefs =
            gwt::load_issue_monitor_prefs(prefs_path).unwrap_or_else(|_| recovery_baseline.clone());
        *monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    }
    result.unwrap_or_default()
}

/// Commit one GUI-local fallback transition only while daemon authority is
/// still absent. On rejection the caller's in-memory view is restored from
/// canonical disk state so a volatile queue/effect is never rendered as won.
fn try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence<T>(
    prefs_path: &Path,
    monitor: &mut gwt::IssueMonitorState,
    mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> T,
) -> std::io::Result<T> {
    let _deadline = gwt_core::operation_deadline::current().is_none().then(|| {
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + std::time::Duration::from_millis(250),
        )
    });
    let recovery_baseline = monitor.prefs();
    let transaction =
        gwt::try_mutate_issue_monitor_prefs_without_authority_fence(prefs_path, |disk| {
            monitor.rebase_gui_observer_prefs(disk);
            let result = mutation(monitor);
            *disk = monitor.prefs();
            Ok(result)
        });
    match transaction {
        Ok((_prefs, result)) => Ok(result),
        Err(error) => {
            let prefs = gwt::load_issue_monitor_prefs(prefs_path)
                .unwrap_or_else(|_| recovery_baseline.clone());
            *monitor =
                gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
            Err(error)
        }
    }
}

fn record_issue_monitor_scan_failures(
    monitor: &mut gwt::IssueMonitorState,
    now: &str,
    merge_reconciliation_error: Option<String>,
    launch_failures: Vec<(u64, String)>,
) {
    if let Some(error) = merge_reconciliation_error {
        tracing::warn!(error = %error, "issue monitor merge reconciliation failed");
        monitor.record_scan_error(now, error);
    }
    for (issue_number, message) in launch_failures {
        monitor.record_launch_failed(issue_number, message);
    }
}

#[cfg_attr(unix, allow(dead_code))]
fn prepare_local_issue_monitor_claim_proposals(
    monitor: &mut gwt::IssueMonitorState,
    loaded: &gwt::issue_monitor_worker::LoadedIssueMonitorCandidates,
    monitor_owner: &str,
    now: &str,
    completed_issues: &std::collections::BTreeSet<u64>,
) {
    if !loaded.authorizes_remote_effects()
        || !monitor.config.enabled
        || !monitor.has_launch_profile()
    {
        return;
    }
    let active_cap = monitor.config.max_active.max(1);
    if monitor.active_count() >= active_cap {
        return;
    }
    monitor.prepare_claim_effects_with_probe(monitor_owner, now, active_cap, |issue_number| {
        completed_issues.contains(&issue_number)
    });
}

enum LocalIssueMonitorEffectOutcome {
    Claim(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimAcquireOutcome>,
    ),
    Revoked(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimReleaseOutcome>,
    ),
    Release(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimReleaseOutcome>,
    ),
}

fn begin_local_issue_monitor_effect_attempt(
    prefs_path: &Path,
    monitor: &mut gwt::IssueMonitorState,
) -> std::io::Result<Option<gwt::PendingIssueMonitorEffect>> {
    try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence(
        prefs_path,
        monitor,
        |latest| {
            if let Some(effect) = latest.pending_effects().iter().find(|effect| {
                effect.state == gwt::IssueMonitorEffectState::Attempting
                    && matches!(
                        effect.payload,
                        gwt::IssueMonitorEffectPayload::AcquireClaim { .. }
                            | gwt::IssueMonitorEffectPayload::ReleaseClaim { .. }
                    )
            }) {
                return Some(effect.clone());
            }
            let effect = latest
                .pending_effects()
                .iter()
                .find(|effect| {
                    effect.state == gwt::IssueMonitorEffectState::Prepared
                        && matches!(
                            effect.payload,
                            gwt::IssueMonitorEffectPayload::AcquireClaim { .. }
                                | gwt::IssueMonitorEffectPayload::ReleaseClaim { .. }
                        )
                })
                .cloned()?;
            let key = effect.attempt_key();
            if !latest.mark_pending_effect_attempting(&key) {
                return None;
            }
            latest
                .pending_effects()
                .iter()
                .find(|pending| {
                    pending.state == gwt::IssueMonitorEffectState::Attempting
                        && pending.attempt_key() == key
                })
                .cloned()
        },
    )
}

fn commit_local_issue_monitor_effect_result(
    prefs_path: &Path,
    monitor: &mut gwt::IssueMonitorState,
    effect: gwt::PendingIssueMonitorEffect,
    outcome: LocalIssueMonitorEffectOutcome,
    now_text: &str,
) -> std::io::Result<u8> {
    use gwt_github::client::OwnerMutationError;
    use gwt_github::issue_auto_claim::ClaimAcquireOutcome;

    let remote_error = match &outcome {
        LocalIssueMonitorEffectOutcome::Claim(Err(error))
        | LocalIssueMonitorEffectOutcome::Revoked(Err(error))
        | LocalIssueMonitorEffectOutcome::Release(Err(error)) => Some(error.to_string()),
        _ => None,
    };
    if let Some(error) = &remote_error {
        tracing::warn!(%error, "local Issue Monitor claim mutation did not complete cleanly");
    }

    let key = effect.attempt_key();
    try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence(
        prefs_path,
        monitor,
        |latest| {
            if !latest.pending_effects().iter().any(|pending| {
                pending.state == gwt::IssueMonitorEffectState::Attempting
                    && pending.attempt_key() == key
            }) {
                return 0_u8;
            }
            let current =
                latest.effect_authority_epoch() == key.authority_epoch && latest.config.enabled;
            match (&effect.payload, outcome) {
                (
                    gwt::IssueMonitorEffectPayload::AcquireClaim {
                        issue_number,
                        owner,
                        ..
                    },
                    LocalIssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Acquired(claim))),
                ) => {
                    latest.complete_pending_effect(&key);
                    if current {
                        latest.apply_confirmed_claim(
                            *issue_number,
                            claim.claim_id,
                            owner,
                            &effect.effect_id,
                            now_text,
                        );
                    }
                    1
                }
                (
                    gwt::IssueMonitorEffectPayload::AcquireClaim { issue_number, .. },
                    LocalIssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Blocked(winner)))
                    | LocalIssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Lost {
                        winning_claim: winner,
                        ..
                    })),
                ) => {
                    latest.complete_pending_effect(&key);
                    if current {
                        if let Some(issue) = latest
                            .inbox_item(*issue_number)
                            .map(|item| item.issue.clone())
                        {
                            latest.record_blocked_by_claim(issue, winner.owner, winner.expires_at);
                        }
                    }
                    1
                }
                (
                    gwt::IssueMonitorEffectPayload::AcquireClaim { .. },
                    LocalIssueMonitorEffectOutcome::Revoked(Ok(_)),
                )
                | (
                    gwt::IssueMonitorEffectPayload::ReleaseClaim { .. },
                    LocalIssueMonitorEffectOutcome::Release(Ok(_)),
                ) => {
                    latest.complete_pending_effect(&key);
                    1
                }
                (
                    gwt::IssueMonitorEffectPayload::ReleaseClaim { .. },
                    LocalIssueMonitorEffectOutcome::Release(Err(
                        error @ OwnerMutationError::PreSubmit(_),
                    )),
                ) => {
                    latest.record_scan_error(
                        now_text,
                        format!("Issue Monitor claim cleanup failed: {error}"),
                    );
                    latest.retry_pending_effect(&key);
                    2
                }
                (
                    gwt::IssueMonitorEffectPayload::AcquireClaim { .. },
                    LocalIssueMonitorEffectOutcome::Claim(Err(
                        error @ OwnerMutationError::PreSubmit(_),
                    )),
                ) if current => {
                    latest.record_scan_error(
                        now_text,
                        format!("Issue Monitor claim acquisition failed: {error}"),
                    );
                    latest.retry_pending_effect(&key);
                    2
                }
                (
                    gwt::IssueMonitorEffectPayload::AcquireClaim { .. },
                    LocalIssueMonitorEffectOutcome::Claim(Err(
                        error @ OwnerMutationError::PreSubmit(_),
                    ))
                    | LocalIssueMonitorEffectOutcome::Revoked(Err(
                        error @ OwnerMutationError::PreSubmit(_),
                    )),
                ) => {
                    latest.record_scan_error(
                        now_text,
                        format!("Issue Monitor revoked claim cleanup failed: {error}"),
                    );
                    latest.complete_pending_effect(&key);
                    1
                }
                (
                    _,
                    LocalIssueMonitorEffectOutcome::Claim(Err(
                        error @ OwnerMutationError::RemoteOutcomeUnknown(_),
                    ))
                    | LocalIssueMonitorEffectOutcome::Revoked(Err(
                        error @ OwnerMutationError::RemoteOutcomeUnknown(_),
                    ))
                    | LocalIssueMonitorEffectOutcome::Release(Err(
                        error @ OwnerMutationError::RemoteOutcomeUnknown(_),
                    )),
                ) => {
                    latest.record_scan_error(
                        now_text,
                        format!("Issue Monitor claim mutation outcome is unknown: {error}"),
                    );
                    0
                }
                _ => 0,
            }
        },
    )
}

fn drive_local_issue_monitor_claim_effects_with(
    prefs_path: &Path,
    monitor: &mut gwt::IssueMonitorState,
    mut execute: impl FnMut(
        &gwt::PendingIssueMonitorEffect,
        bool,
        &chrono::DateTime<chrono::Utc>,
        &str,
    ) -> Result<LocalIssueMonitorEffectOutcome, String>,
) -> Result<(), String> {
    let local_deadline =
        match gwt_core::operation_deadline::ensure_remaining("local Issue Monitor claim driver")
            .map_err(|error| error.to_string())?
        {
            Some(_) => None,
            None => Some(
                gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                    std::time::Instant::now() + std::time::Duration::from_secs(60),
                ),
            ),
        };
    let _local_deadline = local_deadline;
    let initial_count = monitor.pending_effects().len();
    for _ in 0..initial_count.max(1) {
        let Some(effect) = begin_local_issue_monitor_effect_attempt(prefs_path, monitor)
            .map_err(|error| error.to_string())?
        else {
            break;
        };
        let authority_current =
            effect.authority_epoch == monitor.effect_authority_epoch() && monitor.config.enabled;
        gwt_core::operation_deadline::ensure_remaining(
            "local Issue Monitor remote effect submission",
        )
        .map_err(|error| error.to_string())?;
        let now = chrono::Utc::now();
        let now_text = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let outcome = execute(&effect, authority_current, &now, &now_text)?;
        gwt_core::operation_deadline::ensure_remaining(
            "local Issue Monitor remote effect result commit",
        )
        .map_err(|error| error.to_string())?;
        let transition = commit_local_issue_monitor_effect_result(
            prefs_path, monitor, effect, outcome, &now_text,
        )
        .map_err(|error| error.to_string())?;
        if transition != 1 {
            break;
        }
    }
    Ok(())
}

fn execute_local_issue_monitor_claim_effects(
    prefs_path: &Path,
    owner: &str,
    repo: &str,
    monitor: &mut gwt::IssueMonitorState,
    issue_client_factory: &RuntimeIssueClientFactory,
) -> Result<(), String> {
    use gwt_github::issue_auto_claim::{
        acquire_claim_mutation, release_claim_mutation, ClaimComment, ClaimStatus,
    };

    drive_local_issue_monitor_claim_effects_with(
        prefs_path,
        monitor,
        |effect, authority_current, now, now_text| {
            let client = issue_client_factory(owner, repo).map_err(|error| error.to_string())?;
            Ok(match &effect.payload {
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number,
                    claim_id,
                    owner,
                    heartbeat_at,
                    expires_at,
                    launched_work_id,
                } if authority_current => {
                    let ttl = chrono::DateTime::parse_from_rfc3339(heartbeat_at)
                        .ok()
                        .zip(chrono::DateTime::parse_from_rfc3339(expires_at).ok())
                        .and_then(|(start, end)| (end - start).num_seconds().try_into().ok())
                        .filter(|ttl: &u64| *ttl > 0)
                        .unwrap_or(gwt::IssueMonitorConfig::default().claim_ttl_secs);
                    LocalIssueMonitorEffectOutcome::Claim(acquire_claim_mutation(
                        client.as_ref(),
                        gwt_github::IssueNumber(*issue_number),
                        ClaimComment {
                            comment_id: None,
                            claim_id: claim_id.clone(),
                            owner: owner.clone(),
                            issue_number: *issue_number,
                            status: ClaimStatus::Active,
                            heartbeat_at: now_text.to_string(),
                            expires_at: (*now + chrono::Duration::seconds(ttl as i64))
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            launched_work_id: launched_work_id.clone(),
                        },
                        now_text,
                    ))
                }
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number,
                    claim_id,
                    owner,
                    ..
                } => LocalIssueMonitorEffectOutcome::Revoked(release_claim_mutation(
                    client.as_ref(),
                    gwt_github::IssueNumber(*issue_number),
                    claim_id,
                    owner,
                )),
                gwt::IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number,
                    claim_id,
                    owner,
                } => LocalIssueMonitorEffectOutcome::Release(release_claim_mutation(
                    client.as_ref(),
                    gwt_github::IssueNumber(*issue_number),
                    claim_id,
                    owner,
                )),
                _ => return Err("unsupported local Issue Monitor effect".to_string()),
            })
        },
    )
}

pub(crate) type RuntimeIssueClient = Arc<dyn gwt_github::IssueClient>;
pub(crate) type RuntimeIssueClientFactory =
    Arc<dyn Fn(&str, &str) -> Result<RuntimeIssueClient, gwt_github::ApiError> + Send + Sync>;

pub(crate) fn default_issue_client_factory() -> RuntimeIssueClientFactory {
    Arc::new(|owner, repo| {
        let client = gwt_github::client::http::HttpIssueClient::from_gh_auth(owner, repo)?;
        Ok(Arc::new(client) as RuntimeIssueClient)
    })
}

#[derive(Debug, Clone)]
pub(crate) enum ScheduledIssueMonitorScanOutcome {
    Applied(Box<gwt::IssueMonitorState>),
    DeferredToLiveDaemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IssueMonitorScanEnqueueError {
    AlreadyInFlight,
    WorkerUnavailable(String),
}

impl IssueMonitorScanEnqueueError {
    fn reason(&self) -> &'static str {
        match self {
            Self::AlreadyInFlight => "scan_already_in_flight",
            Self::WorkerUnavailable(_) => "scan_worker_unavailable",
        }
    }
}

fn constant_time_issue_monitor_scope_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

/// Issue #3528: probe merged-PR completion only as far as the claim planner
/// will walk. Each probe spawns `gh`, so the scan pays for the slots it can
/// actually fill instead of for every open issue. A completed candidate frees
/// no slot, so the walk continues past it exactly like the planner does.
fn completed_claim_candidates(
    available: usize,
    candidates: Vec<u64>,
    mut completed_probe: impl FnMut(u64) -> bool,
) -> std::collections::BTreeSet<u64> {
    let mut completed = std::collections::BTreeSet::new();
    let mut remaining = available;
    for issue_number in candidates {
        if remaining == 0 {
            break;
        }
        if completed_probe(issue_number) {
            completed.insert(issue_number);
        } else {
            remaining -= 1;
        }
    }
    completed
}

fn run_scheduled_issue_monitor_scan(
    project_root: &Path,
    expected_project_tab_id: Option<&str>,
    now: &str,
    issue_client_factory: &RuntimeIssueClientFactory,
) -> Result<ScheduledIssueMonitorScanOutcome, String> {
    run_scheduled_issue_monitor_scan_with_budgets(
        project_root,
        expected_project_tab_id,
        now,
        issue_client_factory,
        ISSUE_MONITOR_SCAN_BUDGET,
        ISSUE_MONITOR_COMMIT_BUDGET,
    )
}

/// The read/probe phase's own budget. Exceeding it degrades the scan's
/// findings; it must never consume the budget the commit phase needs.
const ISSUE_MONITOR_SCAN_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

/// The authority lease + state commit budget, entered only after the
/// read/probe phase has released its own. Issue #3528: sharing one budget let
/// a slow scan expire before its own commit, so every tick threw away
/// everything it had just learned and the monitor never advanced.
const ISSUE_MONITOR_COMMIT_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

fn run_scheduled_issue_monitor_scan_with_budgets(
    project_root: &Path,
    expected_project_tab_id: Option<&str>,
    now: &str,
    issue_client_factory: &RuntimeIssueClientFactory,
    scan_budget: std::time::Duration,
    commit_budget: std::time::Duration,
) -> Result<ScheduledIssueMonitorScanOutcome, String> {
    // Issue #3609: this runs on the worker thread `enqueue_issue_monitor_scan_worker`
    // spawned, so the prefs path is resolved from the process-global `HOME` here, not
    // from the caller's thread. A test that isolates its gwt home with the
    // `ScopedGwtHome` thread-local pin cannot reach this thread; such tests must hold
    // `env_test_lock()` and repoint `HOME` instead. The rule is enforced by
    // `crates/gwt/tests/bin_gwt_home_isolation_contract_test.rs`.
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);

    // Cheap authority probe before any remote I/O. The lease is intentionally
    // dropped before the side-effect-free GitHub scan so daemon startup is not
    // held off by a slow network. Authority is acquired again immediately
    // before the first durable proposal commit.
    match gwt::try_acquire_issue_monitor_local_fallback_lease(&prefs_path) {
        Ok(lease) => drop(lease),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon);
        }
        Err(error) => return Err(format!("Issue Monitor authority probe failed: {error}")),
    }

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path)
        .map_err(|error| format!("load Issue Monitor prefs failed: {error}"))?;
    let cleanup_only = !prefs.enabled && issue_monitor_prefs_need_local_claim_cleanup(&prefs);
    if !prefs.enabled && !cleanup_only {
        return Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon);
    }
    let mut monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let mut loaded_for_commit = None;
    let mut merge_reconciliation_error = None;
    let mut local_repo_identity = None;
    let mut local_claim_proposal = None;

    // The read/probe phase owns its budget alone. It is released before the
    // commit phase below so a slow scan degrades its findings instead of
    // discarding them (#3528).
    {
        let _scan_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + scan_budget,
        );

        match gwt::issue_monitor_worker::github_remote_owner_and_repo(project_root) {
            Ok((owner, repo)) => {
                local_repo_identity = Some((owner.clone(), repo.clone()));
                if cleanup_only {
                    // Disabling the monitor revokes acquisition authority but does
                    // not cancel its durable compensation journal. Cleanup owns no
                    // scan/proposal and may run while the monitor stays disabled.
                } else {
                    match gwt::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path_with_provenance(
                project_root,
                &owner,
                &repo,
            ) {
                Ok(loaded) => {
                    gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates_for_project_tab(
                        &mut monitor,
                        &loaded,
                        project_root,
                        expected_project_tab_id,
                        now,
                    );
                    merge_reconciliation_error =
                        gwt::issue_monitor_worker::reconcile_issue_monitor_merges(
                            &mut monitor,
                            project_root,
                        )
                        .err()
                        .map(|error| {
                            format!("issue monitor merge reconciliation failed: {error}")
                        });
                    if loaded.authorizes_remote_effects() {
                        let (available, candidates) =
                            monitor.claim_probe_plan(monitor.config.max_active.max(1));
                        let completed_issues =
                            completed_claim_candidates(available, candidates, |issue_number| {
                                loaded
                                    .issues
                                    .iter()
                                    .find(|issue| issue.number == issue_number)
                                    .is_some_and(|issue| {
                                        gwt::issue_monitor_worker::issue_completed_by_merged_pr(
                                            &owner, &repo, issue,
                                        )
                                    })
                            });
                        local_claim_proposal = Some((
                            format!("{}:{}", whoami::username(), std::process::id()),
                            completed_issues,
                        ));
                    }
                    loaded_for_commit = Some(loaded);
                }
                Err(error) => {
                    monitor.record_scan_error(now, format!("issue list failed: {error}"));
                }
            }
                }
            }
            Err(error) => monitor.record_scan_error(now, error.to_string()),
        }
    }

    // A daemon may have started while the side-effect-free scan was running.
    // The second lease acquisition is the commit-time authority decision; the
    // lease remains held through Prepared -> Attempting -> remote result.
    let _commit_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + commit_budget,
    );
    let _local_lease = match gwt::try_acquire_issue_monitor_local_fallback_lease(&prefs_path) {
        Ok(lease) => lease,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon);
        }
        Err(error) => {
            return Err(format!(
                "Issue Monitor authority commit check failed: {error}"
            ))
        }
    };
    #[cfg(test)]
    run_scheduled_scan_after_lease_before_commit_test_hook();
    let commit = try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence(
        &prefs_path,
        &mut monitor,
        |latest| {
            latest.expire_stale_unbound_launches(now);
            if let Some(loaded) = &loaded_for_commit {
                gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates_for_project_tab(
                    latest,
                    loaded,
                    project_root,
                    expected_project_tab_id,
                    now,
                );
                if latest.config.enabled {
                    latest.set_gui_connected(true);
                }
                if let Some((monitor_owner, completed_issues)) = &local_claim_proposal {
                    prepare_local_issue_monitor_claim_proposals(
                        latest,
                        loaded,
                        monitor_owner,
                        now,
                        completed_issues,
                    );
                }
            }
            record_issue_monitor_scan_failures(latest, now, merge_reconciliation_error, Vec::new());
        },
    );
    if let Err(error) = commit {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon);
        }
        return Err(format!(
            "Issue Monitor scheduled scan commit failed: {error}"
        ));
    }

    if let Some((owner, repo)) = local_repo_identity {
        if let Err(error) = execute_local_issue_monitor_claim_effects(
            &prefs_path,
            &owner,
            &repo,
            &mut monitor,
            issue_client_factory,
        ) {
            let error_for_commit = error.clone();
            try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence(
                &prefs_path,
                &mut monitor,
                |latest| {
                    if error_for_commit.contains("deadline") {
                        latest.record_scan_error(now, error_for_commit);
                    } else {
                        latest.record_launch_auth_required(now.to_string());
                    }
                },
            )
            .map_err(|commit_error| {
                format!(
                    "local Issue Monitor effect failed ({error}); recording the failure failed: {commit_error}"
                )
            })?;
            tracing::warn!(error = %error, "local issue monitor claim execution unavailable");
        }
    }

    Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(monitor)))
}

fn issue_monitor_prefs_need_local_claim_cleanup(prefs: &gwt::IssueMonitorPrefs) -> bool {
    prefs.pending_effects.iter().any(|effect| {
        matches!(
            effect.payload,
            gwt::IssueMonitorEffectPayload::ReleaseClaim { .. }
        ) || (effect.state == gwt::IssueMonitorEffectState::Attempting
            && matches!(
                effect.payload,
                gwt::IssueMonitorEffectPayload::AcquireClaim { .. }
            ))
    })
}

fn quick_issue_body(title: &str) -> String {
    format!(
        "## Summary\n\n{title}\n\n## Background\n\nRegistered from the legacy Quick issue compatibility path. Intake session plus gwt-register-issue remains the primary intake workflow.\n\n## Spec Status\n\nALIGNED - Compatibility guard preserves existing web bundle payloads until the withdrawn Quick issue toolbar is fully removed.\n\n## Related SPECs\n\n- SPEC-3214\n\n## Expected Outcome\n\nTriage and route this issue through the normal gwt workflow.\n\n## Notes\n\nCreated by the SPEC-3214 Quick issue compatibility guard.\n"
    )
}

fn issue_registration_failure_message(error: &gwt_github::ApiError) -> String {
    format!(
        "Issue registration failed: {error}. Fallback: create the Issue manually on GitHub, then launch it from Issue Monitor or retry from an intake session after fixing access."
    )
}

fn issue_monitor_issue_from_snapshot(
    snapshot: &gwt_github::IssueSnapshot,
) -> gwt::IssueMonitorIssue {
    gwt::IssueMonitorIssue {
        number: snapshot.number.0,
        title: snapshot.title.clone(),
        labels: snapshot.labels.clone(),
        state: match snapshot.state {
            gwt_github::IssueState::Closed => gwt::IssueMonitorIssueState::Closed,
            gwt_github::IssueState::Open => gwt::IssueMonitorIssueState::Open,
        },
        body: (!snapshot.body.is_empty()).then(|| snapshot.body.clone()),
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: Some(snapshot.updated_at.0.clone()),
    }
}

impl AppRuntime {
    pub(crate) fn new(
        proxy: EventLoopProxy<UserEvent>,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
        blocking_tasks: BlockingTaskSpawner,
    ) -> std::io::Result<Self> {
        let session_state_path = gwt_core::paths::gwt_session_state_path();
        let launch_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let log_dir = gwt_core::paths::gwt_project_logs_dir_for_project_path(&launch_dir);
        let legacy_target = resolve_project_target(&launch_dir)
            .unwrap_or_else(|_| fallback_project_target(launch_dir.clone()));
        migrate_legacy_workspace_state(
            &gwt::legacy_workspace_state_path(),
            &session_state_path,
            &legacy_target.project_root,
            legacy_target.kind,
        )?;
        let persisted = load_session_state(&session_state_path)?;
        let tabs = persisted
            .tabs
            .into_iter()
            .map(|tab| {
                let workspace = load_restored_workspace_state(&tab.project_root)?;
                Ok(ProjectTabRuntime::from_persisted(tab, workspace))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let active_tab_id = normalize_active_tab_id(&tabs, persisted.active_tab_id);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let _ = gwt_agent::reset_runtime_state_dir(&sessions_dir);
        let launch_wizard_cache = LaunchWizardMemoryCache::load(&sessions_dir);

        let persist_dispatcher = persist_dispatcher::PersistDispatcher::new(&blocking_tasks);
        let mut app = Self {
            tabs,
            active_tab_id,
            recent_projects: prune_missing_recent_projects(dedupe_recent_projects(
                normalize_recent_projects(persisted.recent_projects),
            )),
            profile_selections: HashMap::new(),
            profile_config_path: None,
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            launch_error_terminal_details: HashMap::new(),
            window_lookup: HashMap::new(),
            board_all_view_windows: HashSet::new(),
            session_state_path,
            log_dir,
            proxy: AppEventProxy::new(proxy),
            blocking_tasks,
            sessions_dir,
            launch_wizard_cache,
            launch_wizard: None,
            pending_launch_wizard_materializations: HashMap::new(),
            pending_workspace_resume_contexts: HashMap::new(),
            inflight_launches: HashMap::new(),
            pending_pm_launches: HashMap::new(),
            pm_sessions: HashMap::new(),
            pm_wake_seen: HashMap::new(),
            pending_startup_pm_tabs: Vec::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            issue_monitor_launch_deliveries: HashMap::new(),
            issue_monitor_materializer_id: uuid::Uuid::new_v4().to_string(),
            issue_monitor_scheduled_scans_in_flight: HashSet::new(),
            pending_continue_work: HashMap::new(),
            pending_fresh_execution_launches: HashMap::new(),
            continue_work_outcomes: HashMap::new(),
            continue_work_waiters: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_tool_runtime_migrations: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::new(),
            work_merged_branches: HashMap::new(),
            work_dirty_branches: HashMap::new(),
            work_live_process_branches: HashMap::new(),
            work_cleanup_ready_branches: HashMap::new(),
            work_tip_subjects: HashMap::new(),
            work_pr_titles: HashMap::new(),
            work_ai_summaries: HashMap::new(),
            session_ledger_cache: std::cell::RefCell::new(
                crate::session_ledger_cache::SessionLedgerCache::new(),
            ),
            work_items_cache: std::cell::RefCell::new(
                gwt_core::workspace_projection::WorkItemsCache::new(),
            ),
            active_work_projection_cache: std::cell::RefCell::new(HashMap::new()),
            last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
            last_work_pr_titles_scan: std::cell::RefCell::new(HashMap::new()),
            local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
            window_pty_statuses: HashMap::new(),
            window_output_bytes: HashMap::new(),
            window_hook_states: HashMap::new(),
            window_approval_waiting: HashMap::new(),
            approval_settle_epoch: 0,
            recoverable_agent_error_windows: HashSet::new(),
            last_agent_activity: HashMap::new(),
            agent_capability_issuer: None,
            agent_capability_tokens: HashMap::new(),
            pending_agent_self_closes: HashMap::new(),
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            knowledge_related_snapshot: Default::default(),
            knowledge_monitor_snapshot: Default::default(),
            issue_client_factory: default_issue_client_factory(),
            pending_update: None,
            pty_writers,
            attachment_uploads,
            persist_dispatcher,
            file_tree_worktree_roots: HashMap::new(),
            server_url: None,
            usage_refresh: None,
            image_paste_sequence: std::sync::atomic::AtomicU64::new(0),
            agent_launch_stage_counter: std::sync::atomic::AtomicU64::new(1),
            improvement_refresh_epoch: 0,
            improvement_latest_refresh_epochs: HashMap::new(),
            agent_backend_probe_generation: 0,
            agent_backend_latest_probe_generations: HashMap::new(),
        };
        app.rebuild_window_lookup();
        app.seed_window_pty_statuses();
        app.seed_restored_window_details();
        Ok(app)
    }

    /// SPEC-2359 W-15 (FR-386): store the background merged-branch scan
    /// result and rebroadcast the Workspace projection so the "safe to
    /// delete" badge appears. Display-only; never records a close (US-61).
    pub(crate) fn apply_work_merge_status(
        &mut self,
        project_root: &Path,
        merged_branches: HashMap<String, chrono::DateTime<chrono::Utc>>,
        cleanup_ready_branches: HashMap<String, String>,
        dirty_branches: HashSet<String>,
        live_process_branches: HashSet<String>,
    ) -> Vec<OutboundEvent> {
        self.work_merged_branches
            .insert(project_root.to_path_buf(), merged_branches);
        self.work_cleanup_ready_branches
            .insert(project_root.to_path_buf(), cleanup_ready_branches);
        self.work_dirty_branches
            .insert(project_root.to_path_buf(), dirty_branches);
        self.work_live_process_branches
            .insert(project_root.to_path_buf(), live_process_branches);
        self.refresh_active_work_projection_for_project_root(project_root)
    }

    /// SPEC-2359 W-15 / US-84 and SPEC-3170 FR-077: publish cleanup evidence
    /// off the UI thread. A bulk ref snapshot rejects stale historical rows
    /// before merge/readiness checks, and worktree status is measured only for
    /// recorded merged PRs or branches that are actually cleanup-ready. Sends
    /// an event even when the result is empty so stale cleanup-readiness cache
    /// entries are cleared after a branch receives new changes.
    /// SPEC-2359 W-16 (FR-387): note an ingest attempt for `project_root`;
    /// returns false while the 30s throttle window is still open. Bootstrap
    /// and project-open callers pass `force` to bypass the window.
    pub(crate) fn note_work_events_ingest_attempt(&self, project_root: &Path, force: bool) -> bool {
        let now = std::time::Instant::now();
        let mut last = self.last_work_events_ingest.borrow_mut();
        if !force {
            if let Some(previous) = last.get(project_root) {
                if now.duration_since(*previous) < Duration::from_secs(30) {
                    return false;
                }
            }
        }
        last.insert(project_root.to_path_buf(), now);
        true
    }

    /// SPEC-2359 W-16 (FR-387): run the cross-machine work events ingest on a
    /// background thread, then hand control back to the event loop via
    /// [`UserEvent::WorkEventsIngested`] so the worktree reconcile runs in
    /// intake → reconcile order (plan decision 9).
    pub(crate) fn spawn_work_events_ingest(&self, project_root: PathBuf, force: bool) {
        if !self.note_work_events_ingest_attempt(&project_root, force) {
            return;
        }
        // Issue #3604: bootstrap and project-open are the moments a stale rail
        // summary would actually be noticed, so they reopen the PR-title window
        // that the frequent triggers (tab switch, every launch) must respect.
        if force {
            self.reopen_work_pr_titles_window(&project_root);
        }
        let proxy = self.proxy.clone();
        // Resolve the home-projection paths on the calling thread: HOME is
        // process-global and parallel unit tests scope it per test
        // (ScopedEnvVar, #3022) — a late resolution inside the worker would
        // race those scopes and write into another test's home.
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let state_path = gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(
            &project_root,
        );
        let projection_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        thread::spawn(move || {
            let summary = crate::work_events_ingest::ingest_project_work_events_paths(
                &project_root,
                &work_items_path,
                &state_path,
            );
            // #3065: detection-based repair for the resume owner bleed. Runs
            // after every ingest so re-ingested contaminated logs (from other
            // machines / refs) self-heal; converges to a no-op on clean data.
            let repaired = gwt_core::workspace_projection::repair_resume_owner_bleed_paths(
                &work_items_path,
                &projection_path,
                chrono::Utc::now(),
            )
            .map(|report| report.changed())
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "resume owner bleed repair failed");
                false
            });
            proxy.send(UserEvent::WorkEventsIngested {
                project_root,
                changed: summary.changed() || repaired,
            });
        });
    }

    /// Event-loop continuation of [`Self::spawn_work_events_ingest`]:
    /// reconcile worktrees after the intake, kick the merge scan, and
    /// rebroadcast the projection when the intake applied anything.
    pub(crate) fn handle_work_events_ingested(
        &mut self,
        project_root: PathBuf,
        changed: bool,
    ) -> Vec<OutboundEvent> {
        self.reconcile_workspace_worktrees(&project_root);
        self.spawn_work_merge_status_scan(project_root.clone());
        self.spawn_work_tip_subjects_scan(project_root.clone());
        self.spawn_work_pr_titles_scan(project_root.clone());
        self.spawn_work_ai_summaries_scan(project_root.clone());
        if changed {
            self.refresh_active_work_projection_for_project_root(&project_root)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn spawn_work_merge_status_scan(&self, project_root: PathBuf) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let Ok(projection) =
                gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(
                    &project_root,
                )
            else {
                return;
            };
            let mut targets = work_branch_scan_targets(&projection);
            append_workspace_projection_scan_target(&project_root, &mut targets);
            if targets.is_empty() {
                proxy.send(UserEvent::WorkMergeStatus {
                    project_root,
                    merged_branches: HashMap::new(),
                    cleanup_ready_branches: HashMap::new(),
                    dirty_branches: HashSet::new(),
                    live_process_branches: HashSet::new(),
                });
                return;
            }
            let live_process_branches = work_branches_with_live_processes(&targets);
            // One ref snapshot replaces the former per-target `rev-parse`
            // probes. Historical Work rows routinely outlive their branches;
            // rejecting those rows in memory prevents hundreds of short-lived
            // Git processes from competing with the GUI and agent PTYs.
            let tip_times = match gwt_git::refs::branch_tip_committer_times(&project_root) {
                Ok(tip_times) => tip_times,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        project = %project_root.display(),
                        "work merge ref snapshot failed; publishing fail-closed dirty verdicts"
                    );
                    proxy.send(UserEvent::WorkMergeStatus {
                        project_root,
                        merged_branches: HashMap::new(),
                        cleanup_ready_branches: HashMap::new(),
                        dirty_branches: targets
                            .iter()
                            .filter(|target| !target.worktree_paths.is_empty())
                            .map(|target| target.branch.clone())
                            .collect(),
                        live_process_branches,
                    });
                    return;
                }
            };
            let known_refs: HashSet<String> = tip_times.keys().cloned().collect();
            let mut merged: Vec<String> = Vec::new();
            let mut cleanup_ready_branches: HashMap<String, String> = HashMap::new();
            let mut dirty_branches = HashSet::new();
            for target in &targets {
                let branch = target.branch.clone();
                let readiness = gwt_git::branch::cleanup_readiness_base_target_with_known_refs(
                    &project_root,
                    &branch,
                    &known_refs,
                )
                .ok()
                .flatten();
                if !work_merge_scan_needs_dirty_check(readiness.as_ref(), target.has_merged_pr) {
                    continue;
                }
                if work_branch_has_dirty_worktree(target) {
                    dirty_branches.insert(branch);
                    continue;
                }
                if let Some(readiness) = readiness {
                    let reason = match readiness.reason {
                        gwt_git::branch::CleanupReadinessReason::Merged => {
                            merged.push(branch.clone());
                            gwt_core::workspace_projection::WorkspaceCleanupReason::PrMerged
                        }
                        gwt_git::branch::CleanupReadinessReason::NoChanges => {
                            gwt_core::workspace_projection::WorkspaceCleanupReason::NoChanges
                        }
                    };
                    cleanup_ready_branches.insert(branch, reason.as_str().to_string());
                }
            }
            let merged_branches: HashMap<String, chrono::DateTime<chrono::Utc>> = merged
                .into_iter()
                .map(|branch| {
                    let unix = tip_times
                        .get(&branch)
                        .or_else(|| tip_times.get(&format!("origin/{branch}")))
                        .copied();
                    let reference = unix
                        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                        .unwrap_or_else(chrono::Utc::now);
                    (branch, reference)
                })
                .collect();
            proxy.send(UserEvent::WorkMergeStatus {
                project_root,
                merged_branches,
                cleanup_ready_branches,
                dirty_branches,
                live_process_branches,
            });
        });
    }

    /// SPEC-3075: cache the resolved `branch -> tip commit subject` map and
    /// rebroadcast so the Workspace rail re-renders with the historical summary.
    pub(crate) fn apply_work_tip_subjects(
        &mut self,
        project_root: &Path,
        tip_subjects: HashMap<String, String>,
    ) -> Vec<OutboundEvent> {
        self.work_tip_subjects
            .insert(project_root.to_path_buf(), tip_subjects);
        self.refresh_active_work_projection_for_project_root(project_root)
    }

    /// SPEC-3075: resolve every branch's tip commit subject off the UI thread in
    /// ONE `for-each-ref` spawn, then hand the map to the event loop via
    /// [`UserEvent::WorkTipSubjects`]. This is the "what work was running" signal
    /// for historical Works with no recorded purpose. Mirrors
    /// [`Self::spawn_work_merge_status_scan`] but runs for every project (not
    /// just merged branches) since every Workspace row benefits.
    pub(crate) fn spawn_work_tip_subjects_scan(&self, project_root: PathBuf) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let tip_subjects =
                gwt_git::refs::branch_tip_subjects(&project_root).unwrap_or_default();
            if tip_subjects.is_empty() {
                return;
            }
            proxy.send(UserEvent::WorkTipSubjects {
                project_root,
                tip_subjects,
            });
        });
    }

    /// SPEC-3075: cache the resolved `branch -> PR title` map and rebroadcast so
    /// the Workspace rail re-renders with the PR-title summary (top priority).
    pub(crate) fn apply_work_pr_titles(
        &mut self,
        project_root: &Path,
        pr_titles: HashMap<String, String>,
    ) -> Vec<OutboundEvent> {
        self.work_pr_titles
            .insert(project_root.to_path_buf(), pr_titles);
        self.refresh_active_work_projection_for_project_root(project_root)
    }

    /// SPEC-3075: resolve every branch's PR title off the UI thread in ONE
    /// `gh pr list` call (the GitHub API may paginate), then hand the
    /// `branch -> title` map to the event loop via [`UserEvent::WorkPrTitles`].
    /// The PR title is the human-written purpose of the work — the strongest
    /// "what work was running" signal. Network-dependent: an empty map (offline
    /// / `gh` absent / unauthenticated) leaves the commit-subject fallback in
    /// place. Runs once per project-open, after the events ingest.
    ///
    /// Issue #3604: this is the only *network* leg of the ingest continuation,
    /// and `gh pr list --state all --limit 999` costs up to ten GraphQL
    /// requests. Riding the 30s ingest throttle meant an ordinary GUI session
    /// re-enumerated every PR in the repository twice a minute against a
    /// 5000-point hourly budget, so it now has its own window
    /// ([`WORK_PR_TITLES_SCAN_WINDOW`]).
    /// Let the next PR-title enumeration for `project_root` run immediately,
    /// regardless of when the last one happened.
    pub(crate) fn reopen_work_pr_titles_window(&self, project_root: &Path) {
        self.last_work_pr_titles_scan
            .borrow_mut()
            .remove(project_root);
    }

    pub(crate) fn note_work_pr_titles_scan_attempt(&self, project_root: &Path) -> bool {
        let now = std::time::Instant::now();
        let mut last = self.last_work_pr_titles_scan.borrow_mut();
        if let Some(previous) = last.get(project_root) {
            if now.duration_since(*previous) < WORK_PR_TITLES_SCAN_WINDOW {
                return false;
            }
        }
        last.insert(project_root.to_path_buf(), now);
        true
    }

    pub(crate) fn spawn_work_pr_titles_scan(&self, project_root: PathBuf) {
        if !self.note_work_pr_titles_scan_attempt(&project_root) {
            return;
        }
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let pr_titles =
                gwt_git::pr_status::fetch_pr_titles_by_branch(&project_root).unwrap_or_default();
            if pr_titles.is_empty() {
                return;
            }
            proxy.send(UserEvent::WorkPrTitles {
                project_root,
                pr_titles,
            });
        });
    }

    /// SPEC-3075 FR-006: cache the AI-polished `branch -> summary` map and
    /// rebroadcast so the Workspace rail re-renders with the cleaned summaries.
    pub(crate) fn apply_work_ai_summaries(
        &mut self,
        project_root: &Path,
        ai_summaries: HashMap<String, String>,
    ) -> Vec<OutboundEvent> {
        self.work_ai_summaries
            .insert(project_root.to_path_buf(), ai_summaries);
        self.refresh_active_work_projection_for_project_root(project_root)
    }

    /// SPEC-3075 FR-006: optional AI polish for the rail summary. Runs off the UI
    /// thread and ONLY when AI is enabled (`summary_enabled` + valid
    /// endpoint/model). For the Workspaces whose best non-AI summary would be
    /// merge/release commit noise, it feeds the structured meta (owner + recent
    /// non-merge commit subjects — never the session transcript) to the AI and
    /// caches a cleaned one-line purpose. Sends [`UserEvent::WorkAiSummaries`]
    /// when anything was produced; silent (no event) when AI is disabled, the
    /// AI call fails, or nothing needed polishing — the non-AI chain then
    /// stands unchanged (fallback always).
    pub(crate) fn spawn_work_ai_summaries_scan(&self, project_root: PathBuf) {
        let ai = gwt_config::Settings::load().unwrap_or_default().ai;
        if !ai.summary_enabled || !ai.is_enabled() {
            return;
        }
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let inputs = build_ai_summary_inputs(&project_root, AI_SUMMARY_BRANCH_CAP);
            if inputs.is_empty() {
                return;
            }
            let Ok(client) =
                gwt_ai::AIClient::new(&ai.endpoint, ai.api_key.as_deref().unwrap_or(""), &ai.model)
            else {
                return;
            };
            let Ok(ai_summaries) = gwt_ai::summarize_work_purposes(&client, &inputs) else {
                return;
            };
            if ai_summaries.is_empty() {
                return;
            }
            proxy.send(UserEvent::WorkAiSummaries {
                project_root,
                ai_summaries,
            });
        });
    }

    /// SPEC-2359 Phase W-15 (FR-379/FR-380/FR-382): reconcile locally existing
    /// worktrees with the persisted Work records. Worktrees without a record
    /// are backfilled (event into the worktree's own `.gwt/work/events.jsonl`
    /// plus the home works projection) so the Workspace list shows the union
    /// of existing worktrees and unclosed records. Errors are logged and
    /// swallowed — reconciliation must never block startup or project open.
    pub(crate) fn reconcile_workspace_worktrees(&self, project_root: &Path) {
        let entries = match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(
                    "workspace worktree reconcile: enumerate failed for {}: {error}",
                    project_root.display()
                );
                return;
            }
        };
        // SPEC-2359 W16-3 (FR-390): refresh the local-worktree branch set the
        // remote_only view marking reads (cache lookup only — no git spawn at
        // view time).
        let local_branches: std::collections::HashSet<String> = entries
            .iter()
            .filter_map(|entry| entry.branch.as_deref())
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
            .collect();
        self.local_worktree_branches
            .borrow_mut()
            .insert(project_root.to_path_buf(), local_branches);
        let sources = gwt::worktree_inventory::worktree_reconcile_sources(&entries);
        if sources.is_empty() {
            return;
        }
        match gwt_core::workspace_projection::reconcile_worktree_work_items(
            project_root,
            &sources,
            chrono::Utc::now(),
        ) {
            Ok(0) => {}
            Ok(count) => tracing::info!(
                "workspace worktree reconcile: backfilled {count} worktree(s) for {}",
                project_root.display()
            ),
            Err(error) => tracing::warn!(
                "workspace worktree reconcile failed for {}: {error}",
                project_root.display()
            ),
        }
    }

    /// SPEC-2970 FR-009/FR-013: persist the Claude account-usage opt-in and
    /// request an immediate refresh.
    fn set_claude_account_usage_enabled_events(&self, enabled: bool) -> Vec<OutboundEvent> {
        if let Err(error) = gwt_config::Settings::update_global(|settings| {
            settings.usage.claude_account_enabled = enabled;
            Ok(())
        }) {
            tracing::warn!(%error, "failed to persist Claude usage opt-in");
        }
        self.request_usage_refresh_events()
    }

    /// SPEC-2970 FR-022: nudge the background poller to refresh now.
    fn request_usage_refresh_events(&self) -> Vec<OutboundEvent> {
        if let Some(refresh) = &self.usage_refresh {
            refresh.notify_one();
        }
        Vec::new()
    }

    fn issue_monitor_project_root_for_window(&self, window_id: &str) -> Option<PathBuf> {
        let address = self.window_lookup.get(window_id)?;
        self.tab(&address.tab_id)
            .map(|tab| tab.project_root.clone())
    }

    fn issue_monitor_tab_id_for_project_root(&self, project_root: &Path) -> Option<String> {
        self.tabs
            .iter()
            .find(|tab| same_worktree_path(&tab.project_root, project_root))
            .map(|tab| tab.id.clone())
    }

    fn issue_monitor_issue_number_for_window(
        &self,
        project_root: &Path,
        window_id: &str,
    ) -> Option<u64> {
        self.pending_launch_feedback_contexts
            .get(window_id)
            .and_then(|context| context.issue_monitor_issue_number)
            .or_else(|| {
                let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).ok()?;
                gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs)
                    .launched_window_issue(window_id)
            })
    }

    fn publish_active_issue_monitor_control(
        &self,
        payload: serde_json::Value,
    ) -> Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let project_root = self.active_project_root().ok_or_else(|| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "no active project".to_string(),
            )
        })?;
        self.publish_issue_monitor_control(project_root, payload)
    }

    #[cfg(unix)]
    fn publish_issue_monitor_control(
        &self,
        project_root: &Path,
        payload: serde_json::Value,
    ) -> Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let payload = gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            payload,
            std::process::id(),
        );
        gwt::daemon_publisher::publish_issue_monitor_control(project_root, payload)
    }

    #[cfg(not(unix))]
    fn publish_issue_monitor_control(
        &self,
        _project_root: &Path,
        _payload: serde_json::Value,
    ) -> Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "Issue Monitor daemon control is unavailable on this platform".to_string(),
            ),
        )
    }

    fn claim_issue_monitor_launch_delivery(
        &self,
        project_root: &Path,
        issue_number: u64,
        delivery_id: &str,
        materializer_window_id: &str,
    ) -> Result<bool, gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let materializer_id = self.issue_monitor_materializer_id.clone();
        let materializer_pid = std::process::id();
        let publication = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({
                "claim_launch_delivery": {
                    "issue_number": issue_number,
                    "delivery_id": delivery_id,
                    "materializer_id": materializer_id.clone(),
                    "materializer_pid": materializer_pid,
                    "materializer_window_id": materializer_window_id,
                }
            }),
        );
        match publication {
            Ok(()) => {
                let prefs = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
                )
                .map_err(|error| {
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                        format!("launch delivery claim readback failed: {error}"),
                    )
                })?;
                Ok(prefs.pending_launch_deliveries.iter().any(|delivery| {
                    delivery.issue_number == issue_number
                        && delivery.delivery_id == delivery_id
                        && delivery.materializer_id.as_deref() == Some(materializer_id.as_str())
                        && delivery.materializer_pid == Some(materializer_pid)
                        && delivery.materializer_window_id.as_deref()
                            == Some(materializer_window_id)
                }))
            }
            Err(error) if error.allows_local_fallback() => self
                .commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    monitor.claim_launch_delivery(
                        issue_number,
                        delivery_id,
                        &materializer_id,
                        materializer_pid,
                        materializer_window_id,
                        gwt::process::is_host_process_alive,
                    )
                })
                .map(|(_monitor, accepted)| accepted),
            Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(_)) => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn persist_issue_monitor_delivery_workspace(
        &self,
        project_root: &Path,
        window_id: &str,
    ) -> Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let address = self.window_lookup.get(window_id).ok_or_else(|| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(format!(
                "launch delivery window {window_id} is not registered"
            ))
        })?;
        let tab = self.tab(&address.tab_id).ok_or_else(|| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(format!(
                "launch delivery project tab {} is unavailable",
                address.tab_id
            ))
        })?;
        if !same_worktree_path(&tab.project_root, project_root) {
            return Err(
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(format!(
                    "launch delivery window {window_id} belongs to a different project"
                )),
            );
        }
        self.persist_dispatcher
            .flush_workspace_durable(
                gwt::workspace_state_path(&tab.project_root),
                self.persistable_workspace_state(tab),
            )
            .map_err(|error| {
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                    format!("launch delivery workspace persistence failed: {error}"),
                )
            })
    }

    fn mark_issue_monitor_launch_delivery_materialized(
        &self,
        project_root: &Path,
        issue_number: u64,
        delivery_id: &str,
        materializer_window_id: &str,
    ) -> Result<bool, gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let materializer_id = self.issue_monitor_materializer_id.clone();
        let publication = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({
                "launch_delivery_materialized": {
                    "issue_number": issue_number,
                    "delivery_id": delivery_id,
                    "materializer_id": materializer_id.clone(),
                    "materializer_window_id": materializer_window_id,
                }
            }),
        );
        match publication {
            Ok(()) => {
                let prefs = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
                )
                .map_err(|error| {
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                        format!("launch delivery materialization readback failed: {error}"),
                    )
                })?;
                Ok(prefs.pending_launch_deliveries.iter().any(|delivery| {
                    delivery.issue_number == issue_number
                        && delivery.delivery_id == delivery_id
                        && delivery.materializer_id.as_deref() == Some(materializer_id.as_str())
                        && delivery.materialized_window_id.as_deref()
                            == Some(materializer_window_id)
                }))
            }
            Err(error) if error.allows_local_fallback() => self
                .commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    monitor.mark_launch_delivery_materialized(
                        issue_number,
                        delivery_id,
                        &materializer_id,
                        materializer_window_id,
                    )
                })
                .map(|(_monitor, accepted)| accepted),
            Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(_)) => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn mark_issue_monitor_launch_delivery_workspace_durable(
        &self,
        project_root: &Path,
        issue_number: u64,
        delivery_id: &str,
        materializer_window_id: &str,
    ) -> Result<bool, gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
        let materializer_id = self.issue_monitor_materializer_id.clone();
        let publication = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({
                "launch_delivery_workspace_durable": {
                    "issue_number": issue_number,
                    "delivery_id": delivery_id,
                    "materializer_id": materializer_id.clone(),
                    "materializer_window_id": materializer_window_id,
                }
            }),
        );
        match publication {
            Ok(()) => {
                let prefs = gwt::load_issue_monitor_prefs(
                    &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
                )
                .map_err(|error| {
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                        format!("launch delivery workspace durability readback failed: {error}"),
                    )
                })?;
                Ok(prefs.pending_launch_deliveries.iter().any(|delivery| {
                    delivery.issue_number == issue_number
                        && delivery.delivery_id == delivery_id
                        && delivery.materializer_id.as_deref() == Some(materializer_id.as_str())
                        && delivery.workspace_durable_window_id.as_deref()
                            == Some(materializer_window_id)
                }))
            }
            Err(error) if error.allows_local_fallback() => self
                .commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    monitor.mark_launch_delivery_workspace_durable(
                        issue_number,
                        delivery_id,
                        &materializer_id,
                        materializer_window_id,
                    )
                })
                .map(|(_monitor, accepted)| accepted),
            Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(_)) => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn issue_monitor_launch_completed_delivery_events(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        window_id: &str,
        delivery_id: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(delivery_id) = delivery_id else {
            return self.issue_monitor_launch_succeeded_delivery_events(
                project_root,
                issue_number,
                window_id,
                None,
            );
        };
        self.issue_monitor_launch_deliveries.insert(
            delivery_id.to_string(),
            IssueMonitorLaunchDeliveryState::LaunchedPendingAck {
                window_id: window_id.to_string(),
            },
        );
        match self.mark_issue_monitor_launch_delivery_materialized(
            project_root,
            issue_number,
            delivery_id,
            window_id,
        ) {
            Ok(true) => {}
            Ok(false) => return Vec::new(),
            Err(error) => {
                return self.issue_monitor_control_error_events(
                    None,
                    error,
                    "mark-launch-delivery-materialized",
                    Some(issue_number),
                )
            }
        };
        if let Err(error) = self.persist_issue_monitor_delivery_workspace(project_root, window_id) {
            return self.issue_monitor_control_error_events(
                None,
                error,
                "persist-launch-delivery-window",
                Some(issue_number),
            );
        }
        match self.mark_issue_monitor_launch_delivery_workspace_durable(
            project_root,
            issue_number,
            delivery_id,
            window_id,
        ) {
            Ok(true) => self.issue_monitor_launch_succeeded_delivery_events(
                project_root,
                issue_number,
                window_id,
                Some(delivery_id),
            ),
            Ok(false) => Vec::new(),
            Err(error) => self.issue_monitor_control_error_events(
                None,
                error,
                "mark-launch-delivery-workspace-durable",
                Some(issue_number),
            ),
        }
    }

    pub(crate) fn issue_monitor_launch_failed_events(
        &mut self,
        project_root: Option<&Path>,
        issue_number: u64,
        message: &str,
    ) -> Vec<OutboundEvent> {
        self.issue_monitor_launch_failed_delivery_events(project_root, issue_number, message, None)
    }

    pub(crate) fn issue_monitor_launch_failed_delivery_events(
        &mut self,
        project_root: Option<&Path>,
        issue_number: u64,
        message: &str,
        delivery_id: Option<&str>,
    ) -> Vec<OutboundEvent> {
        self.issue_monitor_launch_failed_delivery_events_with_mode(
            project_root,
            issue_number,
            message,
            delivery_id,
            gwt_agent::SessionMode::Normal,
        )
    }

    pub(crate) fn issue_monitor_launch_failed_delivery_events_with_mode(
        &mut self,
        project_root: Option<&Path>,
        issue_number: u64,
        message: &str,
        delivery_id: Option<&str>,
        session_mode: gwt_agent::SessionMode,
    ) -> Vec<OutboundEvent> {
        let message = if gwt::issue_monitor::is_git_https_auth_error(message) {
            gwt::issue_monitor::git_https_auth_setup_message(message)
        } else {
            message.to_string()
        };
        if let Some(delivery_id) = delivery_id {
            self.issue_monitor_launch_deliveries.insert(
                delivery_id.to_string(),
                IssueMonitorLaunchDeliveryState::LaunchFailed {
                    message: message.clone(),
                    session_mode,
                },
            );
        }
        let launch_failed = Self::issue_monitor_launch_failed_payload(
            issue_number,
            &message,
            delivery_id,
            delivery_id.map(|_| self.issue_monitor_materializer_id.as_str()),
            session_mode,
        );
        let publication = project_root.map_or_else(
            || {
                Err(
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                        "no owning project is available for launch failure".to_string(),
                    ),
                )
            },
            |project_root| {
                self.publish_issue_monitor_control(
                    project_root,
                    launch_failed,
                )
            },
        );
        self.issue_monitor_launch_failed_result_events_with_delivery(
            project_root,
            issue_number,
            &message,
            delivery_id,
            session_mode,
            publication,
        )
    }

    pub(crate) fn issue_monitor_launch_failed_payload(
        issue_number: u64,
        message: &str,
        delivery_id: Option<&str>,
        materializer_id: Option<&str>,
        session_mode: gwt_agent::SessionMode,
    ) -> serde_json::Value {
        let mut launch_failed = serde_json::json!({
            "issue_number": issue_number,
            "message": message,
        });
        if let Some(delivery_id) = delivery_id {
            launch_failed["delivery_id"] = serde_json::json!(delivery_id);
        }
        if let Some(materializer_id) = materializer_id {
            launch_failed["materializer_id"] = serde_json::json!(materializer_id);
        }
        if let Some(failure) = runtime_events::classify_issue_monitor_failure(message, session_mode)
        {
            launch_failed["failure"] =
                serde_json::to_value(failure).expect("Issue Monitor failure serializes");
        }
        serde_json::json!({ "launch_failed": launch_failed })
    }

    #[cfg(test)]
    fn issue_monitor_launch_failed_result_events(
        &mut self,
        issue_number: u64,
        message: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let project_root = self.active_project_root().map(Path::to_path_buf);
        self.issue_monitor_launch_failed_result_events_with_delivery(
            project_root.as_deref(),
            issue_number,
            message,
            None,
            gwt_agent::SessionMode::Normal,
            publication,
        )
    }

    fn issue_monitor_launch_failed_result_events_with_delivery(
        &mut self,
        project_root: Option<&Path>,
        issue_number: u64,
        message: &str,
        delivery_id: Option<&str>,
        session_mode: gwt_agent::SessionMode,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let failure = runtime_events::classify_issue_monitor_failure(message, session_mode);
        let (mut events, committed, retain_delivery) = match publication {
            Ok(()) => (
                Vec::new(),
                match &failure {
                    // The daemon only ACKs a typed failure after its exact
                    // source identity committed. A stale delivery is rejected
                    // at the control completion boundary, so no unrelated
                    // FreshRequired state can be mistaken for this result.
                    Some(gwt::IssueMonitorFailure::ResumeWriterConflict { .. }) => true,
                    None => delivery_id.is_none_or(|delivery_id| {
                        project_root.is_some_and(|project_root| {
                            self.issue_monitor_launch_failure_committed(
                                project_root,
                                issue_number,
                                message,
                                delivery_id,
                            )
                        })
                    }),
                },
                false,
            ),
            Err(error) if error.allows_local_fallback() && project_root.is_some() => {
                let project_root = project_root.expect("guarded by is_some");
                match self.commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    match &failure {
                        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                            holder_window_id,
                        }) => {
                            let Some(delivery_id) = delivery_id else {
                                return IssueMonitorFailureCommit::Rejected;
                            };
                            issue_monitor_writer_conflict_commit(
                                monitor.try_requeue_launch_resume_writer_conflict(
                                    issue_number,
                                    delivery_id,
                                    &self.issue_monitor_materializer_id,
                                    message.to_string(),
                                    holder_window_id.as_deref(),
                                ),
                                issue_number,
                            )
                        }
                        None => {
                            if monitor.record_launch_failed_delivery(
                                issue_number,
                                message.to_string(),
                                delivery_id,
                                delivery_id.map(|_| self.issue_monitor_materializer_id.as_str()),
                            ) {
                                IssueMonitorFailureCommit::Committed(Some(issue_number))
                            } else {
                                IssueMonitorFailureCommit::Rejected
                            }
                        }
                    }
                }) {
                    Ok((monitor, IssueMonitorFailureCommit::Committed(_))) => (
                        self.issue_monitor_snapshot_events_for(None, Some(project_root), monitor),
                        true,
                        false,
                    ),
                    Ok((_monitor, IssueMonitorFailureCommit::Rejected)) => {
                        (Vec::new(), false, false)
                    }
                    Ok((_monitor, IssueMonitorFailureCommit::AuthorityExhausted)) => (
                        self.issue_monitor_control_error_events(
                            None,
                            gwt::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked,
                            "launch-failed",
                            Some(issue_number),
                        ),
                        false,
                        true,
                    ),
                    Err(local_error) => (
                        self.issue_monitor_control_error_events(
                            None,
                            local_error,
                            "launch-failed",
                            Some(issue_number),
                        ),
                        false,
                        false,
                    ),
                }
            }
            Err(error) => (
                self.issue_monitor_control_error_events(
                    None,
                    error,
                    "launch-failed",
                    Some(issue_number),
                ),
                false,
                false,
            ),
        };
        if committed {
            events.extend([
                OutboundEvent::broadcast(BackendEvent::IssueMonitorLaunchFailed {
                    issue_number,
                    message: message.to_string(),
                }),
                OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: message.to_string(),
                    issue_number: Some(issue_number),
                }),
            ]);
        } else if !retain_delivery {
            if let Some(delivery_id) = delivery_id {
                self.issue_monitor_launch_deliveries.remove(delivery_id);
            }
        }
        events
    }

    fn issue_monitor_launch_failure_committed(
        &self,
        project_root: &Path,
        issue_number: u64,
        message: &str,
        delivery_id: &str,
    ) -> bool {
        let Ok(prefs) = gwt::load_issue_monitor_prefs(
            &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
        ) else {
            return false;
        };
        !prefs.pending_launch_deliveries.iter().any(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        }) && prefs
            .failed_issues
            .iter()
            .any(|failed| failed.issue_number == issue_number && failed.message == message)
    }

    #[cfg(test)]
    pub(crate) fn issue_monitor_launch_succeeded_events(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        window_id: &str,
    ) -> Vec<OutboundEvent> {
        self.issue_monitor_launch_succeeded_delivery_events(
            project_root,
            issue_number,
            window_id,
            None,
        )
    }

    pub(crate) fn issue_monitor_launch_succeeded_delivery_events(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        window_id: &str,
        delivery_id: Option<&str>,
    ) -> Vec<OutboundEvent> {
        if let Some(delivery_id) = delivery_id {
            self.issue_monitor_launch_deliveries.insert(
                delivery_id.to_string(),
                IssueMonitorLaunchDeliveryState::Launched {
                    window_id: window_id.to_string(),
                },
            );
        }
        let stale_window =
            self.issue_monitor_failed_window_read_only(project_root, issue_number, window_id);
        let mut launched = serde_json::json!({
            "issue_number": issue_number,
            "window_id": window_id,
        });
        if let Some(delivery_id) = delivery_id {
            launched["delivery_id"] = serde_json::json!(delivery_id);
        }
        let publication = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({ "launched": launched }),
        );
        self.issue_monitor_launch_succeeded_result_events_with_stale(
            project_root,
            issue_number,
            window_id,
            delivery_id,
            publication,
            stale_window,
        )
    }

    #[cfg(test)]
    fn issue_monitor_launch_succeeded_result_events(
        &mut self,
        issue_number: u64,
        window_id: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let project_root = self
            .active_project_root()
            .map(Path::to_path_buf)
            .expect("test runtime must have an active project");
        let stale_window =
            self.issue_monitor_failed_window_read_only(&project_root, issue_number, window_id);
        self.issue_monitor_launch_succeeded_result_events_with_stale(
            &project_root,
            issue_number,
            window_id,
            None,
            publication,
            stale_window,
        )
    }

    fn issue_monitor_launch_succeeded_result_events_with_stale(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        window_id: &str,
        delivery_id: Option<&str>,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
        ack_stale_window: Option<String>,
    ) -> Vec<OutboundEvent> {
        let window_id = window_id.to_string();
        // #3165 error-window lifecycle (default mode): when an issue relaunches
        // after a failure, close the stale agent window from the prior attempt so
        // it is replaced rather than left on the canvas. Guard against closing the
        // freshly launched window if it happens to reuse the same id.
        let mut stale_window: Option<String> = None;
        let mut events = match publication {
            Ok(()) => {
                stale_window = ack_stale_window;
                Vec::new()
            }
            Err(error) if error.allows_local_fallback() => {
                match self.commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    let stale_window = monitor
                        .prefs()
                        .failed_issues
                        .iter()
                        .find(|failed| failed.issue_number == issue_number)
                        .and_then(|failed| failed.window_id.clone())
                        .filter(|stale| *stale != window_id);
                    let accepted = monitor.complete_active_launch_delivery(
                        issue_number,
                        window_id.clone(),
                        delivery_id,
                    );
                    (stale_window, accepted)
                }) {
                    Ok((monitor, (committed_stale_window, true))) => {
                        stale_window = committed_stale_window;
                        self.issue_monitor_snapshot_events_for(None, Some(project_root), monitor)
                    }
                    Ok((_monitor, (_committed_stale_window, false))) => Vec::new(),
                    Err(local_error) => self.issue_monitor_control_error_events(
                        None,
                        local_error,
                        "launch-succeeded",
                        Some(issue_number),
                    ),
                }
            }
            Err(error) => self.issue_monitor_control_error_events(
                None,
                error,
                "launch-succeeded",
                Some(issue_number),
            ),
        };
        if let Some(stale) = stale_window {
            events.extend(self.close_window_events(&stale));
        }
        events
    }

    fn issue_monitor_failed_window_read_only(
        &self,
        project_root: &Path,
        issue_number: u64,
        fresh_window_id: &str,
    ) -> Option<String> {
        let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
            project_root,
        ))
        .ok()?;
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        monitor
            .take_failed_window(issue_number)
            .filter(|stale| stale != fresh_window_id)
    }

    pub(crate) fn issue_monitor_agent_failed_events_with_mode(
        &mut self,
        project_root: &Path,
        window_id: &str,
        message: &str,
        session_mode: gwt_agent::SessionMode,
    ) -> Vec<OutboundEvent> {
        let message = message.trim();
        let message = if message.is_empty() {
            "Agent entered error state"
        } else {
            message
        };
        let issue_number_hint = self
            .pending_launch_feedback_contexts
            .get(window_id)
            .and_then(|context| context.issue_monitor_issue_number);
        let failure = self.issue_monitor_failure_for_window(window_id, message, session_mode);
        let publication = self.publish_issue_monitor_control(
            project_root,
            Self::issue_monitor_agent_failed_payload_with_failure(
                window_id,
                message,
                issue_number_hint,
                failure.as_ref(),
            ),
        );
        self.issue_monitor_agent_failed_result_events_for_project(
            project_root,
            window_id,
            message,
            issue_number_hint,
            session_mode,
            publication,
        )
    }

    fn issue_monitor_session_mode_for_window(&self, window_id: &str) -> gwt_agent::SessionMode {
        self.pending_launch_feedback_contexts
            .get(window_id)
            .and_then(|context| context.issue_monitor_session_mode)
            .or_else(|| {
                let active = self.active_agent_sessions.get(window_id)?;
                gwt_agent::Session::load(
                    &self
                        .sessions_dir
                        .join(format!("{}.toml", active.session_id)),
                )
                .ok()
                .map(|session| session.session_mode)
            })
            .unwrap_or(gwt_agent::SessionMode::Normal)
    }

    fn issue_monitor_failure_for_window(
        &self,
        window_id: &str,
        message: &str,
        session_mode: gwt_agent::SessionMode,
    ) -> Option<gwt::IssueMonitorFailure> {
        let failure = runtime_events::classify_issue_monitor_failure(message, session_mode)?;
        let gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: None,
        } = failure
        else {
            return Some(failure);
        };
        let source_session_id = self
            .active_agent_sessions
            .get(window_id)
            .map(|active| active.session_id.clone())
            .or_else(|| {
                let address = self.window_lookup.get(window_id)?;
                self.tab(&address.tab_id)?
                    .workspace
                    .window(&address.raw_id)?
                    .session_id
                    .clone()
            });
        let holder_window_id = source_session_id
            .as_deref()
            .and_then(|session_id| self.issue_monitor_session_by_id(session_id))
            .and_then(|candidate| {
                self.issue_monitor_native_conversation_holder_excluding(&candidate, Some(window_id))
            });
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict { holder_window_id })
    }

    #[cfg(test)]
    pub(crate) fn issue_monitor_agent_failed_payload(
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
        session_mode: gwt_agent::SessionMode,
    ) -> serde_json::Value {
        let failure = runtime_events::classify_issue_monitor_failure(message, session_mode);
        Self::issue_monitor_agent_failed_payload_with_failure(
            window_id,
            message,
            issue_number_hint,
            failure.as_ref(),
        )
    }

    fn issue_monitor_agent_failed_payload_with_failure(
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
        failure: Option<&gwt::IssueMonitorFailure>,
    ) -> serde_json::Value {
        let mut agent_failed = serde_json::json!({
            "window_id": window_id,
            "message": message,
        });
        if let Some(issue_number) = issue_number_hint {
            agent_failed["issue_number"] = serde_json::json!(issue_number);
        }
        if let Some(failure) = failure {
            agent_failed["failure"] =
                serde_json::to_value(failure).expect("Issue Monitor failure serializes");
        }
        serde_json::json!({ "agent_failed": agent_failed })
    }

    #[cfg(test)]
    fn issue_monitor_agent_failed_result_events(
        &mut self,
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let project_root = self
            .issue_monitor_project_root_for_window(window_id)
            .or_else(|| self.active_project_root().map(Path::to_path_buf));
        let Some(project_root) = project_root else {
            return self.issue_monitor_control_error_events(
                None,
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                    "no owning project is available for agent failure".to_string(),
                ),
                "agent-failed",
                issue_number_hint,
            );
        };
        self.issue_monitor_agent_failed_result_events_for_project(
            &project_root,
            window_id,
            message,
            issue_number_hint,
            self.issue_monitor_session_mode_for_window(window_id),
            publication,
        )
    }

    fn issue_monitor_agent_failed_result_events_for_project(
        &mut self,
        project_root: &Path,
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
        session_mode: gwt_agent::SessionMode,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let failure = self.issue_monitor_failure_for_window(window_id, message, session_mode);
        match publication {
            Ok(()) => self.finalize_issue_monitor_agent_failed_events(
                project_root,
                window_id,
                message,
                issue_number_hint,
                None,
                false,
            ),
            Err(error) if error.allows_local_fallback() => {
                let _scan_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                    std::time::Instant::now() + std::time::Duration::from_secs(60),
                );
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let expected_project_tab_id =
                    self.issue_monitor_tab_id_for_project_root(project_root);
                let loaded =
                    match gwt::issue_monitor_worker::github_remote_owner_and_repo(project_root) {
                        Ok((owner, repo)) => gwt::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path_with_provenance(
                            project_root,
                            &owner,
                            &repo,
                        ),
                        Err(error) => Err(error.to_string()),
                    };
                match self.commit_local_issue_monitor_control_for_project(
                    project_root,
                    |monitor| {
                        if failure.is_none() {
                            match loaded {
                                Ok(loaded) => {
                                    gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates_for_project_tab(
                                        monitor,
                                        &loaded,
                                        project_root,
                                        expected_project_tab_id.as_deref(),
                                        &now,
                                    );
                                }
                                Err(error) => monitor.record_scan_error(&now, error),
                            }
                        }
                        let commit = match &failure {
                            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                                holder_window_id,
                            }) => {
                                let issue_number = issue_number_hint
                                    .or_else(|| monitor.launched_window_issue(window_id));
                                let Some(issue_number) = issue_number else {
                                    return IssueMonitorFailureCommit::Rejected;
                                };
                                issue_monitor_writer_conflict_commit(
                                    monitor.try_requeue_agent_resume_writer_conflict(
                                        issue_number,
                                        window_id,
                                        message.to_string(),
                                        holder_window_id.as_deref(),
                                    ),
                                    issue_number,
                                )
                            }
                            None => {
                                let issue_number = if let Some(issue_number) = issue_number_hint {
                                    monitor.record_agent_issue_failed(
                                        issue_number,
                                        message.to_string(),
                                    );
                                    Some(issue_number)
                                } else {
                                    monitor.record_agent_window_failed(
                                        window_id,
                                        message.to_string(),
                                    )
                                };
                                IssueMonitorFailureCommit::Committed(issue_number)
                            }
                        };
                        if commit == IssueMonitorFailureCommit::Committed(None)
                            && failure.is_none()
                        {
                            monitor.record_scan_error(
                                &now,
                                format!(
                                    "agent window {window_id} failed but no monitored Issue mapping was found: {message}"
                                ),
                            );
                        }
                        commit
                    }
                ) {
                    Ok((monitor, IssueMonitorFailureCommit::Committed(issue_number))) => {
                        self.finalize_issue_monitor_agent_failed_events(
                            project_root,
                            window_id,
                            message,
                            issue_number,
                            Some(monitor),
                            true,
                        )
                    }
                    Ok((_monitor, IssueMonitorFailureCommit::Rejected)) => Vec::new(),
                    Ok((_monitor, IssueMonitorFailureCommit::AuthorityExhausted)) => self
                        .issue_monitor_control_error_events(
                            None,
                            gwt::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked,
                            "agent-failed",
                            issue_number_hint,
                        ),
                    Err(local_error) => self.issue_monitor_control_error_events(
                        None,
                        local_error,
                        "agent-failed",
                        issue_number_hint,
                    ),
                }
            }
            Err(error) => self.issue_monitor_control_error_events(
                None,
                error,
                "agent-failed",
                issue_number_hint,
            ),
        }
    }

    fn finalize_issue_monitor_agent_failed_events(
        &mut self,
        project_root: &Path,
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
        committed_monitor: Option<gwt::IssueMonitorState>,
        emit_local_snapshot: bool,
    ) -> Vec<OutboundEvent> {
        let monitor = committed_monitor.or_else(|| {
            let prefs = gwt::load_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(project_root),
            )
            .ok()?;
            Some(gwt::IssueMonitorState::with_prefs(
                gwt::IssueMonitorConfig::default(),
                prefs,
            ))
        });
        let prefs = monitor.as_ref().map(gwt::IssueMonitorState::prefs);
        let issue_number = issue_number_hint.or_else(|| {
            prefs.as_ref().and_then(|prefs| {
                prefs.failed_issues.iter().find_map(|failed| {
                    (failed.window_id.as_deref() == Some(window_id)).then_some(failed.issue_number)
                })
            })
        });
        let autoclose_failed_window = issue_number.is_some_and(|issue_number| {
            prefs.as_ref().is_some_and(|prefs| {
                prefs.autonomous_mode
                    && prefs
                        .autonomous_records
                        .iter()
                        .any(|record| record.issue_number == issue_number)
            })
        });
        self.pending_launch_feedback_contexts.remove(window_id);
        let mut events = if emit_local_snapshot {
            monitor
                .map(|monitor| {
                    self.issue_monitor_snapshot_events_for(None, Some(project_root), monitor)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        events.push(OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
            level: "error".to_string(),
            message: message.to_string(),
            issue_number,
        }));
        if autoclose_failed_window {
            events.extend(self.close_window_after_issue_monitor_finalize_events(window_id));
        }
        events
    }

    /// SPEC #3200 T-045/FR-025: a monitored autonomous agent showed liveness
    /// (a runtime status change). Best-effort refresh of the daemon's
    /// stuck-detection window for the mapped issue. No-op for non-monitor windows.
    /// SPEC-3431 FR-068: the last recorded activity for `window_id`.
    #[cfg(test)]
    pub(crate) fn last_agent_activity_for_test(
        &self,
        window_id: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        self.last_agent_activity.get(window_id).copied()
    }

    /// SPEC-3431 FR-068: minimum gap between heartbeat publications for one
    /// window. Hooks arrive per tool call, which is far more often than a
    /// stall check needs, and each publication is a daemon control round trip.
    const HEARTBEAT_THROTTLE_SECS: i64 = 60;

    pub(crate) fn issue_monitor_heartbeat(&mut self, project_root: &Path, window_id: &str) {
        // Record the observation before deciding whether to publish: activity
        // is a fact about the window, independent of whether this window is
        // currently bound to a monitored issue. Binding can be established
        // later (or lost), and a gap in the local clock would then read as a
        // stall that never happened.
        let now_instant = chrono::Utc::now();
        let recently_published = self.last_agent_activity.get(window_id).is_some_and(|last| {
            (now_instant - *last).num_seconds() < Self::HEARTBEAT_THROTTLE_SECS
        });
        self.last_agent_activity
            .insert(window_id.to_string(), now_instant);
        if recently_published {
            return;
        }
        let issue_number = self.issue_monitor_issue_number_for_window(project_root, window_id);
        let Some(issue_number) = issue_number else {
            return;
        };
        let now = now_instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if let Err(error) = self.publish_issue_monitor_control(
            project_root,
            serde_json::json!({
                "heartbeat": { "issue_number": issue_number, "at": now },
            }),
        ) {
            tracing::debug!(
                error = %error,
                window_id,
                "issue monitor heartbeat daemon publish failed (non-fatal)"
            );
        }
    }

    /// One or more agent windows were closed (single window close or whole
    /// project tab close). For any window that was an Issue Monitor launched
    /// window, return its Issue to pending (`Queued`) and free the active slot —
    /// never a fabricated completion. Cheaply gated so non-monitor window
    /// closes do not trigger a scan.
    pub(crate) fn issue_monitor_windows_closed_events(
        &mut self,
        project_root: &Path,
        window_ids: &[String],
    ) -> Vec<OutboundEvent> {
        let monitor_windows: Vec<String> = {
            let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
            match gwt::load_issue_monitor_prefs(&prefs_path) {
                Ok(prefs) => {
                    let monitor = gwt::IssueMonitorState::with_prefs(
                        gwt::IssueMonitorConfig::default(),
                        prefs,
                    );
                    window_ids
                        .iter()
                        .filter(|window_id| monitor.launched_window_issue(window_id).is_some())
                        .cloned()
                        .collect()
                }
                Err(_) => window_ids.to_vec(),
            }
        };
        if monitor_windows.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        for window_id in monitor_windows {
            let publication = self.publish_issue_monitor_control(
                project_root,
                serde_json::json!({ "window_closed": { "window_id": window_id } }),
            );
            events.extend(self.issue_monitor_window_closed_result_events_for_project(
                project_root,
                &window_id,
                publication,
            ));
        }
        events
    }

    #[cfg(test)]
    fn issue_monitor_window_closed_result_events(
        &mut self,
        window_id: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        let project_root = self
            .issue_monitor_project_root_for_window(window_id)
            .or_else(|| self.active_project_root().map(Path::to_path_buf));
        let Some(project_root) = project_root else {
            return self.issue_monitor_control_error_events(
                None,
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                    "no owning project is available for window close".to_string(),
                ),
                "window-closed",
                None,
            );
        };
        self.issue_monitor_window_closed_result_events_for_project(
            &project_root,
            window_id,
            publication,
        )
    }

    fn issue_monitor_window_closed_result_events_for_project(
        &mut self,
        project_root: &Path,
        window_id: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
    ) -> Vec<OutboundEvent> {
        match publication {
            Ok(()) => Vec::new(),
            Err(error) if error.allows_local_fallback() => {
                match self.commit_local_issue_monitor_control_for_project(project_root, |monitor| {
                    monitor.requeue_window(window_id);
                }) {
                    Ok((monitor, ())) => {
                        self.issue_monitor_snapshot_events_for(None, Some(project_root), monitor)
                    }
                    Err(local_error) => self.issue_monitor_control_error_events(
                        None,
                        local_error,
                        "window-closed",
                        None,
                    ),
                }
            }
            Err(error) => {
                self.issue_monitor_control_error_events(None, error, "window-closed", None)
            }
        }
    }

    fn local_issue_monitor_events_for(
        &mut self,
        client_id: Option<&str>,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        let policy = if cfg!(unix) {
            IssueMonitorScanPolicy::CacheOnly
        } else {
            IssueMonitorScanPolicy::Scan
        };
        self.local_issue_monitor_events_with_policy(client_id, policy, apply)
    }

    fn issue_monitor_control_result_events(
        &mut self,
        client_id: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
        operation: &'static str,
        apply_fallback: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        let project_root = self.active_project_root().map(Path::to_path_buf);
        match publication {
            // The daemon ACK confirms the canonical control transaction
            // committed. Persisting it again here would advance authority
            // twice and leave the daemon's generation stale.
            Ok(()) => Vec::new(),
            Err(error) if error.allows_local_fallback() => {
                tracing::debug!(
                    error = %error,
                    operation,
                    "issue monitor control daemon publish failed; using local fallback"
                );
                match self.commit_local_issue_monitor_control(apply_fallback) {
                    Ok((monitor, ())) => self.issue_monitor_snapshot_events_for(
                        Some(client_id),
                        project_root.as_deref(),
                        monitor,
                    ),
                    Err(local_error) => self.issue_monitor_control_error_events(
                        Some(client_id),
                        local_error,
                        operation,
                        None,
                    ),
                }
            }
            Err(error) => {
                self.issue_monitor_control_error_events(Some(client_id), error, operation, None)
            }
        }
    }

    fn issue_monitor_authorizing_control_result_events(
        &mut self,
        client_id: &str,
        publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
        operation: &'static str,
        apply_fallback: impl FnOnce(&mut gwt::IssueMonitorState) -> Result<(), String>,
    ) -> Vec<OutboundEvent> {
        let project_root = self.active_project_root().map(Path::to_path_buf);
        match publication {
            Ok(()) => Vec::new(),
            Err(error) if error.allows_local_fallback() => {
                match self.commit_local_issue_monitor_authorizing_control(apply_fallback) {
                    Ok((monitor, ())) => self.issue_monitor_snapshot_events_for(
                        Some(client_id),
                        project_root.as_deref(),
                        monitor,
                    ),
                    Err(local_error) => self.issue_monitor_control_error_events(
                        Some(client_id),
                        local_error,
                        operation,
                        None,
                    ),
                }
            }
            Err(error) => {
                self.issue_monitor_control_error_events(Some(client_id), error, operation, None)
            }
        }
    }

    fn commit_local_issue_monitor_control<T>(
        &self,
        mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> T,
    ) -> Result<
        (gwt::IssueMonitorState, T),
        gwt::runtime_daemon_events::IssueMonitorControlPublishError,
    > {
        let project_root = self
            .active_project_root()
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(
                    "local fallback control commit failed: no active project".to_string(),
                )
            })?;
        self.commit_local_issue_monitor_control_for_project(&project_root, mutation)
    }

    fn commit_local_issue_monitor_control_for_project<T>(
        &self,
        project_root: &Path,
        mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> T,
    ) -> Result<
        (gwt::IssueMonitorState, T),
        gwt::runtime_daemon_events::IssueMonitorControlPublishError,
    > {
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let (cached_issues, projection_error, now) =
            Self::load_local_issue_monitor_fallback_projection(project_root);
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + std::time::Duration::from_millis(250),
        );
        gwt::try_mutate_issue_monitor_prefs_without_authority_fence(&prefs_path, |prefs| {
            let mut monitor = gwt::IssueMonitorState::with_prefs(
                gwt::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            let result = mutation(&mut monitor);
            Self::apply_local_issue_monitor_fallback_projection(
                &mut monitor,
                &cached_issues,
                projection_error.as_deref(),
                &now,
            );
            *prefs = monitor.prefs();
            Ok((monitor, result))
        })
        .map(|(_, committed)| {
            #[cfg(test)]
            LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS
                .set(LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.get() + 1);
            committed
        })
        .map_err(|error| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(format!(
                "local fallback control commit failed: {error}"
            ))
        })
    }

    fn commit_local_issue_monitor_authorizing_control<T>(
        &self,
        mutation: impl FnOnce(&mut gwt::IssueMonitorState) -> Result<T, String>,
    ) -> Result<
        (gwt::IssueMonitorState, T),
        gwt::runtime_daemon_events::IssueMonitorControlPublishError,
    > {
        let project_root = self.active_project_root().ok_or_else(|| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(
                "local fallback control commit failed: no active project".to_string(),
            )
        })?;
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let (cached_issues, projection_error, now) =
            Self::load_local_issue_monitor_fallback_projection(project_root);
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + std::time::Duration::from_millis(250),
        );
        gwt::try_mutate_issue_monitor_prefs_without_authority_fence(&prefs_path, |prefs| {
            let mut monitor = gwt::IssueMonitorState::with_prefs(
                gwt::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            let result = mutation(&mut monitor).map_err(std::io::Error::other)?;
            Self::apply_local_issue_monitor_fallback_projection(
                &mut monitor,
                &cached_issues,
                projection_error.as_deref(),
                &now,
            );
            *prefs = monitor.prefs();
            Ok((monitor, result))
        })
        .map(|(_, committed)| committed)
        .map_err(|error| {
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(format!(
                "local fallback control commit failed: {error}"
            ))
        })
    }

    fn load_local_issue_monitor_fallback_projection(
        project_root: &Path,
    ) -> (Vec<gwt::IssueMonitorIssue>, Option<String>, String) {
        let _cache_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + local_issue_monitor_fallback_projection_timeout(),
        );
        let cache_root = gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
        let (cached_issues, cache_error) =
            match gwt::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root) {
                Ok(issues) => (issues, None),
                Err(error) => (Vec::new(), Some(format!("issue cache failed: {error}"))),
            };
        let origin_error = gwt::issue_monitor_worker::github_remote_owner_and_repo(project_root)
            .err()
            .map(|error| error.to_string());
        (
            cached_issues,
            cache_error.or(origin_error),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
    }

    fn apply_local_issue_monitor_fallback_projection(
        monitor: &mut gwt::IssueMonitorState,
        cached_issues: &[gwt::IssueMonitorIssue],
        projection_error: Option<&str>,
        now: &str,
    ) {
        monitor.expire_stale_unbound_launches(now);
        gwt::scan_issue_monitor_candidates(monitor, cached_issues, now);
        if monitor.config.enabled {
            monitor.set_gui_connected(true);
        }
        if let Some(error) = projection_error {
            monitor.record_scan_error(now, error);
        }
    }

    fn issue_monitor_control_error_events(
        &self,
        client_id: Option<&str>,
        error: gwt::runtime_daemon_events::IssueMonitorControlPublishError,
        operation: &'static str,
        issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        tracing::warn!(
            error = %error,
            operation,
            "issue monitor control outcome is rejected or uncertain; local fallback denied"
        );
        let project_root = self.active_project_root().map(Path::to_path_buf);
        let prefs = project_root
            .as_deref()
            .map(gwt::issue_monitor_prefs_path_for_repo_path)
            .and_then(|path| gwt::load_issue_monitor_prefs(&path).ok())
            .unwrap_or_else(gwt::IssueMonitorPrefs::recovery_default);
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        let message = error.to_string();
        monitor.record_control_commit_error(message.clone());
        let mut events = self.issue_monitor_snapshot_events_without_wake(
            client_id,
            project_root.as_deref(),
            monitor,
        );
        let toast = BackendEvent::IssueMonitorToast {
            level: "error".to_string(),
            message,
            issue_number,
        };
        events.push(match client_id {
            Some(client_id) => OutboundEvent::reply(client_id, toast),
            None => OutboundEvent::broadcast(toast),
        });
        events
    }

    fn quick_register_issue_events(
        &mut self,
        client_id: &str,
        title: String,
        launch: bool,
    ) -> Vec<OutboundEvent> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: "Issue title is required".to_string(),
                    issue_number: None,
                },
            )];
        }

        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: "Open a project before registering an Issue".to_string(),
                    issue_number: None,
                },
            )];
        };
        let (owner, repo) =
            match gwt::issue_monitor_worker::github_remote_owner_and_repo(&project_root) {
                Ok(value) => value,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::IssueMonitorToast {
                            level: "error".to_string(),
                            message: format!("GitHub origin remote is unavailable: {error}"),
                            issue_number: None,
                        },
                    )];
                }
            };

        let client = match (self.issue_client_factory)(&owner, &repo) {
            Ok(client) => client,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::IssueMonitorToast {
                        level: "error".to_string(),
                        message: issue_registration_failure_message(&error),
                        issue_number: None,
                    },
                )];
            }
        };

        let labels: Vec<String> = Vec::new();
        let body = quick_issue_body(&title);
        let snapshot = match client.create_issue(&title, &body, &labels) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::IssueMonitorToast {
                        level: "error".to_string(),
                        message: issue_registration_failure_message(&error),
                        issue_number: None,
                    },
                )];
            }
        };

        let cache_root = gwt::issue_cache::issue_cache_root_for_repo_path(&project_root)
            .unwrap_or_else(|| gwt::issue_cache::issue_cache_root_for_repo_slug(&owner, &repo));
        let mut events = Vec::new();
        match gwt_github::Cache::new(cache_root.clone()).write_snapshot(&snapshot) {
            Ok(()) => {}
            Err(error) => events.push(OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: format!(
                        "Issue #{} registered, but local cache update failed: {error}",
                        snapshot.number.0
                    ),
                    issue_number: Some(snapshot.number.0),
                },
            )),
        }

        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::IssueMonitorToast {
                level: "info".to_string(),
                message: "Issue registered".to_string(),
                issue_number: Some(snapshot.number.0),
            },
        ));
        events.extend(self.quick_issue_monitor_snapshot_events(
            Some(client_id),
            &project_root,
            &cache_root,
            &snapshot,
        ));
        if launch {
            events.extend(self.open_issue_monitor_launch_wizard_events(
                client_id,
                snapshot.number.0,
                gwt::LinkedIssueKind::Issue,
            ));
        }
        events
    }

    fn quick_issue_monitor_snapshot_events(
        &self,
        client_id: Option<&str>,
        project_root: &Path,
        cache_root: &Path,
        snapshot: &gwt_github::IssueSnapshot,
    ) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap_or_else(|_| gwt::IssueMonitorPrefs::recovery_default());
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        let mut issues =
            gwt::issue_monitor_worker::load_cached_issue_monitor_candidates(cache_root)
                .unwrap_or_default();
        if !issues.iter().any(|issue| issue.number == snapshot.number.0) {
            issues.push(issue_monitor_issue_from_snapshot(snapshot));
        }
        gwt::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
        // Cache-backed quick view: a read model, not monitor-driven activity —
        // it must not feed (or reset) the PM wake fingerprint.
        self.replace_knowledge_monitor_snapshot(project_root, &monitor.inbox);
        self.issue_monitor_snapshot_events_without_wake(client_id, Some(project_root), monitor)
    }

    fn local_issue_monitor_events_with_policy(
        &mut self,
        client_id: Option<&str>,
        policy: IssueMonitorScanPolicy,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
            monitor.record_scan_error(now, "No active project");
            return self.issue_monitor_snapshot_events_for(client_id, None, monitor);
        };
        self.local_issue_monitor_events_with_policy_for_project(
            client_id,
            &project_root,
            policy,
            apply,
        )
    }

    fn local_issue_monitor_events_with_policy_for_project(
        &mut self,
        client_id: Option<&str>,
        project_root: &Path,
        policy: IssueMonitorScanPolicy,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let project_root = project_root.to_path_buf();
        let expected_project_tab_id = self.issue_monitor_tab_id_for_project_root(&project_root);
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
        let (mut monitor, ()) =
            load_mutate_and_persist_issue_monitor_state(&prefs_path, |monitor| {
                apply(monitor);
                // #3223 follow-up: release claimed-but-never-acked launches whose
                // claim anchor exceeded claim_ttl_secs so a crash cannot leak a slot.
                monitor.expire_stale_unbound_launches(&now);
            });
        if policy == IssueMonitorScanPolicy::CacheOnly {
            let (cached_issues, cache_error, origin_error) = {
                // Remote resolution performs `git rev-parse` followed by
                // `git remote get-url`. Give both commands one shared,
                // still-UI-bounded budget; 250ms could expire between them on
                // a loaded machine and misreport a configured origin as an
                // unstartable process.
                let _cache_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                    std::time::Instant::now() + std::time::Duration::from_secs(1),
                );
                let cache_root =
                    gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(&project_root);
                let (cached_issues, cache_error) =
                    match gwt::issue_monitor_worker::load_cached_issue_monitor_candidates(
                        &cache_root,
                    ) {
                        Ok(issues) => (issues, None),
                        Err(error) => (Vec::new(), Some(format!("issue cache failed: {error}"))),
                    };
                let origin_error =
                    gwt::issue_monitor_worker::github_remote_owner_and_repo(&project_root)
                        .err()
                        .map(|error| error.to_string());
                (cached_issues, cache_error, origin_error)
            };
            rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut monitor, |monitor| {
                gwt::scan_issue_monitor_candidates(monitor, &cached_issues, &now);
                if monitor.config.enabled {
                    monitor.set_gui_connected(true);
                }
                if let Some(error) = cache_error.as_deref().or(origin_error.as_deref()) {
                    monitor.record_scan_error(&now, error);
                }
            });
            return self.issue_monitor_snapshot_events_for(client_id, Some(&project_root), monitor);
        }
        #[cfg(test)]
        LOCAL_ISSUE_MONITOR_REMOTE_SCANS
            .set(LOCAL_ISSUE_MONITOR_REMOTE_SCANS.get().saturating_add(1));
        let _scan_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        );
        let mut loaded_for_commit = None;
        let mut merge_reconciliation_error = None;
        #[cfg(not(unix))]
        let mut local_repo_identity = None;
        #[cfg(not(unix))]
        let mut local_claim_proposal = None;

        match gwt::issue_monitor_worker::github_remote_owner_and_repo(&project_root) {
            Ok((owner, repo)) => {
                #[cfg(not(unix))]
                {
                    // Existing Attempting/safety journal entries reconcile by
                    // exact remote readback even when the candidate list is
                    // unavailable or stale. Only *new* claim proposals below
                    // require authoritative Live provenance.
                    local_repo_identity = Some((owner.clone(), repo.clone()));
                }
                match gwt::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path_with_provenance(
                    &project_root,
                    &owner,
                    &repo,
                ) {
                    Ok(loaded) => {
                        gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates_for_project_tab(
                            &mut monitor,
                            &loaded,
                            &project_root,
                            expected_project_tab_id.as_deref(),
                            &now,
                        );
                        merge_reconciliation_error =
                            gwt::issue_monitor_worker::reconcile_issue_monitor_merges(
                                &mut monitor,
                                &project_root,
                            )
                            .err()
                            .map(|error| {
                                format!("issue monitor merge reconciliation failed: {error}")
                            });
                        if monitor.config.enabled {
                            monitor.set_gui_connected(true);
                        }
                        #[cfg(not(unix))]
                        if loaded.authorizes_remote_effects() {
                            let completed_issues = loaded
                                .issues
                                .iter()
                                .filter_map(|issue| {
                                    gwt::issue_monitor_worker::issue_completed_by_merged_pr(
                                        &owner,
                                        &repo,
                                        issue,
                                    )
                                    .then_some(issue.number)
                                })
                                .collect();
                            local_claim_proposal = Some((
                                format!("{}:{}", whoami::username(), std::process::id()),
                                completed_issues,
                            ));
                        }
                        loaded_for_commit = Some(loaded);
                    }
                    Err(error) => {
                        monitor
                            .record_scan_error(now.as_str(), format!("issue list failed: {error}"));
                    }
                }
            }
            Err(error) => {
                monitor.record_scan_error(now.as_str(), error.to_string());
            }
        }

        // Persist the refreshed read model only. Remote-effect proposals are
        // produced by the daemon scan and executed only after its durable
        // Prepared -> Attempting fence.
        rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut monitor, |monitor| {
            if let Some(loaded) = &loaded_for_commit {
                gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates_for_project_tab(
                    monitor,
                    loaded,
                    &project_root,
                    expected_project_tab_id.as_deref(),
                    &now,
                );
                #[cfg(not(unix))]
                if let Some((monitor_owner, completed_issues)) = &local_claim_proposal {
                    prepare_local_issue_monitor_claim_proposals(
                        monitor,
                        loaded,
                        monitor_owner,
                        &now,
                        completed_issues,
                    );
                }
            }
            record_issue_monitor_scan_failures(
                monitor,
                now.as_str(),
                merge_reconciliation_error,
                Vec::new(),
            );
        });
        #[cfg(not(unix))]
        let mut launch_events = local_repo_identity
            .map(|(owner, repo)| {
                self.drive_local_issue_monitor_claim_effects(
                    &prefs_path,
                    &owner,
                    &repo,
                    &project_root,
                    &mut monitor,
                )
            })
            .unwrap_or_default();
        #[cfg(unix)]
        let mut launch_events = Vec::new();
        launch_events.extend(self.issue_monitor_snapshot_events_for(
            client_id,
            Some(&project_root),
            monitor,
        ));
        launch_events
    }

    /// Windows compatibility driver. Until named-pipe daemon support exists,
    /// the GUI owns a synchronous single-flight claim executor, but it follows
    /// the same durable two-transaction authority protocol as the Unix daemon:
    /// proposal commit -> Attempting fence commit -> remote call -> exact result
    /// commit. It never executes arm/disarm effects; autonomous delivery remains
    /// daemon-only on platforms that provide the runtime daemon.
    #[cfg_attr(unix, allow(dead_code))]
    fn drive_local_issue_monitor_claim_effects(
        &mut self,
        prefs_path: &Path,
        owner: &str,
        repo: &str,
        project_root: &Path,
        monitor: &mut gwt::IssueMonitorState,
    ) -> Vec<OutboundEvent> {
        let _local_lease = match gwt::try_acquire_issue_monitor_local_fallback_lease(prefs_path) {
            Ok(lease) => lease,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tracing::debug!(
                    prefs_path = %prefs_path.display(),
                    "local Issue Monitor claim execution deferred to the current authority"
                );
                return Vec::new();
            }
            Err(error) => {
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                monitor.record_scan_error(
                    now,
                    format!("Issue Monitor local authority acquisition failed: {error}"),
                );
                tracing::warn!(%error, "local Issue Monitor authority acquisition failed");
                return Vec::new();
            }
        };
        let execution = execute_local_issue_monitor_claim_effects(
            prefs_path,
            owner,
            repo,
            monitor,
            &self.issue_client_factory,
        );
        if let Err(error) = execution {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            if error.contains("deadline") {
                monitor.record_scan_error(now, error.clone());
            } else {
                monitor.record_launch_auth_required(now);
            }
            tracing::warn!(error = %error, "local issue monitor claim execution unavailable");
        }

        let mut events = Vec::new();
        for request in monitor.take_pending_launch_requests() {
            events.extend(self.auto_launch_issue_monitor_delivery_events_for_project(
                project_root,
                request.issue_number,
                request.linked_issue_kind,
                request.delivery_id.clone(),
                request.launch_session_strategy,
            ));
        }
        events
    }

    /// Issue #3505: enqueue one non-blocking, authority-fenced scheduled scan
    /// for each enabled canonical project scope. GitHub and claim I/O stays
    /// off tao's event loop, while the in-flight set drops duplicate ticks.
    pub(crate) fn issue_monitor_scheduled_tick_events(&mut self) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.issue_monitor_scheduled_tick_events_at(&now)
    }

    pub(crate) fn issue_monitor_scheduled_tick_events_at(
        &mut self,
        now: &str,
    ) -> Vec<OutboundEvent> {
        let mut seen_prefs_paths = HashSet::new();
        let projects: Vec<(PathBuf, PathBuf, String)> = self
            .tabs
            .iter()
            .filter(|tab| tab.kind == gwt::ProjectKind::Git && !tab.migration_pending)
            .filter_map(|tab| {
                let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&tab.project_root);
                seen_prefs_paths
                    .insert(prefs_path.clone())
                    .then(|| (tab.project_root.clone(), prefs_path, tab.id.clone()))
            })
            .collect();
        let mut events = Vec::new();
        for (project_root, prefs_path, expected_project_tab_id) in projects {
            let enabled_or_cleanup = gwt::load_issue_monitor_prefs(&prefs_path)
                .map(|prefs| prefs.enabled || issue_monitor_prefs_need_local_claim_cleanup(&prefs))
                .unwrap_or(false);
            if !enabled_or_cleanup {
                continue;
            }
            match self.enqueue_issue_monitor_scan_worker(
                &project_root,
                &prefs_path,
                &expected_project_tab_id,
                now,
            ) {
                Ok(()) | Err(IssueMonitorScanEnqueueError::AlreadyInFlight) => {}
                Err(IssueMonitorScanEnqueueError::WorkerUnavailable(error)) => {
                    events.push(OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                        level: "error".to_string(),
                        message: format!("Issue Monitor scheduled worker could not start: {error}"),
                        issue_number: None,
                    }));
                }
            }
        }
        events
    }

    fn enqueue_issue_monitor_scan_worker(
        &mut self,
        project_root: &Path,
        prefs_path: &Path,
        expected_project_tab_id: &str,
        now: &str,
    ) -> Result<(), IssueMonitorScanEnqueueError> {
        if !self
            .issue_monitor_scheduled_scans_in_flight
            .insert(prefs_path.to_path_buf())
        {
            return Err(IssueMonitorScanEnqueueError::AlreadyInFlight);
        }
        let proxy = self.proxy.clone();
        let worker_project_root = project_root.to_path_buf();
        let worker_prefs_path = prefs_path.to_path_buf();
        let worker_expected_project_tab_id = expected_project_tab_id.to_string();
        let worker_now = now.to_string();
        let issue_client_factory = self.issue_client_factory.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_scheduled_issue_monitor_scan(
                    &worker_project_root,
                    Some(&worker_expected_project_tab_id),
                    &worker_now,
                    &issue_client_factory,
                )
            }))
            .unwrap_or_else(|panic| {
                let detail = panic
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                Err(format!("Issue Monitor scheduled worker panicked: {detail}"))
            });
            proxy.send(UserEvent::IssueMonitorScheduledScanComplete {
                project_root: worker_project_root,
                prefs_path: worker_prefs_path,
                now: worker_now,
                outcome: result,
            });
        });
        if let Err(error) = spawn {
            self.issue_monitor_scheduled_scans_in_flight
                .remove(prefs_path);
            tracing::error!(%error, "failed to spawn Issue Monitor scheduled worker");
            return Err(IssueMonitorScanEnqueueError::WorkerUnavailable(error));
        }
        Ok(())
    }

    fn authenticated_issue_monitor_scan_now_events(
        &mut self,
        client_id: ClientId,
        principal: &AgentSessionPrincipal,
        expected_project_scope: &str,
    ) -> Vec<OutboundEvent> {
        let reply = |accepted: bool, reason: Option<&str>| {
            vec![OutboundEvent::reply(
                client_id.clone(),
                BackendEvent::IssueMonitorScanRequestResult {
                    accepted,
                    reason: reason.map(str::to_string),
                },
            )]
        };
        let project_root = principal.canonical_project_root().to_path_buf();
        let principal_scope = gwt_core::paths::project_scope_hash(&project_root);
        if !constant_time_issue_monitor_scope_eq(expected_project_scope, principal_scope.as_str()) {
            return reply(false, Some("project_scope_mismatch"));
        }
        let pm_prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let registered_pm = gwt::pm_registry::load_pm_prefs(&pm_prefs_path)
            .ok()
            .and_then(|prefs| prefs.registration)
            .filter(|registration| registration.session_id == principal.session_id())
            .is_some_and(|registration| self.pm_registration_is_live(&registration));
        if !registered_pm {
            return reply(false, Some("caller_not_registered_pm"));
        }

        let Some((worker_project_root, prefs_path, expected_project_tab_id)) = self
            .tabs
            .iter()
            .find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && principal.authorizes_project_root(&tab.project_root)
            })
            .map(|tab| {
                (
                    tab.project_root.clone(),
                    gwt::issue_monitor_prefs_path_for_repo_path(&tab.project_root),
                    tab.id.clone(),
                )
            })
        else {
            return reply(false, Some("project_not_open"));
        };
        match gwt::load_issue_monitor_prefs(&prefs_path) {
            Ok(prefs) if prefs.enabled => {}
            Ok(_) => return reply(false, Some("monitor_disabled")),
            Err(error) => {
                tracing::warn!(%error, "Issue Monitor scan request could not load prefs");
                return reply(false, Some("monitor_state_unavailable"));
            }
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        match self.enqueue_issue_monitor_scan_worker(
            &worker_project_root,
            &prefs_path,
            &expected_project_tab_id,
            &now,
        ) {
            Ok(()) => reply(true, None),
            Err(error) => reply(false, Some(error.reason())),
        }
    }

    pub(crate) fn issue_monitor_scheduled_scan_complete_events(
        &mut self,
        _worker_project_root: &Path,
        prefs_path: &Path,
        now: &str,
        outcome: Result<ScheduledIssueMonitorScanOutcome, String>,
    ) -> Vec<OutboundEvent> {
        if !self
            .issue_monitor_scheduled_scans_in_flight
            .remove(prefs_path)
        {
            return Vec::new();
        }
        let Some(project_root) = self.tabs.iter().find_map(|tab| {
            (tab.kind == gwt::ProjectKind::Git
                && !tab.migration_pending
                && gwt::issue_monitor_prefs_path_for_repo_path(&tab.project_root) == prefs_path)
                .then(|| tab.project_root.clone())
        }) else {
            return Vec::new();
        };
        let mut monitor = match outcome {
            Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon) => {
                return self.pm_periodic_wake_events_at(&project_root, now);
            }
            Ok(ScheduledIssueMonitorScanOutcome::Applied(monitor)) => *monitor,
            Err(error) => {
                tracing::error!(%error, "Issue Monitor scheduled worker failed");
                let mut events = vec![OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: error,
                    issue_number: None,
                })];
                events.extend(self.pm_periodic_wake_events_at(&project_root, now));
                return events;
            }
        };
        let latest = match gwt::load_issue_monitor_prefs(prefs_path) {
            Ok(prefs) if prefs.enabled => prefs,
            Ok(_) => return Vec::new(),
            Err(error) => {
                tracing::error!(%error, "Issue Monitor scheduled completion could not reload prefs");
                let mut events = vec![OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                    level: "error".to_string(),
                    message: format!("Issue Monitor scheduled completion failed: {error}"),
                    issue_number: None,
                })];
                events.extend(self.pm_periodic_wake_events_for_monitor_at(
                    &project_root,
                    &monitor,
                    now,
                ));
                return events;
            }
        };
        // The worker carries the ephemeral live queue/inbox, while disk owns
        // all concurrent controls and durable delivery state. Rebase combines
        // both before any UI projection or materialization decision.
        monitor.rebase_gui_observer_prefs(&latest);

        let mut events = Vec::new();
        for request in monitor.take_pending_launch_requests() {
            events.extend(self.auto_launch_issue_monitor_delivery_events_for_project(
                &project_root,
                request.issue_number,
                request.linked_issue_kind,
                request.delivery_id,
                request.launch_session_strategy,
            ));
        }
        if let Ok(latest) = gwt::load_issue_monitor_prefs(prefs_path) {
            monitor.rebase_gui_observer_prefs(&latest);
        }
        events.extend(self.issue_monitor_snapshot_events_for(
            None,
            Some(&project_root),
            monitor.clone(),
        ));
        events.extend(self.pm_periodic_wake_events_for_monitor_at(&project_root, &monitor, now));
        events
    }

    fn issue_monitor_snapshot_events_for(
        &mut self,
        client_id: Option<&str>,
        project_root: Option<&Path>,
        monitor: gwt::IssueMonitorState,
    ) -> Vec<OutboundEvent> {
        // SPEC-3431 T-093 (FR-012): every real local snapshot also feeds the
        // PM wake path, before the active-tab filter — the resident PM watches
        // its project regardless of which tab the user is looking at.
        // Synthetic snapshots (the control-error fallback) must instead use
        // [`Self::issue_monitor_snapshot_events_without_wake`]: their empty
        // inbox would reset the wake baseline and replay old rows as news.
        let mut events = Vec::new();
        if let Some(project_root) = project_root {
            self.replace_knowledge_monitor_snapshot(project_root, &monitor.inbox);
            events.extend(self.pm_wake_events(project_root, &monitor.inbox));
        }
        events.extend(self.issue_monitor_snapshot_events_without_wake(
            client_id,
            project_root,
            monitor,
        ));
        events
    }

    fn issue_monitor_snapshot_events_without_wake(
        &self,
        client_id: Option<&str>,
        project_root: Option<&Path>,
        monitor: gwt::IssueMonitorState,
    ) -> Vec<OutboundEvent> {
        if project_root.is_some_and(|project_root| {
            self.active_project_root()
                .is_none_or(|active_root| !same_worktree_path(active_root, project_root))
        }) {
            return Vec::new();
        }
        let mut status = monitor.status_view();
        self.apply_issue_monitor_launch_profile_status(&mut status, project_root);
        let status_event = BackendEvent::IssueMonitorStatus { status };
        let inbox_event = BackendEvent::IssueMonitorInbox {
            items: monitor.inbox,
        };
        match client_id {
            Some(client_id) => vec![
                OutboundEvent::reply(client_id.to_string(), status_event),
                OutboundEvent::reply(client_id.to_string(), inbox_event),
            ],
            None => vec![
                OutboundEvent::broadcast(status_event),
                OutboundEvent::broadcast(inbox_event),
            ],
        }
    }

    fn apply_issue_monitor_launch_profile_status(
        &self,
        status: &mut gwt::IssueMonitorStatusView,
        project_root: Option<&Path>,
    ) {
        if status.launch_profile_source == gwt::IssueMonitorLaunchProfileSource::Saved {
            return;
        }
        let previous_profiles = project_root
            .map(|project_root| self.issue_monitor_previous_profiles(project_root))
            .unwrap_or_else(|| self.launch_wizard_cache.agent_preferences());
        if let Some(profile) = previous_profiles.preferred_profile() {
            status.launch_profile_source = gwt::IssueMonitorLaunchProfileSource::LastSettings;
            status.launch_profile_summary = gwt::issue_monitor_launch_profile_summary(profile);
            if status.state == "settings_required" {
                status.state = "idle".to_string();
            }
        } else if status.state == "settings_required" {
            status.launch_profile_summary = "configure before auto start".to_string();
        }
    }

    pub(crate) fn register_agent_backend_connection_probe(
        &mut self,
        client_id: &str,
        agent: gwt_agent::BuiltinAgentId,
    ) -> u64 {
        if self.agent_backend_probe_generation == u64::MAX {
            self.agent_backend_probe_generation = 0;
            self.agent_backend_latest_probe_generations.clear();
        }
        self.agent_backend_probe_generation += 1;
        let generation = self.agent_backend_probe_generation;
        self.agent_backend_latest_probe_generations
            .insert((client_id.to_string(), agent), generation);
        generation
    }

    fn spawn_agent_backend_connection_probe(
        &mut self,
        client_id: ClientId,
        agent: gwt_agent::BuiltinAgentId,
        base_url: String,
        api_key: String,
    ) {
        let generation = self.register_agent_backend_connection_probe(&client_id, agent);
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event =
                gwt::agent_backend_dispatch::test_connection_event(agent, &base_url, &api_key);
            proxy.send(UserEvent::AgentBackendConnectionProbeComplete {
                client_id,
                agent,
                generation,
                event,
            });
        });
    }

    pub(crate) fn handle_agent_backend_connection_probe_complete(
        &mut self,
        client_id: ClientId,
        agent: gwt_agent::BuiltinAgentId,
        generation: u64,
        event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        let key = (client_id.clone(), agent);
        if self
            .agent_backend_latest_probe_generations
            .get(&key)
            .copied()
            != Some(generation)
        {
            return Vec::new();
        }
        self.agent_backend_latest_probe_generations.remove(&key);
        vec![OutboundEvent::reply(client_id, event)]
    }

    pub(crate) fn handle_frontend_event(
        &mut self,
        client_id: ClientId,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        log_frontend_user_action(&client_id, &event);
        match event {
            FrontendEvent::FrontendReady => {
                // SPEC-2970: kick an immediate usage poll on connect so the
                // status-bar pill populates right away instead of waiting for
                // the next 30s poller tick (otherwise a freshly loaded page
                // shows an empty usage cell).
                if let Some(refresh) = &self.usage_refresh {
                    refresh.notify_one();
                }
                self.frontend_sync_events(&client_id)
            }
            FrontendEvent::SetClaudeAccountUsageEnabled { enabled } => {
                self.set_claude_account_usage_enabled_events(enabled)
            }
            FrontendEvent::RefreshUsage => self.request_usage_refresh_events(),
            FrontendEvent::StartupAutoResumeReady { bounds } => {
                self.startup_auto_resume_ready_events(bounds)
            }
            // SPEC-3431 FR-018/FR-019: one click always lands the user on the
            // PM — existing pane gets framed, a missing one is started first.
            FrontendEvent::OpenPmAgent { bounds } => {
                let Some(tab_id) = self.active_tab_id.clone() else {
                    return Vec::new();
                };
                self.ensure_pm_agent_for_tab_with_bounds(
                    &tab_id,
                    bounds,
                    pm::PmEnsureTrigger::Explicit,
                )
            }
            // SPEC-3431 FR-026: PM settings. The two writes never touch the
            // running pane; only the explicit restart does.
            FrontendEvent::SetPmAutoStart { enabled } => self.set_pm_auto_start_events(enabled),
            FrontendEvent::SetPmLaunchProfile {
                agent_id,
                model,
                reasoning,
            } => self.set_pm_launch_profile_events(&agent_id, model, reasoning),
            FrontendEvent::RestartPmAgent => self.restart_pm_agent_events(),
            FrontendEvent::OpenProjectDialog => self.open_project_dialog_events(),
            FrontendEvent::SelectCloneProjectParent => {
                self.select_clone_project_parent_events(&client_id)
            }
            FrontendEvent::GithubRepositorySearch { query } => {
                self.github_repository_search_events(&client_id, &query)
            }
            FrontendEvent::CloneProjectStart { url, parent_path } => {
                self.clone_project_start_events(&client_id, &url, &parent_path)
            }
            FrontendEvent::ReopenRecentProject { path } => {
                self.open_project_path_events(PathBuf::from(path))
            }
            FrontendEvent::SelectProjectTab { tab_id } => self.select_project_tab_events(&tab_id),
            FrontendEvent::CloseProjectTab { tab_id } => self.close_project_tab_events(&tab_id),
            FrontendEvent::CreateWindow { preset, bounds } => {
                self.create_window_events(preset, bounds)
            }
            FrontendEvent::LoadProcessConsole { id } => {
                // SPEC-2809 Phase F2 — Console window mount asks for the
                // current ring buffer. Use the global hub installed by
                // `gwt_core::logging::init`. Reply to the requesting
                // client only so other Consoles do not see duplicates.
                let hub = gwt_core::process_console::global();
                vec![OutboundEvent::reply(
                    client_id.clone(),
                    BackendEvent::ProcessConsoleSnapshot {
                        id,
                        lines: hub.snapshot_all(),
                    },
                )]
            }
            FrontendEvent::FocusWindow { id, bounds } => self.focus_window_events(&id, bounds),
            FrontendEvent::CycleFocus { direction, bounds } => {
                self.cycle_focus_events(direction, bounds)
            }
            FrontendEvent::UpdateViewport { viewport } => self.update_viewport_events(viewport),
            FrontendEvent::ArrangeWindows { mode, bounds } => {
                self.arrange_windows_events(mode, bounds)
            }
            FrontendEvent::DockWindowTab { id, target_id } => {
                self.dock_window_tab_events(&id, &target_id)
            }
            FrontendEvent::ActivateWindowTab { id } => self.activate_window_tab_events(&id),
            FrontendEvent::DetachWindowTab { id, geometry } => {
                self.detach_window_tab_events(&id, geometry)
            }
            FrontendEvent::PlaceAgentWindowInKanban {
                id,
                board_id,
                lane_id,
                order,
            } => self.place_agent_window_in_kanban_events(&id, &board_id, lane_id, order),
            FrontendEvent::MoveAgentKanbanCard {
                id,
                board_id,
                lane_id,
                order,
            } => self.move_agent_kanban_card_events(&id, &board_id, lane_id, order),
            FrontendEvent::UndockAgentWindow { id, geometry } => {
                self.undock_agent_window_events(&id, geometry)
            }
            FrontendEvent::SetAgentKanbanCardCollapsed { id, collapsed } => {
                self.set_agent_kanban_card_collapsed_events(&id, collapsed)
            }
            FrontendEvent::UpdateTerminalGrid { id, cols, rows } => {
                self.update_terminal_grid_events(&id, cols, rows)
            }
            FrontendEvent::ListWindows => {
                vec![OutboundEvent::reply(client_id, self.list_windows_event())]
            }
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry,
                cols,
                rows,
                base_geometry_revision,
            } => self.update_window_geometry_events(
                &id,
                geometry,
                cols,
                rows,
                base_geometry_revision,
            ),
            FrontendEvent::CloseWindow { id, .. } => self.close_window_events(&id),
            FrontendEvent::StopWindow { id } => self.stop_window_events(&id),
            FrontendEvent::StopAllWindows {} => self.stop_all_windows_events(),
            FrontendEvent::RestartWindow { id } => self.restart_window_events(&id),
            FrontendEvent::TerminalInput { id, data } => self.terminal_input_events(&id, &data),
            FrontendEvent::PaneSendInput { session_id, text } => {
                self.pane_send_input_events(client_id, &session_id, &text)
            }
            FrontendEvent::PmPaneSendInput {
                operation_id,
                window_id,
                ..
            } => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PmMessageSendResult {
                    operation_id,
                    status: "failed".to_string(),
                    window_id: Some(window_id),
                    reason: Some(
                        "pm.message.send requires an authenticated agent WebSocket principal"
                            .to_string(),
                    ),
                },
            )],
            FrontendEvent::PasteImage {
                id,
                data_base64,
                mime_type,
                filename,
            } => self.paste_image_events(&id, &data_base64, &mime_type, filename.as_deref()),
            FrontendEvent::PasteImageUploaded {
                id,
                operation_id,
                upload_id,
                mime_type,
                filename,
                size,
            } => {
                if operation_id.is_some() {
                    self.paste_image_uploaded_operation_events(
                        client_id,
                        id,
                        operation_id,
                        UploadedImagePasteOperation {
                            upload_id,
                            mime_type,
                            filename,
                            size,
                        },
                    )
                } else {
                    self.paste_image_uploaded_events(
                        &id,
                        &upload_id,
                        &mime_type,
                        filename.as_deref(),
                        size,
                    )
                }
            }
            FrontendEvent::AttachFiles {
                id,
                operation_id,
                files,
            } => {
                if operation_id.is_some() {
                    self.attach_files_operation_events(client_id, id, operation_id, files)
                } else {
                    self.attach_files_events(&id, files)
                }
            }
            FrontendEvent::LoadFileTree { id, path } => {
                let path = path.unwrap_or_default();
                vec![OutboundEvent::reply(
                    client_id,
                    self.load_file_tree_event(&id, &path),
                )]
            }
            FrontendEvent::ListFileTreeWorktrees { id } => vec![OutboundEvent::reply(
                client_id,
                self.list_file_tree_worktrees_event(&id),
            )],
            FrontendEvent::SelectFileTreeWorktree { id, worktree_id } => {
                vec![OutboundEvent::reply(
                    client_id,
                    self.select_file_tree_worktree_event(&id, &worktree_id),
                )]
            }
            FrontendEvent::LoadFileContent {
                id,
                path,
                mode,
                hex_offset,
                hex_length,
            } => vec![OutboundEvent::reply(
                client_id,
                self.load_file_content_event(&id, &path, mode, hex_offset, hex_length),
            )],
            FrontendEvent::SaveFileContent {
                id,
                path,
                mode,
                expected_mtime,
                expected_size,
                text,
                encoding,
                newline,
                has_bom,
                hex_offset,
                hex_byte,
            } => vec![OutboundEvent::reply(
                client_id,
                self.save_file_content_event(
                    &id,
                    &path,
                    mode,
                    expected_mtime,
                    expected_size,
                    text,
                    encoding,
                    newline,
                    has_bom,
                    hex_offset,
                    hex_byte,
                ),
            )],
            FrontendEvent::LoadBranches { id } => self.load_branches_events(&client_id, &id),
            FrontendEvent::RequestRemoteStartWorkBranches { id } => {
                self.request_remote_start_work_branches_events(&client_id, &id)
            }
            FrontendEvent::LoadBoard { id, all } => self.load_board_events(&client_id, &id, all),
            FrontendEvent::LoadBoardHistory {
                id,
                before_entry_id,
                limit,
                all,
            } => self.load_board_history_events(
                &client_id,
                &id,
                before_entry_id.as_deref(),
                limit,
                all,
            ),
            FrontendEvent::LoadProfile { id } => self.load_profile_events(&client_id, &id),
            FrontendEvent::LoadLogs { id } => self.load_logs_events(&client_id, &id),
            FrontendEvent::LoadKnowledgeBridge {
                id,
                knowledge_kind,
                request_id,
                selected_number,
                refresh,
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number,
                    refresh,
                },
            ),
            FrontendEvent::SearchKnowledgeBridge {
                id,
                knowledge_kind,
                query,
                request_id,
                selected_number,
            } => self.search_knowledge_bridge_events(
                &client_id,
                KnowledgeSearchRequest {
                    id: &id,
                    kind: knowledge_kind,
                    query: &query,
                    request_id,
                    selected_number,
                },
            ),
            FrontendEvent::SearchProjectIndex {
                id,
                query,
                request_id,
                scopes,
                worktree_hash,
                match_mode,
            } => self.search_project_index_events(
                &client_id,
                ProjectIndexSearchRequest {
                    id: &id,
                    query: &query,
                    request_id,
                    scopes,
                    worktree_hash,
                    match_mode,
                },
            ),
            FrontendEvent::RequestWorkAdvisory {
                id,
                query,
                request_id,
            } => self.request_work_advisory_events(&client_id, &id, &query, request_id),
            FrontendEvent::SelectKnowledgeBridgeEntry {
                id,
                knowledge_kind,
                request_id,
                number,
            } => match knowledge_kind {
                KnowledgeKind::Issue | KnowledgeKind::Spec => self
                    .select_knowledge_bridge_entry_events(
                        &client_id,
                        &id,
                        knowledge_kind,
                        request_id,
                        number,
                    ),
                // FR-102 intentionally covers only Issue/SPEC. Keep the PR
                // surface on its pre-existing full-load selection path.
                KnowledgeKind::Pr => self.load_knowledge_bridge_events(
                    &client_id,
                    KnowledgeLoadRequest {
                        id: &id,
                        kind: knowledge_kind,
                        request_id,
                        selected_number: Some(number),
                        refresh: false,
                    },
                ),
            },
            FrontendEvent::UpdateKnowledgeBridgePhase {
                id,
                request_id,
                issue_number,
                target_phase,
            } => self.update_knowledge_bridge_phase_events(
                &client_id,
                &id,
                request_id,
                issue_number,
                target_phase.as_deref(),
            ),
            FrontendEvent::RunBranchCleanup {
                id,
                branches,
                delete_remote,
                force_filesystem_delete,
            } => self.run_branch_cleanup_events(
                &client_id,
                &id,
                &branches,
                delete_remote,
                force_filesystem_delete,
            ),
            FrontendEvent::RunWorkspaceCleanup {
                branch,
                delete_remote,
                force_filesystem_delete,
            } => self.run_workspace_cleanup_events(
                &client_id,
                &branch,
                delete_remote,
                force_filesystem_delete,
            ),
            FrontendEvent::RebuildIndexCell {
                project_root,
                scope,
                worktree_hash,
            } => self.rebuild_index_cell_events(project_root, scope, worktree_hash),
            FrontendEvent::RefreshIndexStatus { project_root } => {
                self.refresh_index_status_events(project_root)
            }
            FrontendEvent::PostBoardEntry {
                id,
                entry_kind,
                body,
                title,
                parent_id,
                topics,
                owners,
                targets,
                mentions,
                target_workspace,
                broadcast,
            } => self.post_board_entry_events(
                &client_id,
                BoardPostRequest {
                    id,
                    entry_kind,
                    body,
                    title,
                    parent_id,
                    topics,
                    owners,
                    targets,
                    mentions,
                    target_workspace,
                    broadcast,
                },
            ),
            FrontendEvent::OpenBoardOriginAgent {
                id,
                origin_session_id,
                bounds,
            } => self.open_board_origin_agent_events(&client_id, &id, &origin_session_id, bounds),
            FrontendEvent::SelectProfile { id, profile_name } => {
                self.select_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::CreateProfile { id, name } => {
                self.create_profile_events(&client_id, &id, &name)
            }
            FrontendEvent::SetActiveProfile { id, profile_name } => {
                self.set_active_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::SaveProfile {
                id,
                current_name,
                name,
                description,
                env_vars,
                disabled_env,
            } => self.save_profile_events(
                &client_id,
                &id,
                ProfileSaveRequest {
                    current_name,
                    name,
                    description,
                    env_vars,
                    disabled_env,
                },
            ),
            FrontendEvent::DeleteProfile { id, profile_name } => {
                self.delete_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
                self.open_issue_launch_wizard_events(&client_id, &id, issue_number)
            }
            FrontendEvent::OpenIntakeSession => self.open_intake_session(&client_id),
            FrontendEvent::OpenStartWorkInAgentKanban { board_id, lane_id } => {
                self.open_start_work_in_agent_kanban(&client_id, &board_id, lane_id)
            }
            FrontendEvent::OpenAgentKanbanLaunchWizard { board_id, lane_id } => {
                self.open_agent_kanban_launch_wizard(&client_id, &board_id, lane_id)
            }
            FrontendEvent::ResumeWorkspace { source, journal_id } => {
                self.resume_workspace_events(&client_id, source, journal_id)
            }
            FrontendEvent::ListResumableAgents {
                operation_id,
                workspace_id,
            } => self.list_resumable_agents_events(&client_id, operation_id, workspace_id),
            FrontendEvent::ResumeWorkspaceAgent {
                operation_id,
                session_id,
                agent_session_id,
                bounds,
            } => self.resume_workspace_agent_events(
                &client_id,
                operation_id,
                session_id,
                agent_session_id,
                bounds,
            ),
            FrontendEvent::ContinueWork {
                operation_id,
                work_id,
                bounds,
            } => self.continue_work_events(&client_id, operation_id, work_id, bounds),
            FrontendEvent::ResumeBranchLatestAgent {
                id,
                branch_name,
                bounds,
            } => self.resume_branch_latest_agent_events(&client_id, &id, &branch_name, bounds),
            FrontendEvent::OpenLaunchWizard {
                id,
                branch_name,
                linked_issue_number,
            } => self.open_launch_wizard(&client_id, &id, &branch_name, linked_issue_number),
            FrontendEvent::OpenActiveWorkLaunchWizard {
                branch_name,
                linked_issue_number,
            } => self.open_active_work_launch_wizard(&client_id, &branch_name, linked_issue_number),
            FrontendEvent::LaunchWizardAction { action, bounds } => {
                self.handle_launch_wizard_action_for_client(Some(&client_id), action, bounds)
            }
            FrontendEvent::SetIssueMonitorEnabled { enabled } => {
                if enabled {
                    let has_saved_profile = self
                        .active_project_root()
                        .map(|project_root| {
                            let prefs_path =
                                gwt::issue_monitor_prefs_path_for_repo_path(project_root);
                            gwt::load_issue_monitor_prefs(&prefs_path)
                                .ok()
                                .and_then(|prefs| prefs.launch_profile)
                                .is_some()
                        })
                        .unwrap_or(false);
                    if !has_saved_profile {
                        let project_root = self.active_project_root().map(Path::to_path_buf);
                        let mut events =
                            self.open_issue_monitor_configure_profile_wizard_events(&client_id);
                        if let Some(project_root) = project_root {
                            let prefs_path =
                                gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
                            let prefs = gwt::load_issue_monitor_prefs(&prefs_path)
                                .unwrap_or_else(|_| gwt::IssueMonitorPrefs::recovery_default());
                            let monitor = gwt::IssueMonitorState::with_prefs(
                                gwt::IssueMonitorConfig::default(),
                                prefs,
                            );
                            let mut status = monitor.status_view();
                            self.apply_issue_monitor_launch_profile_status(
                                &mut status,
                                Some(project_root.as_path()),
                            );
                            events.push(OutboundEvent::reply(
                                client_id.clone(),
                                BackendEvent::IssueMonitorStatus { status },
                            ));
                        }
                        return events;
                    }
                }
                let publication = self.publish_active_issue_monitor_control(
                    serde_json::json!({ "enabled": enabled }),
                );
                self.issue_monitor_authorizing_control_result_events(
                    &client_id,
                    publication,
                    "enabled",
                    |monitor| {
                        monitor
                            .set_enabled_with_effect_revocation(enabled)
                            .ok_or_else(|| "authority epoch exhausted".to_string())?;
                        Ok(())
                    },
                )
            }
            FrontendEvent::SetIssueMonitorAutonomousMode { enabled } => {
                let publication = self.publish_active_issue_monitor_control(
                    serde_json::json!({ "autonomous_mode": enabled }),
                );
                self.issue_monitor_authorizing_control_result_events(
                    &client_id,
                    publication,
                    "autonomous-mode",
                    |monitor| {
                        monitor
                            .set_autonomous_mode_with_effect_revocation(enabled)
                            .ok_or_else(|| "authority epoch exhausted".to_string())?;
                        Ok(())
                    },
                )
            }
            FrontendEvent::SetIssueMonitorMaxActiveAgents { max_active_agents } => {
                let publication = self.publish_active_issue_monitor_control(
                    serde_json::json!({ "max_active_agents": max_active_agents }),
                );
                self.issue_monitor_control_result_events(
                    &client_id,
                    publication,
                    "max-active",
                    |monitor| {
                        monitor.set_max_active_agents(max_active_agents);
                    },
                )
            }
            FrontendEvent::ReorderIssueMonitorIssues { issue_numbers } => {
                let priority_order = issue_numbers;
                let publication = self.publish_active_issue_monitor_control(
                    serde_json::json!({ "priority_order": priority_order.clone() }),
                );
                self.issue_monitor_control_result_events(
                    &client_id,
                    publication,
                    "reorder",
                    |monitor| {
                        monitor.reorder_queued_issues(&priority_order);
                    },
                )
            }
            FrontendEvent::ListIssueMonitor => self.local_issue_monitor_events_with_policy(
                Some(&client_id),
                IssueMonitorScanPolicy::Scan,
                |_| {},
            ),
            // Internal agent-listener command. Browser-scoped requests are
            // deliberately inert; the authenticated route below owns it.
            FrontendEvent::AgentIssueMonitorScanNow { .. } => Vec::new(),
            FrontendEvent::QuickRegisterIssue { title, launch } => {
                self.quick_register_issue_events(&client_id, title, launch)
            }
            FrontendEvent::IssueMonitorLaunchNow {
                issue_number,
                linked_issue_kind,
            } => self.open_issue_monitor_launch_wizard_events(
                &client_id,
                issue_number,
                linked_issue_kind.unwrap_or(gwt::LinkedIssueKind::Issue),
            ),
            FrontendEvent::IssueMonitorConfigureIssue {
                issue_number,
                linked_issue_kind,
            } => self.open_issue_monitor_configure_wizard_events(
                &client_id,
                issue_number,
                linked_issue_kind.unwrap_or(gwt::LinkedIssueKind::Issue),
            ),
            FrontendEvent::IssueMonitorConfigureProfile => {
                self.open_issue_monitor_configure_profile_wizard_events(&client_id)
            }
            FrontendEvent::ApplyUpdate => self.apply_pending_update_events(&client_id),
            FrontendEvent::ApplyUpdateStart => self.apply_update_start_events(&client_id),
            FrontendEvent::ApplyUpdateToVersion { version } => {
                self.apply_update_to_version_events(&client_id, version)
            }
            FrontendEvent::CloseWork {
                work_id,
                close_kind,
            } => self.close_work(&work_id, &close_kind),
            FrontendEvent::ImprovementPromoteIssue { id } => {
                self.improvement_promote_issue_events(&client_id, &id)
            }
            FrontendEvent::ImprovementResolve {
                id,
                expected_resolver_revision,
            } => self.improvement_resolve_events(
                &client_id,
                &id,
                expected_resolver_revision.as_deref(),
            ),
            FrontendEvent::ImprovementSelectOwner {
                id,
                owner_number,
                resolver_revision,
            } => self.improvement_select_owner_events(
                &client_id,
                &id,
                owner_number,
                &resolver_revision,
            ),
            FrontendEvent::ImprovementDismiss { id, reason } => {
                self.improvement_dismiss_events(&client_id, &id, reason.as_deref())
            }
            FrontendEvent::CancelUpdateDownload => self.cancel_update_download_events(&client_id),
            FrontendEvent::ApplyUpdateLater => self.apply_update_later_events(&client_id),
            FrontendEvent::ApplyUpdateRestartNow => {
                self.apply_update_restart_now_events(&client_id)
            }
            FrontendEvent::OpenUpdateLog { log_path } => {
                self.open_update_log_events(&client_id, log_path)
            }
            FrontendEvent::OpenServerUrl { url } => self.open_server_url_events(&client_id, url),
            FrontendEvent::ListCustomAgents => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::list_event(),
            )],
            FrontendEvent::ListCustomAgentPresets => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::list_presets_event(),
            )],
            FrontendEvent::AddCustomAgentFromPreset { input } => {
                let event = gwt::custom_agents_dispatch::add_from_preset_event(
                    gwt::PresetId::ClaudeCodeOpenaiCompat,
                    serde_json::to_value(input)
                        .expect("custom agent preset payload should serialize"),
                );
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::UpdateCustomAgent { agent } => {
                let event = gwt::custom_agents_dispatch::update_event(*agent);
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::DeleteCustomAgent { agent_id } => {
                let event = gwt::custom_agents_dispatch::delete_event(agent_id);
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::TestBackendConnection { base_url, api_key } => {
                self.spawn_backend_connection_probe(client_id, base_url, api_key);
                Vec::new()
            }
            FrontendEvent::ListAgentBackends { agent } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::list_event(agent),
            )],
            FrontendEvent::AddAgentBackend { agent, profile } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::add_event(agent, *profile),
            )],
            FrontendEvent::UpdateAgentBackend { agent, id, profile } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::update_event(agent, id, *profile),
            )],
            FrontendEvent::DeleteAgentBackend { agent, id } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::delete_event(agent, id),
            )],
            FrontendEvent::TestAgentBackendConnection {
                agent,
                base_url,
                api_key,
            } => {
                self.spawn_agent_backend_connection_probe(client_id, agent, base_url, api_key);
                Vec::new()
            }
            FrontendEvent::StartMigration { tab_id } => self.start_migration_events(&tab_id),
            FrontendEvent::SkipMigration { tab_id } => self.skip_migration_events(&tab_id),
            FrontendEvent::QuitMigration { tab_id } => self.quit_migration_events(&tab_id),
            FrontendEvent::GetSystemSettings => self.system_settings_get_events(client_id),
            FrontendEvent::GetBoardAuthStatus => self.board_auth_status_events(client_id, None),
            FrontendEvent::BoardProviderSignIn { provider } => {
                self.board_provider_sign_in_events(client_id, &provider)
            }
            FrontendEvent::BoardProviderSignOut { provider } => {
                self.board_provider_sign_out_events(client_id, &provider)
            }
            FrontendEvent::UpdateBoardProviderConfig {
                provider,
                client_id: provider_client_id,
                default_channel,
                tenant_id,
                client_secret,
            } => self.board_provider_config_update_events(
                client_id,
                &provider,
                provider_client_id,
                default_channel,
                tenant_id,
                client_secret,
            ),
            FrontendEvent::UpdateBoardOauthPort { port } => {
                self.board_oauth_port_update_events(client_id, port)
            }
            FrontendEvent::GetProjectBoardConfig { project_root } => {
                self.project_board_config_events(client_id, project_root)
            }
            FrontendEvent::UpdateProjectBoardConfig {
                project_root,
                provider,
                channel,
                tenant,
            } => self.project_board_config_update_events(
                client_id,
                project_root,
                provider,
                channel,
                tenant,
            ),
            FrontendEvent::UpdateSystemSettings {
                language,
                codex_trust_managed_hooks,
                board_provider,
            } => self.system_settings_update_events(
                client_id,
                language,
                codex_trust_managed_hooks,
                board_provider,
            ),
            FrontendEvent::GetAutostartStatus => self.autostart_status_events(client_id),
            FrontendEvent::UpdateAutostart { enabled } => {
                self.autostart_update_events(client_id, enabled)
            }
            FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
                self.workspace_projection_prune_events(client_id, dry_run, ids)
            }
            FrontendEvent::SaveUiTrace { trace } => self.save_ui_trace_events(client_id, trace),
            FrontendEvent::OpenReleaseNotes { id, focus_version } => {
                self.release_notes_events(client_id, id, focus_version)
            }
        }
    }

    /// Handle the deliberately small protocol exposed to a gwt-launched
    /// agent. The authenticated server-side principal, not any WebSocket
    /// payload field, remains authoritative for both project and Session.
    pub(crate) fn handle_agent_frontend_event_if_current(
        &mut self,
        client_id: ClientId,
        grant: AgentCapabilityGrant,
        request: AgentFrontendRequest,
    ) -> AgentFrontendDispatchOutcome {
        let Some(issuer) = self.agent_capability_issuer.clone() else {
            return AgentFrontendDispatchOutcome::StaleCapability;
        };
        if !issuer.grant_is_current(&grant) {
            tracing::warn!(
                target: "gwt_security",
                "queued agent pane request rejected after capability rotation or revoke"
            );
            return AgentFrontendDispatchOutcome::StaleCapability;
        }
        if request.requires_producing_authority()
            && !grant.principal().authorizes_producing_mutation()
        {
            tracing::warn!(
                target: "gwt_security",
                "observation-only agent pane request rejected before runtime mutation"
            );
            return AgentFrontendDispatchOutcome::StaleCapability;
        }
        let durable_mutation =
            grant.principal().authorizes_producing_mutation() && request.mutates_host_state();
        if durable_mutation {
            match issuer.durable_authority(&grant) {
                AgentDurableAuthority::Current => {}
                AgentDurableAuthority::Stale | AgentDurableAuthority::ObservationOnly => {
                    tracing::warn!(
                        target: "gwt_security",
                        "queued agent pane request rejected by durable execution binding fence"
                    );
                    return AgentFrontendDispatchOutcome::StaleCapability;
                }
                AgentDurableAuthority::Unavailable => {
                    tracing::warn!(
                        target: "gwt_security",
                        "queued agent pane request rejected because execution authority is unavailable"
                    );
                    return AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable;
                }
            }
            #[cfg(test)]
            run_agent_dispatch_test_hook(&AGENT_AFTER_DURABLE_CHECK_TEST_HOOK);
            if request.requires_producing_authority() {
                let binding = grant
                    .principal()
                    .active_execution_binding()
                    .expect("producing principal carries an active binding")
                    .clone();
                return match gwt::cli::execution_state::with_current_active_execution_binding_lease_wait(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &binding,
                    std::time::Duration::ZERO,
                    || {
                        #[cfg(test)]
                        run_agent_dispatch_test_hook(&AGENT_LEASED_MUTATION_TEST_HOOK);
                        self.dispatch_agent_frontend_event_with_current_grant(
                            &issuer, client_id, grant, request,
                        )
                    },
                ) {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => {
                        tracing::warn!(
                            target: "gwt_security",
                            "queued agent pane request rejected after leased execution authority changed"
                        );
                        AgentFrontendDispatchOutcome::StaleCapability
                    }
                    Err(_) => {
                        tracing::warn!(
                            target: "gwt_security",
                            "queued agent pane request rejected because leased execution authority is unavailable"
                        );
                        AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable
                    }
                };
            }
        }

        self.dispatch_agent_frontend_event_with_current_grant(&issuer, client_id, grant, request)
    }

    fn dispatch_agent_frontend_event_with_current_grant(
        &mut self,
        issuer: &AgentCapabilityIssuer,
        client_id: ClientId,
        grant: AgentCapabilityGrant,
        request: AgentFrontendRequest,
    ) -> AgentFrontendDispatchOutcome {
        match request {
            AgentFrontendRequest::CloseWindow {
                id,
                request_id: Some(request_id),
                responder: Some(responder),
            } => AgentFrontendDispatchOutcome::Dispatched(
                self.accept_agent_self_close(grant, id, request_id, responder),
            ),
            AgentFrontendRequest::CloseWindow {
                request_id: Some(_),
                ..
            }
            | AgentFrontendRequest::CloseWindow {
                responder: Some(_), ..
            } => {
                tracing::warn!(
                    target: "gwt_security",
                    "correlated agent self-close rejected without an origin response channel"
                );
                AgentFrontendDispatchOutcome::Dispatched(Vec::new())
            }
            AgentFrontendRequest::PmSendInput {
                operation_id,
                window_id,
                text,
                responder,
            } => AgentFrontendDispatchOutcome::Dispatched(
                self.authenticated_pm_pane_send_input_events(
                    issuer,
                    client_id,
                    grant,
                    &operation_id,
                    &window_id,
                    &text,
                    responder,
                ),
            ),
            request => {
                let principal = grant.principal().clone();
                issuer
                    .with_current_grant(&grant, || {
                        self.handle_agent_frontend_event(client_id, principal, request)
                    })
                    .map_or(AgentFrontendDispatchOutcome::StaleCapability, |events| {
                        AgentFrontendDispatchOutcome::Dispatched(events)
                    })
            }
        }
    }

    pub(crate) fn handle_agent_frontend_event(
        &mut self,
        client_id: ClientId,
        principal: AgentSessionPrincipal,
        request: AgentFrontendRequest,
    ) -> Vec<OutboundEvent> {
        match request {
            AgentFrontendRequest::Ready => self.agent_frontend_sync_events(&client_id, &principal),
            AgentFrontendRequest::CloseWindow {
                id,
                request_id,
                responder,
            } => {
                if request_id.is_some() || responder.is_some() {
                    tracing::warn!(
                        target: "gwt_security",
                        "correlated agent self-close bypassed generation-aware dispatch"
                    );
                    return Vec::new();
                }
                if !self.agent_principal_authorizes_window(&principal, &id) {
                    tracing::warn!(
                        target: "gwt_security",
                        "cross-project pane lifecycle request denied"
                    );
                    return Vec::new();
                }
                let closes_own_session = self
                    .window_lookup
                    .get(&id)
                    .and_then(|address| self.tab(&address.tab_id).map(|tab| (tab, address)))
                    .and_then(|(tab, address)| tab.workspace.window(&address.raw_id))
                    .and_then(|window| window.session_id.as_deref())
                    == Some(principal.session_id());
                if closes_own_session {
                    tracing::warn!(
                        target: "gwt_security",
                        "uncorrelated agent self-close rejected"
                    );
                    return Vec::new();
                }
                self.close_window_events(&id)
            }
            AgentFrontendRequest::SendInput { text } => {
                let window_id = match self.agent_principal_session_window(&principal) {
                    Ok(window_id) => window_id,
                    Err(error) => {
                        return vec![OutboundEvent::reply(
                            client_id,
                            BackendEvent::PaneSendResult {
                                ok: false,
                                window_id: None,
                                error: Some(error),
                            },
                        )];
                    }
                };
                self.pane_send_input_to_window_events(client_id, &window_id, &text)
            }
            AgentFrontendRequest::PmSendInput { .. } => {
                tracing::warn!(
                    target: "gwt_security",
                    "privileged PM send bypassed generation-aware dispatch"
                );
                Vec::new()
            }
            AgentFrontendRequest::IssueMonitorScanNow {
                expected_project_scope,
            } => self.authenticated_issue_monitor_scan_now_events(
                client_id,
                &principal,
                &expected_project_scope,
            ),
        }
    }

    fn accept_agent_self_close(
        &mut self,
        grant: AgentCapabilityGrant,
        requested_window_id: String,
        request_id: String,
        responder: AgentSelfCloseResponder,
    ) -> Vec<OutboundEvent> {
        let canonical_request_id = Uuid::parse_str(&request_id)
            .ok()
            .is_some_and(|parsed| parsed.hyphenated().to_string() == request_id);
        if !canonical_request_id {
            tracing::warn!(
                target: "gwt_security",
                "agent self-close rejected an invalid correlation id"
            );
            return Vec::new();
        }

        let own_window_id = match self.agent_principal_session_window(grant.principal()) {
            Ok(window_id) => window_id,
            Err(error) => {
                tracing::warn!(
                    target: "gwt_security",
                    reason = %error,
                    "agent self-close could not resolve one exact Session window"
                );
                return Vec::new();
            }
        };
        if requested_window_id != own_window_id {
            tracing::warn!(
                target: "gwt_security",
                "correlated agent self-close targeted a peer or foreign window"
            );
            return Vec::new();
        }

        let Some(address) = self.window_lookup.get(&own_window_id).cloned() else {
            return Vec::new();
        };
        let captured_session_id = self
            .tab(&address.tab_id)
            .and_then(|tab| tab.workspace.window(&address.raw_id))
            .and_then(|window| window.session_id.clone());
        if captured_session_id.as_deref() != Some(grant.principal().session_id()) {
            tracing::warn!(
                target: "gwt_security",
                "agent self-close Session changed before acceptance"
            );
            return Vec::new();
        }
        let Some(issuer) = self.agent_capability_issuer.clone() else {
            return Vec::new();
        };
        let Some(ticket) = issuer.begin_self_close_if_current(&grant) else {
            tracing::warn!(
                target: "gwt_security",
                "agent self-close capability changed before acceptance"
            );
            return Vec::new();
        };

        let pending = PendingAgentSelfClose {
            ticket: ticket.clone(),
            window_id: own_window_id.clone(),
            address,
            session_id: captured_session_id.expect("validated Session id"),
        };
        self.pending_agent_self_closes
            .insert(ticket.id().to_string(), pending);
        let acceptance = AgentSelfCloseDirectAcceptance::new(
            request_id,
            own_window_id,
            ticket,
            self.proxy.clone(),
        );
        if let Err(acceptance) = responder.send(acceptance) {
            let ticket = acceptance.disarm();
            self.pending_agent_self_closes.remove(ticket.id());
            issuer.rollback_self_close(&ticket);
        }
        Vec::new()
    }

    pub(crate) fn commit_agent_self_close(
        &mut self,
        ticket: AgentSelfCloseCapabilityTicket,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_agent_self_closes.remove(ticket.id()) else {
            return Vec::new();
        };
        let still_captured_window =
            self.window_lookup
                .get(&pending.window_id)
                .is_some_and(|current| {
                    current.tab_id == pending.address.tab_id
                        && current.raw_id == pending.address.raw_id
                })
                && self
                    .tab(&pending.address.tab_id)
                    .and_then(|tab| tab.workspace.window(&pending.address.raw_id))
                    .and_then(|window| window.session_id.as_deref())
                    == Some(pending.session_id.as_str());

        let events = if still_captured_window {
            self.close_window_events(&pending.window_id)
        } else {
            Vec::new()
        };
        if let Some(issuer) = self.agent_capability_issuer.as_ref() {
            issuer.finish_self_close(&pending.ticket);
        }
        events
    }

    pub(crate) fn has_pending_agent_self_closes(&self) -> bool {
        !self.pending_agent_self_closes.is_empty()
    }

    fn agent_principal_authorizes_window(
        &self,
        principal: &AgentSessionPrincipal,
        window_id: &str,
    ) -> bool {
        self.window_lookup
            .get(window_id)
            .and_then(|address| self.tab(&address.tab_id))
            .is_some_and(|tab| principal.authorizes_project_root(&tab.project_root))
    }

    /// Resolve the authenticated Session only inside its capability project.
    /// More than one match in that project fails closed instead of restoring
    /// the old process-global first-match behavior.
    fn agent_principal_session_window(
        &self,
        principal: &AgentSessionPrincipal,
    ) -> Result<String, String> {
        let mut matches = self
            .tabs
            .iter()
            .filter(|tab| principal.authorizes_project_root(&tab.project_root))
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| window.session_id.as_deref() == Some(principal.session_id()))
                    .map(|window| combined_window_id(&tab.id, &window.id))
            });
        let Some(window_id) = matches.next() else {
            return Err("authenticated session is not bound to this project".to_string());
        };
        if matches.next().is_some() {
            return Err("authenticated session has multiple panes in this project".to_string());
        }
        Ok(window_id)
    }

    fn agent_frontend_sync_events(
        &self,
        client_id: &str,
        principal: &AgentSessionPrincipal,
    ) -> Vec<OutboundEvent> {
        let mut workspace = self.app_state_view();
        workspace
            .tabs
            .retain(|tab| principal.authorizes_project_root(Path::new(&tab.project_root)));
        workspace.active_tab_id = workspace
            .active_tab_id
            .filter(|active| workspace.tabs.iter().any(|tab| &tab.id == active))
            .or_else(|| workspace.tabs.first().map(|tab| tab.id.clone()));
        // Recent-project history is process-global and is not required by
        // pane.list/read. Never expose it on the agent capability route.
        workspace.recent_projects.clear();

        let allowed_window_ids = self
            .window_lookup
            .iter()
            .filter_map(|(window_id, address)| {
                self.tab(&address.tab_id)
                    .filter(|tab| principal.authorizes_project_root(&tab.project_root))
                    .map(|_| window_id.clone())
            })
            .collect::<std::collections::HashSet<_>>();
        let mut terminal_snapshots = self
            .runtimes
            .iter()
            .filter(|(id, _)| allowed_window_ids.contains(*id))
            .filter_map(|(id, runtime)| {
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.snapshot_bytes())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then_some((id.clone(), snapshot))
            })
            .collect::<Vec<_>>();
        let runtime_snapshot_ids = terminal_snapshots
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        for (id, detail) in &self.launch_error_terminal_details {
            if allowed_window_ids.contains(id)
                && !runtime_snapshot_ids.contains(id)
                && self.window_status(id) == Some(WindowProcessStatus::Error)
            {
                terminal_snapshots.push((id.clone(), Self::launch_error_terminal_bytes(detail)));
            }
        }

        let mut events = vec![OutboundEvent::reply(
            client_id,
            BackendEvent::WindowCanvasState { workspace },
        )];
        events.extend(terminal_snapshots.into_iter().map(|(id, snapshot)| {
            OutboundEvent::reply(
                client_id,
                BackendEvent::TerminalSnapshot {
                    id,
                    data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                },
            )
        }));
        events
    }

    pub(crate) fn frontend_sync_events(&mut self, client_id: &str) -> Vec<OutboundEvent> {
        let terminal_statuses = self
            .window_details
            .iter()
            .filter_map(|(id, detail)| {
                self.window_status(id)
                    .map(|status| (id.clone(), status, detail.clone()))
            })
            .collect();
        let mut terminal_snapshots = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                // SPEC-1919 FR-001a / SPEC-2008 Phase 26.F: snapshot replay
                // must preserve the current formatted screen and enough
                // scrollback history for a fresh xterm.js instance to scroll
                // immediately after reconnect.
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.snapshot_bytes())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then_some((id.clone(), snapshot))
            })
            .collect::<Vec<_>>();
        let runtime_snapshot_ids = terminal_snapshots
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        for (id, detail) in &self.launch_error_terminal_details {
            if !runtime_snapshot_ids.contains(id)
                && self.window_status(id) == Some(WindowProcessStatus::Error)
            {
                terminal_snapshots.push((id.clone(), Self::launch_error_terminal_bytes(detail)));
            }
        }

        let mut events = build_frontend_sync_events(
            client_id,
            self.app_state_view(),
            terminal_statuses,
            terminal_snapshots,
            self.launch_wizard
                .as_ref()
                .map(|wizard| wizard.wizard.view()),
            self.pending_update.clone(),
        );
        if let Some(event) = self.active_work_projection_reply(client_id) {
            events.insert(1, event);
        }
        // SPEC-3431 FR-026: hydrate the PM settings panel on connect. Without
        // this a freshly loaded page shows the panel's built-in defaults until
        // some unrelated PM transition happens to broadcast.
        if let Some(event) = self.pm_status_event() {
            events.push(OutboundEvent::reply(client_id.to_string(), event));
        }
        self.schedule_active_improvement_candidates_refresh();
        // SPEC-1934 US-6.1: surface pending migrations to a newly-connected
        // frontend during state hydration so the modal opens without waiting
        // for another roundtrip.
        events.extend(self.migration_detected_replies(client_id));
        events.extend(self.migration_recovery_replies(client_id));
        events
    }
}

impl AppRuntime {
    pub(crate) fn app_state_view(&self) -> gwt::AppStateView {
        gwt::AppStateView {
            app_version: crate::runtime_support::current_app_version().to_string(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    let workspace = self.workspace_view_for_tab(tab);
                    let running_agents =
                        crate::runtime_support::collect_running_agents(&workspace.windows);
                    gwt::ProjectTabView {
                        id: tab.id.clone(),
                        title: tab.title.clone(),
                        project_root: tab.project_root.display().to_string(),
                        kind: tab.kind,
                        workspace,
                        running_agent_count: running_agents.len() as u32,
                        running_agents,
                    }
                })
                .collect(),
            active_tab_id: self.active_tab_id.clone(),
            recent_projects: self
                .recent_projects
                .iter()
                .map(|project| gwt::RecentProjectView {
                    path: project.path.display().to_string(),
                    title: project.title.clone(),
                    kind: project.kind,
                })
                .collect(),
        }
    }

    fn workspace_view_for_tab(&self, tab: &ProjectTabRuntime) -> gwt::WorkspaceView {
        gwt::WorkspaceView {
            viewport: tab.workspace.persisted().viewport.clone(),
            windows: tab
                .workspace
                .persisted()
                .windows
                .iter()
                .cloned()
                .map(|mut window| {
                    let raw_id = window.id.clone();
                    window.id = combined_window_id(&tab.id, &raw_id);
                    // SPEC-3431 FR-020: mark the resident PM window so the
                    // frontend can give it distinct chrome and target it from
                    // the PM launcher.
                    window.is_pm =
                        self.pm_sessions
                            .get(&tab.project_root)
                            .is_some_and(|pm_session| {
                                window.session_id.as_deref() == Some(pm_session.as_str())
                            });
                    window.worktree_form = self
                        .active_agent_sessions
                        .get(&window.id)
                        .map(|session| {
                            if self.session_uses_ephemeral_worktree(session) {
                                gwt::WindowWorktreeForm::Ephemeral
                            } else {
                                gwt::WindowWorktreeForm::BranchBacked
                            }
                        })
                        .unwrap_or(gwt::WindowWorktreeForm::Unknown);
                    if let gwt::WindowPlacement::AgentKanban {
                        board_id,
                        lane_id,
                        order,
                        collapsed,
                    } = window.placement
                    {
                        window.placement = gwt::WindowPlacement::AgentKanban {
                            board_id: combined_window_id(&tab.id, &board_id),
                            lane_id,
                            order,
                            collapsed,
                        };
                    }
                    if let Some(status) = self.window_status(&window.id) {
                        window.status = status;
                    }
                    window
                })
                .collect(),
            work_items: Vec::new(),
        }
    }

    pub(crate) fn workspace_state_broadcast(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::WindowCanvasState {
            workspace: self.app_state_view(),
        })
    }

    fn improvement_action_error(
        &self,
        client_id: &str,
        action: &str,
        id: Option<&str>,
        message: impl Into<String>,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id.to_string(),
            BackendEvent::ImprovementActionError {
                project_root: self
                    .active_project_root()
                    .map(|root| root.display().to_string()),
                id: id.map(str::to_string),
                action: action.to_string(),
                message: improvement_action_error_message(message.into()),
            },
        )]
    }

    fn improvement_promote_issue_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        self.spawn_improvement_action(
            client_id,
            "promote_issue",
            id,
            gwt::cli::ImprovementCommand::PromoteIssue(
                gwt::cli::improvement::ImprovementPromoteIssueCommand {
                    id: id.to_string(),
                    force: false,
                    labels: Vec::new(),
                },
            ),
        )
    }

    fn improvement_resolve_events(
        &self,
        client_id: &str,
        id: &str,
        expected_resolver_revision: Option<&str>,
    ) -> Vec<OutboundEvent> {
        self.spawn_improvement_action(
            client_id,
            "resolve",
            id,
            gwt::cli::ImprovementCommand::Resolve(
                gwt::cli::improvement::ImprovementResolveCommand {
                    id: id.to_string(),
                    expected_resolver_revision: expected_resolver_revision.map(str::to_string),
                },
            ),
        )
    }

    fn improvement_select_owner_events(
        &self,
        client_id: &str,
        id: &str,
        owner_number: u64,
        resolver_revision: &str,
    ) -> Vec<OutboundEvent> {
        self.spawn_improvement_action(
            client_id,
            "link_issue",
            id,
            gwt::cli::ImprovementCommand::LinkIssue(
                gwt::cli::improvement::ImprovementLinkIssueCommand {
                    id: id.to_string(),
                    owner_number,
                    resolver_revision: resolver_revision.to_string(),
                },
            ),
        )
    }

    fn improvement_dismiss_events(
        &self,
        client_id: &str,
        id: &str,
        reason: Option<&str>,
    ) -> Vec<OutboundEvent> {
        self.spawn_improvement_action(
            client_id,
            "dismiss",
            id,
            gwt::cli::ImprovementCommand::Dismiss(
                gwt::cli::improvement::ImprovementDismissCommand {
                    id: id.to_string(),
                    reason: reason
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("Dismissed from Improvement Inbox.")
                        .to_string(),
                },
            ),
        )
    }

    fn spawn_improvement_action(
        &self,
        client_id: &str,
        action: &'static str,
        id: &str,
        command: gwt::cli::ImprovementCommand,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            return self.improvement_action_error(
                client_id,
                action,
                Some(id),
                "No active project is selected.",
            );
        };
        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id = id.to_string();
        self.blocking_tasks.spawn(move || {
            let outcome = run_improvement_action_for_project(&project_root, command);
            proxy.send(UserEvent::ImprovementActionComplete {
                project_root,
                client_id,
                action: action.to_string(),
                id,
                outcome,
            });
        });
        Vec::new()
    }

    pub(crate) fn handle_improvement_action_complete(
        &mut self,
        project_root: &Path,
        client_id: &str,
        action: &str,
        id: &str,
        outcome: ImprovementActionOutcome,
    ) -> Vec<OutboundEvent> {
        let project_scope = project_root.display().to_string();
        self.schedule_improvement_candidates_refresh(project_root.to_path_buf());
        vec![OutboundEvent::reply(
            client_id.to_string(),
            match outcome {
                ImprovementActionOutcome::Success(message) => {
                    BackendEvent::ImprovementActionResult {
                        project_root: project_scope.clone(),
                        id: id.to_string(),
                        action: action.to_string(),
                        message: Some(message),
                    }
                }
                ImprovementActionOutcome::Error(message) => BackendEvent::ImprovementActionError {
                    project_root: Some(project_scope.clone()),
                    id: Some(id.to_string()),
                    action: action.to_string(),
                    message: improvement_action_error_message(message),
                },
            },
        )]
    }

    pub(crate) fn schedule_improvement_candidates_refresh(&mut self, project_root: PathBuf) {
        if self.improvement_refresh_epoch == u64::MAX {
            self.improvement_refresh_epoch = 0;
            self.improvement_latest_refresh_epochs.clear();
        }
        self.improvement_refresh_epoch += 1;
        let epoch = self.improvement_refresh_epoch;
        self.improvement_latest_refresh_epochs
            .insert(project_root.clone(), epoch);
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let result = gwt::cli::improvement::try_candidate_public_values(&project_root)
                .map_err(|error| error.to_string());
            proxy.send(UserEvent::ImprovementCandidatesLoaded {
                project_root,
                epoch,
                result,
            });
        });
    }

    pub(crate) fn schedule_active_improvement_candidates_refresh(&mut self) {
        if let Some(project_root) = self.active_project_root().map(Path::to_path_buf) {
            self.schedule_improvement_candidates_refresh(project_root);
        }
    }

    pub(crate) fn handle_improvement_candidates_loaded(
        &mut self,
        project_root: PathBuf,
        epoch: u64,
        result: Result<Vec<serde_json::Value>, String>,
    ) -> Vec<OutboundEvent> {
        if self
            .improvement_latest_refresh_epochs
            .get(&project_root)
            .copied()
            != Some(epoch)
        {
            return Vec::new();
        }
        self.improvement_latest_refresh_epochs.remove(&project_root);
        match result {
            Ok(candidates) => vec![OutboundEvent::broadcast(
                BackendEvent::ImprovementCandidates {
                    project_root: project_root.display().to_string(),
                    candidates,
                },
            )],
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    error = %error,
                    "Improvement candidate refresh failed; preserving the previous frontend snapshot"
                );
                Vec::new()
            }
        }
    }

    pub(crate) fn push_workspace_and_active_work_projection_broadcasts(
        &self,
        events: &mut Vec<OutboundEvent>,
    ) {
        events.push(self.workspace_state_broadcast());
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
    }

    pub(crate) fn window_status(&self, window_id: &str) -> Option<WindowProcessStatus> {
        let base = self.base_window_status(window_id)?;
        let preset = self.window_preset(window_id)?;
        Some(
            if self.window_approval_waiting.contains_key(window_id)
                && gwt::window_state::uses_agent_hook_state(preset)
                && !matches!(
                    base,
                    WindowProcessStatus::Stopped | WindowProcessStatus::Error
                )
            {
                WindowProcessStatus::Waiting
            } else {
                base
            },
        )
    }

    fn base_window_status(&self, window_id: &str) -> Option<WindowProcessStatus> {
        let preset = self.window_preset(window_id)?;
        let pty_state = if preset.requires_process() {
            self.window_pty_statuses
                .get(window_id)
                .copied()
                .or_else(|| {
                    let address = self.window_lookup.get(window_id)?;
                    let tab = self.tab(&address.tab_id)?;
                    Some(tab.workspace.window(&address.raw_id)?.status)
                })?
        } else {
            self.window_pty_statuses
                .get(window_id)
                .copied()
                .or_else(|| {
                    let address = self.window_lookup.get(window_id)?;
                    let tab = self.tab(&address.tab_id)?;
                    let window = tab.workspace.window(&address.raw_id)?;
                    Some(window.status)
                })?
        };
        let hook_state = self.window_hook_states.get(window_id).copied();
        Some(gwt::window_state::compose_window_state_with_active_session(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
        ))
    }

    fn persistable_workspace_state(
        &self,
        tab: &ProjectTabRuntime,
    ) -> gwt::PersistedWindowCanvasState {
        let mut state = tab.workspace.persistable_state();
        for window in &mut state.windows {
            let window_id = combined_window_id(&tab.id, &window.id);
            if self.window_approval_waiting.contains_key(&window_id) {
                if let Some(base) = self.base_window_status(&window_id) {
                    window.status = base;
                }
            }
        }
        state
    }

    pub(crate) fn register_window(&mut self, tab_id: &str, raw_id: &str) {
        self.window_lookup.insert(
            combined_window_id(tab_id, raw_id),
            WindowAddress {
                tab_id: tab_id.to_string(),
                raw_id: raw_id.to_string(),
            },
        );
    }

    pub(crate) fn set_window_status(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        status: WindowProcessStatus,
    ) {
        if let Some(tab) = self.tab_mut(tab_id) {
            let _ = tab.workspace.set_status(raw_id, status);
            if let Some(window) = tab.workspace.window(raw_id) {
                let window_id = combined_window_id(tab_id, raw_id);
                if window.preset.requires_process() {
                    self.window_pty_statuses.insert(window_id, status);
                } else {
                    self.window_pty_statuses.remove(&window_id);
                }
            }
        }
    }

    pub(crate) fn tab(&self, tab_id: &str) -> Option<&ProjectTabRuntime> {
        self.tabs.iter().find(|tab| tab.id == tab_id)
    }

    pub(crate) fn active_project_root(&self) -> Option<&Path> {
        let active_tab_id = self.active_tab_id.as_ref()?;
        self.tab(active_tab_id)
            .map(|tab| tab.project_root.as_path())
    }

    pub(crate) fn tab_mut(&mut self, tab_id: &str) -> Option<&mut ProjectTabRuntime> {
        self.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }

    pub(crate) fn active_tab_mut(&mut self) -> Option<&mut ProjectTabRuntime> {
        let active_tab_id = self.active_tab_id.clone()?;
        self.tab_mut(&active_tab_id)
    }

    pub(crate) fn set_active_tab(&mut self, tab_id: String) -> bool {
        let previous_project_root = self.active_project_root().map(Path::to_path_buf);
        let wizard_closed = self
            .launch_wizard
            .as_ref()
            .is_some_and(|wizard| wizard.tab_id != tab_id);
        self.active_tab_id = Some(tab_id);
        if wizard_closed {
            self.launch_wizard = None;
        }
        if self.active_project_root().map(Path::to_path_buf) != previous_project_root {
            self.schedule_active_improvement_candidates_refresh();
        }
        wizard_closed
    }

    pub(crate) fn rebuild_window_lookup(&mut self) {
        self.window_lookup.clear();
        let pairs = self
            .tabs
            .iter()
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| (tab.id.clone(), window.id.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (tab_id, raw_id) in pairs {
            self.register_window(&tab_id, &raw_id);
        }
    }

    fn window_preset(&self, window_id: &str) -> Option<WindowPreset> {
        let address = self.window_lookup.get(window_id)?;
        let tab = self.tab(&address.tab_id)?;
        let window = tab.workspace.window(&address.raw_id)?;
        Some(window.preset)
    }

    pub(crate) fn seed_window_pty_statuses(&mut self) {
        self.window_pty_statuses.clear();
        self.window_output_bytes.clear();
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if window.preset.requires_process() {
                    self.window_pty_statuses
                        .insert(combined_window_id(&tab.id, &window.id), window.status);
                }
            }
        }
        self.window_hook_states.clear();
        self.window_approval_waiting.clear();
        self.recoverable_agent_error_windows.clear();
    }

    fn active_window_for_runtime_event(&self, event: &gwt::RuntimeHookEvent) -> Option<String> {
        let window_for_session_id = |session_id: &str| {
            self.active_agent_sessions
                .iter()
                .find(|(_, session)| session.session_id == session_id)
                .map(|(window_id, _)| window_id.clone())
        };
        // SessionStart is the readiness receipt that can finalize or abort a
        // Prepared execution. Its Host-issued gwt identity is authoritative;
        // provider conversation ids remain a compatibility fallback only for
        // non-readiness runtime events.
        if event.source_event.as_deref() == Some("SessionStart") {
            return event
                .gwt_session_id
                .as_deref()
                .and_then(window_for_session_id);
        }
        [
            event.gwt_session_id.as_deref(),
            event.agent_session_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(window_for_session_id)
    }

    fn recompute_window_state(&mut self, window_id: &str) -> Option<WindowProcessStatus> {
        let preset = self.window_preset(window_id)?;
        let pty_state = if preset.requires_process() {
            self.window_pty_statuses.get(window_id).copied()?
        } else {
            let address = self.window_lookup.get(window_id)?;
            let tab = self.tab(&address.tab_id)?;
            tab.workspace.window(&address.raw_id)?.status
        };
        let hook_state = self.window_hook_states.get(window_id).copied();
        let composed = gwt::window_state::compose_window_state_with_approval_wait(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
            self.window_approval_waiting.contains_key(window_id),
        );
        let address = self.window_lookup.get(window_id)?.clone();
        if let Some(tab) = self.tab_mut(&address.tab_id) {
            let _ = tab.workspace.set_status(&address.raw_id, composed);
        }
        Some(composed)
    }

    fn remove_window_state_tracking(&mut self, window_id: &str) {
        self.window_pty_statuses.remove(window_id);
        self.window_output_bytes.remove(window_id);
        self.window_hook_states.remove(window_id);
        self.clear_runtime_approval_latch_without_status(window_id, true);
        self.recoverable_agent_error_windows.remove(window_id);
        self.board_all_view_windows.remove(window_id);
    }

    fn tracked_window_exists(&self, window_id: &str) -> bool {
        let Some(address) = self.window_lookup.get(window_id) else {
            return false;
        };
        self.tab(&address.tab_id)
            .and_then(|tab| tab.workspace.window(&address.raw_id))
            .is_some()
    }

    pub(crate) fn seed_restored_window_details(&mut self) {
        self.window_details.clear();
        // SPEC-1921 Phase 65 (T337): classify restored Agent-family
        // placeholders before seeding details. An exact auto-resume candidate
        // resumes as soon as the frontend canvas is ready, so labeling it with
        // the generic paused message would flash a wrong state; an agent
        // placeholder without an exact provider session id stays stopped and
        // must explain why instead of implying a plain paused terminal.
        let mut seeded = Vec::new();
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if !(window.preset.requires_process()
                    && window.status == WindowProcessStatus::Stopped)
                {
                    continue;
                }
                let combined = combined_window_id(&tab.id, &window.id);
                if crate::runtime_support::window_is_agent_pane(window) {
                    if self.restored_window_is_exact_auto_resume_candidate(window) {
                        continue;
                    }
                    seeded.push((
                        combined,
                        "Exact session restore is unavailable: no exact provider session id \
                         is recorded for this agent window. It stays paused instead of \
                         resuming a different conversation; launch a new agent session when \
                         you want to continue."
                            .to_string(),
                    ));
                } else {
                    seeded.push((
                        combined,
                        "Restored window is paused. Launch a new terminal when you want to start it."
                            .to_string(),
                    ));
                }
            }
        }
        self.window_details.extend(seeded);
    }

    /// True when a restored, stopped Agent-family placeholder is backed by a
    /// persisted session that supports exact resume (real provider session id
    /// and an existing worktree) — the same gate the startup auto-resume queue
    /// applies to placeholder-backed sessions (SPEC-1921 Phase 65).
    fn restored_window_is_exact_auto_resume_candidate(
        &self,
        window: &gwt::PersistedWindowState,
    ) -> bool {
        let Some(session_id) = window.session_id.as_deref() else {
            return false;
        };
        let path = self.sessions_dir.join(format!("{session_id}.toml"));
        gwt_agent::Session::load_and_migrate(&path).is_ok_and(|session| {
            session.exact_resume_session_id().is_some() && session.worktree_path.exists()
        })
    }

    /// Capture the current session + workspace state and hand it off to the
    /// persist dispatcher. The dispatcher writes the snapshot atomically on a
    /// worker thread, so this call returns without blocking on disk I/O.
    /// Bursts of `persist()` calls collapse to a single disk write because the
    /// dispatcher keeps only the latest snapshot.
    ///
    /// Issue #2694 Phase B: prior to this change the call wrote
    /// `session-state.json` and every active workspace file synchronously on
    /// the tao event-loop thread, which Windows Defender / EDR scans amplified
    /// into multi-hundred-millisecond freezes during routine UI interactions.
    pub(crate) fn persist(&self) -> std::io::Result<()> {
        let snapshot = persist_dispatcher::PersistSnapshot {
            session_path: self.session_state_path.clone(),
            session: gwt::PersistedSessionState {
                tabs: self
                    .tabs
                    .iter()
                    .map(|tab| gwt::PersistedSessionTabState {
                        id: tab.id.clone(),
                        title: tab.title.clone(),
                        project_root: tab.project_root.clone(),
                        kind: tab.kind,
                    })
                    .collect(),
                active_tab_id: normalize_active_tab_id(&self.tabs, self.active_tab_id.clone()),
                recent_projects: self.recent_projects.clone(),
            },
            workspaces: self
                .tabs
                .iter()
                .map(|tab| {
                    (
                        workspace_state_path(&tab.project_root),
                        self.persistable_workspace_state(tab),
                    )
                })
                .collect(),
        };
        self.persist_dispatcher.enqueue(snapshot);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkBranchScanTarget {
    branch: String,
    worktree_paths: Vec<PathBuf>,
    has_merged_pr: bool,
}

fn work_branch_scan_targets(
    projection: &gwt_core::workspace_projection::WorkItemsProjection,
) -> Vec<WorkBranchScanTarget> {
    let mut by_branch: HashMap<String, WorkBranchScanTarget> = HashMap::new();
    for item in &projection.work_items {
        if item.is_terminal() {
            continue;
        }
        for container in &item.execution_containers {
            let Some(branch) = container
                .branch
                .as_deref()
                .map(crate::runtime_support::normalize_branch_name)
                .filter(|branch| !branch.is_empty())
            else {
                continue;
            };
            let target = by_branch
                .entry(branch.clone())
                .or_insert_with(|| WorkBranchScanTarget {
                    branch,
                    worktree_paths: Vec::new(),
                    has_merged_pr: false,
                });
            target.has_merged_pr |= container
                .pr_state
                .as_deref()
                .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
            if let Some(path) = &container.worktree_path {
                if !target
                    .worktree_paths
                    .iter()
                    .any(|existing| existing == path)
                {
                    target.worktree_paths.push(path.clone());
                }
            }
        }
    }
    let mut targets: Vec<WorkBranchScanTarget> = by_branch.into_values().collect();
    targets.sort_by(|left, right| left.branch.cmp(&right.branch));
    targets
}

fn append_workspace_projection_scan_target(
    project_root: &Path,
    targets: &mut Vec<WorkBranchScanTarget>,
) {
    let Ok(Some(projection)) =
        gwt_core::workspace_projection::load_workspace_projection(project_root)
    else {
        return;
    };
    let Some(details) = projection.git_details else {
        return;
    };
    let Some(branch) = details
        .branch
        .as_deref()
        .map(crate::runtime_support::normalize_branch_name)
        .filter(|branch| !branch.is_empty())
    else {
        return;
    };
    let target = if let Some(target) = targets.iter_mut().find(|target| target.branch == branch) {
        target
    } else {
        targets.push(WorkBranchScanTarget {
            branch: branch.clone(),
            worktree_paths: Vec::new(),
            has_merged_pr: false,
        });
        targets.last_mut().expect("just pushed scan target")
    };
    target.has_merged_pr |= details
        .pr_state
        .as_deref()
        .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
    if let Some(path) = details.worktree_path {
        if !target
            .worktree_paths
            .iter()
            .any(|existing| existing == &path)
        {
            target.worktree_paths.push(path);
        }
    }
    targets.sort_by(|left, right| left.branch.cmp(&right.branch));
}

fn work_merge_scan_needs_dirty_check(
    readiness: Option<&gwt_git::branch::CleanupReadinessTarget>,
    has_merged_pr: bool,
) -> bool {
    readiness.is_some() || has_merged_pr
}

fn work_branch_has_dirty_worktree(target: &WorkBranchScanTarget) -> bool {
    target.worktree_paths.iter().any(|path| {
        gwt_git::diff::get_status(path)
            .map(|entries| !entries.is_empty())
            .unwrap_or(true)
    })
}

fn work_branches_with_live_processes(targets: &[WorkBranchScanTarget]) -> HashSet<String> {
    let mut worktrees = Vec::new();
    for target in targets {
        for path in &target.worktree_paths {
            let Ok(path) = dunce::canonicalize(path) else {
                continue;
            };
            if !worktrees
                .iter()
                .any(|(branch, existing)| branch == &target.branch && existing == &path)
            {
                worktrees.push((target.branch.clone(), path));
            }
        }
    }
    if worktrees.is_empty() {
        return HashSet::new();
    }

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );

    let mut live_branches = HashSet::new();
    for process in system.processes().values() {
        let Some(cwd) = process.cwd() else {
            continue;
        };
        let Ok(cwd) = dunce::canonicalize(cwd) else {
            continue;
        };
        for (branch, worktree) in &worktrees {
            if cwd == *worktree || cwd.starts_with(worktree) {
                live_branches.insert(branch.clone());
            }
        }
    }
    live_branches
}

fn improvement_action_error_message(message: impl Into<String>) -> String {
    let message = message.into();
    if message
        .to_ascii_lowercase()
        .contains("authentication required")
    {
        return "GitHub authentication is required to promote this improvement. Run `gh auth login`, or restart browser-check with `GH_TOKEN` available."
            .to_string();
    }
    message
}

fn run_improvement_action_for_project(
    project_root: &Path,
    command: gwt::cli::ImprovementCommand,
) -> ImprovementActionOutcome {
    let mut env = gwt::cli::DefaultCliEnv::new("akiojin", "gwt", project_root.to_path_buf());
    let mut output = String::new();
    match gwt::cli::improvement::run(&mut env, command, &mut output) {
        Ok(_) => ImprovementActionOutcome::Success(output.trim().to_string()),
        Err(error) => ImprovementActionOutcome::Error(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
