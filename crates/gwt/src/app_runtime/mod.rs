use super::*;

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

#[derive(Clone)]
pub enum BlockingTaskSpawner {
    Tokio(tokio::runtime::Handle),
    #[cfg(test)]
    Thread,
}

impl BlockingTaskSpawner {
    pub(crate) fn tokio(handle: tokio::runtime::Handle) -> Self {
        Self::Tokio(handle)
    }

    #[cfg(test)]
    pub(crate) fn thread() -> Self {
        Self::Thread
    }

    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        match self {
            Self::Tokio(handle) => {
                drop(handle.spawn_blocking(task));
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
                    .expect("spawn test blocking task");
            }
        }
    }
}

pub struct WindowRuntime {
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
mod file_windows;
mod frontend_action_log;
mod knowledge;
mod launch;
mod launch_errors;
mod launch_output_mirror;
mod loaders;
mod migration;
pub(crate) mod persist_dispatcher;
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
use launch::AgentLaunchCompletion;
#[cfg(test)]
use launch::{
    codex_hook_discovery_mode_from_codex_version_output,
    codex_hook_discovery_mode_from_selected_codex_version, dispatch_agent_launch_success,
    maybe_register_codex_managed_hook_trust_for_launch,
};
use launch::{launch_config_from_persisted_session, IssueBranchLinkStore};
pub use launch::{AgentLaunchResult, LaunchWizardMemoryCache, ProcessLaunch};
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
    workspace_work_agent_view_from_ref, workspace_work_event_kind_wire,
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
pub struct OutboundEvent {
    pub(crate) target: DispatchTarget,
    pub(crate) event: BackendEvent,
}

impl OutboundEvent {
    pub(crate) fn broadcast(event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Broadcast,
            event,
        }
    }

    pub(crate) fn reply(client_id: impl Into<ClientId>, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
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
}

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
    pub(crate) pending_workspace_resume_contexts: HashMap<String, WorkspaceResumeContext>,
    pub(crate) pending_launch_feedback_contexts: HashMap<String, LaunchFeedbackContext>,
    /// SPEC-2359 W-17 (FR-398, Issue #3034): launches whose window is
    /// registered but whose agent session is not live yet, keyed by
    /// (tab, branch, working dir). A re-click in this window focuses the
    /// pending window instead of spawning a duplicate. Entries clear on
    /// launch completion/failure or after a TTL.
    pub(crate) inflight_launches: HashMap<String, (String, std::time::Instant)>,
    pub(crate) pending_auto_resume_sources: HashMap<String, String>,
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
    /// SPEC-2359 W-16 (FR-387): last work-events ingest per project — the
    /// 30s throttle for tab-change / post-launch triggers.
    pub(crate) last_work_events_ingest: std::cell::RefCell<HashMap<PathBuf, std::time::Instant>>,
    /// SPEC-2359 W16-3 (FR-390): normalized branch names that currently have
    /// a LOCAL worktree, per project — refreshed by the worktree reconcile.
    /// The view marks `remote_only` by cache lookup alone (no git spawn on
    /// the projection build path).
    pub(crate) local_worktree_branches:
        std::cell::RefCell<HashMap<PathBuf, std::collections::HashSet<String>>>,
    pub(crate) window_pty_statuses: HashMap<String, WindowProcessStatus>,
    pub(crate) window_hook_states: HashMap<String, WindowProcessStatus>,
    pub(crate) recoverable_agent_error_windows: HashSet<String>,
    pub(crate) hook_forward_target: Option<HookForwardTarget>,
    pub(crate) issue_link_cache_dir: PathBuf,
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

/// Issue #3222: whether a local Issue Monitor pass may claim + launch, or must
/// only observe (scan for a fresh snapshot without side effects). ACK and
/// window-close handling runs `Observe`; user-driven refresh/config flows run
/// `ClaimAndLaunch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorScanPolicy {
    ClaimAndLaunch,
    Observe,
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
            pending_workspace_resume_contexts: HashMap::new(),
            inflight_launches: HashMap::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::new(),
            work_merged_branches: HashMap::new(),
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
            last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
            local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            recoverable_agent_error_windows: HashSet::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
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
    ) -> Vec<OutboundEvent> {
        self.work_merged_branches
            .insert(project_root.to_path_buf(), merged_branches);
        self.work_cleanup_ready_branches
            .insert(project_root.to_path_buf(), cleanup_ready_branches);
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    /// SPEC-2359 W-15 / US-84: scan the project's unclosed Workspace branches
    /// for merge-into-base or no effective changes off the UI thread. Sends an
    /// event even when the result is empty so stale cleanup-readiness cache
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
        self.spawn_work_ai_summaries_scan(project_root);
        if changed {
            self.active_work_projection_broadcast_for_active_tab()
                .into_iter()
                .collect()
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
            let targets = work_branch_scan_targets(&projection);
            if targets.is_empty() {
                proxy.send(UserEvent::WorkMergeStatus {
                    project_root,
                    merged_branches: HashMap::new(),
                    cleanup_ready_branches: HashMap::new(),
                });
                return;
            }
            let mut merged: Vec<String> = Vec::new();
            let mut cleanup_ready_branches: HashMap<String, String> = HashMap::new();
            for target in targets {
                if work_branch_has_dirty_worktree(&target) {
                    continue;
                }
                let branch = target.branch;
                if let Ok(Some(readiness)) =
                    gwt_git::branch::cleanup_readiness_base_target(&project_root, &branch)
                {
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
            // SPEC-2359 W16-4 (FR-391): one extra spawn resolves every tip
            // committer time — the merge-reference-time proxy for the derived
            // Done classification (plan decision 8).
            let tip_times =
                gwt_git::refs::branch_tip_committer_times(&project_root).unwrap_or_default();
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
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
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
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    /// SPEC-3075: resolve every branch's PR title off the UI thread in ONE
    /// `gh pr list` call (the GitHub API may paginate), then hand the
    /// `branch -> title` map to the event loop via [`UserEvent::WorkPrTitles`].
    /// The PR title is the human-written purpose of the work — the strongest
    /// "what work was running" signal. Network-dependent: an empty map (offline
    /// / `gh` absent / unauthenticated) leaves the commit-subject fallback in
    /// place. Runs once per project-open, after the events ingest.
    pub(crate) fn spawn_work_pr_titles_scan(&self, project_root: PathBuf) {
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
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
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

    #[cfg(unix)]
    fn publish_issue_monitor_control(&self, payload: serde_json::Value) -> Result<(), String> {
        let Some(project_root) = self.active_project_root() else {
            return Err("no active project".to_string());
        };
        let payload = gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            payload,
            std::process::id(),
        );
        gwt::daemon_publisher::publish_event(
            project_root,
            gwt::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
            payload,
        )
    }

    #[cfg(not(unix))]
    fn publish_issue_monitor_control(&self, _payload: serde_json::Value) -> Result<(), String> {
        Err("Issue Monitor daemon control is unavailable on this platform".to_string())
    }

    pub(crate) fn issue_monitor_launch_failed_events(
        &self,
        issue_number: u64,
        message: &str,
    ) -> Vec<OutboundEvent> {
        let message = if gwt::issue_monitor::is_git_https_auth_error(message) {
            gwt::issue_monitor::git_https_auth_setup_message(message)
        } else {
            message.to_string()
        };
        if let Err(error) = self.publish_issue_monitor_control(serde_json::json!({
            "launch_failed": {
                "issue_number": issue_number,
                "message": message,
            }
        })) {
            tracing::debug!(
                error = %error,
                issue_number,
                "issue monitor launch-failed daemon publish failed"
            );
        }
        vec![
            OutboundEvent::broadcast(BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message: message.clone(),
            }),
            OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
                level: "error".to_string(),
                message,
                issue_number: Some(issue_number),
            }),
        ]
    }

    pub(crate) fn issue_monitor_launch_succeeded_events(
        &mut self,
        issue_number: u64,
        window_id: &str,
    ) -> Vec<OutboundEvent> {
        if let Err(error) = self.publish_issue_monitor_control(serde_json::json!({
            "launched": {
                "issue_number": issue_number,
                "window_id": window_id,
            }
        })) {
            tracing::debug!(
                error = %error,
                issue_number,
                window_id,
                "issue monitor launched daemon publish failed"
            );
        }
        let window_id = window_id.to_string();
        // #3165 error-window lifecycle (default mode): when an issue relaunches
        // after a failure, close the stale agent window from the prior attempt so
        // it is replaced rather than left on the canvas. Guard against closing the
        // freshly launched window if it happens to reuse the same id.
        let mut stale_window: Option<String> = None;
        let mut events = self.local_issue_monitor_events_with_policy(
            None,
            IssueMonitorScanPolicy::Observe,
            |monitor| {
                stale_window = monitor
                    .take_failed_window(issue_number)
                    .filter(|stale| *stale != window_id);
                monitor.complete_active_launch(issue_number, window_id.clone());
            },
        );
        if let Some(stale) = stale_window {
            events.extend(self.close_window_events(&stale));
        }
        events
    }

    pub(crate) fn issue_monitor_agent_failed_events(
        &mut self,
        window_id: &str,
        message: &str,
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
        let mut agent_failed_payload = serde_json::json!({
            "window_id": window_id,
            "message": message,
        });
        if let Some(issue_number) = issue_number_hint {
            agent_failed_payload["issue_number"] = serde_json::json!(issue_number);
        }
        if let Err(error) = self.publish_issue_monitor_control(serde_json::json!({
            "agent_failed": agent_failed_payload,
        })) {
            tracing::debug!(
                error = %error,
                window_id,
                "issue monitor agent-failed daemon publish failed"
            );
        }
        self.local_issue_monitor_agent_failed_events(window_id, message, issue_number_hint)
    }

    /// SPEC #3200 T-045/FR-025: a monitored autonomous agent showed liveness
    /// (a runtime status change). Best-effort refresh of the daemon's
    /// stuck-detection window for the mapped issue. No-op for non-monitor windows.
    pub(crate) fn issue_monitor_heartbeat(&mut self, window_id: &str) {
        let Some(issue_number) = self
            .pending_launch_feedback_contexts
            .get(window_id)
            .and_then(|context| context.issue_monitor_issue_number)
        else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if let Err(error) = self.publish_issue_monitor_control(serde_json::json!({
            "heartbeat": { "issue_number": issue_number, "at": now },
        })) {
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
        window_ids: &[String],
    ) -> Vec<OutboundEvent> {
        let monitor_windows: Vec<String> = {
            let Some(project_root) = self.active_project_root() else {
                return Vec::new();
            };
            let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
            let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
            let monitor =
                gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
            window_ids
                .iter()
                .filter(|window_id| monitor.launched_window_issue(window_id).is_some())
                .cloned()
                .collect()
        };
        if monitor_windows.is_empty() {
            return Vec::new();
        }
        for window_id in &monitor_windows {
            if let Err(error) = self.publish_issue_monitor_control(serde_json::json!({
                "window_closed": { "window_id": window_id },
            })) {
                tracing::debug!(
                    error = %error,
                    window_id = %window_id,
                    "issue monitor window-closed daemon publish failed"
                );
            }
        }
        let requeue_windows = monitor_windows;
        self.local_issue_monitor_events_with_policy(
            None,
            IssueMonitorScanPolicy::Observe,
            move |monitor| {
                for window_id in &requeue_windows {
                    monitor.requeue_window(window_id);
                }
            },
        )
    }

    fn local_issue_monitor_events(
        &mut self,
        client_id: &str,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        self.local_issue_monitor_events_for(Some(client_id), apply)
    }

    fn local_issue_monitor_events_for(
        &mut self,
        client_id: Option<&str>,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        self.local_issue_monitor_events_with_policy(
            client_id,
            IssueMonitorScanPolicy::ClaimAndLaunch,
            apply,
        )
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
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        let mut issues =
            gwt::issue_monitor_worker::load_cached_issue_monitor_candidates(cache_root)
                .unwrap_or_default();
        if !issues.iter().any(|issue| issue.number == snapshot.number.0) {
            issues.push(issue_monitor_issue_from_snapshot(snapshot));
        }
        gwt::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
        self.issue_monitor_snapshot_events_for(client_id, monitor)
    }

    fn local_issue_monitor_events_with_policy(
        &mut self,
        client_id: Option<&str>,
        policy: IssueMonitorScanPolicy,
        apply: impl FnOnce(&mut gwt::IssueMonitorState),
    ) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
            monitor.record_scan_error(now, "No active project");
            return self.issue_monitor_snapshot_events_for(client_id, monitor);
        };

        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        apply(&mut monitor);
        // #3223 follow-up: release claimed-but-never-acked launches whose claim
        // anchor exceeded claim_ttl_secs so a crash cannot leak a slot forever.
        monitor.expire_stale_unbound_launches(&now);
        let _ = gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs());
        let mut launch_requests = Vec::new();
        let mut settings_required_request = None;

        match gwt::issue_monitor_worker::github_remote_owner_and_repo(&project_root) {
            Ok((owner, repo)) => {
                match gwt::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path(
                    &project_root,
                    &owner,
                    &repo,
                ) {
                    Ok(issues) => {
                        gwt::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
                        gwt::issue_monitor_worker::reconcile_issue_monitor_merges(
                            &mut monitor,
                            &project_root,
                        );
                        if monitor.config.enabled {
                            monitor.set_gui_connected(true);
                        }
                        // Issue #3222: ACK / window-close flows scan for a fresh
                        // snapshot (inbox rows for the UI) but must NEVER claim
                        // or launch — re-entrant claiming on a disk snapshot
                        // that cannot see other in-flight claims is what
                        // spawned duplicate windows past max_active.
                        if monitor.config.enabled
                            && matches!(policy, IssueMonitorScanPolicy::ClaimAndLaunch)
                        {
                            let launch_profile_ready = monitor.has_launch_profile();
                            let active_cap = if launch_profile_ready {
                                monitor.config.max_active.max(1)
                            } else {
                                0
                            };
                            if !launch_profile_ready {
                                settings_required_request = monitor.inbox.iter().find_map(|item| {
                                    (item.state == gwt::MonitorInboxState::Queued).then(|| {
                                        (
                                            item.issue.number,
                                            gwt::issue_monitor::issue_monitor_linked_issue_kind(
                                                &item.issue,
                                            ),
                                        )
                                    })
                                });
                            } else if monitor.active_count() < active_cap {
                                let monitor_owner =
                                    format!("{}:{}", whoami::username(), std::process::id());
                                match gwt_github::client::http::HttpIssueClient::from_gh_auth(
                                    &owner, &repo,
                                ) {
                                    Ok(client) => {
                                        launch_requests = monitor
                                            .claim_next_launch_requests_with_probe(
                                                &client,
                                                &monitor_owner,
                                                &now,
                                                active_cap,
                                                |issue_number| {
                                                    gwt::issue_monitor_worker::issue_completed_by_merged_pr(
                                                        &owner,
                                                        &repo,
                                                        issue_number,
                                                    )
                                                },
                                            );
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            error = %error,
                                            "issue monitor GitHub claim authentication unavailable"
                                        );
                                        monitor.record_launch_auth_required(now);
                                    }
                                }
                            }
                        }
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

        let mut launch_events = Vec::new();
        if let Some((issue_number, linked_issue_kind)) = settings_required_request {
            launch_events.extend(self.open_issue_monitor_settings_required_events(
                client_id,
                issue_number,
                linked_issue_kind,
            ));
        }
        for request in launch_requests {
            let request_events = self.auto_launch_issue_monitor_request_events(
                request.issue_number,
                request.linked_issue_kind,
            );
            if let Some(message) = request_events.iter().find_map(|event| match &event.event {
                BackendEvent::IssueMonitorLaunchFailed {
                    issue_number,
                    message,
                } if *issue_number == request.issue_number => Some(message.clone()),
                _ => None,
            }) {
                monitor.record_launch_failed(request.issue_number, message);
            }
            launch_events.extend(request_events);
        }
        // Issue #3222: persist the claim-side state (Launching without a bound
        // window yet, plus any recorded launch failures) so the next handler /
        // the async launch ACK sees the in-flight claims and cannot re-claim
        // them into duplicate windows.
        let _ = gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs());
        let mut events = launch_events;
        events.extend(self.issue_monitor_snapshot_events_for(client_id, monitor));
        events
    }

    fn open_issue_monitor_settings_required_events(
        &mut self,
        client_id: Option<&str>,
        issue_number: u64,
        linked_issue_kind: gwt::LinkedIssueKind,
    ) -> Vec<OutboundEvent> {
        if self.launch_wizard.is_some() {
            return Vec::new();
        }
        let target_client_id = client_id.unwrap_or("__issue_monitor__");
        let events = self.open_issue_monitor_configure_wizard_events(
            target_client_id,
            issue_number,
            linked_issue_kind,
        );
        if client_id.is_some() {
            return events;
        }
        events
            .into_iter()
            .map(|mut event| {
                if matches!(event.target, DispatchTarget::Client(_)) {
                    event.target = DispatchTarget::Broadcast;
                }
                event
            })
            .collect()
    }

    fn local_issue_monitor_agent_failed_events(
        &mut self,
        window_id: &str,
        message: &str,
        issue_number_hint: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            return Vec::new();
        };

        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);

        match gwt::issue_monitor_worker::github_remote_owner_and_repo(&project_root) {
            Ok((owner, repo)) => {
                match gwt::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path(
                    &project_root,
                    &owner,
                    &repo,
                ) {
                    Ok(issues) => {
                        gwt::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
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

        let issue_number = if let Some(issue_number) = issue_number_hint {
            monitor.record_agent_issue_failed(issue_number, message.to_string());
            Some(issue_number)
        } else {
            monitor.record_agent_window_failed(window_id, message.to_string())
        };
        if issue_number.is_none() {
            monitor.record_scan_error(
                now.as_str(),
                format!("agent window {window_id} failed but no monitored Issue mapping was found: {message}"),
            );
        }
        let _ = gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs());
        if issue_number_hint.is_some() {
            self.pending_launch_feedback_contexts.remove(window_id);
        }

        // #3165/#3200 error-window lifecycle: an autonomous (two-stage opt-in)
        // failure auto-closes its stale agent window so the bounded retry
        // relaunches into a clean canvas. A default failure keeps the window on
        // the canvas for human inspection (closed later on explicit Launch Now).
        let autoclose_failed_window = issue_number
            .map(|number| monitor.should_autoclose_failed_window(number))
            .unwrap_or(false);

        let mut events = self.issue_monitor_snapshot_events_for(None, monitor);
        events.push(OutboundEvent::broadcast(BackendEvent::IssueMonitorToast {
            level: "error".to_string(),
            message: message.to_string(),
            issue_number,
        }));
        if autoclose_failed_window {
            events.extend(self.close_window_events(window_id));
        }
        events
    }

    fn issue_monitor_snapshot_events_for(
        &self,
        client_id: Option<&str>,
        monitor: gwt::IssueMonitorState,
    ) -> Vec<OutboundEvent> {
        let mut status = monitor.status_view();
        let project_root = self
            .active_tab_id
            .as_deref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| tab.project_root.clone());
        self.apply_issue_monitor_launch_profile_status(&mut status, project_root.as_deref());
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
            FrontendEvent::CloseWindow { id } => self.close_window_events(&id),
            FrontendEvent::StopWindow { id } => self.stop_window_events(&id),
            FrontendEvent::StopAllWindows {} => self.stop_all_windows_events(),
            FrontendEvent::RestartWindow { id } => self.restart_window_events(&id),
            FrontendEvent::TerminalInput { id, data } => self.terminal_input_events(&id, &data),
            FrontendEvent::PaneSendInput { session_id, text } => {
                self.pane_send_input_events(client_id, &session_id, &text)
            }
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
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number: Some(number),
                    refresh: false,
                },
            ),
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
            FrontendEvent::ListResumableAgents { workspace_id } => {
                self.list_resumable_agents_events(&client_id, workspace_id)
            }
            FrontendEvent::ResumeWorkspaceAgent {
                session_id,
                agent_session_id,
                bounds,
            } => {
                self.resume_workspace_agent_events(&client_id, session_id, agent_session_id, bounds)
            }
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
                            let prefs =
                                gwt::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
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
                match self.publish_issue_monitor_control(serde_json::json!({ "enabled": enabled }))
                {
                    Ok(()) => self.local_issue_monitor_events(&client_id, |monitor| {
                        monitor.set_enabled(enabled)
                    }),
                    Err(error) => {
                        tracing::debug!(
                            error = %error,
                            "issue monitor control daemon publish failed; using local fallback"
                        );
                        self.local_issue_monitor_events(&client_id, |monitor| {
                            monitor.set_enabled(enabled)
                        })
                    }
                }
            }
            FrontendEvent::SetIssueMonitorAutonomousMode { enabled } => {
                match self.publish_issue_monitor_control(
                    serde_json::json!({ "autonomous_mode": enabled }),
                ) {
                    Ok(()) => self.local_issue_monitor_events(&client_id, |monitor| {
                        monitor.set_autonomous_mode(enabled)
                    }),
                    Err(error) => {
                        tracing::debug!(
                            error = %error,
                            "issue monitor autonomous-mode daemon publish failed; using local fallback"
                        );
                        self.local_issue_monitor_events(&client_id, |monitor| {
                            monitor.set_autonomous_mode(enabled)
                        })
                    }
                }
            }
            FrontendEvent::SetIssueMonitorMaxActiveAgents { max_active_agents } => {
                match self.publish_issue_monitor_control(
                    serde_json::json!({ "max_active_agents": max_active_agents }),
                ) {
                    Ok(()) => self.local_issue_monitor_events(&client_id, |monitor| {
                        monitor.set_max_active_agents(max_active_agents)
                    }),
                    Err(error) => {
                        tracing::debug!(
                            error = %error,
                            "issue monitor max-active daemon publish failed; using local fallback"
                        );
                        self.local_issue_monitor_events(&client_id, |monitor| {
                            monitor.set_max_active_agents(max_active_agents)
                        })
                    }
                }
            }
            FrontendEvent::ReorderIssueMonitorIssues { issue_numbers } => {
                let priority_order = issue_numbers;
                match self.publish_issue_monitor_control(
                    serde_json::json!({ "priority_order": priority_order.clone() }),
                ) {
                    Ok(()) => self.local_issue_monitor_events(&client_id, |monitor| {
                        monitor.reorder_queued_issues(&priority_order)
                    }),
                    Err(error) => {
                        tracing::debug!(
                            error = %error,
                            "issue monitor reorder daemon publish failed; using local fallback"
                        );
                        self.local_issue_monitor_events(&client_id, |monitor| {
                            monitor.reorder_queued_issues(&priority_order)
                        })
                    }
                }
            }
            FrontendEvent::ListIssueMonitor => self.local_issue_monitor_events(&client_id, |_| {}),
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
            } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::test_connection_event(agent, &base_url, &api_key),
            )],
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
                    window.lane_kind = self
                        .active_agent_sessions
                        .get(&window.id)
                        .map(|session| {
                            if self.is_ephemeral_intake_session(session) {
                                gwt::WindowLaneKind::Intake
                            } else {
                                gwt::WindowLaneKind::Execution
                            }
                        })
                        .unwrap_or(gwt::WindowLaneKind::Unknown);
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
        let pty_state = self
            .window_pty_statuses
            .get(window_id)
            .copied()
            .or_else(|| {
                let address = self.window_lookup.get(window_id)?;
                let tab = self.tab(&address.tab_id)?;
                let window = tab.workspace.window(&address.raw_id)?;
                Some(window.status)
            })?;
        let hook_state = self.window_hook_states.get(window_id).copied();
        let preset = self.window_preset(window_id)?;
        Some(gwt::window_state::compose_window_state_with_active_session(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
        ))
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

    pub(crate) fn resize_runtime_to_window(&self, window_id: &str) {
        let Some(address) = self.window_lookup.get(window_id) else {
            return;
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return;
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return;
        };
        if !window.preset.requires_process() {
            return;
        }
        if let Some(runtime) = self.runtimes.get(window_id) {
            if let Ok(mut pane) = runtime.pane.lock() {
                let (cols, rows) = geometry_to_pty_size(&window.geometry);
                let _ = pane.resize(cols.max(20), rows.max(6));
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
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if window.preset.requires_process() {
                    self.window_pty_statuses
                        .insert(combined_window_id(&tab.id, &window.id), window.status);
                }
            }
        }
        self.window_hook_states.clear();
        self.recoverable_agent_error_windows.clear();
    }

    fn active_window_for_runtime_event(&self, event: &gwt::RuntimeHookEvent) -> Option<String> {
        [
            event.gwt_session_id.as_deref(),
            event.agent_session_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(|session_id| {
            self.active_agent_sessions
                .iter()
                .find(|(_, session)| session.session_id == session_id)
                .map(|(window_id, _)| window_id.clone())
        })
    }

    fn recompute_window_state(&mut self, window_id: &str) -> Option<WindowProcessStatus> {
        let pty_state = self
            .window_pty_statuses
            .get(window_id)
            .copied()
            .or_else(|| self.window_status(window_id))?;
        let hook_state = self.window_hook_states.get(window_id).copied();
        let preset = self.window_preset(window_id)?;
        let composed = gwt::window_state::compose_window_state_with_active_session(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
        );
        let address = self.window_lookup.get(window_id)?.clone();
        if let Some(tab) = self.tab_mut(&address.tab_id) {
            let _ = tab.workspace.set_status(&address.raw_id, composed);
        }
        Some(composed)
    }

    fn remove_window_state_tracking(&mut self, window_id: &str) {
        self.window_pty_statuses.remove(window_id);
        self.window_hook_states.remove(window_id);
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
    /// and a materializable worktree) — the same gate the startup auto-resume
    /// queue applies to placeholder-backed sessions (SPEC-1921 Phase 65).
    fn restored_window_is_exact_auto_resume_candidate(
        &self,
        window: &gwt::PersistedWindowState,
    ) -> bool {
        let Some(session_id) = window.session_id.as_deref() else {
            return false;
        };
        let path = self.sessions_dir.join(format!("{session_id}.toml"));
        gwt_agent::Session::load_and_migrate(&path).is_ok_and(|session| {
            session.supports_exact_session_resume()
                && session.exact_resume_worktree_materializable()
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
                        tab.workspace.persistable_state(),
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
                });
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

fn work_branch_has_dirty_worktree(target: &WorkBranchScanTarget) -> bool {
    target.worktree_paths.iter().any(|path| {
        gwt_git::diff::get_status(path)
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
    })
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
