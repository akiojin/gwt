//! TUI Application with Elm Architecture

#![allow(dead_code)] // TUI application components for future expansion

mod ai_wizard;

use super::widgets::ProgressModalState;
use crate::{
    prepare_launch_plan, InstallPlan, LaunchPlan, LaunchProgress, ProgressStepKind, StepStatus,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::agent::codex::supports_collaboration_modes;
use gwt_core::ai::{
    summarize_session, AIClient, AIError, AgentHistoryStore, AgentType, ChatMessage,
    ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, ModelInfo, OpenCodeSessionParser,
    SessionParseError, SessionParser,
};
use gwt_core::config::get_branch_tool_history;
use gwt_core::config::{
    get_claude_settings_path, is_gwt_hooks_registered, is_gwt_marketplace_registered,
    is_temporary_execution, register_gwt_hooks, reregister_gwt_hooks, save_session_entry,
    setup_gwt_plugin, AISettings, CustomCodingAgent, Profile, ProfilesConfig, ResolvedAISettings,
    ToolSessionEntry,
};
use gwt_core::docker::port::PortAllocator;
use gwt_core::docker::{ContainerStatus, DockerManager};
use gwt_core::error::GwtError;
use gwt_core::git::{
    detect_repo_type, get_header_context, Branch, PrCache, Remote, RepoType, Repository,
};
use gwt_core::tmux::{
    break_pane, compute_equal_splits, get_current_session, group_panes_by_left,
    join_pane_to_target, kill_pane, launcher, list_pane_geometries, resize_pane_height,
    resize_pane_width, AgentPane, PaneColumn, PaneGeometry, SplitDirection,
};
use gwt_core::worktree::WorktreeManager;
use gwt_core::TmuxMode;
use ratatui::{prelude::*, widgets::*};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, error, info, warn};

const BRANCH_LIST_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
const SESSION_POLL_INTERVAL: Duration = Duration::from_secs(60);
const SESSION_SUMMARY_QUIET_PERIOD: Duration = Duration::from_secs(5);
const FAST_EXIT_THRESHOLD_SECS: u64 = 2;
const AGENT_SYSTEM_PROMPT: &str = "You are the master agent. Analyze tasks and propose a plan.";
const FOOTER_VISIBLE_HEIGHT: usize = 1;
const FOOTER_SCROLL_TICKS_PER_LINE: u16 = 12; // 3.0s per line (tick = 250ms)
const FOOTER_SCROLL_PAUSE_TICKS: u16 = 0; // no pause at ends

use super::screens::branch_list::{
    BranchSummaryRequest, BranchSummaryUpdate, PrInfo, WorktreeStatus,
};
use super::screens::environment::EditField;
use super::screens::git_view::{build_git_view_data, build_git_view_data_no_worktree, GitViewData};
use super::screens::pane_list::PaneListState;
use super::screens::split_layout::{calculate_split_layout, SplitLayoutState};
use super::screens::worktree_create::WorktreeCreateStep;
use super::screens::{
    collect_os_env, render_agent_mode, render_ai_wizard, render_branch_list, render_clone_wizard,
    render_confirm, render_environment, render_error_with_queue, render_git_view, render_help,
    render_logs, render_migration_dialog, render_port_select, render_profiles,
    render_service_select, render_settings, render_wizard, render_worktree_create, AIWizardState,
    AgentMessage, AgentModeState, AgentRole, BranchItem, BranchListState, BranchType,
    CloneWizardState, CloneWizardStep, CodingAgent, ConfirmState, EnvironmentState, ErrorQueue,
    ErrorState, ExecutionMode, GitViewCache, GitViewState, HelpState, LogsState,
    MigrationDialogPhase, MigrationDialogState, PortSelectState, ProfilesState,
    QuickStartDockerSettings, QuickStartEntry, ReasoningLevel, ServiceSelectState, SettingsState,
    WizardConfirmResult, WizardState, WizardStep, WorktreeCreateState,
};
// log_gwt_error is available for use when GwtError types are available

fn resolve_orphaned_agent_name(
    fallback_name: &str,
    session_entry: Option<&ToolSessionEntry>,
) -> String {
    if let Some(entry) = session_entry {
        if !entry.tool_id.trim().is_empty() {
            return entry.tool_id.clone();
        }
    }

    let trimmed = fallback_name.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_branch_name(name: &str) -> String {
    if let Some(stripped) = name.strip_prefix("remotes/") {
        if let Some((_, branch)) = stripped.split_once('/') {
            return branch.to_string();
        }
        return stripped.to_string();
    }
    name.to_string()
}

fn format_ai_error(err: AIError) -> String {
    err.to_string()
}

fn background_window_name(branch_name: &str) -> String {
    branch_name.to_string()
}

fn normalize_branch_name_for_history(branch_name: &str) -> Cow<'_, str> {
    if let Some(stripped) = branch_name.strip_prefix("remotes/") {
        if let Some((_, name)) = stripped.split_once('/') {
            return Cow::Owned(name.to_string());
        }
        return Cow::Owned(stripped.to_string());
    }
    Cow::Borrowed(branch_name)
}

fn resolve_existing_worktree_path(
    branch_name: &str,
    branches: &[BranchItem],
    create_new_branch: bool,
) -> Option<PathBuf> {
    if create_new_branch {
        return None;
    }

    let normalized = normalize_branch_name_for_history(branch_name);
    let find_path = |name: &str| {
        branches
            .iter()
            .find(|item| {
                item.name == name
                    && item.has_worktree
                    && item.worktree_status == WorktreeStatus::Active
            })
            .and_then(|item| item.worktree_path.as_ref())
            .map(PathBuf::from)
    };

    find_path(branch_name).or_else(|| {
        if normalized.as_ref() != branch_name {
            find_path(normalized.as_ref())
        } else {
            None
        }
    })
}

fn apply_last_tool_usage(
    item: &mut BranchItem,
    repo_root: &Path,
    tool_usage_map: &HashMap<String, ToolSessionEntry>,
    agent_history: &AgentHistoryStore,
) {
    let lookup = normalize_branch_name_for_history(&item.name);
    if let Some(entry) = tool_usage_map.get(lookup.as_ref()) {
        item.last_tool_usage = Some(entry.format_tool_usage());
        item.last_tool_id = Some(entry.tool_id.clone());
        item.last_session_id = entry.session_id.clone();
        let session_timestamp = entry.timestamp / 1000;
        let git_timestamp = item.last_commit_timestamp.unwrap_or(0);
        item.last_commit_timestamp = Some(session_timestamp.max(git_timestamp));
        return;
    }

    if let Some(history_entry) = agent_history.get(repo_root, lookup.as_ref()) {
        item.last_tool_usage = Some(history_entry.agent_label.clone());
        item.last_tool_id = Some(history_entry.agent_id.clone());
    }
}

fn session_poll_due(last_poll: Option<Instant>, now: Instant) -> bool {
    last_poll
        .map(|last| now.duration_since(last) >= SESSION_POLL_INTERVAL)
        .unwrap_or(true)
}

fn defer_poll_for_quiet(now: Instant) -> Instant {
    if SESSION_POLL_INTERVAL > SESSION_SUMMARY_QUIET_PERIOD {
        now - (SESSION_POLL_INTERVAL - SESSION_SUMMARY_QUIET_PERIOD)
    } else {
        now
    }
}

fn session_file_is_quiet(mtime: SystemTime, now: SystemTime) -> bool {
    match now.duration_since(mtime) {
        Ok(elapsed) => elapsed >= SESSION_SUMMARY_QUIET_PERIOD,
        Err(_) => false,
    }
}

fn session_parser_for_tool(tool_id: &str) -> Option<Box<dyn SessionParser>> {
    let agent = AgentType::from_tool_id(tool_id)?;
    match agent {
        AgentType::ClaudeCode => ClaudeSessionParser::with_default_home()
            .map(|parser| Box::new(parser) as Box<dyn SessionParser>),
        AgentType::CodexCli => CodexSessionParser::with_default_home()
            .map(|parser| Box::new(parser) as Box<dyn SessionParser>),
        AgentType::GeminiCli => GeminiSessionParser::with_default_home()
            .map(|parser| Box::new(parser) as Box<dyn SessionParser>),
        AgentType::OpenCode => OpenCodeSessionParser::with_default_home()
            .map(|parser| Box::new(parser) as Box<dyn SessionParser>),
    }
}

/// Configuration for launching a coding agent after TUI exits
#[derive(Debug, Clone)]
pub struct AgentLaunchConfig {
    /// Repository root (bare repository path for SPEC-a70a1ece)
    pub repo_root: PathBuf,
    /// Worktree path where agent should run
    pub worktree_path: PathBuf,
    /// Branch name
    pub branch_name: String,
    /// Coding agent to launch (builtin)
    pub agent: CodingAgent,
    /// Custom agent configuration (SPEC-71f2742d)
    pub custom_agent: Option<CustomCodingAgent>,
    /// Model to use
    pub model: Option<String>,
    /// Reasoning level (Codex only)
    pub reasoning_level: Option<ReasoningLevel>,
    /// Version to use
    pub version: String,
    /// Execution mode
    pub execution_mode: ExecutionMode,
    /// Session ID for resume/continue when available
    pub session_id: Option<String>,
    /// Skip permission prompts
    pub skip_permissions: bool,
    /// Environment variables to apply
    pub env: Vec<(String, String)>,
    /// Environment variables to remove
    pub env_remove: Vec<String>,
    /// Auto install dependencies before launching agent
    pub auto_install_deps: bool,
    /// Enable collaboration_modes (Codex v0.91.0+, SPEC-fdebd681)
    pub collaboration_modes: bool,
}

impl AgentLaunchConfig {
    /// Check if this config is for a custom agent
    pub fn is_custom(&self) -> bool {
        self.custom_agent.is_some()
    }

    /// Get the agent ID (builtin or custom)
    pub fn agent_id(&self) -> String {
        if let Some(ref custom) = self.custom_agent {
            custom.id.clone()
        } else {
            self.agent.id().to_string()
        }
    }

    /// Get the display name (builtin or custom)
    pub fn display_name(&self) -> String {
        if let Some(ref custom) = self.custom_agent {
            custom.display_name.clone()
        } else {
            self.agent.label().to_string()
        }
    }
}

#[derive(Debug, Clone)]
pub struct TuiEntryContext {
    status_message: Option<String>,
    error_message: Option<String>,
    /// Repository root for re-entry after agent termination (SPEC-a70a1ece)
    repo_root: Option<PathBuf>,
}

impl TuiEntryContext {
    pub fn success(message: String) -> Self {
        Self {
            status_message: Some(message),
            error_message: None,
            repo_root: None,
        }
    }

    pub fn warning(message: String) -> Self {
        Self {
            status_message: Some(message),
            error_message: None,
            repo_root: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            status_message: None,
            error_message: Some(message),
            repo_root: None,
        }
    }

    /// Set repo_root for single mode re-entry (SPEC-a70a1ece)
    pub fn with_repo_root(mut self, repo_root: PathBuf) -> Self {
        self.repo_root = Some(repo_root);
        self
    }
}

struct MouseClick {
    index: usize,
    at: Instant,
}

struct PrTitleUpdate {
    info: HashMap<String, PrInfo>,
}

struct SafetyUpdate {
    branch: String,
    has_unpushed: bool,
    is_unmerged: bool,
    safe_to_cleanup: bool,
}

struct SafetyCheckTarget {
    branch: String,
    upstream: Option<String>,
}

struct WorktreeStatusUpdate {
    branch: String,
    worktree_status: WorktreeStatus,
    has_changes: bool,
    failed: bool,
}

struct WorktreeStatusTarget {
    branch: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct CleanupPlanItem {
    branch: String,
    force_remove: bool,
}

#[derive(Debug)]
enum CleanupUpdate {
    BranchStarted { branch: String },
    BranchFinished { branch: String, success: bool },
    Completed { deleted: usize, errors: Vec<String> },
}

struct SessionSummaryTask {
    branch: String,
    session_id: String,
    tool_id: String,
}

struct SessionSummaryUpdate {
    branch: String,
    session_id: String,
    summary: Option<gwt_core::ai::SessionSummary>,
    error: Option<String>,
    warning: Option<String>,
    mtime: Option<std::time::SystemTime>,
    missing: bool,
}

struct AgentModeUpdate {
    response: Option<String>,
    error: Option<String>,
}

struct AiWizardUpdate {
    result: Result<Vec<ModelInfo>, AIError>,
}

struct BranchListUpdate {
    branches: Vec<BranchItem>,
    branch_names: Vec<String>,
    worktree_targets: Vec<WorktreeStatusTarget>,
    safety_targets: Vec<SafetyCheckTarget>,
    base_branches: Vec<String>,
    base_branch: String,
    base_branch_exists: bool,
    total_count: usize,
    active_count: usize,
}

/// Update for GitView cache (SPEC-1ea18899 FR-050)
struct GitViewCacheUpdate {
    branch: String,
    data: GitViewData,
}

/// Update for GitView PR info (SPEC-1ea18899)
struct GitViewPrUpdate {
    branch: String,
    info: Option<PrInfo>,
}

struct LaunchRequest {
    branch_name: String,
    create_new_branch: bool,
    base_branch: Option<String>,
    /// Existing worktree path to reuse (Quick Start or known worktree)
    existing_worktree_path: Option<PathBuf>,
    agent: CodingAgent,
    /// Custom agent configuration (SPEC-71f2742d)
    custom_agent: Option<CustomCodingAgent>,
    model: Option<String>,
    reasoning_level: Option<ReasoningLevel>,
    version: String,
    execution_mode: ExecutionMode,
    session_id: Option<String>,
    skip_permissions: bool,
    /// Collaboration modes (Codex v0.91.0+, SPEC-fdebd681)
    collaboration_modes: bool,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    auto_install_deps: bool,
    /// SPEC-e4798383 US6: Selected GitHub Issue for branch linking
    selected_issue: Option<gwt_core::git::GitHubIssue>,
}

enum LaunchUpdate {
    Progress(LaunchProgress),
    /// Progress step update for modal display (FR-041)
    ProgressStep {
        kind: ProgressStepKind,
        status: StepStatus,
    },
    /// Progress step error (FR-052)
    ProgressStepError {
        kind: ProgressStepKind,
        message: String,
    },
    WorktreeReady {
        branch: String,
        path: PathBuf,
    },
    Ready(Box<LaunchPlan>),
    Failed(String),
}

/// Application state (Model in Elm Architecture)
pub struct Model {
    /// Whether the app should quit
    should_quit: bool,
    /// Ctrl+C press count
    ctrl_c_count: u8,
    /// Last Ctrl+C press time
    last_ctrl_c: Option<Instant>,
    /// Last mouse click for double click detection
    last_mouse_click: Option<MouseClick>,
    /// Current screen
    screen: Screen,
    /// Screen stack for navigation
    screen_stack: Vec<Screen>,
    /// Repository root
    repo_root: PathBuf,
    /// Branch list state
    branch_list: BranchListState,
    /// Worktree create state
    worktree_create: WorktreeCreateState,
    /// Settings state
    settings: SettingsState,
    /// Logs state
    logs: LogsState,
    /// Help state
    help: HelpState,
    /// Agent mode state
    agent_mode: AgentModeState,
    /// Confirm dialog state
    confirm: ConfirmState,
    /// Error queue for managing multiple errors
    error_queue: ErrorQueue,
    /// Profiles state
    profiles: ProfilesState,
    /// Profiles configuration
    profiles_config: ProfilesConfig,
    /// Environment variables state
    environment: EnvironmentState,
    /// Docker service selection state
    service_select: ServiceSelectState,
    /// Docker port conflict resolution state
    port_select: PortSelectState,
    /// Wizard popup state
    wizard: WizardState,
    /// AI settings wizard state (FR-100)
    ai_wizard: AIWizardState,
    /// Status message
    status_message: Option<String>,
    /// Status message timestamp (for auto-clear)
    status_message_time: Option<Instant>,
    /// Footer scroll offset (top line index)
    footer_scroll_offset: usize,
    /// Footer scroll direction (1 = down, -1 = up)
    footer_scroll_dir: i8,
    /// Footer scroll tick counter
    footer_scroll_tick: u16,
    /// Footer scroll pause counter at ends
    footer_scroll_pause: u16,
    /// Footer line count (last computed)
    footer_line_count: usize,
    /// Footer width (last computed)
    footer_last_width: u16,
    /// Launch progress message (not auto-cleared)
    launch_status: Option<String>,
    /// Launch preparation update receiver
    launch_rx: Option<Receiver<LaunchUpdate>>,
    /// Whether launch preparation is in progress
    launch_in_progress: bool,
    /// Is offline
    is_offline: bool,
    /// Active worktree count
    active_count: usize,
    /// Total branch count
    total_count: usize,
    /// Pending agent launch plan (set when wizard completes)
    pending_agent_launch: Option<LaunchPlan>,
    /// Pending unsafe branch selection (FR-029b)
    pending_unsafe_selection: Option<String>,
    /// Pending agent termination branch (FR-040)
    pending_agent_termination: Option<String>,
    /// Pending cleanup branches (FR-010)
    pending_cleanup_branches: Vec<String>,
    /// Pending hook setup (SPEC-861d8cdf T-104)
    pending_hook_setup: bool,
    /// Pending plugin setup (SPEC-f8dab6e2 T-110)
    pending_plugin_setup: bool,
    /// Pending launch plan for plugin setup (SPEC-f8dab6e2 T-110)
    pending_plugin_setup_launch: Option<LaunchPlan>,
    /// Pending launch plan for Docker service selection
    pending_service_select: Option<PendingServiceSelect>,
    /// Pending launch plan for Docker build selection
    pending_build_select: Option<PendingBuildSelect>,
    /// Pending launch plan for Docker recreate selection
    pending_recreate_select: Option<PendingRecreateSelect>,
    /// Pending launch plan for Docker cleanup selection
    pending_cleanup_select: Option<PendingCleanupSelect>,
    /// Pending launch plan for Docker port selection
    pending_port_select: Option<PendingPortSelect>,
    /// Pending launch plan for Docker host fallback confirmation
    pending_docker_host_launch: Option<LaunchPlan>,
    /// Pending Quick Start Docker settings (applied at launch)
    pending_quick_start_docker: Option<QuickStartDockerSettings>,
    /// Branch list update receiver
    branch_list_rx: Option<Receiver<BranchListUpdate>>,
    /// Branch summary update receiver
    branch_summary_rx: Option<Receiver<BranchSummaryUpdate>>,
    /// PR title update receiver
    pr_title_rx: Option<Receiver<PrTitleUpdate>>,
    /// Safety check update receiver
    safety_rx: Option<Receiver<SafetyUpdate>>,
    /// Worktree status update receiver
    worktree_status_rx: Option<Receiver<WorktreeStatusUpdate>>,
    /// Cleanup update receiver
    cleanup_rx: Option<Receiver<CleanupUpdate>>,
    /// Session summary update sender
    session_summary_tx: Option<Sender<SessionSummaryUpdate>>,
    /// Session summary update receiver
    session_summary_rx: Option<Receiver<SessionSummaryUpdate>>,
    /// Agent mode update sender
    agent_mode_tx: Option<Sender<AgentModeUpdate>>,
    /// Agent mode update receiver
    agent_mode_rx: Option<Receiver<AgentModeUpdate>>,
    /// AI wizard model fetch receiver
    ai_wizard_rx: Option<Receiver<AiWizardUpdate>>,
    /// Tmux mode (Single or Multi)
    tmux_mode: TmuxMode,
    /// Tmux session name (when in multi mode)
    tmux_session: Option<String>,
    /// The pane ID where gwt is running (for splitting)
    gwt_pane_id: Option<String>,
    /// Launched agent pane IDs (visible panes only)
    agent_panes: Vec<String>,
    /// Pane list state for tmux multi-mode
    pane_list: PaneListState,
    /// Split layout state for tmux multi-mode
    split_layout: SplitLayoutState,
    /// Last time pane list was updated (for 1-second polling)
    last_pane_update: Option<Instant>,
    /// Last time spinner was updated (for 250ms refresh)
    last_spinner_update: Option<Instant>,
    /// Last time session polling ran (60s interval)
    last_session_poll: Option<Instant>,
    /// Session poll deferred while generation is in-flight
    session_poll_deferred: bool,
    /// Agent history store for persisting agent usage per branch (FR-088)
    agent_history: AgentHistoryStore,
    /// Progress modal state for worktree preparation (FR-041)
    progress_modal: Option<ProgressModalState>,
    /// Startup branch name for header display (SPEC-a70a1ece FR-103)
    /// This is fixed at startup and does not change when selecting other branches.
    startup_branch: Option<String>,
    /// Repository type detected at startup (SPEC-a70a1ece US2)
    repo_type: RepoType,
    /// Clone wizard state (SPEC-a70a1ece US3)
    clone_wizard: CloneWizardState,
    /// Bare repository name when inside a bare-based worktree (SPEC-a70a1ece T506)
    bare_name: Option<String>,
    /// Migration dialog state (SPEC-a70a1ece US7 T705-T710)
    migration_dialog: MigrationDialogState,
    /// Migration result receiver (for background migration)
    migration_rx: Option<mpsc::Receiver<Result<(), gwt_core::migration::MigrationError>>>,
    /// Path to bare repository when in bare project (SPEC-a70a1ece)
    bare_repo_path: Option<PathBuf>,
    /// GitView state (SPEC-1ea18899)
    git_view: GitViewState,
    /// GitView cache for all branches (SPEC-1ea18899 FR-050)
    git_view_cache: GitViewCache,
    /// GitView cache update receiver (SPEC-1ea18899)
    git_view_cache_rx: Option<Receiver<GitViewCacheUpdate>>,
    /// GitView PR update receiver (SPEC-1ea18899)
    git_view_pr_rx: Option<Receiver<GitViewPrUpdate>>,
}

#[derive(Debug, Clone)]
struct PendingServiceSelect {
    plan: LaunchPlan,
    services: Vec<String>,
}

#[derive(Debug, Clone)]
struct PendingBuildSelect {
    plan: LaunchPlan,
    service: Option<String>,
    force_host: bool,
    force_recreate: bool,
    quick_start_keep: Option<bool>,
}

#[derive(Debug, Clone)]
struct PendingRecreateSelect {
    plan: LaunchPlan,
    service: Option<String>,
    force_host: bool,
}

#[derive(Debug, Clone)]
struct PendingCleanupSelect {
    plan: LaunchPlan,
    service: Option<String>,
    force_host: bool,
    force_recreate: bool,
    build: bool,
}

#[derive(Debug, Clone)]
struct PendingPortSelect {
    plan: LaunchPlan,
    service: Option<String>,
    force_host: bool,
    build: bool,
    force_recreate: bool,
    stop_on_exit: bool,
}

enum ServiceSelectionDecision {
    Proceed {
        service: Option<String>,
        force_host: bool,
    },
    AwaitSelection,
}

/// Screen types
#[derive(Clone, Debug)]
pub enum Screen {
    BranchList,
    AgentMode,
    WorktreeCreate,
    Settings,
    Logs,
    Help,
    Confirm,
    Error,
    Profiles,
    Environment,
    ServiceSelect,
    PortSelect,
    /// AI settings wizard (FR-100)
    AISettingsWizard,
    /// Clone wizard for empty/non-repo directories (SPEC-a70a1ece US3)
    CloneWizard,
    /// Migration dialog for .worktrees/ method conversion (SPEC-a70a1ece US7)
    MigrationDialog,
    /// Git status view for selected branch (SPEC-1ea18899)
    GitView,
}

/// Messages (Events in Elm Architecture)
#[derive(Debug)]
pub enum Message {
    Quit,
    CtrlC,
    NavigateTo(Screen),
    NavigateBack,
    Tick,
    SelectNext,
    SelectPrev,
    PageUp,
    PageDown,
    GoHome,
    GoEnd,
    Enter,
    Char(char),
    Backspace,
    CursorLeft,
    CursorRight,
    RefreshData,
    Tab,
    CycleFilter,
    ToggleSearch,
    /// Toggle filter mode in branch list
    ToggleFilterMode,
    /// Cycle view mode (All/Local/Remote)
    CycleViewMode,
    /// Cycle sort mode (Default/Name/Updated)
    CycleSortMode,
    /// Toggle branch selection
    ToggleSelection,
    /// Space key for selection
    Space,
    /// Open wizard for selected branch
    OpenWizard,
    /// Open wizard for new branch
    OpenWizardNewBranch,
    /// Wizard: select next item
    WizardNext,
    /// Wizard: select prev item
    WizardPrev,
    /// Wizard: confirm current step
    WizardConfirm,
    /// Wizard: go back or close
    WizardBack,
    /// Copy selected log to clipboard
    CopyLogToClipboard,
    /// FR-095: Hide active agent pane (ESC key in branch list)
    HideActiveAgentPane,
    /// FR-040: Confirm agent termination (d key)
    ConfirmAgentTermination,
    /// Execute agent termination after confirmation
    ExecuteAgentTermination,
    /// FR-102g: Manually re-register Claude Code hooks (u key)
    ReregisterHooks,
}

impl Model {
    /// Create a new model
    pub fn new() -> Self {
        Self::new_with_context(None)
    }

    pub fn new_with_context(context: Option<TuiEntryContext>) -> Self {
        // SPEC-a70a1ece: Use repo_root from context if available (single mode re-entry)
        let repo_root = context
            .as_ref()
            .and_then(|ctx| ctx.repo_root.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        debug!(
            category = "tui",
            repo_root = %repo_root.display(),
            "Initializing TUI model"
        );

        // SPEC-a70a1ece: First check if there's a *.git bare repository in the directory
        // This takes priority because parent directory's .git might be detected otherwise
        let (repo_type, bare_repo_path) =
            if let Some(bare_path) = gwt_core::git::find_bare_repo_in_dir(&repo_root) {
                debug!(
                    category = "tui",
                    bare_path = %bare_path.display(),
                    "Found bare repository in directory, treating as bare project"
                );
                (RepoType::Bare, Some(bare_path))
            } else {
                (detect_repo_type(&repo_root), None)
            };

        // SPEC-a70a1ece: Capture startup context
        // For bare projects, use the bare repo path; otherwise use repo_root
        let (startup_branch, bare_name) = if let Some(ref bare_path) = bare_repo_path {
            // Bare project: no startup branch, get bare name from path
            let name = bare_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            (None, name)
        } else {
            let header_ctx = get_header_context(&repo_root);
            (header_ctx.branch_name, header_ctx.bare_name)
        };

        let (agent_mode_tx, agent_mode_rx) = mpsc::channel();

        let mut model = Self {
            should_quit: false,
            ctrl_c_count: 0,
            last_ctrl_c: None,
            last_mouse_click: None,
            screen: Screen::BranchList,
            screen_stack: Vec::new(),
            repo_root,
            branch_list: BranchListState::new(),
            worktree_create: WorktreeCreateState::new(),
            settings: SettingsState::new(),
            logs: LogsState::new(),
            help: HelpState::new(),
            agent_mode: AgentModeState::new(),
            confirm: ConfirmState::new(),
            error_queue: ErrorQueue::new(),
            profiles: ProfilesState::new(),
            profiles_config: ProfilesConfig::default(),
            environment: EnvironmentState::new(),
            service_select: ServiceSelectState::new(),
            port_select: PortSelectState::default(),
            wizard: WizardState::new(),
            ai_wizard: AIWizardState::new(),
            status_message: None,
            status_message_time: None,
            footer_scroll_offset: 0,
            footer_scroll_dir: 1,
            footer_scroll_tick: 0,
            footer_scroll_pause: 0,
            footer_line_count: 0,
            footer_last_width: 0,
            launch_status: None,
            launch_rx: None,
            launch_in_progress: false,
            is_offline: false,
            active_count: 0,
            total_count: 0,
            pending_agent_launch: None,
            pending_unsafe_selection: None,
            pending_agent_termination: None,
            pending_cleanup_branches: Vec::new(),
            pending_hook_setup: false,
            pending_plugin_setup: false,
            pending_plugin_setup_launch: None,
            pending_service_select: None,
            pending_build_select: None,
            pending_recreate_select: None,
            pending_cleanup_select: None,
            pending_port_select: None,
            pending_docker_host_launch: None,
            pending_quick_start_docker: None,
            branch_list_rx: None,
            branch_summary_rx: None,
            pr_title_rx: None,
            safety_rx: None,
            worktree_status_rx: None,
            cleanup_rx: None,
            session_summary_tx: None,
            session_summary_rx: None,
            agent_mode_tx: Some(agent_mode_tx),
            agent_mode_rx: Some(agent_mode_rx),
            ai_wizard_rx: None,
            tmux_mode: TmuxMode::detect(),
            tmux_session: None,
            gwt_pane_id: None,
            agent_panes: Vec::new(),
            pane_list: PaneListState::new(),
            split_layout: SplitLayoutState::new(),
            last_pane_update: None,
            last_spinner_update: None,
            last_session_poll: None,
            session_poll_deferred: false,
            agent_history: AgentHistoryStore::load().unwrap_or_default(),
            progress_modal: None,
            startup_branch,
            repo_type,
            clone_wizard: CloneWizardState::new(),
            bare_name,
            migration_dialog: MigrationDialogState::default(),
            migration_rx: None,
            bare_repo_path,
            git_view: GitViewState::default(),
            git_view_cache: GitViewCache::new(),
            git_view_cache_rx: None,
            git_view_pr_rx: None,
        };

        model
            .branch_list
            .set_repo_web_url(resolve_repo_web_url(&model.repo_root));

        let (session_tx, session_rx) = mpsc::channel();
        model.session_summary_tx = Some(session_tx);
        model.session_summary_rx = Some(session_rx);

        // Initialize tmux session if in multi mode
        if model.tmux_mode.is_multi() {
            // Use the current tmux session, not a generated one
            model.tmux_session = get_current_session();
            // Capture the gwt pane ID for splitting
            model.gwt_pane_id = gwt_core::tmux::get_current_pane_id();
            debug!(
                category = "tui",
                mode = %model.tmux_mode,
                session = ?model.tmux_session,
                gwt_pane_id = ?model.gwt_pane_id,
                "Tmux multi-mode detected"
            );
        }

        // Load initial data
        model.refresh_data();

        // Reconnect to orphaned agent panes (FR-060~062)
        model.reconnect_orphaned_panes();

        model.apply_entry_context(context);

        if let Some(settings_path) = get_claude_settings_path() {
            // SPEC-861d8cdf T-107: Re-register hooks on startup to sync gwt path
            if let Err(e) = reregister_gwt_hooks(&settings_path) {
                warn!(
                    category = "tui",
                    error = %e,
                    "Failed to re-register gwt hooks"
                );
            }

            // SPEC-861d8cdf T-104: Check if hook setup is needed on first startup
            // FR-102i: Show warning if running from temporary execution environment
            if model.tmux_mode.is_multi() && !is_gwt_hooks_registered(&settings_path) {
                model.pending_hook_setup = true;
                model.confirm = if let Some(exe_path) = is_temporary_execution() {
                    ConfirmState::hook_setup_with_warning(&exe_path)
                } else {
                    ConfirmState::hook_setup()
                };
                model.screen_stack.push(model.screen.clone());
                model.screen = Screen::Confirm;
            }
        }

        // SPEC-a70a1ece T310-T311: Show clone wizard for empty/non-repo directories
        if matches!(model.repo_type, RepoType::Empty | RepoType::NonRepo) {
            debug!(
                category = "tui",
                repo_type = ?model.repo_type,
                "Empty or non-repo directory detected, showing clone wizard"
            );
            model.screen = Screen::CloneWizard;
        }

        // SPEC-a70a1ece FR-200: Show migration dialog for ALL normal repositories
        // (regardless of whether .worktrees/ exists)
        debug!(
            category = "tui",
            repo_type = ?model.repo_type,
            repo_root = %model.repo_root.display(),
            "Checking for migration dialog eligibility"
        );
        if model.repo_type == RepoType::Normal {
            debug!(
                category = "tui",
                repo_root = %model.repo_root.display(),
                "Normal repository detected, showing migration dialog for bare conversion"
            );
            // Create migration config
            let bare_repo_name =
                gwt_core::migration::derive_bare_repo_name(&model.repo_root.display().to_string());
            // SPEC-a70a1ece FR-150: target_root is the same as source_root
            // Migration creates bare repo and worktrees INSIDE the original repo directory
            let target_root = model.repo_root.clone();
            let config = gwt_core::migration::MigrationConfig::new(
                model.repo_root.clone(),
                target_root,
                bare_repo_name,
            );
            model.migration_dialog = MigrationDialogState::new(config);
            model.screen = Screen::MigrationDialog;
        }

        model
    }

    /// Refresh data from repository
    fn refresh_data(&mut self) {
        debug!(
            category = "tui",
            repo_root = %self.repo_root.display(),
            "Refreshing data from repository"
        );

        let settings = gwt_core::config::Settings::load(&self.repo_root).unwrap_or_default();
        self.settings = SettingsState::new().with_settings(settings.clone());

        // Load logs from configured log directory
        let log_dir = settings.log_dir(&self.repo_root);
        if log_dir.exists() {
            let reader = gwt_core::logging::LogReader::new(&log_dir);
            if let Ok(entries) = reader.read_latest(100) {
                // Convert gwt_core LogEntry to TUI LogEntry
                let tui_entries: Vec<super::screens::logs::LogEntry> = entries
                    .into_iter()
                    .map(|e| {
                        let message = e.message().to_string();
                        let category = e.category().map(|s| s.to_string());
                        // Convert extra fields to HashMap<String, String>
                        let extra: std::collections::HashMap<String, String> = e
                            .fields
                            .extra
                            .iter()
                            .map(|(k, v)| (k.clone(), v.to_string()))
                            .collect();
                        super::screens::logs::LogEntry {
                            timestamp: e.timestamp,
                            level: e.level,
                            message,
                            target: e.target,
                            category,
                            extra,
                        }
                    })
                    .collect();
                self.logs = LogsState::new().with_entries(tui_entries);
            }
        }

        self.load_profiles();
        // SPEC-1ea18899 FR-052: Clear GitView cache on refresh
        self.git_view_cache.clear();
        self.start_branch_list_refresh(settings);
    }

    fn start_branch_list_refresh(&mut self, settings: gwt_core::config::Settings) {
        let cleanup_snapshot = self.branch_list.cleanup_snapshot();
        let sort_mode = self.branch_list.sort_mode;
        self.pr_title_rx = None;
        self.git_view_pr_rx = None;
        self.safety_rx = None;
        self.worktree_status_rx = None;
        self.branch_list_rx = None;
        self.total_count = 0;
        self.active_count = 0;

        let mut branch_list = BranchListState::new();
        branch_list.sort_mode = sort_mode;
        branch_list.active_profile = self.profiles_config.active.clone();
        branch_list.ai_enabled = self.active_ai_enabled();
        branch_list.session_summary_enabled = self.active_session_summary_enabled();
        branch_list.working_directory = Some(self.repo_root.display().to_string());
        branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
        branch_list.set_loading(true);
        branch_list.restore_cleanup_snapshot(&cleanup_snapshot);
        self.branch_list = branch_list;

        let repo_root = self.repo_root.clone();
        let repo_type = self.repo_type;
        let bare_repo_path = self.bare_repo_path.clone();
        let configured_base_branch = settings.default_base_branch.clone();
        let agent_history = self.agent_history.clone();
        let (tx, rx) = mpsc::channel();
        self.branch_list_rx = Some(rx);

        thread::spawn(move || {
            // SPEC-a70a1ece: Use bare repo path for git commands in bare projects
            let git_path = bare_repo_path.as_ref().unwrap_or(&repo_root);
            let (base_branch, base_branch_exists) =
                resolve_safety_base(git_path, &configured_base_branch);
            let worktrees = WorktreeManager::new(git_path)
                .ok()
                .and_then(|manager| manager.list_basic().ok())
                .unwrap_or_default();
            let all_branches = Branch::list_basic(git_path).unwrap_or_default();

            // SPEC-a70a1ece FR-170/171: For bare repos, only show worktree branches as Local
            let worktree_branch_names: HashSet<String> = worktrees
                .iter()
                .filter_map(|wt| wt.branch.clone())
                .collect();

            let branches: Vec<_> = if repo_type == RepoType::Bare {
                // Bare repo: Local = only branches with worktrees
                all_branches
                    .into_iter()
                    .filter(|b| worktree_branch_names.contains(&b.name))
                    .collect()
            } else {
                all_branches
            };

            // SPEC-a70a1ece FR-171: For bare repos, branches without worktrees go to Remote
            let remote_branches = if repo_type == RepoType::Bare {
                // Get all branches and filter out ones with worktrees
                let all_for_remote = Branch::list_basic(git_path).unwrap_or_default();
                all_for_remote
                    .into_iter()
                    .filter(|b| !worktree_branch_names.contains(&b.name))
                    .collect::<Vec<_>>()
            } else {
                Branch::list_remote(git_path).unwrap_or_default()
            };

            let mut remote_display_branches = Vec::new();
            for mut branch in remote_branches {
                if repo_type != RepoType::Bare && !branch.name.starts_with("remotes/") {
                    branch.name = format!("remotes/{}", branch.name);
                }
                remote_display_branches.push(branch);
            }
            let worktree_targets: Vec<WorktreeStatusTarget> = worktrees
                .iter()
                .filter_map(|wt| {
                    wt.branch.clone().map(|branch| WorktreeStatusTarget {
                        branch,
                        path: wt.path.clone(),
                    })
                })
                .collect();

            // Load tool usage from TypeScript session file (FR-070)
            let tool_usage_map = gwt_core::config::get_last_tool_usage_map(&repo_root);
            let mut safety_targets = Vec::new();

            let mut branch_items: Vec<BranchItem> = branches
                .iter()
                .map(|b| {
                    let mut item = BranchItem::from_branch_minimal(b, &worktrees);

                    // Set tool usage from TypeScript session history or agent history (FR-070/088)
                    apply_last_tool_usage(&mut item, &repo_root, &tool_usage_map, &agent_history);

                    // Safety check short-circuit: use immediate signals first
                    if item.branch_type == BranchType::Local {
                        if !base_branch_exists {
                            item.safe_to_cleanup = Some(false);
                        } else {
                            // Check safety even without upstream (FR-004b)
                            item.safe_to_cleanup = None;
                            safety_targets.push(SafetyCheckTarget {
                                branch: b.name.clone(),
                                upstream: b.upstream.clone(),
                            });
                        }
                    }

                    item.update_safety_status();
                    item
                })
                .collect();
            branch_items.extend(remote_display_branches.iter().map(|b| {
                let mut item = BranchItem::from_branch_minimal(b, &worktrees);
                // SPEC-a70a1ece FR-171: For bare repos, branches without worktrees are Remote
                if repo_type == RepoType::Bare {
                    item.branch_type = BranchType::Remote;
                }
                apply_last_tool_usage(&mut item, &repo_root, &tool_usage_map, &agent_history);
                item
            }));

            // Sort branches by timestamp for those with sessions
            branch_items.iter_mut().for_each(|item| {
                if item.last_commit_timestamp.is_none() {
                    // Try to get timestamp from git (fallback)
                    // For now, leave as None - the sort will handle it
                }
            });

            let total_count = branch_items.len();
            let active_count = branch_items.iter().filter(|b| b.has_worktree).count();
            let base_branches: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
            let branch_names: Vec<String> = branch_items.iter().map(|b| b.name.clone()).collect();

            let _ = tx.send(BranchListUpdate {
                branches: branch_items,
                branch_names,
                worktree_targets,
                safety_targets,
                base_branches,
                base_branch,
                base_branch_exists,
                total_count,
                active_count,
            });
        });
    }

    fn spawn_pr_title_fetch(&mut self, branch_names: Vec<String>) {
        if branch_names.is_empty() {
            self.pr_title_rx = None;
            return;
        }

        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.pr_title_rx = Some(rx);

        thread::spawn(move || {
            let mut cache = PrCache::new();
            cache.populate(&repo_root);

            let mut info = HashMap::new();
            for name in branch_names {
                let lookup = normalize_branch_name(&name);
                if let Some(pr) = cache.get(&lookup) {
                    info.insert(
                        name,
                        PrInfo {
                            title: pr.title.clone(),
                            number: pr.number,
                            url: pr.url.clone(),
                            state: pr.state.clone(),
                        },
                    );
                }
            }

            let _ = tx.send(PrTitleUpdate { info });
        });
    }

    fn spawn_branch_summary_fetch(&mut self, request: BranchSummaryRequest) {
        let (tx, rx) = mpsc::channel();
        self.branch_summary_rx = Some(rx);

        thread::spawn(move || {
            let summary =
                BranchListState::build_branch_summary(&request.repo_root, &request.branch_item);
            let _ = tx.send(BranchSummaryUpdate {
                branch: request.branch,
                summary,
            });
        });
    }

    /// SPEC-1ea18899: Spawn background fetch for GitView data
    fn spawn_git_view_data_fetch(&mut self, branch: &str, worktree_path: Option<&Path>) {
        let branch = branch.to_string();
        let worktree_path = worktree_path.map(|p| p.to_path_buf());
        // Use bare repo path if available for branches without worktree
        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.git_view_cache_rx = Some(rx);

        thread::spawn(move || {
            let data = if let Some(ref wt_path) = worktree_path {
                build_git_view_data(wt_path)
            } else {
                build_git_view_data_no_worktree(&repo_root, &branch)
            };
            let _ = tx.send(GitViewCacheUpdate { branch, data });
        });
    }

    /// SPEC-1ea18899: Spawn background fetch for GitView PR info
    fn spawn_git_view_pr_fetch(&mut self, branch: &str) {
        let branch = branch.to_string();
        let lookup = normalize_branch_name(&branch);
        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.git_view_pr_rx = Some(rx);

        thread::spawn(move || {
            let info = PrCache::fetch_latest_for_branch(&repo_root, &lookup).map(|pr| PrInfo {
                title: pr.title,
                number: pr.number,
                url: pr.url,
                state: pr.state,
            });
            let _ = tx.send(GitViewPrUpdate { branch, info });
        });
    }

    /// SPEC-1ea18899: Apply GitView cache updates from background fetch
    fn apply_git_view_cache_updates(&mut self) {
        let Some(rx) = &self.git_view_cache_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                // Update cache
                self.git_view_cache
                    .insert(update.branch.clone(), update.data.clone());
                // If we're on GitView for this branch, update state
                if matches!(self.screen, Screen::GitView)
                    && self.git_view.branch_name == update.branch
                {
                    self.git_view.load_from_cache(&update.data);
                }
                self.git_view_cache_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.git_view_cache_rx = None;
            }
        }
    }

    /// SPEC-1ea18899: Apply GitView PR updates from background fetch
    fn apply_git_view_pr_updates(&mut self) {
        let Some(rx) = &self.git_view_pr_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                if let Some(info) = update.info {
                    let mut map = HashMap::new();
                    map.insert(update.branch.clone(), info.clone());
                    self.branch_list.apply_pr_info(&map);
                    if matches!(self.screen, Screen::GitView)
                        && self.git_view.branch_name == update.branch
                    {
                        self.git_view.update_pr_info(
                            Some(info.number),
                            Some(info.title.clone()),
                            info.url.clone(),
                            Some(info.state.clone()),
                        );
                    }
                }
                self.git_view_pr_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.git_view_pr_rx = None;
            }
        }
    }

    fn spawn_safety_checks(
        &mut self,
        targets: Vec<SafetyCheckTarget>,
        base_branch: String,
        base_branch_exists: bool,
    ) {
        if targets.is_empty() {
            self.safety_rx = None;
            return;
        }

        // SPEC-a70a1ece: Use bare repo path for git commands in bare projects
        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.safety_rx = Some(rx);

        thread::spawn(move || {
            let mut pr_cache = PrCache::new();
            pr_cache.populate(&repo_root);

            for target in targets {
                let mut has_unpushed = false;
                let mut is_unmerged = false;
                let mut safe_to_cleanup = false;

                // Check for unpushed commits (skip if upstream is not configured)
                if let Some(ref upstream) = target.upstream {
                    if let Ok((ahead, _)) =
                        Branch::divergence_between(&repo_root, &target.branch, upstream)
                    {
                        if ahead > 0 {
                            has_unpushed = true;
                        }
                    }
                }
                // If upstream is not configured, continue to check against base branch

                if has_unpushed {
                    let _ = tx.send(SafetyUpdate {
                        branch: target.branch,
                        has_unpushed,
                        is_unmerged,
                        safe_to_cleanup,
                    });
                    continue;
                }

                if pr_cache.is_merged(&target.branch) {
                    safe_to_cleanup = true;
                    let _ = tx.send(SafetyUpdate {
                        branch: target.branch,
                        has_unpushed,
                        is_unmerged,
                        safe_to_cleanup,
                    });
                    continue;
                }

                // Check if branch is merged into base using merge-base --is-ancestor
                // This correctly handles cases where base_branch has advanced beyond the merge point
                if base_branch_exists {
                    if let Ok(is_merged) =
                        Branch::is_merged_into(&repo_root, &target.branch, &base_branch)
                    {
                        is_unmerged = !is_merged;
                        safe_to_cleanup = is_merged;
                    }
                }

                let _ = tx.send(SafetyUpdate {
                    branch: target.branch,
                    has_unpushed,
                    is_unmerged,
                    safe_to_cleanup,
                });
            }
        });
    }

    fn spawn_worktree_status_checks(&mut self, targets: Vec<WorktreeStatusTarget>) {
        if targets.is_empty() {
            self.worktree_status_rx = None;
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.worktree_status_rx = Some(rx);

        thread::spawn(move || {
            for target in targets {
                let mut failed = false;
                let mut worktree_status = WorktreeStatus::Active;
                let mut has_changes = false;

                if !target.path.exists() {
                    worktree_status = WorktreeStatus::Inaccessible;
                } else {
                    match Repository::open(&target.path) {
                        Ok(repo) => match repo.has_uncommitted_changes() {
                            Ok(changes) => {
                                has_changes = changes;
                            }
                            Err(err) => {
                                failed = true;
                                tracing::warn!(
                                    "Failed to check worktree changes for {}: {}",
                                    target.branch,
                                    err
                                );
                            }
                        },
                        Err(err) => {
                            failed = true;
                            tracing::warn!(
                                "Failed to open worktree for {}: {}",
                                target.branch,
                                err
                            );
                        }
                    }
                }

                let _ = tx.send(WorktreeStatusUpdate {
                    branch: target.branch,
                    worktree_status,
                    has_changes,
                    failed,
                });
            }
        });
    }

    fn refresh_branch_summary(&mut self) {
        self.branch_list.ai_enabled = self.active_ai_enabled();
        self.branch_list.session_summary_enabled = self.active_session_summary_enabled();
        if let Some(request) = self.branch_list.prepare_branch_summary(&self.repo_root) {
            self.spawn_branch_summary_fetch(request);
        }
        self.maybe_request_session_summary_for_selected(false);
    }

    fn enter_agent_mode(&mut self) {
        self.update_agent_mode_ai_status();
        self.agent_mode.last_error = None;
        self.screen = Screen::AgentMode;
    }

    fn update_agent_mode_ai_status(&mut self) {
        if !self.active_ai_enabled() {
            self.agent_mode
                .set_ai_status(false, Some("AI settings are required.".to_string()));
            return;
        }

        let (ready, error) = match self.active_ai_settings() {
            Some(settings) => {
                if settings.endpoint.trim().is_empty() || settings.model.trim().is_empty() {
                    (false, Some("AI settings are required.".to_string()))
                } else {
                    match AIClient::new(settings) {
                        Ok(_) => (true, None),
                        Err(err) => (false, Some(format_ai_error(err))),
                    }
                }
            }
            None => (false, Some("AI settings are required.".to_string())),
        };
        self.agent_mode.set_ai_status(ready, error);
    }

    fn spawn_agent_mode_request(&mut self, messages: Vec<AgentMessage>) {
        let Some(settings) = self.active_ai_settings() else {
            self.agent_mode.last_error = Some("AI settings are required.".to_string());
            self.agent_mode.set_waiting(false);
            return;
        };

        let Some(tx) = &self.agent_mode_tx else {
            self.agent_mode.last_error = Some("Agent channel unavailable.".to_string());
            self.agent_mode.set_waiting(false);
            return;
        };

        let tx = tx.clone();
        thread::spawn(move || {
            let client = match AIClient::new(settings) {
                Ok(client) => client,
                Err(err) => {
                    let _ = tx.send(AgentModeUpdate {
                        response: None,
                        error: Some(format_ai_error(err)),
                    });
                    return;
                }
            };

            let mut chat_messages = Vec::new();
            if !AGENT_SYSTEM_PROMPT.trim().is_empty() {
                chat_messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: AGENT_SYSTEM_PROMPT.to_string(),
                });
            }

            for msg in messages {
                let role = match msg.role {
                    AgentRole::User => "user",
                    AgentRole::Assistant => "assistant",
                    AgentRole::System => "system",
                };
                chat_messages.push(ChatMessage {
                    role: role.to_string(),
                    content: msg.content,
                });
            }

            match client.create_response(chat_messages) {
                Ok(response) => {
                    let _ = tx.send(AgentModeUpdate {
                        response: Some(response),
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AgentModeUpdate {
                        response: None,
                        error: Some(format_ai_error(err)),
                    });
                }
            }
        });
    }

    fn maybe_request_session_summary_for_selected(&mut self, force: bool) {
        if !self.branch_list.session_summary_enabled {
            return;
        }

        let Some(branch) = self.branch_list.selected_branch().cloned() else {
            return;
        };

        let branch_name = branch.name.clone();
        let mut session_id = branch.last_session_id.clone();
        let mut tool_id = branch.last_tool_id.clone();

        if tool_id.is_none() {
            if let Some(agent) = self.branch_list.get_running_agent(&branch.name) {
                tool_id = Some(agent.agent_name.clone());
            }
        }

        if session_id.is_none() || self.branch_list.is_session_missing(&branch_name) {
            if let (Some(tool_id), Some(worktree_path)) =
                (tool_id.as_ref(), branch.worktree_path.as_deref())
            {
                if let Some(found) =
                    crate::detect_session_id_for_tool(tool_id, Path::new(worktree_path))
                {
                    session_id = Some(found);
                }
            }
        }

        let (Some(session_id), Some(tool_id)) = (session_id, tool_id) else {
            self.branch_list.mark_session_missing(&branch_name);
            return;
        };
        let canonical_tool_id = canonical_tool_id(&tool_id);
        self.branch_list.clear_session_missing(&branch_name);
        if branch.last_session_id.as_deref() != Some(&session_id)
            || branch.last_tool_id.as_deref() != Some(&canonical_tool_id)
        {
            self.branch_list.set_session_identity(
                &branch_name,
                canonical_tool_id.clone(),
                session_id.clone(),
            );
            self.persist_detected_session(&branch, &canonical_tool_id, &session_id);
        }

        if !force {
            if self.branch_list.session_summary_cached(&branch_name)
                || self.branch_list.session_summary_inflight(&branch_name)
            {
                return;
            }
        } else if self.branch_list.session_summary_inflight(&branch_name) {
            return;
        }

        let Some(settings) = self.active_ai_settings() else {
            return;
        };

        let task = SessionSummaryTask {
            branch: branch_name,
            session_id,
            tool_id: canonical_tool_id,
        };
        self.spawn_session_summaries(vec![task], settings);
    }

    fn extract_tool_version_from_usage(branch: &BranchItem, tool_id: &str) -> Option<String> {
        let usage = branch.last_tool_usage.as_ref()?;
        let (label, version) = usage.rsplit_once('@')?;
        let version = version.trim();
        if version.is_empty() {
            return None;
        }
        if let Some(last_id) = branch.last_tool_id.as_deref() {
            if last_id != tool_id {
                return None;
            }
        } else {
            let normalized = crate::tui::normalize_agent_label(tool_id);
            if !label.trim().eq_ignore_ascii_case(&normalized) {
                return None;
            }
        }
        Some(version.to_string())
    }

    fn persist_detected_session(&self, branch: &BranchItem, tool_id: &str, session_id: &str) {
        if branch.worktree_path.is_none() {
            return;
        }
        let tool_version = Self::extract_tool_version_from_usage(branch, tool_id);
        let collaboration_modes = if tool_id.contains("codex") {
            match tool_version.as_deref() {
                Some("latest") => Some(true),
                Some(version) => Some(supports_collaboration_modes(Some(version))),
                None => None,
            }
        } else {
            None
        };
        let entry = ToolSessionEntry {
            branch: branch.name.clone(),
            worktree_path: branch.worktree_path.clone(),
            tool_id: tool_id.to_string(),
            tool_label: crate::tui::normalize_agent_label(tool_id),
            session_id: Some(session_id.to_string()),
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version,
            collaboration_modes,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        let _ = save_session_entry(&self.repo_root, entry);
    }

    fn spawn_session_summaries(
        &mut self,
        tasks: Vec<SessionSummaryTask>,
        settings: ResolvedAISettings,
    ) {
        if tasks.is_empty() {
            return;
        }
        let Some(tx) = &self.session_summary_tx else {
            return;
        };

        for task in &tasks {
            self.branch_list.mark_session_summary_inflight(&task.branch);
            if let Some(current) = self.branch_list.branch_summary.as_mut() {
                if current.branch_name == task.branch {
                    current.loading.session_summary = true;
                    current.errors.session_summary = None;
                }
            }
        }

        let tx = tx.clone();
        thread::spawn(move || {
            let client = match AIClient::new(settings) {
                Ok(client) => client,
                Err(err) => {
                    let message = err.to_string();
                    for task in tasks {
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: None,
                            error: Some(message.clone()),
                            warning: None,
                            mtime: None,
                            missing: false,
                        });
                    }
                    return;
                }
            };

            for task in tasks {
                let parser = match session_parser_for_tool(&task.tool_id) {
                    Some(parser) => parser,
                    None => {
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: None,
                            error: Some("Unsupported agent session".to_string()),
                            warning: None,
                            mtime: None,
                            missing: true,
                        });
                        continue;
                    }
                };

                let path = parser.session_file_path(&task.session_id);
                let metadata = match std::fs::metadata(&path) {
                    Ok(meta) => meta,
                    Err(err) => {
                        let missing = err.kind() == std::io::ErrorKind::NotFound;
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: None,
                            error: Some(err.to_string()),
                            warning: None,
                            mtime: None,
                            missing,
                        });
                        continue;
                    }
                };

                let mtime = metadata.modified().ok();
                let parsed = match parser.parse(&task.session_id) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        let missing = matches!(err, SessionParseError::FileNotFound(_));
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: None,
                            error: Some(err.to_string()),
                            warning: None,
                            mtime,
                            missing,
                        });
                        continue;
                    }
                };

                match summarize_session(&client, &parsed) {
                    Ok(summary) => {
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: Some(summary),
                            error: None,
                            warning: None,
                            mtime,
                            missing: false,
                        });
                    }
                    Err(err) => match err {
                        AIError::IncompleteSummary => {
                            let _ = tx.send(SessionSummaryUpdate {
                                branch: task.branch,
                                session_id: task.session_id,
                                summary: None,
                                error: None,
                                warning: Some("Incomplete summary; keeping previous".to_string()),
                                mtime,
                                missing: false,
                            });
                        }
                        other => {
                            let _ = tx.send(SessionSummaryUpdate {
                                branch: task.branch,
                                session_id: task.session_id,
                                summary: None,
                                error: Some(format_ai_error(other)),
                                warning: None,
                                mtime,
                                missing: false,
                            });
                        }
                    },
                }
            }
        });
    }

    fn apply_session_summary_updates(&mut self) {
        let Some(rx) = &self.session_summary_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => {
                    if update.missing {
                        self.branch_list.mark_session_missing(&update.branch);
                        self.branch_list.apply_session_error(
                            &update.branch,
                            update.error.unwrap_or_else(|| "No session".to_string()),
                        );
                        if self.session_poll_deferred {
                            if let Some(selected) = self.branch_list.selected_branch() {
                                if selected.name == update.branch {
                                    self.last_session_poll = None;
                                    self.session_poll_deferred = false;
                                }
                            }
                        }
                        continue;
                    }

                    if let Some(summary) = update.summary {
                        let mtime = update.mtime.unwrap_or(SystemTime::now());
                        self.branch_list.apply_session_summary(
                            &update.branch,
                            &update.session_id,
                            summary,
                            mtime,
                        );
                        if self.session_poll_deferred {
                            if let Some(selected) = self.branch_list.selected_branch() {
                                if selected.name == update.branch {
                                    self.last_session_poll = None;
                                    self.session_poll_deferred = false;
                                }
                            }
                        }
                    } else if let Some(warning) = update.warning {
                        self.branch_list
                            .apply_session_warning(&update.branch, warning);
                        if self.session_poll_deferred {
                            if let Some(selected) = self.branch_list.selected_branch() {
                                if selected.name == update.branch {
                                    self.last_session_poll = None;
                                    self.session_poll_deferred = false;
                                }
                            }
                        }
                    } else if let Some(error) = update.error {
                        self.branch_list.apply_session_error(&update.branch, error);
                        if self.session_poll_deferred {
                            if let Some(selected) = self.branch_list.selected_branch() {
                                if selected.name == update.branch {
                                    self.last_session_poll = None;
                                    self.session_poll_deferred = false;
                                }
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.session_summary_rx = None;
                    break;
                }
            }
        }
    }

    fn apply_agent_mode_updates(&mut self) {
        let Some(rx) = &self.agent_mode_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.agent_mode.set_waiting(false);
                    if let Some(error) = update.error {
                        self.agent_mode.last_error = Some(error);
                        continue;
                    }
                    if let Some(response) = update.response {
                        self.agent_mode.last_error = None;
                        self.agent_mode.messages.push(AgentMessage {
                            role: AgentRole::Assistant,
                            content: response,
                        });
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.agent_mode_rx = None;
                    break;
                }
            }
        }
    }

    fn poll_session_summary_if_needed(&mut self) {
        if !self.branch_list.session_summary_enabled {
            return;
        }

        let now = Instant::now();
        if !session_poll_due(self.last_session_poll, now) {
            return;
        }

        let (branch_name, session_id, tool_id) = match self.branch_list.selected_branch() {
            Some(branch) => (
                branch.name.clone(),
                branch.last_session_id.clone(),
                branch.last_tool_id.clone(),
            ),
            None => return,
        };

        if self.branch_list.session_summary_inflight(&branch_name) {
            self.session_poll_deferred = true;
            return;
        }

        self.last_session_poll = Some(now);
        self.session_poll_deferred = false;

        let Some(session_id) = session_id else {
            self.branch_list.mark_session_missing(&branch_name);
            return;
        };
        let Some(tool_id) = tool_id else {
            self.branch_list.mark_session_missing(&branch_name);
            return;
        };

        let Some(settings) = self.active_ai_settings() else {
            return;
        };

        let parser = match session_parser_for_tool(&tool_id) {
            Some(parser) => parser,
            None => {
                self.branch_list.mark_session_missing(&branch_name);
                return;
            }
        };

        let path = parser.session_file_path(&session_id);
        let metadata = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    self.branch_list.mark_session_missing(&branch_name);
                }
                return;
            }
        };
        self.branch_list.clear_session_missing(&branch_name);
        let Some(mtime) = metadata.modified().ok() else {
            return;
        };

        if !self
            .branch_list
            .session_summary_stale(&branch_name, &session_id, mtime)
        {
            return;
        }

        if !session_file_is_quiet(mtime, SystemTime::now()) {
            self.last_session_poll = Some(defer_poll_for_quiet(now));
            return;
        }

        let task = SessionSummaryTask {
            branch: branch_name,
            session_id,
            tool_id,
        };
        self.spawn_session_summaries(vec![task], settings);
    }

    /// Reconnect to orphaned agent panes on startup (FR-060~062)
    ///
    /// This function detects panes that were running before gwt restarted
    /// by matching their working directory to worktree paths.
    fn reconnect_orphaned_panes(&mut self) {
        // Only in tmux multi-mode
        if !self.tmux_mode.is_multi() {
            return;
        }

        let Some(session) = &self.tmux_session else {
            return;
        };

        // Get worktree list synchronously for matching
        // SPEC-a70a1ece: Use bare repo path for worktree operations in bare projects
        let git_path = self.bare_repo_path.as_ref().unwrap_or(&self.repo_root);
        let worktrees: Vec<(String, std::path::PathBuf)> = match WorktreeManager::new(git_path) {
            Ok(manager) => match manager.list_basic() {
                Ok(wts) => wts
                    .into_iter()
                    .filter_map(|wt| wt.branch.map(|b| (b, wt.path)))
                    .collect(),
                Err(_) => return,
            },
            Err(_) => return,
        };

        if worktrees.is_empty() {
            return;
        }

        let tool_usage_map = gwt_core::config::get_last_tool_usage_map(&self.repo_root);

        // Detect orphaned panes
        let gwt_pane_id = self.gwt_pane_id.as_deref();
        match gwt_core::tmux::detect_orphaned_panes(session, &worktrees, gwt_pane_id) {
            Ok(mut orphaned_panes) => {
                if !orphaned_panes.is_empty() {
                    debug!(
                        category = "tui",
                        count = orphaned_panes.len(),
                        "Reconnected to orphaned agent panes"
                    );

                    for pane in orphaned_panes.iter_mut() {
                        let entry = tool_usage_map.get(&pane.branch_name);
                        pane.agent_name = resolve_orphaned_agent_name(&pane.agent_name, entry);
                    }

                    for pane in orphaned_panes {
                        // Add to agent_panes list
                        self.agent_panes.push(pane.pane_id.clone());
                        // Add to pane_list for display
                        self.pane_list.panes.push(pane);
                    }

                    // Update branch_list running_agents
                    self.branch_list
                        .update_running_agents(&self.pane_list.panes);
                    self.reflow_agent_layout(None);
                }
            }
            Err(e) => {
                debug!(
                    category = "tui",
                    error = %e,
                    "Failed to detect orphaned panes"
                );
            }
        }
    }

    fn apply_entry_context(&mut self, context: Option<TuiEntryContext>) {
        if let Some(context) = context {
            if let Some(message) = context.status_message {
                self.status_message = Some(message);
                self.status_message_time = Some(Instant::now());
            }
            if let Some(message) = context.error_message {
                // Log the error
                gwt_core::logging::log_error_message("E9001", "cli", &message, None);
                let error_state = ErrorState::from_error(&message);
                self.error_queue.push(error_state);
                self.screen_stack.push(self.screen.clone());
                self.screen = Screen::Error;
            }
        }
    }

    fn active_status_message(&self) -> Option<&str> {
        self.launch_status
            .as_deref()
            .or(self.status_message.as_deref())
    }

    fn apply_branch_list_updates(&mut self) {
        let Some(rx) = &self.branch_list_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                let cleanup_snapshot = self.branch_list.cleanup_snapshot();
                let session_cache = self.branch_list.clone_session_cache();
                let session_inflight = self.branch_list.clone_session_inflight();
                let session_missing = self.branch_list.clone_session_missing();
                let session_warnings = self.branch_list.clone_session_warnings();
                let sort_mode = self.branch_list.sort_mode;
                let mut branch_list = BranchListState::new();
                branch_list.sort_mode = sort_mode;
                let mut branch_list = branch_list.with_branches(update.branches);
                branch_list.active_profile = self.profiles_config.active.clone();
                branch_list.ai_enabled = self.active_ai_enabled();
                branch_list.session_summary_enabled = self.active_session_summary_enabled();
                branch_list.set_session_cache(session_cache);
                branch_list.set_session_inflight(session_inflight);
                branch_list.set_session_missing(session_missing);
                branch_list.set_session_warnings(session_warnings);
                branch_list.set_repo_web_url(self.branch_list.repo_web_url().cloned());
                branch_list.working_directory = Some(self.repo_root.display().to_string());
                branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
                // セッション警告のクリーンアップ（削除されたブランチ分）
                let remaining_branches: std::collections::HashSet<String> = branch_list
                    .branches
                    .iter()
                    .map(|b| b.name.clone())
                    .collect();
                branch_list.cleanup_session_warnings(&remaining_branches);
                branch_list.restore_cleanup_snapshot(&cleanup_snapshot);
                self.branch_list = branch_list;
                // SPEC-4b893dae: Update branch summary after branches are loaded
                self.refresh_branch_summary();

                self.total_count = update.total_count;
                self.active_count = update.active_count;

                let total_updates = update.safety_targets.len() + update.worktree_targets.len();
                self.branch_list.reset_status_progress(total_updates);
                self.spawn_safety_checks(
                    update.safety_targets,
                    update.base_branch,
                    update.base_branch_exists,
                );
                self.spawn_worktree_status_checks(update.worktree_targets);
                self.spawn_pr_title_fetch(update.branch_names);

                self.worktree_create =
                    WorktreeCreateState::new().with_base_branches(update.base_branches);
                self.branch_list_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.branch_list.set_loading(false);
                self.branch_list_rx = None;
            }
        }
    }

    fn apply_pr_title_updates(&mut self) {
        let Some(rx) = &self.pr_title_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                self.branch_list.apply_pr_info(&update.info);
                if matches!(self.screen, Screen::GitView) {
                    if let Some(branch) = self
                        .branch_list
                        .branches
                        .iter()
                        .find(|b| b.name == self.git_view.branch_name)
                    {
                        let branch_has_pr = branch.pr_number.is_some()
                            || branch.pr_title.is_some()
                            || branch.pr_state.is_some();
                        let git_view_has_pr = self.git_view.pr_number.is_some()
                            || self.git_view.pr_title.is_some()
                            || self.git_view.pr_state.is_some();
                        if branch_has_pr || !git_view_has_pr {
                            self.git_view.update_pr_info(
                                branch.pr_number,
                                branch.pr_title.clone(),
                                branch.pr_url.clone(),
                                branch.pr_state.clone(),
                            );
                        }
                    }
                }
                self.pr_title_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pr_title_rx = None;
            }
        }
    }

    fn apply_branch_summary_updates(&mut self) {
        let Some(rx) = &self.branch_summary_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                self.branch_list.apply_branch_summary_update(update);
                self.branch_summary_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.branch_summary_rx = None;
            }
        }
    }

    fn apply_launch_updates(&mut self) {
        let refresh_after = {
            let Some(rx) = &self.launch_rx else {
                return;
            };

            let mut refresh_after = false;

            loop {
                match rx.try_recv() {
                    Ok(update) => match update {
                        LaunchUpdate::Progress(progress) => {
                            // Only update launch_status if modal is not showing (FR-057)
                            if self.progress_modal.is_none() {
                                self.launch_status = Some(progress.message());
                            }
                        }
                        LaunchUpdate::ProgressStep { kind, status } => {
                            // Update progress modal step (FR-041)
                            if let Some(ref mut modal) = self.progress_modal {
                                modal.update_step(kind, status);
                                // Check if all steps are done
                                if modal.all_done() && !modal.has_failed() {
                                    modal.mark_completed();
                                }
                            }
                        }
                        LaunchUpdate::ProgressStepError { kind, message } => {
                            // Set error on progress modal step (FR-052)
                            if let Some(ref mut modal) = self.progress_modal {
                                modal.set_step_error(kind, message);
                            }
                        }
                        LaunchUpdate::WorktreeReady { branch, path } => {
                            if self.branch_list.apply_worktree_created(&branch, &path) {
                                self.active_count = self.branch_list.stats.worktree_count;
                            } else {
                                refresh_after = true;
                            }
                        }
                        LaunchUpdate::Ready(plan) => {
                            self.launch_in_progress = false;
                            self.launch_rx = None;
                            // FR-051: Mark progress modal as completed
                            if let Some(ref mut modal) = self.progress_modal {
                                modal.mark_completed();
                            }
                            let next_status = match &plan.install_plan {
                                InstallPlan::Install { manager } => {
                                    LaunchProgress::InstallingDependencies {
                                        manager: manager.clone(),
                                    }
                                    .message()
                                }
                                _ => "Launching agent...".to_string(),
                            };
                            self.launch_status = Some(next_status);
                            self.handle_launch_plan(*plan);
                            break;
                        }
                        LaunchUpdate::Failed(message) => {
                            self.launch_in_progress = false;
                            self.launch_rx = None;
                            self.launch_status = None;
                            // FR-052: Show error in modal (waiting_for_key is set by set_step_error)
                            gwt_core::logging::log_error_message("E4001", "agent", &message, None);
                            self.worktree_create.error_message = Some(message.clone());
                            self.status_message = Some(format!("Error: {}", message));
                            self.status_message_time = Some(Instant::now());
                            break;
                        }
                    },
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.launch_rx = None;
                        self.launch_in_progress = false;
                        self.launch_status = None;
                        break;
                    }
                }
            }

            refresh_after
        };

        if refresh_after {
            self.refresh_data();
        }
    }

    fn apply_safety_updates(&mut self) {
        let Some(rx) = &self.safety_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.branch_list.apply_safety_update(
                        &update.branch,
                        update.has_unpushed,
                        update.is_unmerged,
                        update.safe_to_cleanup,
                    );
                    self.branch_list.increment_status_progress();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.safety_rx = None;
                    break;
                }
            }
        }
    }

    fn apply_worktree_updates(&mut self) {
        let Some(rx) = &self.worktree_status_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => {
                    if !update.failed {
                        self.branch_list.apply_worktree_update(
                            &update.branch,
                            update.worktree_status,
                            update.has_changes,
                        );
                    }
                    self.branch_list.increment_status_progress();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worktree_status_rx = None;
                    break;
                }
            }
        }
    }

    fn apply_cleanup_updates(&mut self) {
        let Some(rx) = &self.cleanup_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => match update {
                    CleanupUpdate::BranchStarted { branch } => {
                        self.branch_list.set_cleanup_active_branch(Some(branch));
                    }
                    CleanupUpdate::BranchFinished { branch, success } => {
                        self.branch_list.increment_cleanup_progress();
                        if success {
                            self.branch_list.selected_branches.remove(&branch);
                        }
                        if self.branch_list.cleanup_active_branch() == Some(branch.as_str()) {
                            self.branch_list.set_cleanup_active_branch(None);
                        }
                    }
                    CleanupUpdate::Completed { deleted, errors } => {
                        self.finish_cleanup(deleted, errors);
                        self.cleanup_rx = None;
                        break;
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.cleanup_rx = None;
                    self.branch_list.finish_cleanup_progress();
                    break;
                }
            }
        }
    }

    /// FR-033: Update pane list by polling tmux (1-second interval)
    /// FR-031e: Update agent states based on pane content changes
    fn update_pane_list(&mut self) {
        // Only in tmux multi mode
        if !self.tmux_mode.is_multi() {
            return;
        }

        // Check if 1 second has passed since last update
        let now = Instant::now();
        let mut spinner_updated = false;
        if self
            .last_spinner_update
            .map(|last| now.duration_since(last) >= Duration::from_millis(250))
            .unwrap_or(true)
        {
            self.pane_list.spinner_frame = self.pane_list.spinner_frame.wrapping_add(1);
            self.last_spinner_update = Some(now);
            spinner_updated = true;
        }

        if let Some(last) = self.last_pane_update {
            if now.duration_since(last) < Duration::from_secs(1) {
                return;
            }
        }
        self.last_pane_update = Some(now);
        if !spinner_updated {
            self.pane_list.spinner_frame = self.pane_list.spinner_frame.wrapping_add(1);
            self.last_spinner_update = Some(now);
        }

        if self.pane_list.panes.is_empty() {
            return;
        }

        // Get current tmux panes
        let Some(session) = &self.tmux_session else {
            return;
        };

        let Ok(current_panes) = gwt_core::tmux::pane::list_panes(session) else {
            return;
        };

        // Filter out panes that no longer exist
        let current_pane_ids: std::collections::HashSet<_> =
            current_panes.iter().map(|p| p.pane_id.as_str()).collect();

        // FR-072: Detect removed panes and update session
        let removed_panes: Vec<AgentPane> = self
            .pane_list
            .panes
            .iter()
            .filter(|p| !current_pane_ids.contains(p.pane_id.as_str()))
            .cloned()
            .collect();

        let last_tool_usage_map = gwt_core::config::get_last_tool_usage_map(&self.repo_root);
        for pane in &removed_panes {
            let last_entry = last_tool_usage_map.get(&pane.branch_name);
            // Save session entry for terminated agent (FR-072)
            let tool_id = last_entry
                .map(|entry| entry.tool_id.clone())
                .unwrap_or_else(|| pane.agent_name.clone());
            let tool_label = last_entry
                .map(|entry| entry.tool_label.clone())
                .unwrap_or_else(|| crate::tui::normalize_agent_label(&pane.agent_name));
            let session_entry = ToolSessionEntry {
                branch: pane.branch_name.clone(),
                worktree_path: last_entry.and_then(|entry| entry.worktree_path.clone()),
                tool_id,
                tool_label,
                session_id: last_entry.and_then(|entry| entry.session_id.clone()),
                mode: last_entry.and_then(|entry| entry.mode.clone()),
                model: last_entry.and_then(|entry| entry.model.clone()),
                reasoning_level: last_entry.and_then(|entry| entry.reasoning_level.clone()),
                skip_permissions: last_entry.and_then(|entry| entry.skip_permissions),
                tool_version: last_entry.and_then(|entry| entry.tool_version.clone()),
                collaboration_modes: last_entry.and_then(|entry| entry.collaboration_modes),
                docker_service: last_entry.and_then(|entry| entry.docker_service.clone()),
                docker_force_host: last_entry.and_then(|entry| entry.docker_force_host),
                docker_recreate: last_entry.and_then(|entry| entry.docker_recreate),
                docker_build: None,
                docker_keep: last_entry.and_then(|entry| entry.docker_keep),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            };
            if let Err(e) = save_session_entry(&self.repo_root, session_entry) {
                debug!(
                    category = "tui",
                    pane_id = %pane.pane_id,
                    error = %e,
                    "Failed to save session entry for terminated agent"
                );
            }
        }

        // Update state for each remaining pane (FR-031e)
        let mut updated_panes: Vec<_> = self
            .pane_list
            .panes
            .iter()
            .filter(|p| current_pane_ids.contains(p.pane_id.as_str()))
            .cloned()
            .collect();

        // Also update agent_panes list
        self.agent_panes
            .retain(|id| current_pane_ids.contains(id.as_str()));

        // Update pane list if count changed
        if updated_panes.len() != self.pane_list.panes.len() {
            self.pane_list.update_panes(updated_panes);
        } else {
            // Keep existing panes (with their is_background state)
            std::mem::swap(&mut self.pane_list.panes, &mut updated_panes);
        }

        // Sync running_agents in branch_list with current panes
        self.branch_list
            .update_running_agents(&self.pane_list.panes);
        // Update spinner frame for branch list display
        self.branch_list.spinner_frame = self.pane_list.spinner_frame;
        if !removed_panes.is_empty() {
            self.reflow_agent_layout(None);
        }
    }

    /// FR-095: Check if there is an active (visible) agent pane
    pub fn has_active_agent_pane(&self) -> bool {
        self.pane_list.panes.iter().any(|p| !p.is_background)
    }

    /// FR-095: Hide the active agent pane (ESC key handler)
    /// シングルアクティブ制約: アクティブペインは最大1つなので、それを非表示にする
    fn hide_active_agent_pane(&mut self) {
        // Find the active pane (is_background == false)
        let active_idx = self.pane_list.panes.iter().position(|p| !p.is_background);

        let Some(idx) = active_idx else {
            // No active pane to hide (FR-096: do nothing)
            return;
        };

        let Some(pane) = self.pane_list.panes.get_mut(idx) else {
            return;
        };

        // Hide the pane (break to background window)
        let window_name = background_window_name(&pane.branch_name);

        match gwt_core::tmux::hide_pane(&pane.pane_id, &window_name) {
            Ok(background_window) => {
                // Remove from agent_panes list
                self.agent_panes.retain(|id| id != &pane.pane_id);

                // Update pane state
                pane.is_background = true;
                pane.background_window = Some(background_window);

                self.status_message = Some("Pane hidden".to_string());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to hide pane: {}", e));
            }
        }
        self.status_message_time = Some(Instant::now());

        // Update branch list with new pane state
        self.branch_list
            .update_running_agents(&self.pane_list.panes);
    }

    /// FR-042: Terminate agent pane for the specified branch
    fn terminate_agent_pane(&mut self, branch_name: &str) {
        // Find the agent pane for this branch
        let pane_to_kill = self
            .pane_list
            .panes
            .iter()
            .find(|p| p.branch_name == branch_name)
            .cloned();

        let Some(pane) = pane_to_kill else {
            self.status_message = Some("Agent pane not found".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        };

        // Kill the pane using tmux kill-pane
        let pane_target = if pane.is_background {
            // For background panes, kill the background window
            if let Some(ref bg_window) = pane.background_window {
                bg_window.clone()
            } else {
                pane.pane_id.clone()
            }
        } else {
            pane.pane_id.clone()
        };

        match kill_pane(&pane_target) {
            Ok(()) => {
                // Remove from pane_list
                self.pane_list
                    .panes
                    .retain(|p| p.branch_name != branch_name);
                // Remove from agent_panes
                self.agent_panes.retain(|id| id != &pane.pane_id);
                // Update branch_list running_agents
                self.branch_list
                    .update_running_agents(&self.pane_list.panes);
                self.reflow_agent_layout(None);

                self.status_message = Some(format!("Agent terminated on '{}'", branch_name));
                self.status_message_time = Some(Instant::now());
            }
            Err(e) => {
                self.status_message = Some(format!("Failed to terminate: {}", e));
                self.status_message_time = Some(Instant::now());
            }
        }
    }

    /// Show a hidden pane and focus it
    ///
    /// If the pane is already visible, just focus it.
    /// If the pane is in background, show it first then focus.
    fn show_and_focus_selected_pane(&mut self) {
        let selected_idx = self.pane_list.selected;
        let Some(pane) = self.pane_list.panes.get_mut(selected_idx) else {
            return;
        };

        if pane.is_background {
            // Show the pane first (join back to GWT window)
            if pane.background_window.is_none() {
                self.status_message = Some("No background window to restore".to_string());
                self.status_message_time = Some(Instant::now());
                return;
            }

            let Some(gwt_pane_id) = self.gwt_pane_id.clone() else {
                self.status_message = Some("GWT pane ID not available".to_string());
                self.status_message_time = Some(Instant::now());
                return;
            };

            // FR-037: Hide any currently active pane before showing this one
            self.hide_active_agent_pane();

            // Re-fetch the pane since hide_active_agent_pane may have modified the list
            let Some(pane) = self.pane_list.panes.get_mut(selected_idx) else {
                return;
            };
            let pane_id = pane.pane_id.clone();
            match gwt_core::tmux::show_pane(&pane_id, &gwt_pane_id) {
                Ok(new_pane_id) => {
                    // Update the pane ID and clear background state
                    pane.pane_id = new_pane_id.clone();
                    pane.is_background = false;
                    pane.background_window = None;

                    // Update agent_panes list
                    self.agent_panes.push(new_pane_id.clone());

                    self.branch_list
                        .update_running_agents(&self.pane_list.panes);
                    self.reflow_agent_layout(Some(&new_pane_id));

                    // Focus the pane
                    if let Err(e) = gwt_core::tmux::pane::select_pane(&new_pane_id) {
                        self.status_message = Some(format!("Failed to focus pane: {}", e));
                        self.status_message_time = Some(Instant::now());
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("Failed to show pane: {}", e));
                    self.status_message_time = Some(Instant::now());
                }
            }
        } else {
            // Pane is visible, just focus it
            if let Err(e) = gwt_core::tmux::pane::select_pane(&pane.pane_id) {
                self.status_message = Some(format!("Failed to focus pane: {}", e));
                self.status_message_time = Some(Instant::now());
            }
        }
    }

    /// Handle Enter key on branch list
    /// Always opens wizard with branch action options
    /// If agent is running: "Focus agent pane" / "Create new branch from this"
    /// If no agent: "Use selected branch" / "Create new from selected"
    fn handle_branch_enter(&mut self) {
        let Some(branch) = self.branch_list.selected_branch() else {
            return;
        };
        let branch_name = branch.name.clone();

        // Check if agent is running for this branch
        let running_pane_idx = self
            .pane_list
            .panes
            .iter()
            .position(|p| p.branch_name == branch_name);

        // Always open wizard (pass running_pane_idx to show appropriate options)
        // FR-050: Load session history for Quick Start feature
        let ts_history = get_branch_tool_history(&self.repo_root, &branch_name);
        let history: Vec<QuickStartEntry> = ts_history
            .into_iter()
            .map(|entry| QuickStartEntry {
                tool_id: entry.tool_id,
                tool_label: entry.tool_label,
                model: entry.model,
                reasoning_level: entry.reasoning_level,
                version: entry.tool_version,
                session_id: entry.session_id,
                skip_permissions: entry.skip_permissions,
                collaboration_modes: entry.collaboration_modes,
                docker_service: entry.docker_service,
                docker_force_host: entry.docker_force_host,
                docker_recreate: entry.docker_recreate,
                docker_build: None,
                docker_keep: entry.docker_keep,
            })
            .collect();
        self.wizard
            .open_for_branch(&branch_name, history, running_pane_idx);
    }

    fn handle_branch_list_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::BranchList) {
            self.last_mouse_click = None;
            return;
        }
        if self.wizard.visible {
            self.last_mouse_click = None;
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        // Check if clicking on a URL
        let clicked_url = self.branch_list.link_at_point(mouse.column, mouse.row);

        let Some(index) = self
            .branch_list
            .selection_index_from_point(mouse.column, mouse.row)
        else {
            self.last_mouse_click = None;
            return;
        };

        if self.branch_list.is_cleanup_target_index(index) {
            self.last_mouse_click = None;
            return;
        }

        let now = Instant::now();
        let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
            last.index == index && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
        });

        if is_double_click {
            self.last_mouse_click = None;
            // Double click: open URL or wizard
            if let Some(url) = clicked_url {
                self.open_url(&url);
            } else {
                self.handle_branch_enter();
            }
        } else {
            // Single click: select branch and record for potential double click
            if self.branch_list.select_index(index) {
                self.refresh_branch_summary();
            }
            self.last_mouse_click = Some(MouseClick { index, at: now });
        }
    }

    fn handle_branch_list_scroll(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::BranchList) {
            return;
        }
        if self.wizard.visible || self.ai_wizard.visible {
            return;
        }
        if !self
            .branch_list
            .session_panel_contains(mouse.column, mouse.row)
        {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.branch_list.scroll_session_line_up(),
            MouseEventKind::ScrollDown => self.branch_list.scroll_session_line_down(),
            _ => {}
        }
    }

    fn handle_wizard_mouse(&mut self, mouse: MouseEvent) {
        if !self.wizard.visible {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        // Check if clicking outside popup area - close wizard
        if !self.wizard.is_point_in_popup(mouse.column, mouse.row) {
            self.wizard.close();
            self.last_mouse_click = None;
            return;
        }

        // Check if clicking on a list item
        let Some(index) = self
            .wizard
            .selection_index_from_point(mouse.column, mouse.row)
        else {
            self.last_mouse_click = None;
            return;
        };

        let now = Instant::now();
        let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
            last.index == index && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
        });

        if is_double_click {
            self.last_mouse_click = None;
            // Double click: confirm selection (equivalent to Enter)
            self.update(Message::WizardConfirm);
        } else {
            // Single click: select item and record for potential double click
            if self.wizard.set_selection_index(index) {
                // Selection changed
            }
            self.last_mouse_click = Some(MouseClick { index, at: now });
        }
    }

    fn handle_confirm_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Confirm) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        // Check if clicking outside popup area - close confirm dialog
        if !self.confirm.is_point_in_popup(mouse.column, mouse.row) {
            // Pop screen stack to return to previous screen
            if let Some(prev_screen) = self.screen_stack.pop() {
                self.screen = prev_screen;
            }
            self.last_mouse_click = None;
            return;
        }

        // Check if clicking on cancel button
        if self.confirm.is_cancel_button_at(mouse.column, mouse.row) {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == 0 && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if is_double_click {
                self.last_mouse_click = None;
                // Double click on cancel: close dialog
                if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                }
            } else {
                self.confirm.select_cancel();
                self.last_mouse_click = Some(MouseClick { index: 0, at: now });
            }
            return;
        }

        // Check if clicking on confirm button
        if self.confirm.is_confirm_button_at(mouse.column, mouse.row) {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == 1 && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if is_double_click {
                self.last_mouse_click = None;
                // Double click on confirm: execute confirm action
                self.handle_confirm_action();
            } else {
                self.confirm.select_confirm();
                self.last_mouse_click = Some(MouseClick { index: 1, at: now });
            }
        }
    }

    fn handle_service_select_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::ServiceSelect) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        if !self.service_select.handle_click(mouse.column, mouse.row) {
            self.last_mouse_click = None;
            return;
        }

        let index = self.service_select.selected;
        let now = Instant::now();
        let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
            last.index == index && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
        });

        if is_double_click {
            self.last_mouse_click = None;
            self.handle_service_select_confirm();
        } else {
            self.last_mouse_click = Some(MouseClick { index, at: now });
        }
    }

    fn handle_docker_confirm_launch_on_host(&mut self) -> bool {
        let plan = if let Some(pending) = self.pending_cleanup_select.take() {
            pending.plan
        } else if let Some(pending) = self.pending_build_select.take() {
            pending.plan
        } else if let Some(pending) = self.pending_recreate_select.take() {
            pending.plan
        } else {
            return false;
        };

        // Ensure no stale pending prompts remain after the host override.
        self.pending_cleanup_select = None;
        self.pending_build_select = None;
        self.pending_recreate_select = None;
        self.launch_status = None;
        self.last_mouse_click = None;

        let keep_launch_status = matches!(plan.install_plan, InstallPlan::Install { .. });
        self.launch_plan_in_tmux(
            &plan,
            None,
            true,
            keep_launch_status,
            None,
            false,
            false,
            false,
        );

        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }

        true
    }

    fn handle_confirm_action(&mut self) {
        if let Some(pending) = self.pending_cleanup_select.clone() {
            let stop_on_exit = self.confirm.is_confirmed();
            let keep_launch_status =
                matches!(pending.plan.install_plan, InstallPlan::Install { .. });
            if self.maybe_request_port_selection(
                &pending.plan,
                pending.service.as_deref(),
                pending.force_host,
                keep_launch_status,
                pending.build,
                pending.force_recreate,
                stop_on_exit,
            ) {
                // Port selection is shown. Keep the cleanup prompt state so users can go back.
                return;
            }

            // Launch started without port selection; clear pending state and close prompt.
            self.pending_cleanup_select = None;
            if let Some(prev_screen) = self.screen_stack.pop() {
                self.screen = prev_screen;
            }
            return;
        }

        if let Some(pending) = self.pending_recreate_select.take() {
            let force_recreate = self.confirm.is_confirmed();
            let keep_launch_status =
                matches!(pending.plan.install_plan, InstallPlan::Install { .. });
            self.maybe_request_build_selection(
                &pending.plan,
                pending.service.as_deref(),
                pending.force_host,
                keep_launch_status,
                force_recreate,
                None,
            );
            if let Some(prev_screen) = self.screen_stack.pop() {
                self.screen = prev_screen;
            }
            return;
        }

        if let Some(pending) = self.pending_build_select.take() {
            let build = self.confirm.is_confirmed();
            let keep_launch_status =
                matches!(pending.plan.install_plan, InstallPlan::Install { .. });
            self.maybe_request_cleanup_selection(
                &pending.plan,
                pending.service.as_deref(),
                pending.force_host,
                keep_launch_status,
                build,
                pending.force_recreate,
                pending.quick_start_keep,
            );
            if let Some(prev_screen) = self.screen_stack.pop() {
                self.screen = prev_screen;
            }
            return;
        }

        if self.confirm.is_confirmed() {
            // FR-029d: Handle unsafe branch selection confirmation
            if let Some(branch_name) = self.pending_unsafe_selection.take() {
                // User confirmed - add branch to selection (FR-030)
                self.branch_list.selected_branches.insert(branch_name);
            }
            // FR-010: Handle cleanup confirmation
            if !self.pending_cleanup_branches.is_empty() {
                let branches = std::mem::take(&mut self.pending_cleanup_branches);
                self.execute_cleanup(&branches);
            }
            // FR-040: Handle agent termination confirmation
            if self.pending_agent_termination.is_some() {
                self.update(Message::ExecuteAgentTermination);
            }
            // SPEC-861d8cdf T-104: Handle hook setup confirmation
            if self.pending_hook_setup {
                if let Some(settings_path) = get_claude_settings_path() {
                    if let Err(e) = register_gwt_hooks(&settings_path) {
                        debug!(category = "tui", error = %e, "Failed to register gwt hooks");
                    }
                }
            }
            // SPEC-f8dab6e2 T-110: Handle plugin setup confirmation
            if self.pending_plugin_setup {
                if let Err(e) = setup_gwt_plugin() {
                    debug!(category = "tui", error = %e, "Failed to setup gwt plugin");
                }
            }
            if let Some(plan) = self.pending_docker_host_launch.take() {
                let keep_launch_status = matches!(plan.install_plan, InstallPlan::Install { .. });
                self.launch_plan_in_tmux(
                    &plan,
                    None,
                    true,
                    keep_launch_status,
                    None,
                    false,
                    false,
                    false,
                );
            }
        }

        // SPEC-f8dab6e2: Continue agent launch after plugin setup confirmation
        let pending_launch = self.pending_plugin_setup_launch.take();

        // Clear pending state and return to previous screen
        self.pending_unsafe_selection = None;
        self.pending_agent_termination = None;
        self.pending_cleanup_branches.clear();
        self.pending_hook_setup = false;
        self.pending_plugin_setup = false;
        self.pending_docker_host_launch = None;
        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }

        // Continue agent launch if pending (after plugin setup dialog)
        if let Some(plan) = pending_launch {
            self.handle_launch_plan_internal(plan);
        }
    }

    fn handle_profiles_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Profiles) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        // Don't handle mouse clicks in create mode
        if self.profiles.create_mode {
            return;
        }

        if let Some(index) = self
            .profiles
            .selection_index_from_point(mouse.column, mouse.row)
        {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == index
                    && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if is_double_click {
                self.last_mouse_click = None;
                // Double click: open environment screen for selected profile (same as Enter)
                if self.profiles.select_index(index) {
                    self.update(Message::Enter);
                }
            } else {
                // Single click: just select the profile
                self.profiles.select_index(index);
                self.last_mouse_click = Some(MouseClick { index, at: now });
            }
        }
    }

    fn handle_environment_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Environment) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        // Don't handle mouse clicks in edit mode
        if self.environment.edit_mode {
            return;
        }

        if let Some(index) = self
            .environment
            .selection_index_from_point(mouse.column, mouse.row)
        {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == index
                    && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if is_double_click {
                self.last_mouse_click = None;
                // Double click: enter edit mode for selected item (same as Enter)
                if self.environment.select_index(index) {
                    self.update(Message::Enter);
                }
            } else {
                // Single click: just select the item
                self.environment.select_index(index);
                self.last_mouse_click = Some(MouseClick { index, at: now });
            }
        }
    }

    fn handle_logs_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::Logs) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }
        // Don't handle mouse clicks during search or detail view
        if self.logs.is_searching || self.logs.is_detail_shown() {
            return;
        }

        if let Some(index) = self
            .logs
            .selection_index_from_point(mouse.column, mouse.row)
        {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == index
                    && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if is_double_click {
                self.last_mouse_click = None;
                // Double click: show detail view for selected log entry (same as Enter)
                if self.logs.select_index(index) {
                    self.update(Message::Enter);
                }
            } else {
                // Single click: just select the log entry
                self.logs.select_index(index);
                self.last_mouse_click = Some(MouseClick { index, at: now });
            }
        }
    }

    /// SPEC-1ea18899: Handle mouse events for GitView screen (FR-007)
    fn handle_gitview_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.screen, Screen::GitView) {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        // Check if click is within PR link region
        if let Some(ref link_region) = self.git_view.pr_link_region {
            let x = mouse.column;
            let y = mouse.row;
            if y == link_region.area.y
                && x >= link_region.area.x
                && x < link_region.area.x + link_region.area.width
            {
                // Click on PR link - open in browser
                let url = link_region.url.clone();
                self.open_url(&url);
            }
        }
    }

    /// Handle mouse events for the error popup
    fn handle_error_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Click outside the popup area closes it (simplified: any click dismisses)
                // In a more sophisticated implementation, we'd track popup bounds
                self.error_queue.dismiss_current();
                if self.error_queue.is_empty() {
                    self.update(Message::NavigateBack);
                }
            }
            MouseEventKind::ScrollUp => {
                // Scroll up in details
                if let Some(error) = self.error_queue.current_mut() {
                    error.scroll_up();
                }
            }
            MouseEventKind::ScrollDown => {
                // Scroll down in details
                if let Some(error) = self.error_queue.current_mut() {
                    error.scroll_down();
                }
            }
            _ => {}
        }
    }

    fn open_url(&mut self, url: &str) {
        if url.trim().is_empty() {
            self.status_message = Some("Failed to open URL: empty URL".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }

        let mut last_error: Option<std::io::Error> = None;
        let mut try_open = |cmd: &str, args: &[&str]| -> bool {
            match Command::new(cmd)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
            {
                Ok(output) => {
                    if output.status.success() {
                        return true;
                    }
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let message = stderr.trim();
                    let message = if message.is_empty() {
                        format!("{} exited with status {}", cmd, output.status)
                    } else {
                        message.to_string()
                    };
                    last_error = Some(std::io::Error::other(message));
                    false
                }
                Err(err) => {
                    last_error = Some(err);
                    false
                }
            }
        };
        #[cfg(target_os = "macos")]
        {
            if try_open("open", &[url]) {
                return;
            }
        }

        #[cfg(target_os = "linux")]
        {
            let candidates = [
                ("xdg-open", vec![url]),
                ("gio", vec!["open", url]),
                ("x-www-browser", vec![url]),
            ];
            for (cmd, args) in candidates {
                if try_open(cmd, &args) {
                    return;
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            if try_open("cmd", &["/C", "start", "", url]) {
                return;
            }

            if try_open(
                "powershell",
                &["-NoProfile", "-Command", "Start-Process", url],
            ) {
                return;
            }
        }

        if std::env::var_os("TMUX").is_some()
            && std::process::Command::new("tmux")
                .args(["set-buffer", "--", url])
                .spawn()
                .is_ok()
        {
            self.status_message = Some(format!("URL copied to tmux buffer: {}", url));
            self.status_message_time = Some(Instant::now());
            return;
        }

        let mut extra_hint = String::new();
        if std::path::Path::new("/.dockerenv").exists()
            || std::env::var_os("container").is_some()
            || std::env::var_os("DOCKER_CONTAINER").is_some()
        {
            let write_ok = std::fs::write("/tmp/gwt-open-url.txt", url).is_ok();
            if write_ok {
                extra_hint = " URL saved to /tmp/gwt-open-url.txt".to_string();
            }
        }

        let err = last_error.unwrap_or_else(|| std::io::Error::other("unknown error"));
        let message = if err.kind() == std::io::ErrorKind::NotFound {
            #[cfg(target_os = "linux")]
            {
                format!(
                    "Failed to open URL: opener not found (xdg-open/gio/x-www-browser). URL: {}{}",
                    url, extra_hint
                )
            }
            #[cfg(target_os = "macos")]
            {
                format!(
                    "Failed to open URL: opener not found (open). URL: {}{}",
                    url, extra_hint
                )
            }
            #[cfg(target_os = "windows")]
            {
                format!(
                    "Failed to open URL: opener not found (cmd/powershell). URL: {}{}",
                    url, extra_hint
                )
            }
        } else {
            format!("Failed to open URL: {}. URL: {}{}", err, url, extra_hint)
        };
        self.status_message = Some(message);
        self.status_message_time = Some(Instant::now());
    }

    /// Check if currently selected branch has a running agent
    fn selected_branch_has_agent(&self) -> bool {
        self.branch_list
            .selected_branch()
            .map(|branch| self.branch_list.get_running_agent(&branch.name).is_some())
            .unwrap_or(false)
    }

    fn load_profiles(&mut self) {
        let profiles_config = ProfilesConfig::load().unwrap_or_default();
        self.profiles_config = profiles_config.clone();

        let mut names: Vec<String> = profiles_config.profiles.keys().cloned().collect();
        names.sort();

        let mut profiles = Vec::new();
        let (default_ai_label, default_ai_enabled) = match &profiles_config.default_ai {
            Some(ai) if ai.is_enabled() => ("AI: on".to_string(), true),
            Some(_) => ("AI: off".to_string(), false),
            None => ("AI: off".to_string(), false),
        };
        profiles.push(super::screens::profiles::ProfileItem {
            name: "AI (default)".to_string(),
            is_active: false,
            env_count: 0,
            description: Some("Global AI settings".to_string()),
            ai_label: default_ai_label,
            ai_enabled: default_ai_enabled,
            is_default_ai: true,
        });

        let profile_items: Vec<_> = names
            .into_iter()
            .filter_map(|name| {
                profiles_config.profiles.get(&name).map(|profile| {
                    let (ai_label, ai_enabled) = match &profile.ai {
                        Some(ai) if ai.is_enabled() => ("AI: override".to_string(), true),
                        Some(_) => ("AI: off".to_string(), false),
                        None => match &profiles_config.default_ai {
                            Some(default_ai) if default_ai.is_enabled() => {
                                ("AI: default".to_string(), true)
                            }
                            _ => ("AI: off".to_string(), false),
                        },
                    };
                    super::screens::profiles::ProfileItem {
                        name: name.clone(),
                        is_active: profiles_config.active.as_deref() == Some(name.as_str()),
                        env_count: profile.env.len(),
                        description: if profile.description.is_empty() {
                            None
                        } else {
                            Some(profile.description.clone())
                        },
                        ai_label,
                        ai_enabled,
                        is_default_ai: false,
                    }
                })
            })
            .collect();

        profiles.extend(profile_items);

        self.profiles = ProfilesState::new().with_profiles(profiles);
        self.branch_list.active_profile = self.profiles_config.active.clone();
        self.branch_list.ai_enabled = self.active_ai_enabled();
        self.branch_list.session_summary_enabled = self.active_session_summary_enabled();
    }

    fn save_profiles(&mut self) {
        if let Err(e) = self.profiles_config.save() {
            self.status_message = Some(format!("Failed to save profiles: {}", e));
            self.status_message_time = Some(Instant::now());
            return;
        }
        self.load_profiles();
        self.refresh_branch_summary();
    }

    fn active_ai_settings(&self) -> Option<ResolvedAISettings> {
        if let Some(profile) = self.profiles_config.active_profile() {
            if let Some(settings) = profile.resolved_ai_settings() {
                return Some(settings);
            }
        }
        self.profiles_config
            .default_ai
            .as_ref()
            .map(|settings| settings.resolved())
    }

    fn active_ai_enabled(&self) -> bool {
        if let Some(profile) = self.profiles_config.active_profile() {
            if profile.ai.is_some() {
                return profile.ai_enabled();
            }
        }
        self.profiles_config
            .default_ai
            .as_ref()
            .map(|settings| settings.is_enabled())
            .unwrap_or(false)
    }

    fn active_session_summary_enabled(&self) -> bool {
        if let Some(profile) = self.profiles_config.active_profile() {
            if let Some(settings) = profile.ai.as_ref() {
                return settings.is_summary_enabled();
            }
        }
        self.profiles_config
            .default_ai
            .as_ref()
            .map(|settings| settings.is_summary_enabled())
            .unwrap_or(false)
    }

    fn active_env_overrides(&self) -> Vec<(String, String)> {
        self.profiles_config
            .active_profile()
            .map(|profile| {
                let mut pairs: Vec<(String, String)> = profile
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                pairs
            })
            .unwrap_or_default()
    }

    fn active_env_removals(&self) -> Vec<String> {
        self.profiles_config
            .active_profile()
            .map(|profile| {
                let mut removals = profile.disabled_env.clone();
                removals.retain(|key| !profile.env.contains_key(key));
                removals.sort();
                removals.dedup();
                removals
            })
            .unwrap_or_default()
    }

    fn open_environment_editor(&mut self, profile_name: &str) {
        let (
            vars,
            disabled_keys,
            ai_enabled,
            ai_endpoint,
            ai_api_key,
            ai_model,
            ai_summary_enabled,
        ) = self
            .profiles_config
            .profiles
            .get(profile_name)
            .map(|profile| {
                let mut items: Vec<super::screens::environment::EnvItem> = profile
                    .env
                    .iter()
                    .map(|(k, v)| super::screens::environment::EnvItem {
                        key: k.clone(),
                        value: v.clone(),
                        is_secret: false,
                    })
                    .collect();
                items.sort_by(|a, b| a.key.cmp(&b.key));
                let (ai_enabled, ai_endpoint, ai_api_key, ai_model, ai_summary_enabled) =
                    match &profile.ai {
                        Some(ai) => (
                            true,
                            ai.endpoint.clone(),
                            ai.api_key.clone(),
                            ai.model.clone(),
                            ai.summary_enabled,
                        ),
                        None => {
                            let defaults = AISettings::default();
                            (
                                false,
                                defaults.endpoint,
                                String::new(),
                                defaults.model,
                                true,
                            )
                        }
                    };
                (
                    items,
                    profile.disabled_env.clone(),
                    ai_enabled,
                    ai_endpoint,
                    ai_api_key,
                    ai_model,
                    ai_summary_enabled,
                )
            })
            .unwrap_or_else(|| {
                let defaults = AISettings::default();
                (
                    Vec::new(),
                    Vec::new(),
                    false,
                    defaults.endpoint,
                    String::new(),
                    defaults.model,
                    true,
                )
            });

        self.environment = EnvironmentState::new()
            .with_profile(profile_name)
            .with_variables(vars)
            .with_disabled_keys(disabled_keys)
            .with_ai_settings(
                ai_enabled,
                ai_endpoint,
                ai_api_key,
                ai_model,
                ai_summary_enabled,
            )
            .with_os_variables(collect_os_env());
        self.environment.selected = 3;
        self.environment.refresh_selection();
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::Environment;
    }

    fn open_default_ai_editor(&mut self) {
        // FR-100: Use AI settings wizard for default AI settings
        if let Some(ai) = &self.profiles_config.default_ai {
            // Edit existing settings
            self.ai_wizard.open_edit(
                true, // is_default_ai
                None, // no profile name
                &ai.endpoint,
                &ai.api_key,
                &ai.model,
                ai.summary_enabled,
            );
        } else {
            // Create new settings
            self.ai_wizard.open_new(true, None);
        }
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::AISettingsWizard;
    }

    /// Open AI settings wizard for a specific profile
    fn open_profile_ai_editor(&mut self, profile_name: &str) {
        // FR-100: Use AI settings wizard for profile AI settings
        if let Some(profile) = self.profiles_config.profiles.get(profile_name) {
            if let Some(ai) = &profile.ai {
                // Edit existing settings
                self.ai_wizard.open_edit(
                    false, // not default AI
                    Some(profile_name.to_string()),
                    &ai.endpoint,
                    &ai.api_key,
                    &ai.model,
                    ai.summary_enabled,
                );
            } else {
                // Create new settings
                self.ai_wizard
                    .open_new(false, Some(profile_name.to_string()));
            }
            self.screen_stack.push(self.screen.clone());
            self.screen = Screen::AISettingsWizard;
        }
    }

    fn open_ai_settings_for_agent_mode(&mut self) {
        if let Some(profile_name) = self.profiles_config.active.clone() {
            if self.profiles_config.profiles.contains_key(&profile_name) {
                self.open_profile_ai_editor(&profile_name);
                return;
            }
        }
        self.open_default_ai_editor();
    }

    /// Handle Enter key in Settings screen (SPEC-71f2742d US3)
    fn handle_settings_enter(&mut self) {
        use super::screens::settings::{AgentFormField, CustomAgentMode, SettingsCategory};

        match self.settings.category {
            SettingsCategory::CustomAgents => {
                match &self.settings.custom_agent_mode {
                    CustomAgentMode::List => {
                        // Enter on list: add or edit
                        if self.settings.is_add_agent_selected() {
                            self.settings.enter_add_mode();
                        } else if self.settings.selected_custom_agent().is_some() {
                            self.settings.enter_edit_mode();
                        }
                    }
                    CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
                        // Enter in form: save if on last field, otherwise cycle type or next field
                        if self.settings.agent_form.current_field == AgentFormField::Type {
                            self.settings.agent_form.cycle_type();
                        } else if self.settings.agent_form.current_field == AgentFormField::Command
                        {
                            // On last field, try to save
                            match self.settings.save_agent() {
                                Ok(()) => {
                                    // Save to file
                                    if let Some(ref config) = self.settings.tools_config {
                                        if let Err(e) = config.save_global() {
                                            self.settings.error_message =
                                                Some(format!("Failed to save: {}", e));
                                        }
                                    }
                                }
                                Err(msg) => {
                                    self.settings.error_message = Some(msg.to_string());
                                }
                            }
                        } else {
                            self.settings.agent_form.next_field();
                        }
                    }
                    CustomAgentMode::ConfirmDelete(_) => {
                        // Enter in delete confirm: execute if Yes selected
                        if self.settings.delete_confirm {
                            if self.settings.delete_agent() {
                                // Save to file
                                if let Some(ref config) = self.settings.tools_config {
                                    if let Err(e) = config.save_global() {
                                        self.settings.error_message =
                                            Some(format!("Failed to save: {}", e));
                                    }
                                }
                            }
                        } else {
                            self.settings.cancel_mode();
                        }
                    }
                }
            }
            SettingsCategory::Environment => {
                use super::screens::settings::ProfileMode;
                match &self.settings.profile_mode {
                    ProfileMode::List => {
                        // FR-029: Enter opens environment variable edit mode
                        if self.settings.is_add_profile_selected() {
                            // Enter add mode for new profile
                            self.settings.enter_profile_add_mode();
                        } else if self.settings.selected_profile().is_some() {
                            // Enter env edit mode for selected profile
                            self.settings.enter_env_edit_mode();
                        }
                    }
                    ProfileMode::Add | ProfileMode::Edit(_) => {
                        // Save profile
                        if let Err(e) = self.settings.save_profile() {
                            self.settings.error_message = Some(e.to_string());
                        } else {
                            // Save to file
                            if let Some(ref config) = self.settings.profiles_config {
                                if let Err(e) = config.save() {
                                    self.settings.error_message =
                                        Some(format!("Failed to save: {}", e));
                                } else {
                                    self.load_profiles();
                                }
                            }
                        }
                    }
                    ProfileMode::ConfirmDelete(_) => {
                        // Confirm delete
                        if self.settings.profile_delete_confirm {
                            if self.settings.delete_profile() {
                                // Save to file
                                if let Some(ref config) = self.settings.profiles_config {
                                    if let Err(e) = config.save() {
                                        self.settings.error_message =
                                            Some(format!("Failed to save: {}", e));
                                    } else {
                                        self.load_profiles();
                                    }
                                }
                            }
                        } else {
                            self.settings.cancel_profile_mode();
                        }
                    }
                    ProfileMode::EnvEdit(_) => {
                        // Use EnvironmentState methods (SPEC-dafff079)
                        let env_state = &mut self.settings.env_state;
                        if env_state.edit_mode {
                            if env_state.is_new && env_state.edit_field == EditField::Key {
                                // New variable: Enter on key field switches to value input (FR-016)
                                env_state.switch_field();
                                return;
                            }
                            // Finish editing - validate and apply
                            if let Some(ai_field) = env_state.editing_ai_field() {
                                match env_state.validate_ai_value() {
                                    Ok(value) => {
                                        env_state.apply_ai_value(ai_field, value);
                                        env_state.cancel_edit();
                                        self.persist_settings_env_changes();
                                    }
                                    Err(msg) => {
                                        env_state.error = Some(msg.to_string());
                                    }
                                }
                            } else {
                                match env_state.validate() {
                                    Ok((key, value)) => {
                                        if env_state.is_new {
                                            // New variable: add
                                            env_state.variables.push(
                                                super::screens::environment::EnvItem {
                                                    key,
                                                    value,
                                                    is_secret: false,
                                                },
                                            );
                                        } else if let Some(index) =
                                            env_state.selected_profile_index()
                                        {
                                            // Existing variable: update
                                            if let Some(var) = env_state.variables.get_mut(index) {
                                                var.key = key;
                                                var.value = value;
                                            }
                                        }
                                        env_state.cancel_edit();
                                        env_state.refresh_selection();
                                        self.persist_settings_env_changes();
                                    }
                                    Err(msg) => {
                                        env_state.error = Some(msg.to_string());
                                    }
                                }
                            }
                        } else {
                            // Start editing selected item
                            env_state.start_edit_selected();
                        }
                    }
                }
            }
            SettingsCategory::AISettings => {
                if self.settings.is_ai_clear_mode() {
                    if self.settings.ai_clear_confirm {
                        self.clear_default_ai_settings();
                    }
                    self.settings.cancel_ai_clear_confirm();
                    return;
                }
                // Enter: open AI Settings Wizard
                // Check if default_ai exists in profiles_config
                if let Some(ai) = &self.profiles_config.default_ai {
                    // Edit existing settings
                    self.ai_wizard.open_edit(
                        true, // is_default_ai
                        None, // no profile name
                        &ai.endpoint,
                        &ai.api_key,
                        &ai.model,
                        ai.summary_enabled,
                    );
                } else {
                    // Create new settings
                    self.ai_wizard.open_new(true, None);
                }
                self.screen_stack.push(self.screen.clone());
                self.screen = Screen::AISettingsWizard;
            }
            _ => {}
        }
    }

    /// Handle character input in Settings screen (SPEC-71f2742d US3)
    fn handle_settings_char(&mut self, c: char) {
        use super::screens::settings::{AgentFormField, CustomAgentMode, SettingsCategory};

        match self.settings.category {
            SettingsCategory::CustomAgents => {
                match &self.settings.custom_agent_mode {
                    CustomAgentMode::List => {
                        // 'd' or 'D' to enter delete mode
                        if (c == 'd' || c == 'D') && self.settings.selected_custom_agent().is_some()
                        {
                            self.settings.enter_delete_mode();
                        }
                    }
                    CustomAgentMode::Add | CustomAgentMode::Edit(_) => {
                        // In form mode: insert char or cycle type
                        if self.settings.agent_form.current_field == AgentFormField::Type {
                            if c == ' ' {
                                self.settings.agent_form.cycle_type();
                            }
                        } else {
                            self.settings.agent_form.insert_char(c);
                        }
                    }
                    CustomAgentMode::ConfirmDelete(_) => {
                        // In delete confirm: ignore chars
                    }
                }
            }
            SettingsCategory::Environment => {
                use super::screens::settings::ProfileMode;
                match &self.settings.profile_mode {
                    ProfileMode::List => {
                        // 'd' or 'D' to enter delete mode
                        if (c == 'd' || c == 'D') && self.settings.selected_profile().is_some() {
                            self.settings.enter_profile_delete_mode();
                        }
                        // FR-030: 'e' or 'E' to enter profile edit mode (name/description)
                        else if (c == 'e' || c == 'E')
                            && self.settings.selected_profile().is_some()
                        {
                            self.settings.enter_profile_edit_mode();
                        }
                        // Space to toggle active profile
                        else if c == ' ' && self.settings.selected_profile().is_some() {
                            self.settings.toggle_active_profile();
                            // Save to file
                            if let Some(ref config) = self.settings.profiles_config {
                                let _ = config.save();
                                self.load_profiles();
                            }
                        }
                    }
                    ProfileMode::Add | ProfileMode::Edit(_) => {
                        // Insert character in form
                        self.settings.profile_form.insert_char(c);
                    }
                    ProfileMode::ConfirmDelete(_) => {
                        // Ignore chars
                    }
                    ProfileMode::EnvEdit(_) => {
                        // Use EnvironmentState methods (SPEC-dafff079)
                        let env_state = &mut self.settings.env_state;
                        if env_state.edit_mode {
                            // Insert character while editing
                            env_state.insert_char(c);
                        } else {
                            // Handle special keys when not editing
                            match c {
                                'n' | 'N' => {
                                    // Add new variable
                                    env_state.start_new();
                                }
                                'd' | 'D' => {
                                    if env_state.selected_is_overridden() {
                                        self.status_message = Some(
                                            "Use 'r' to reset overridden environment variable."
                                                .to_string(),
                                        );
                                        self.status_message_time = Some(Instant::now());
                                    } else if env_state.selected_is_os_entry() {
                                        // SPEC-dafff079 FR-020: Toggle disable for OS variables
                                        let disabled = env_state.toggle_selected_disabled();
                                        self.status_message = Some(if disabled {
                                            "OS environment variable disabled.".to_string()
                                        } else {
                                            "OS environment variable enabled.".to_string()
                                        });
                                        self.status_message_time = Some(Instant::now());
                                        self.persist_settings_env_changes();
                                    } else if let Some(index) = env_state.selected_profile_index() {
                                        if index < env_state.variables.len() {
                                            env_state.variables.remove(index);
                                            env_state.refresh_selection();
                                            self.persist_settings_env_changes();
                                        }
                                    }
                                }
                                'r' | 'R' => {
                                    // SPEC-dafff079 FR-019: Reset to OS value
                                    // Delete the override (profile variable) to reveal OS value
                                    if env_state.selected_is_overridden() {
                                        env_state.delete_selected_override();
                                        env_state.refresh_selection();
                                        self.status_message = Some(
                                            "Environment variable reset to OS value.".to_string(),
                                        );
                                        self.status_message_time = Some(Instant::now());
                                        self.persist_settings_env_changes();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            SettingsCategory::AISettings => {
                if self.settings.is_ai_clear_mode() {
                    return;
                }
                match c {
                    't' | 'T' => {
                        self.toggle_default_ai_summary();
                    }
                    'c' | 'C' => {
                        if self.profiles_config.default_ai.is_some() {
                            self.settings.enter_ai_clear_confirm();
                        } else {
                            self.status_message = Some("No AI settings configured.".to_string());
                            self.status_message_time = Some(Instant::now());
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                // Other categories don't handle char input
            }
        }
    }

    /// Handle Backspace in EnvEdit mode (SPEC-dafff079)
    fn handle_settings_env_backspace(&mut self) {
        let env_state = &mut self.settings.env_state;
        if env_state.edit_mode {
            env_state.delete_char();
        }
    }

    fn toggle_default_ai_summary(&mut self) {
        if let Some(ai) = self.profiles_config.default_ai.as_mut() {
            ai.summary_enabled = !ai.summary_enabled;
            self.save_profiles();
            self.settings.load_profiles_config();
        } else {
            self.status_message = Some("No AI settings configured.".to_string());
            self.status_message_time = Some(Instant::now());
        }
    }

    fn clear_default_ai_settings(&mut self) {
        if self.profiles_config.default_ai.is_none() {
            self.status_message = Some("No AI settings configured.".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }
        self.profiles_config.default_ai = None;
        self.save_profiles();
        self.settings.load_profiles_config();
    }
    fn persist_environment(&mut self) {
        if self.environment.is_ai_only() {
            if self.environment.ai_enabled {
                if self.environment.ai_fields_empty() {
                    self.profiles_config.default_ai = None;
                    self.environment.ai_enabled = false;
                } else {
                    self.profiles_config.default_ai = Some(AISettings {
                        endpoint: self.environment.ai_endpoint.clone(),
                        api_key: self.environment.ai_api_key.clone(),
                        model: self.environment.ai_model.clone(),
                        summary_enabled: self.environment.ai_summary_enabled,
                    });
                }
            } else {
                self.profiles_config.default_ai = None;
            }
            self.save_profiles();
            return;
        }

        let profile_name = match self.environment.profile_name.clone() {
            Some(name) => name,
            None => return,
        };
        let profile = self
            .profiles_config
            .profiles
            .entry(profile_name.clone())
            .or_insert_with(|| Profile::new(profile_name.clone()));
        profile.env.clear();
        for var in &self.environment.variables {
            profile.env.insert(var.key.clone(), var.value.clone());
        }
        let mut disabled = self.environment.disabled_keys.clone();
        disabled.retain(|key| !profile.env.contains_key(key));
        disabled.sort();
        disabled.dedup();
        profile.disabled_env = disabled.clone();
        self.environment.disabled_keys = disabled;
        if self.environment.ai_enabled {
            if self.environment.ai_fields_empty() {
                profile.ai = None;
                self.environment.ai_enabled = false;
            } else {
                profile.ai = Some(AISettings {
                    endpoint: self.environment.ai_endpoint.clone(),
                    api_key: self.environment.ai_api_key.clone(),
                    model: self.environment.ai_model.clone(),
                    summary_enabled: self.environment.ai_summary_enabled,
                });
            }
        } else {
            profile.ai = None;
        }
        self.save_profiles();
    }

    fn persist_settings_env_changes(&mut self) {
        match self.settings.persist_env_edit() {
            Ok(_) => {
                self.load_profiles();
            }
            Err(message) => {
                self.status_message =
                    Some(format!("Failed to save environment changes: {}", message));
                self.status_message_time = Some(Instant::now());
            }
        }
    }

    fn delete_selected_profile(&mut self) {
        let selected = match self.profiles.selected_profile() {
            Some(item) => item.name.clone(),
            None => return,
        };
        if let Some(item) = self.profiles.selected_profile() {
            if item.is_default_ai {
                self.status_message = Some("Default AI settings cannot be deleted.".to_string());
                self.status_message_time = Some(Instant::now());
                return;
            }
        }

        if self.profiles_config.active.as_deref() == Some(selected.as_str()) {
            self.status_message = Some("Active profile cannot be deleted.".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }

        self.profiles_config.profiles.remove(&selected);
        if self.profiles_config.profiles.is_empty() {
            self.profiles_config = ProfilesConfig::default();
        }
        self.save_profiles();
    }

    fn activate_selected_profile(&mut self) {
        let selected = match self.profiles.selected_profile() {
            Some(item) => item.name.clone(),
            None => return,
        };
        if let Some(item) = self.profiles.selected_profile() {
            if item.is_default_ai {
                self.status_message = Some("Default AI settings are not a profile.".to_string());
                self.status_message_time = Some(Instant::now());
                return;
            }
        }
        self.profiles_config.set_active(Some(selected.clone()));
        self.save_profiles();
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|profile| profile.name == selected)
        {
            self.profiles.selected = index;
        }
    }

    fn delete_selected_env(&mut self) {
        if self.environment.selected_is_overridden() {
            self.status_message =
                Some("Use 'r' to reset overridden environment variable.".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }
        if self.environment.selected_is_os_entry() {
            if let Some(key) = self.environment.selected_key() {
                let disabled = self.environment.toggle_disabled_key(&key);
                self.persist_environment();
                self.status_message = Some(if disabled {
                    "OS environment variable disabled.".to_string()
                } else {
                    "OS environment variable enabled.".to_string()
                });
                self.status_message_time = Some(Instant::now());
            }
            return;
        }
        let Some(selected_index) = self.environment.selected_profile_index() else {
            return;
        };

        if selected_index < self.environment.variables.len() {
            self.environment.variables.remove(selected_index);
            self.environment.refresh_selection();
            self.persist_environment();
        }
    }

    fn reset_selected_env(&mut self) {
        if !self.environment.selected_is_overridden() {
            self.status_message = Some("No overridden environment variable selected.".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }
        let Some(selected_index) = self.environment.selected_profile_index() else {
            return;
        };
        if selected_index < self.environment.variables.len() {
            self.environment.variables.remove(selected_index);
            self.environment.refresh_selection();
            self.persist_environment();
            self.status_message = Some("Environment variable reset to OS value.".to_string());
            self.status_message_time = Some(Instant::now());
        }
    }

    /// Update function (Elm Architecture)
    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::Quit => {
                self.should_quit = true;
            }
            Message::CtrlC => {
                let now = Instant::now();
                if let Some(last) = self.last_ctrl_c {
                    if now.duration_since(last) < Duration::from_secs(2) {
                        self.ctrl_c_count += 1;
                        if self.ctrl_c_count >= 2 {
                            self.should_quit = true;
                        }
                    } else {
                        self.ctrl_c_count = 1;
                    }
                } else {
                    self.ctrl_c_count = 1;
                }
                self.last_ctrl_c = Some(now);
                self.status_message = Some("Press Ctrl+C again to quit".to_string());
                self.status_message_time = Some(Instant::now());
            }
            Message::NavigateTo(screen) => {
                // SPEC-71f2742d US3: Load tools config when entering Settings
                if matches!(screen, Screen::Settings) {
                    self.settings.load_tools_config();
                    self.settings.load_profiles_config();
                }
                // SPEC-1ea18899: Initialize GitViewState when entering GitView
                if matches!(screen, Screen::GitView) {
                    if let Some(branch) = self.branch_list.selected_branch().cloned() {
                        let worktree_path = branch.worktree_path.clone().map(PathBuf::from);
                        let worktree_path_for_fetch = worktree_path.clone();
                        self.git_view = GitViewState::new(
                            branch.name.clone(),
                            worktree_path,
                            branch.pr_number,
                            branch.pr_url.clone(),
                            branch.pr_title.clone(),
                            branch.pr_state.clone(),
                            branch.divergence,
                        );
                        let has_pr = branch.pr_number.is_some()
                            || branch.pr_title.is_some()
                            || branch.pr_state.is_some();
                        if !has_pr {
                            self.spawn_git_view_pr_fetch(&branch.name);
                        }
                        // Try to load from cache
                        if let Some(cached) = self.git_view_cache.get(&branch.name) {
                            self.git_view.load_from_cache(cached);
                        } else {
                            // Trigger background data fetch
                            self.spawn_git_view_data_fetch(
                                &branch.name,
                                worktree_path_for_fetch.as_deref(),
                            );
                        }
                    }
                }
                self.screen_stack.push(self.screen.clone());
                self.screen = screen;
                self.status_message = None;
                self.reset_footer_scroll();
            }
            Message::NavigateBack => {
                // Check if we're in filter mode first
                if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode {
                    self.branch_list.exit_filter_mode();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    // Exit profile create mode
                    self.profiles.exit_create_mode();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.cancel_edit();
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Exit log search mode
                    self.logs.toggle_search();
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_detail_shown() {
                    // Close log detail view
                    self.logs.close_detail();
                } else if matches!(self.screen, Screen::ServiceSelect) {
                    // Cancel service selection
                    self.pending_service_select = None;
                    self.launch_status = None;
                    self.last_mouse_click = None;
                    if let Some(prev_screen) = self.screen_stack.pop() {
                        self.screen = prev_screen;
                    }
                } else if matches!(self.screen, Screen::PortSelect) {
                    // Cancel port selection (or close custom input)
                    if self.port_select.custom_input.is_some() {
                        self.port_select.cancel_custom_input();
                    } else {
                        self.pending_port_select = None;
                        self.launch_status = None;
                        self.last_mouse_click = None;
                        if let Some(prev_screen) = self.screen_stack.pop() {
                            self.screen = prev_screen;
                        }
                    }
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029d: Cancel confirm dialog without executing action
                    self.pending_unsafe_selection = None;
                    self.pending_cleanup_branches.clear();
                    self.pending_hook_setup = false;
                    self.pending_plugin_setup = false;
                    self.pending_plugin_setup_launch = None;
                    self.pending_docker_host_launch = None;
                    self.pending_build_select = None;
                    self.pending_recreate_select = None;
                    self.pending_cleanup_select = None;
                    self.pending_port_select = None;
                    self.launch_status = None;
                    if let Some(prev_screen) = self.screen_stack.pop() {
                        self.screen = prev_screen;
                    }
                // SPEC-71f2742d US3: Cancel form/delete mode in Settings
                } else if matches!(self.screen, Screen::Settings) {
                    // Check Profile modes first
                    if self.settings.is_ai_clear_mode() {
                        self.settings.cancel_ai_clear_confirm();
                    } else if self.settings.is_profile_form_mode()
                        || self.settings.is_profile_delete_mode()
                    {
                        self.settings.cancel_profile_mode();
                    } else if self.settings.is_env_edit_mode() {
                        if self.settings.env_state.edit_mode {
                            // Cancel current env edit without leaving EnvEdit mode
                            self.settings.env_state.cancel_edit();
                        } else {
                            // Exit EnvEdit mode without saving (auto-save happens on edit)
                            self.settings.cancel_profile_mode();
                        }
                    } else if self.settings.is_form_mode() || self.settings.is_delete_mode() {
                        // CustomAgents mode
                        self.settings.cancel_mode();
                    } else if let Some(prev_screen) = self.screen_stack.pop() {
                        // Not in any special mode, navigate back
                        self.screen = prev_screen;
                    }
                } else if matches!(self.screen, Screen::AISettingsWizard) {
                    // Go back in AI wizard or close if at first step
                    if self.ai_wizard.show_delete_confirm {
                        self.ai_wizard.cancel_delete();
                    } else {
                        self.ai_wizard.prev_step();
                        if !self.ai_wizard.visible {
                            self.ai_wizard_rx = None;
                            // Wizard was closed
                            if let Some(prev_screen) = self.screen_stack.pop() {
                                self.screen = prev_screen;
                                if matches!(self.screen, Screen::AgentMode) {
                                    self.update_agent_mode_ai_status();
                                }
                            }
                        }
                    }
                } else if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                    if matches!(self.screen, Screen::AgentMode) {
                        self.update_agent_mode_ai_status();
                    }
                }
                self.status_message = None;
                self.reset_footer_scroll();
            }
            Message::Tick => {
                // Reset Ctrl+C counter after timeout
                if let Some(last) = self.last_ctrl_c {
                    if Instant::now().duration_since(last) > Duration::from_secs(2) {
                        self.ctrl_c_count = 0;
                    }
                }
                // Auto-clear status message after 3 seconds
                if let Some(time) = self.status_message_time {
                    if Instant::now().duration_since(time) > Duration::from_secs(3) {
                        self.status_message = None;
                        self.status_message_time = None;
                    }
                }
                // Update spinner animation
                self.branch_list.tick_spinner();
                self.update_footer_scroll();
                self.apply_branch_list_updates();
                self.apply_branch_summary_updates();
                self.apply_pr_title_updates();
                self.apply_safety_updates();
                self.apply_worktree_updates();
                self.apply_cleanup_updates();
                self.apply_launch_updates();
                // FR-051: Auto-close progress modal after 2-second summary display
                if let Some(ref modal) = self.progress_modal {
                    if modal.completed && modal.summary_display_elapsed() && !modal.has_failed() {
                        self.progress_modal = None;
                    }
                }
                self.apply_session_summary_updates();
                self.apply_agent_mode_updates();
                self.apply_ai_wizard_updates();
                self.poll_session_summary_if_needed();
                // SPEC-1ea18899: Apply GitView cache updates
                self.apply_git_view_cache_updates();
                // SPEC-1ea18899: Apply GitView PR updates
                self.apply_git_view_pr_updates();
                // FR-033: Update pane list every 1 second in tmux multi mode
                self.update_pane_list();
                // SPEC-a70a1ece: Poll clone wizard progress
                if matches!(self.screen, Screen::CloneWizard) && self.clone_wizard.is_cloning() {
                    self.clone_wizard.poll_clone();
                }
                // SPEC-a70a1ece: Poll migration progress
                if matches!(self.screen, Screen::MigrationDialog)
                    && matches!(
                        self.migration_dialog.phase,
                        MigrationDialogPhase::InProgress | MigrationDialogPhase::Validating
                    )
                {
                    if let Some(ref rx) = self.migration_rx {
                        match rx.try_recv() {
                            Ok(Ok(())) => {
                                self.migration_dialog.phase = MigrationDialogPhase::Completed;
                                self.migration_rx = None;
                                // SPEC-a70a1ece: Update repo_type to Bare after successful migration
                                self.repo_type = RepoType::Bare;
                                // Update bare_name and bare_repo_path from migration config
                                if let Some(ref config) = self.migration_dialog.config {
                                    self.bare_name = Some(config.bare_repo_name.clone());
                                    self.bare_repo_path =
                                        Some(self.repo_root.join(&config.bare_repo_name));
                                }
                                self.startup_branch = None;
                            }
                            Ok(Err(e)) => {
                                self.migration_dialog.phase = MigrationDialogPhase::Failed;
                                self.migration_dialog.error = Some(format!("{}", e));
                                self.migration_rx = None;
                            }
                            Err(mpsc::TryRecvError::Empty) => {
                                // Still running, keep polling
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {
                                // Thread crashed or channel closed
                                self.migration_dialog.phase = MigrationDialogPhase::Failed;
                                self.migration_dialog.error =
                                    Some("Migration thread disconnected".to_string());
                                self.migration_rx = None;
                            }
                        }
                    }
                }
            }
            Message::SelectNext => match self.screen {
                Screen::BranchList => {
                    self.branch_list.select_next();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::AgentMode => {}
                Screen::WorktreeCreate => self.worktree_create.select_next_base(),
                Screen::Settings => self.settings.select_next(),
                Screen::Logs => self.logs.select_next(),
                Screen::Help => self.help.scroll_down(),
                Screen::Error => {
                    if let Some(error) = self.error_queue.current_mut() {
                        error.scroll_down();
                    }
                }
                Screen::Profiles => self.profiles.select_next(),
                Screen::Environment => self.environment.select_next(),
                Screen::ServiceSelect => self.service_select.select_next(),
                Screen::PortSelect => {
                    if self.port_select.custom_input.is_none() {
                        self.port_select.select_next();
                    }
                }
                Screen::AISettingsWizard => self.ai_wizard.select_next_model(),
                Screen::CloneWizard => self.clone_wizard.down(),
                Screen::MigrationDialog => self.migration_dialog.toggle_selection(),
                Screen::Confirm => {}
                // SPEC-1ea18899: GitView navigation
                Screen::GitView => self.git_view.select_next(),
            },
            Message::SelectPrev => match self.screen {
                Screen::BranchList => {
                    self.branch_list.select_prev();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::AgentMode => {}
                Screen::WorktreeCreate => self.worktree_create.select_prev_base(),
                Screen::Settings => self.settings.select_prev(),
                Screen::Logs => self.logs.select_prev(),
                Screen::Help => self.help.scroll_up(),
                Screen::Error => {
                    if let Some(error) = self.error_queue.current_mut() {
                        error.scroll_up();
                    }
                }
                Screen::Profiles => self.profiles.select_prev(),
                Screen::Environment => self.environment.select_prev(),
                Screen::ServiceSelect => self.service_select.select_previous(),
                Screen::PortSelect => {
                    if self.port_select.custom_input.is_none() {
                        self.port_select.select_previous();
                    }
                }
                Screen::AISettingsWizard => self.ai_wizard.select_prev_model(),
                Screen::CloneWizard => self.clone_wizard.up(),
                Screen::MigrationDialog => self.migration_dialog.toggle_selection(),
                Screen::Confirm => {}
                // SPEC-1ea18899: GitView navigation
                Screen::GitView => self.git_view.select_prev(),
            },
            Message::PageUp => match self.screen {
                Screen::BranchList => {
                    self.branch_list.scroll_session_page_up();
                }
                Screen::Logs => self.logs.page_up(10),
                Screen::Help => self.help.page_up(),
                Screen::Environment => self.environment.page_up(),
                _ => {}
            },
            Message::PageDown => match self.screen {
                Screen::BranchList => {
                    self.branch_list.scroll_session_page_down();
                }
                Screen::Logs => self.logs.page_down(10),
                Screen::Help => self.help.page_down(),
                Screen::Environment => self.environment.page_down(),
                _ => {}
            },
            Message::GoHome => match self.screen {
                Screen::BranchList => {
                    self.branch_list.go_home();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::Logs => self.logs.go_home(),
                Screen::Environment => self.environment.go_home(),
                _ => {}
            },
            Message::GoEnd => match self.screen {
                Screen::BranchList => {
                    self.branch_list.go_end();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::Logs => self.logs.go_end(),
                Screen::Environment => self.environment.go_end(),
                _ => {}
            },
            Message::Enter => match &self.screen {
                Screen::BranchList => {
                    if self.branch_list.filter_mode {
                        // FR-020: Enter in filter mode exits filter mode
                        self.branch_list.exit_filter_mode();
                    } else {
                        // FR-040: Enter on branch with running agent focuses the pane
                        // FR-041: Enter on branch without agent opens wizard
                        self.handle_branch_enter();
                    }
                }
                Screen::AgentMode => {
                    if !self.agent_mode.ai_ready {
                        self.open_ai_settings_for_agent_mode();
                    } else if self.agent_mode.is_waiting {
                        // Ignore input while waiting for response
                    } else if !self.agent_mode.input.trim().is_empty() {
                        let content = self.agent_mode.input.trim().to_string();
                        self.agent_mode.messages.push(AgentMessage {
                            role: AgentRole::User,
                            content,
                        });
                        self.agent_mode.clear_input();
                        self.agent_mode.last_error = None;
                        self.agent_mode.set_waiting(true);
                        let messages = self.agent_mode.messages.clone();
                        self.spawn_agent_mode_request(messages);
                    }
                }
                Screen::WorktreeCreate => {
                    if self.worktree_create.is_confirm_step() {
                        // Create the worktree
                        self.create_worktree();
                    } else {
                        self.worktree_create.next_step();
                    }
                }
                Screen::Confirm => {
                    self.handle_confirm_action();
                }
                Screen::ServiceSelect => {
                    self.handle_service_select_confirm();
                }
                Screen::PortSelect => {
                    self.handle_port_select_confirm();
                }
                Screen::Profiles => {
                    if self.profiles.create_mode {
                        match self.profiles.validate_new_name() {
                            Ok(name) => {
                                if self.profiles_config.profiles.contains_key(&name) {
                                    self.profiles.error =
                                        Some("Profile already exists".to_string());
                                } else {
                                    self.profiles_config
                                        .profiles
                                        .insert(name.clone(), Profile::new(&name));
                                    self.profiles_config.set_active(Some(name.clone()));
                                    self.profiles.exit_create_mode();
                                    self.save_profiles();
                                }
                            }
                            Err(msg) => {
                                self.profiles.error = Some(msg.to_string());
                            }
                        }
                    } else if let Some(item) = self.profiles.selected_profile() {
                        if item.is_default_ai {
                            self.open_default_ai_editor();
                        } else {
                            let name = item.name.clone();
                            self.open_environment_editor(&name);
                        }
                    }
                }
                Screen::Environment => {
                    if self.environment.edit_mode {
                        if let Some(ai_field) = self.environment.editing_ai_field() {
                            match self.environment.validate_ai_value() {
                                Ok(value) => {
                                    self.environment.apply_ai_value(ai_field, value);
                                    self.environment.cancel_edit();
                                    self.environment.refresh_selection();
                                    self.persist_environment();
                                }
                                Err(msg) => {
                                    self.environment.error = Some(msg.to_string());
                                }
                            }
                        } else {
                            match self.environment.validate() {
                                Ok((key, value)) => {
                                    if self.environment.is_new {
                                        self.environment.variables.push(
                                            super::screens::environment::EnvItem {
                                                key,
                                                value,
                                                is_secret: false,
                                            },
                                        );
                                    } else if let Some(index) =
                                        self.environment.selected_profile_index()
                                    {
                                        if let Some(var) = self.environment.variables.get_mut(index)
                                        {
                                            var.key = key;
                                            var.value = value;
                                        }
                                    }
                                    self.environment.cancel_edit();
                                    self.environment.refresh_selection();
                                    self.persist_environment();
                                }
                                Err(msg) => {
                                    self.environment.error = Some(msg.to_string());
                                }
                            }
                        }
                    } else {
                        self.environment.start_edit_selected();
                    }
                }
                Screen::Help => {
                    self.update(Message::NavigateBack);
                }
                Screen::Error => {
                    // Dismiss current error and show next, or close if no more errors
                    self.error_queue.dismiss_current();
                    if self.error_queue.is_empty() {
                        self.update(Message::NavigateBack);
                    }
                }
                Screen::Logs => {
                    self.logs.toggle_detail();
                }
                Screen::AISettingsWizard => {
                    self.handle_ai_wizard_enter();
                }
                // SPEC-71f2742d US3: Settings screen Enter handling
                Screen::Settings => {
                    self.handle_settings_enter();
                }
                // SPEC-a70a1ece US3: Clone wizard Enter handling
                Screen::CloneWizard => {
                    if self.clone_wizard.is_complete() {
                        // Clone successful - reinitialize with new bare repo
                        if let Some(cloned_path) = self.clone_wizard.cloned_path.take() {
                            let msg = format!("Cloned to {}", cloned_path.display());
                            // SPEC-a70a1ece: Change process working directory to bare repo
                            // This ensures subsequent TUI restarts detect the correct repo type
                            if let Err(e) = std::env::set_current_dir(&cloned_path) {
                                debug!(
                                    category = "tui",
                                    error = %e,
                                    path = %cloned_path.display(),
                                    "Failed to change working directory to bare repo"
                                );
                            }
                            self.repo_root = cloned_path;
                            self.repo_type = RepoType::Bare;
                            self.refresh_data();
                            self.screen = Screen::BranchList;
                            self.status_message = Some(msg);
                            self.status_message_time = Some(Instant::now());
                        }
                    } else {
                        self.clone_wizard.next();
                    }
                }
                // SPEC-a70a1ece T709-T710: Migration dialog Enter handling
                Screen::MigrationDialog => {
                    if self.migration_dialog.phase == MigrationDialogPhase::Confirmation {
                        if self.migration_dialog.selected_proceed {
                            self.migration_dialog.accept();
                            // Start actual migration in background
                            if let Some(config) = self.migration_dialog.config.clone() {
                                let (tx, rx) = mpsc::channel();
                                self.migration_rx = Some(rx);
                                std::thread::spawn(move || {
                                    let result =
                                        gwt_core::migration::execute_migration(&config, None);
                                    let _ = tx.send(result);
                                });
                                self.migration_dialog.phase = MigrationDialogPhase::InProgress;
                            } else {
                                // No config, show error
                                self.migration_dialog.phase = MigrationDialogPhase::Failed;
                                self.migration_dialog.error =
                                    Some("Migration config not available".to_string());
                            }
                        } else {
                            // T710: User chose to exit - quit gwt
                            self.migration_dialog.reject();
                            self.should_quit = true;
                        }
                    } else if matches!(
                        self.migration_dialog.phase,
                        MigrationDialogPhase::Completed | MigrationDialogPhase::Failed
                    ) {
                        // Continue or exit after migration completion/failure
                        if self.migration_dialog.is_completed() {
                            self.screen = Screen::BranchList;
                            self.refresh_data();
                        } else {
                            self.should_quit = true;
                        }
                    }
                }
                // SPEC-1ea18899: GitView Enter handling - open PR link (FR-007)
                Screen::GitView => {
                    // If PR link is selected (selected_index == 0 and pr_url exists), open it
                    if self.git_view.selected_index == 0 {
                        if let Some(url) = self.git_view.pr_url.clone() {
                            self.open_url(&url);
                        }
                    }
                }
            },
            Message::Char(c) => {
                if matches!(self.screen, Screen::Confirm)
                    && (c == 'h' || c == 'H')
                    && self.handle_docker_confirm_launch_on_host()
                {
                    return;
                }
                if matches!(self.screen, Screen::ServiceSelect) {
                    if c == 's' || c == 'S' {
                        self.handle_service_select_skip();
                    }
                } else if matches!(self.screen, Screen::PortSelect) {
                    if self.port_select.custom_input.is_some() {
                        if c.is_ascii_digit() {
                            self.port_select.insert_custom_char(c);
                        }
                    } else if c == 'c' || c == 'C' {
                        self.port_select.open_custom_input();
                    } else if c == 'a' || c == 'A' {
                        self.port_select.reset_selected_to_suggested();
                    }
                } else if matches!(self.screen, Screen::CloneWizard)
                    && self.clone_wizard.step == CloneWizardStep::UrlInput
                {
                    self.clone_wizard.handle_char(c);
                } else if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.insert_char(c);
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode
                {
                    // Filter mode - add character to filter
                    self.branch_list.filter_push(c);
                    self.refresh_branch_summary();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    // Profile create mode - add character to name
                    self.profiles.insert_char(c);
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.insert_char(c);
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Log search mode - add character to search
                    self.logs.search.push(c);
                } else if matches!(self.screen, Screen::AgentMode) && self.agent_mode.ai_ready {
                    self.agent_mode.insert_char(c);
                // SPEC-71f2742d US3: Settings screen character input
                } else if matches!(self.screen, Screen::Settings) {
                    self.handle_settings_char(c);
                } else if matches!(self.screen, Screen::AISettingsWizard) {
                    if self.ai_wizard.show_delete_confirm {
                        // Handle delete confirmation
                        if c == 'y' || c == 'Y' {
                            self.delete_ai_wizard_settings();
                        } else if c == 'n' || c == 'N' {
                            self.ai_wizard.cancel_delete();
                        }
                    } else if self.ai_wizard.is_text_input() {
                        // Text input mode: insert character (including 'd')
                        self.ai_wizard.insert_char(c);
                    } else if matches!(
                        self.ai_wizard.step,
                        super::screens::ai_wizard::AIWizardStep::ModelSelect
                    ) && (c == 't' || c == 'T')
                    {
                        self.ai_wizard.toggle_summary_enabled();
                    } else if matches!(
                        self.ai_wizard.step,
                        super::screens::ai_wizard::AIWizardStep::ModelSelect
                    ) && (c == 'c' || c == 'C' || c == 'd' || c == 'D')
                    {
                        // Show clear confirmation (only in edit mode, non-text-input steps)
                        if self.ai_wizard.is_edit {
                            self.ai_wizard.show_delete();
                        }
                    }
                } else if matches!(self.screen, Screen::Error) {
                    // Error screen shortcuts
                    match c {
                        'l' | 'L' => {
                            // Navigate to Logs screen
                            self.screen_stack.push(self.screen.clone());
                            self.screen = Screen::Logs;
                        }
                        'c' | 'C' => {
                            // Copy error to clipboard as JSON
                            if let Some(error) = self.error_queue.current() {
                                let json = error.to_json();
                                match arboard::Clipboard::new() {
                                    Ok(mut clipboard) => match clipboard.set_text(&json) {
                                        Ok(()) => {
                                            self.status_message =
                                                Some("Error copied to clipboard".to_string());
                                        }
                                        Err(e) => {
                                            self.status_message =
                                                Some(format!("Failed to copy: {}", e));
                                        }
                                    },
                                    Err(e) => {
                                        self.status_message =
                                            Some(format!("Clipboard unavailable: {}", e));
                                    }
                                }
                                self.status_message_time = Some(Instant::now());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::Backspace => {
                if matches!(self.screen, Screen::CloneWizard) {
                    if self.clone_wizard.step == CloneWizardStep::UrlInput {
                        self.clone_wizard.handle_backspace();
                    } else {
                        self.clone_wizard.prev();
                    }
                } else if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.delete_char();
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode
                {
                    self.branch_list.filter_pop();
                    self.refresh_branch_summary();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.delete_char();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.delete_char();
                } else if matches!(self.screen, Screen::PortSelect)
                    && self.port_select.custom_input.is_some()
                {
                    self.port_select.backspace_custom();
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Log search mode - delete character
                    self.logs.search.pop();
                } else if matches!(self.screen, Screen::AgentMode) && self.agent_mode.ai_ready {
                    self.agent_mode.backspace();
                // SPEC-71f2742d US3: Settings screen backspace
                } else if matches!(self.screen, Screen::Settings) {
                    if self.settings.is_profile_form_mode() {
                        self.settings.profile_form.delete_char();
                    } else if self.settings.is_env_edit_mode() {
                        self.handle_settings_env_backspace();
                    } else if self.settings.is_form_mode() {
                        self.settings.agent_form.delete_char();
                    }
                } else if matches!(self.screen, Screen::AISettingsWizard)
                    && self.ai_wizard.is_text_input()
                {
                    self.ai_wizard.delete_char();
                }
            }
            Message::CursorLeft => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_left();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.cursor_left();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.cursor_left();
                } else if matches!(self.screen, Screen::PortSelect)
                    && self.port_select.custom_input.is_none()
                {
                    self.port_select.cycle_candidate_prev();
                } else if matches!(self.screen, Screen::Settings)
                    && self.settings.is_env_edit_mode()
                    && self.settings.env_state.edit_mode
                {
                    self.settings.env_state.cursor_left();
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029c: Left/Right toggle selection in confirm dialog
                    self.confirm.toggle_selection();
                // SPEC-71f2742d US3: Settings delete confirmation toggle (CustomAgents or Profile)
                } else if matches!(self.screen, Screen::Settings)
                    && (self.settings.is_delete_mode()
                        || self.settings.is_profile_delete_mode()
                        || self.settings.is_ai_clear_mode())
                {
                    if self.settings.is_profile_delete_mode() {
                        self.settings.profile_delete_confirm =
                            !self.settings.profile_delete_confirm;
                    } else if self.settings.is_delete_mode() {
                        self.settings.delete_confirm = !self.settings.delete_confirm;
                    } else {
                        self.settings.ai_clear_confirm = !self.settings.ai_clear_confirm;
                    }
                // SPEC-71f2742d US4: Settings category navigation with Left/Right
                } else if matches!(self.screen, Screen::Settings)
                    && !self.settings.is_form_mode()
                    && !self.settings.is_delete_mode()
                    && !self.settings.is_profile_delete_mode()
                    && !self.settings.is_ai_clear_mode()
                    && !self.settings.is_env_edit_mode()
                {
                    self.settings.prev_category();
                } else if matches!(self.screen, Screen::AISettingsWizard)
                    && self.ai_wizard.is_text_input()
                {
                    self.ai_wizard.cursor_left();
                } else if matches!(self.screen, Screen::AgentMode) && self.agent_mode.ai_ready {
                    self.agent_mode.cursor_left();
                }
            }
            Message::CursorRight => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_right();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.cursor_right();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.cursor_right();
                } else if matches!(self.screen, Screen::PortSelect)
                    && self.port_select.custom_input.is_none()
                {
                    self.port_select.cycle_candidate_next();
                } else if matches!(self.screen, Screen::Settings)
                    && self.settings.is_env_edit_mode()
                    && self.settings.env_state.edit_mode
                {
                    self.settings.env_state.cursor_right();
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029c: Left/Right toggle selection in confirm dialog
                    self.confirm.toggle_selection();
                // SPEC-71f2742d US3: Settings delete confirmation toggle (CustomAgents or Profile)
                } else if matches!(self.screen, Screen::Settings)
                    && (self.settings.is_delete_mode()
                        || self.settings.is_profile_delete_mode()
                        || self.settings.is_ai_clear_mode())
                {
                    if self.settings.is_profile_delete_mode() {
                        self.settings.profile_delete_confirm =
                            !self.settings.profile_delete_confirm;
                    } else if self.settings.is_delete_mode() {
                        self.settings.delete_confirm = !self.settings.delete_confirm;
                    } else {
                        self.settings.ai_clear_confirm = !self.settings.ai_clear_confirm;
                    }
                // SPEC-71f2742d US4: Settings category navigation with Left/Right
                } else if matches!(self.screen, Screen::Settings)
                    && !self.settings.is_form_mode()
                    && !self.settings.is_delete_mode()
                    && !self.settings.is_profile_delete_mode()
                    && !self.settings.is_ai_clear_mode()
                    && !self.settings.is_env_edit_mode()
                {
                    self.settings.next_category();
                } else if matches!(self.screen, Screen::AISettingsWizard)
                    && self.ai_wizard.is_text_input()
                {
                    self.ai_wizard.cursor_right();
                } else if matches!(self.screen, Screen::AgentMode) && self.agent_mode.ai_ready {
                    self.agent_mode.cursor_right();
                }
            }
            Message::RefreshData => {
                self.refresh_data();
            }
            Message::CopyLogToClipboard => {
                if matches!(self.screen, Screen::Logs) {
                    if let Some(entry) = self.logs.selected_entry() {
                        let text = entry.to_clipboard_string();
                        match arboard::Clipboard::new() {
                            Ok(mut clipboard) => match clipboard.set_text(&text) {
                                Ok(()) => {
                                    self.status_message = Some("Copied to clipboard".to_string());
                                }
                                Err(e) => {
                                    self.status_message = Some(format!("Failed to copy: {}", e));
                                }
                            },
                            Err(e) => {
                                self.status_message = Some(format!("Clipboard unavailable: {}", e));
                            }
                        }
                        self.status_message_time = Some(Instant::now());
                    }
                }
            }
            Message::HideActiveAgentPane => {
                self.hide_active_agent_pane();
            }
            Message::ConfirmAgentTermination => {
                // FR-041: Show confirmation dialog before terminating agent
                if let Some(branch) = self.branch_list.selected_branch() {
                    if self.branch_list.get_running_agent(&branch.name).is_some() {
                        self.pending_agent_termination = Some(branch.name.clone());
                        self.confirm = ConfirmState {
                            title: "Terminate Agent".to_string(),
                            message: format!("Terminate agent on '{}'?", branch.name),
                            details: vec!["The agent process will be killed.".to_string()],
                            confirm_label: "Terminate".to_string(),
                            cancel_label: "Cancel".to_string(),
                            selected_confirm: true, // Default to Terminate
                            is_dangerous: true,
                            ..Default::default()
                        };
                        self.screen_stack.push(self.screen.clone());
                        self.screen = Screen::Confirm;
                    }
                }
            }
            Message::ExecuteAgentTermination => {
                // FR-042: Execute tmux kill-pane
                if let Some(branch_name) = self.pending_agent_termination.take() {
                    self.terminate_agent_pane(&branch_name);
                }
            }
            Message::ReregisterHooks => {
                // FR-102g: Manually re-register Claude Code hooks
                // FR-102i: Show warning if running from temporary execution environment
                if let Some(exe_path) = is_temporary_execution() {
                    // Show confirmation dialog for temporary execution
                    self.pending_hook_setup = true;
                    self.confirm = ConfirmState::hook_setup_with_warning(&exe_path);
                    self.screen_stack.push(self.screen.clone());
                    self.screen = Screen::Confirm;
                } else if let Some(settings_path) = get_claude_settings_path() {
                    match register_gwt_hooks(&settings_path) {
                        Ok(()) => {
                            self.status_message = Some("Claude Code hooks registered.".to_string());
                            self.status_message_time = Some(Instant::now());
                            debug!(
                                category = "hooks",
                                "Manually re-registered Claude Code hooks"
                            );
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Failed to register hooks: {}", e));
                            self.status_message_time = Some(Instant::now());
                            warn!(
                                category = "hooks",
                                error = %e,
                                "Failed to manually re-register Claude Code hooks"
                            );
                        }
                    }
                } else {
                    self.status_message = Some("Could not find Claude settings path.".to_string());
                    self.status_message_time = Some(Instant::now());
                }
            }
            // FR-020 SPEC-71f2742d: Tab cycles BranchList → AgentMode → Settings → BranchList
            Message::Tab => match self.screen {
                Screen::Settings => {
                    // In profile form mode, Tab cycles profile form fields
                    if self.settings.is_profile_form_mode() {
                        self.settings.profile_form.next_field();
                    } else if self.settings.is_env_edit_mode() {
                        // Toggle between Key/Value editing (SPEC-dafff079)
                        self.settings.env_state.switch_field();
                    } else if self.settings.is_form_mode() {
                        // In agent form mode, Tab cycles agent form fields
                        self.settings.agent_form.next_field();
                    } else {
                        // Exit Settings and go to BranchList (FR-020)
                        self.screen = Screen::BranchList;
                    }
                }
                Screen::BranchList => {
                    if !self.branch_list.filter_mode {
                        self.enter_agent_mode();
                    }
                }
                Screen::AgentMode => {
                    // Go to Settings (FR-020)
                    self.settings.load_tools_config();
                    self.settings.load_profiles_config();
                    self.screen = Screen::Settings;
                }
                _ => {}
            },
            Message::CycleFilter => {
                if matches!(self.screen, Screen::Logs) {
                    self.logs.cycle_filter();
                }
            }
            Message::ToggleSearch => {
                if matches!(self.screen, Screen::Logs) {
                    self.logs.toggle_search();
                }
            }
            Message::ToggleFilterMode => {
                if matches!(self.screen, Screen::BranchList) {
                    self.branch_list.toggle_filter_mode();
                }
            }
            Message::CycleViewMode => {
                // FR-036: 'm' key disabled in filter mode
                if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                    self.branch_list.cycle_view_mode();
                    self.refresh_branch_summary();
                }
            }
            Message::CycleSortMode => {
                if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                    self.branch_list.cycle_sort_mode();
                    self.refresh_branch_summary();
                }
            }
            Message::ToggleSelection | Message::Space => {
                if matches!(self.screen, Screen::BranchList) {
                    // FR-029b-e: Check if branch is unsafe before selecting
                    if let Some(branch) = self.branch_list.selected_branch() {
                        if !branch.has_worktree {
                            // FR-028d: Branches without worktrees cannot be selected.
                            return;
                        }
                        let is_selected = self.branch_list.selected_branches.contains(&branch.name);

                        // Only show warning when selecting (not deselecting)
                        if !is_selected {
                            // Check if branch is unsafe (FR-029b/FR-029e)
                            let is_unsafe = branch.is_unsafe();

                            if is_unsafe && branch.branch_type == BranchType::Local {
                                // Show warning dialog for unsafe branch selection
                                self.confirm = ConfirmState::unsafe_selection_warning(
                                    &branch.name,
                                    branch.has_changes,
                                    branch.has_unpushed,
                                    branch.is_unmerged,
                                );
                                self.pending_unsafe_selection = Some(branch.name.clone());
                                self.screen_stack.push(self.screen.clone());
                                self.screen = Screen::Confirm;
                            } else {
                                // Safe to select directly
                                self.branch_list.toggle_selection();
                            }
                        } else {
                            // Always allow deselection
                            self.branch_list.toggle_selection();
                        }
                    }
                } else if matches!(self.screen, Screen::Profiles) && !self.profiles.create_mode {
                    self.activate_selected_profile();
                } else if matches!(self.screen, Screen::GitView) {
                    // SPEC-1ea18899: Toggle expand in GitView (FR-003)
                    self.git_view.toggle_expand();
                }
            }
            Message::OpenWizard => {
                // Open wizard for selected branch (FR-044)
                // Always open wizard regardless of worktree status (matches TypeScript behavior)
                if let Some(branch) = self.branch_list.selected_branch() {
                    let branch_name = branch.name.clone();
                    // Check if agent is running for this branch
                    let running_pane_idx = self
                        .pane_list
                        .panes
                        .iter()
                        .position(|p| p.branch_name == branch_name);
                    // FR-050: Load session history for Quick Start feature
                    let ts_history = get_branch_tool_history(&self.repo_root, &branch_name);
                    let history: Vec<QuickStartEntry> = ts_history
                        .into_iter()
                        .map(|entry| QuickStartEntry {
                            tool_id: entry.tool_id,
                            tool_label: entry.tool_label,
                            model: entry.model,
                            reasoning_level: entry.reasoning_level,
                            version: entry.tool_version,
                            session_id: entry.session_id,
                            skip_permissions: entry.skip_permissions,
                            collaboration_modes: entry.collaboration_modes,
                            docker_service: entry.docker_service,
                            docker_force_host: entry.docker_force_host,
                            docker_recreate: entry.docker_recreate,
                            docker_build: None,
                            docker_keep: entry.docker_keep,
                        })
                        .collect();
                    self.wizard
                        .open_for_branch(&branch_name, history, running_pane_idx);
                } else {
                    self.status_message = Some("No branch selected".to_string());
                    self.status_message_time = Some(Instant::now());
                }
            }
            Message::OpenWizardNewBranch => {
                // Open wizard for new branch
                self.wizard.open_for_new_branch();
            }
            Message::WizardNext => {
                if self.wizard.visible {
                    self.wizard.select_next();
                }
            }
            Message::WizardPrev => {
                if self.wizard.visible {
                    self.wizard.select_prev();
                }
            }
            Message::WizardConfirm => {
                if self.wizard.visible {
                    // SPEC-e4798383: Check for duplicate branch before confirming IssueSelect
                    if self.wizard.step == WizardStep::IssueSelect {
                        self.wizard.check_issue_duplicate(&self.repo_root);
                    }

                    let prev_step = self.wizard.step;
                    match self.wizard.confirm() {
                        WizardConfirmResult::Complete => {
                            // Start worktree creation with wizard settings
                            let branch_name = if self.wizard.is_new_branch {
                                self.wizard.full_branch_name()
                            } else {
                                self.wizard.branch_name.clone()
                            };
                            self.worktree_create.branch_name = branch_name;
                            self.worktree_create.branch_name_cursor =
                                self.worktree_create.branch_name.len();
                            self.worktree_create.create_new_branch = self.wizard.is_new_branch;
                            // Store wizard settings for later use
                            self.wizard.close();
                            // Create the worktree directly
                            self.create_worktree();
                        }
                        WizardConfirmResult::Advance => {
                            // SPEC-e4798383: Load issues when entering IssueSelect step
                            if self.wizard.step == WizardStep::IssueSelect
                                && prev_step != WizardStep::IssueSelect
                            {
                                self.wizard.load_issues(&self.repo_root);
                            }
                        }
                        WizardConfirmResult::FocusPane(pane_idx) => {
                            // Focus on existing agent pane (wizard already closed itself)
                            self.pane_list.selected = pane_idx;
                            self.show_and_focus_selected_pane();
                        }
                    }
                }
            }
            Message::WizardBack => {
                if self.wizard.visible {
                    self.wizard.prev_step();
                }
            }
        }
    }

    /// Create worktree from wizard state and prepare agent launch
    fn create_worktree(&mut self) {
        if self.launch_in_progress {
            self.status_message = Some("Launch already in progress".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }

        let branch = self.worktree_create.branch_name.clone();
        if branch.trim().is_empty() {
            self.status_message = Some("No branch selected".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }

        let base = if self.worktree_create.create_new_branch {
            self.wizard
                .base_branch_override
                .as_deref()
                .or_else(|| self.worktree_create.selected_base_branch())
                .map(|value| value.to_string())
        } else {
            None
        };
        let existing_worktree_path = resolve_existing_worktree_path(
            &branch,
            &self.branch_list.branches,
            self.worktree_create.create_new_branch,
        );

        let auto_install_deps = self
            .settings
            .settings
            .as_ref()
            .map(|settings| settings.agent.auto_install_deps)
            .unwrap_or(false);

        // SPEC-71f2742d: Get custom agent from selected_agent_entry
        let custom_agent = self
            .wizard
            .selected_agent_entry
            .as_ref()
            .and_then(|e| e.custom.clone());

        let request = LaunchRequest {
            branch_name: branch,
            create_new_branch: self.worktree_create.create_new_branch,
            base_branch: base,
            existing_worktree_path,
            agent: self.wizard.agent,
            custom_agent,
            model: if self.wizard.model.is_empty() {
                None
            } else {
                Some(self.wizard.model.clone())
            },
            reasoning_level: if self.wizard.agent == CodingAgent::CodexCli {
                Some(self.wizard.reasoning_level)
            } else {
                None
            },
            version: self.wizard.version.clone(),
            execution_mode: self.wizard.execution_mode,
            session_id: self.wizard.session_id.clone(),
            skip_permissions: self.wizard.skip_permissions,
            // SPEC-fdebd681: Collaboration modes for Codex v0.91.0+
            collaboration_modes: self.wizard.collaboration_modes,
            env: self.active_env_overrides(),
            env_remove: self.active_env_removals(),
            auto_install_deps,
            // SPEC-e4798383 US6: Pass selected issue for GitHub linking
            selected_issue: self.wizard.selected_issue.clone(),
        };

        self.pending_quick_start_docker = self.wizard.quick_start_docker.clone();
        self.start_launch_preparation(request);

        // Close wizard immediately so user can see launch progress in branch list
        self.wizard.visible = false;
        self.screen = Screen::BranchList;
    }

    fn start_launch_preparation(&mut self, request: LaunchRequest) {
        if self.launch_in_progress {
            return;
        }

        self.launch_in_progress = true;
        // FR-041: Show progress modal instead of status bar
        self.progress_modal = Some(ProgressModalState::new());
        // Clear launch_status since modal is showing (FR-057)
        self.launch_status = None;

        // SPEC-a70a1ece: Use bare repo path for worktree operations in bare projects
        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.launch_rx = Some(rx);

        thread::spawn(move || {
            let send = |update: LaunchUpdate| {
                let _ = tx.send(update);
            };

            // Helper to send progress step updates
            let step = |kind: ProgressStepKind, status: StepStatus| {
                let _ = tx.send(LaunchUpdate::ProgressStep { kind, status });
            };

            // Step 1: Fetch remote (initialize manager)
            step(ProgressStepKind::FetchRemote, StepStatus::Running);
            let manager = match WorktreeManager::new(&repo_root) {
                Ok(manager) => manager,
                Err(e) => {
                    let _ = tx.send(LaunchUpdate::ProgressStepError {
                        kind: ProgressStepKind::FetchRemote,
                        message: e.to_string(),
                    });
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };
            step(ProgressStepKind::FetchRemote, StepStatus::Completed);

            // Step 2: Validate branch (lightweight)
            step(ProgressStepKind::ValidateBranch, StepStatus::Running);
            let normalized_branch = normalize_branch_name_for_history(&request.branch_name);
            let existing_wt = manager.list_basic().ok().and_then(|worktrees| {
                if let Some(ref path) = request.existing_worktree_path {
                    worktrees
                        .iter()
                        .find(|wt| wt.path == *path)
                        .cloned()
                        .or_else(|| {
                            worktrees
                                .iter()
                                .find(|wt| wt.branch.as_deref() == Some(normalized_branch.as_ref()))
                                .cloned()
                        })
                } else {
                    worktrees
                        .iter()
                        .find(|wt| wt.branch.as_deref() == Some(normalized_branch.as_ref()))
                        .cloned()
                }
            });
            let has_existing_wt = existing_wt.is_some();
            step(ProgressStepKind::ValidateBranch, StepStatus::Completed);

            // Step 3: Generate path
            step(ProgressStepKind::GeneratePath, StepStatus::Running);
            // Path generation happens inside create_for_branch/create_new_branch
            step(ProgressStepKind::GeneratePath, StepStatus::Completed);

            // Step 4: Check conflicts (skip if existing worktree)
            if has_existing_wt {
                step(ProgressStepKind::CheckConflicts, StepStatus::Skipped);
                step(ProgressStepKind::CreateWorktree, StepStatus::Skipped);
            } else {
                step(ProgressStepKind::CheckConflicts, StepStatus::Running);
                step(ProgressStepKind::CheckConflicts, StepStatus::Completed);

                // Step 5: Create worktree
                step(ProgressStepKind::CreateWorktree, StepStatus::Running);
            }
            let result = if let Some(wt) = existing_wt {
                Ok(wt)
            } else if request.create_new_branch {
                // SPEC-e4798383 US6: Try GitHub Issue linking if issue is selected
                if let Some(ref issue) = request.selected_issue {
                    match gwt_core::git::create_linked_branch(
                        &repo_root,
                        issue.number,
                        &request.branch_name,
                    ) {
                        Ok(()) => {
                            // FR-019: Log success
                            tracing::info!("Branch linked to issue #{} on GitHub", issue.number);
                            // Branch created by gh, now create worktree for it
                            manager.create_for_branch(&request.branch_name)
                        }
                        Err(e) => {
                            // FR-017/FR-017a: Fallback to local branch creation with warning
                            tracing::warn!("GitHub linking failed, creating local branch: {}", e);
                            manager.create_new_branch(
                                &request.branch_name,
                                request.base_branch.as_deref(),
                            )
                        }
                    }
                } else {
                    // FR-018: No issue selected, use regular local branch creation
                    manager.create_new_branch(&request.branch_name, request.base_branch.as_deref())
                }
            } else {
                manager.create_for_branch(&request.branch_name)
            };

            let worktree = match result {
                Ok(wt) => {
                    if !has_existing_wt {
                        step(ProgressStepKind::CreateWorktree, StepStatus::Completed);
                    }
                    wt
                }
                Err(e) => {
                    let _ = tx.send(LaunchUpdate::ProgressStepError {
                        kind: ProgressStepKind::CreateWorktree,
                        message: e.to_string(),
                    });
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };

            let branch_name = worktree
                .branch
                .clone()
                .unwrap_or_else(|| request.branch_name.clone());
            send(LaunchUpdate::WorktreeReady {
                branch: branch_name,
                path: worktree.path.clone(),
            });

            // Step 6: Check dependencies
            step(ProgressStepKind::CheckDependencies, StepStatus::Running);

            let config = AgentLaunchConfig {
                repo_root: repo_root.clone(),
                worktree_path: worktree.path.clone(),
                branch_name: request.branch_name.clone(),
                agent: request.agent,
                custom_agent: request.custom_agent.clone(),
                model: request.model.clone(),
                reasoning_level: request.reasoning_level,
                version: request.version.clone(),
                execution_mode: request.execution_mode,
                session_id: request.session_id.clone(),
                skip_permissions: request.skip_permissions,
                env: request.env.clone(),
                env_remove: request.env_remove.clone(),
                auto_install_deps: request.auto_install_deps,
                collaboration_modes: request.collaboration_modes,
            };

            let plan = match prepare_launch_plan(config, |progress| {
                send(LaunchUpdate::Progress(progress))
            }) {
                Ok(plan) => plan,
                Err(e) => {
                    let _ = tx.send(LaunchUpdate::ProgressStepError {
                        kind: ProgressStepKind::CheckDependencies,
                        message: e.to_string(),
                    });
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };

            step(ProgressStepKind::CheckDependencies, StepStatus::Completed);

            send(LaunchUpdate::Ready(Box::new(plan)));
        });
    }

    fn handle_launch_plan(&mut self, plan: LaunchPlan) {
        // SPEC-f8dab6e2 T-110: Check if plugin setup is needed for Claude Code
        // FR-007: Only show for Claude Code (not Codex or other agents)
        // FR-008: Skip if marketplace is already registered
        if plan.config.agent == CodingAgent::ClaudeCode && !is_gwt_marketplace_registered() {
            self.pending_plugin_setup = true;
            self.pending_plugin_setup_launch = Some(plan);
            self.confirm = ConfirmState::plugin_setup();
            self.screen_stack.push(self.screen.clone());
            self.screen = Screen::Confirm;
            return;
        }

        self.handle_launch_plan_internal(plan);
    }

    /// Internal implementation of launch plan handling (called after plugin setup confirmation)
    fn handle_launch_plan_internal(&mut self, plan: LaunchPlan) {
        // Note: refresh_data() removed for startup optimization - TUI exits after
        // agent launch in single mode, and tmux mode updates status message directly.
        // The branch list refresh is unnecessary here. (FR-008b still satisfied)

        if let InstallPlan::Skip { message } = &plan.install_plan {
            self.status_message = Some(message.clone());
            self.status_message_time = Some(Instant::now());
        }

        let keep_launch_status = matches!(plan.install_plan, InstallPlan::Install { .. });

        if self.tmux_mode.is_multi() && self.gwt_pane_id.is_some() {
            if let Some(quick_start) = self.pending_quick_start_docker.take() {
                if self.try_apply_quick_start_docker(&plan, quick_start, keep_launch_status) {
                    return;
                }
            }
            let decision = match self.prepare_docker_service_selection(&plan) {
                Ok(decision) => decision,
                Err(e) => {
                    self.launch_status = None;
                    self.status_message = Some(format!("Failed to inspect docker services: {}", e));
                    self.status_message_time = Some(Instant::now());
                    return;
                }
            };

            match decision {
                ServiceSelectionDecision::AwaitSelection => {}
                ServiceSelectionDecision::Proceed {
                    service,
                    force_host,
                } => {
                    self.maybe_request_recreate_selection(
                        &plan,
                        service.as_deref(),
                        force_host,
                        keep_launch_status,
                    );
                }
            }
        } else {
            self.pending_agent_launch = Some(plan);
            self.should_quit = true;
        }
    }

    fn docker_force_host_enabled(&self) -> bool {
        self.settings
            .settings
            .as_ref()
            .map(|settings| settings.docker.force_host)
            .unwrap_or(false)
    }

    fn try_apply_quick_start_docker(
        &mut self,
        plan: &LaunchPlan,
        quick_start: QuickStartDockerSettings,
        keep_launch_status: bool,
    ) -> bool {
        if self.docker_force_host_enabled() {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Docker force_host enabled; launching on host"
            );
            self.launch_plan_in_tmux(
                plan,
                None,
                true,
                keep_launch_status,
                None,
                false,
                false,
                false,
            );
            return true;
        }

        if quick_start.force_host.unwrap_or(false) {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Quick Start docker settings: force host"
            );
            self.launch_plan_in_tmux(
                plan,
                None,
                true,
                keep_launch_status,
                None,
                false,
                false,
                false,
            );
            return true;
        }

        let docker_file_type = match launcher::detect_docker_environment(&plan.config.worktree_path)
        {
            Some(dtype) => dtype,
            None => return false,
        };

        let service = if docker_file_type.is_compose() {
            let manager = DockerManager::new(
                &plan.config.worktree_path,
                &plan.config.branch_name,
                docker_file_type.clone(),
            );
            let services = match manager.list_services() {
                Ok(services) if !services.is_empty() => services,
                _ => {
                    info!(
                        category = "docker",
                        branch = %plan.config.branch_name,
                        "Quick Start docker settings: service list unavailable"
                    );
                    return false;
                }
            };

            if services.len() == 1 {
                Some(services[0].clone())
            } else if let Some(selected) = quick_start.service.as_ref() {
                if services.contains(selected) {
                    Some(selected.clone())
                } else {
                    info!(
                        category = "docker",
                        branch = %plan.config.branch_name,
                        requested = %selected,
                        "Quick Start docker settings: service not found; fallback to wizard"
                    );
                    return false;
                }
            } else {
                info!(
                    category = "docker",
                    branch = %plan.config.branch_name,
                    "Quick Start docker settings missing service; fallback to wizard"
                );
                return false;
            }
        } else {
            None
        };

        let (Some(force_recreate), Some(keep)) = (quick_start.recreate, quick_start.keep) else {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Quick Start docker settings incomplete; fallback to wizard"
            );
            return false;
        };

        let needs_rebuild = {
            let manager = DockerManager::new(
                &plan.config.worktree_path,
                &plan.config.branch_name,
                docker_file_type,
            );
            manager.needs_rebuild()
        };
        let force_recreate = Self::quick_start_recreate_allowed(needs_rebuild, force_recreate);
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            needs_rebuild = needs_rebuild,
            force_recreate = force_recreate,
            "Quick Start recreate decision"
        );

        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            service = %service.clone().unwrap_or_default(),
            force_recreate = force_recreate,
            keep = keep,
            "Applying Quick Start docker settings"
        );
        self.maybe_request_build_selection(
            plan,
            service.as_deref(),
            false,
            keep_launch_status,
            force_recreate,
            Some(keep),
        );
        true
    }

    fn prepare_docker_service_selection(
        &mut self,
        plan: &LaunchPlan,
    ) -> Result<ServiceSelectionDecision, String> {
        if self.docker_force_host_enabled() {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Docker force_host enabled; skipping docker service selection"
            );
            return Ok(ServiceSelectionDecision::Proceed {
                service: None,
                force_host: true,
            });
        }

        let docker_file_type = match launcher::detect_docker_environment(&plan.config.worktree_path)
        {
            Some(dtype) => dtype,
            None => {
                return Ok(ServiceSelectionDecision::Proceed {
                    service: None,
                    force_host: false,
                })
            }
        };

        if !docker_file_type.is_compose() {
            self.pending_service_select = Some(PendingServiceSelect {
                plan: plan.clone(),
                services: Vec::new(),
            });
            let mut state = ServiceSelectState::with_dockerfile();
            // Preserve existing behavior (Docker default) while allowing HostOS selection.
            if state.items.len() > 1 {
                state.selected = 1;
            }
            let container_name = DockerManager::generate_container_name(&plan.config.branch_name);
            state.set_container_info(&container_name, &plan.config.branch_name);
            self.service_select = state;
            self.launch_status = None;
            self.last_mouse_click = None;
            self.screen_stack.push(self.screen.clone());
            self.screen = Screen::ServiceSelect;
            return Ok(ServiceSelectionDecision::AwaitSelection);
        }

        let manager = DockerManager::new(
            &plan.config.worktree_path,
            &plan.config.branch_name,
            docker_file_type,
        );

        let services = match manager.list_services() {
            Ok(services) if !services.is_empty() => services,
            Ok(_) | Err(_) => {
                self.pending_docker_host_launch = Some(plan.clone());
                self.confirm = ConfirmState {
                    title: "Docker Services Unavailable".to_string(),
                    message: "Could not read docker compose services. Launch on host?".to_string(),
                    details: vec![
                        "Service selection is required when multiple services exist.".to_string(),
                        "If you continue, the agent will run on the host.".to_string(),
                    ],
                    confirm_label: "Launch".to_string(),
                    cancel_label: "Cancel".to_string(),
                    selected_confirm: false,
                    is_dangerous: false,
                    ..Default::default()
                };
                self.screen_stack.push(self.screen.clone());
                self.screen = Screen::Confirm;
                return Ok(ServiceSelectionDecision::AwaitSelection);
            }
        };

        if services.len() == 1 {
            self.pending_service_select = Some(PendingServiceSelect {
                plan: plan.clone(),
                services: services.clone(),
            });
            let mut state = ServiceSelectState::with_services(services);
            // Preserve existing behavior (Docker default) while allowing HostOS selection.
            if state.items.len() > 1 {
                state.selected = 1;
            }
            let container_name = DockerManager::generate_container_name(&plan.config.branch_name);
            state.set_container_info(&container_name, &plan.config.branch_name);
            self.service_select = state;
            self.launch_status = None;
            self.last_mouse_click = None;
            self.screen_stack.push(self.screen.clone());
            self.screen = Screen::ServiceSelect;
            return Ok(ServiceSelectionDecision::AwaitSelection);
        }

        self.pending_service_select = Some(PendingServiceSelect {
            plan: plan.clone(),
            services: services.clone(),
        });
        self.service_select = ServiceSelectState::with_services(services);
        let container_name = DockerManager::generate_container_name(&plan.config.branch_name);
        self.service_select
            .set_container_info(&container_name, &plan.config.branch_name);
        self.launch_status = None;
        self.last_mouse_click = None;
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::ServiceSelect;

        Ok(ServiceSelectionDecision::AwaitSelection)
    }

    fn maybe_request_recreate_selection(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        keep_launch_status: bool,
    ) {
        if force_host {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                false,
                false,
                false,
            );
            return;
        }

        let docker_file_type = match launcher::detect_docker_environment(&plan.config.worktree_path)
        {
            Some(dtype) => dtype,
            None => {
                self.launch_plan_in_tmux(
                    plan,
                    service,
                    force_host,
                    keep_launch_status,
                    None,
                    false,
                    false,
                    false,
                );
                return;
            }
        };
        let manager = DockerManager::new(
            &plan.config.worktree_path,
            &plan.config.branch_name,
            docker_file_type,
        );
        let status = manager.get_status();
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            status = ?status,
            "Detected docker container status for recreate prompt"
        );
        if !Self::should_prompt_recreate(&status) {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                status = ?status,
                "Skipping docker recreate prompt (container not eligible)"
            );
            self.maybe_request_build_selection(
                plan,
                service,
                force_host,
                keep_launch_status,
                false,
                None,
            );
            return;
        }
        let needs_rebuild = manager.needs_rebuild();
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            needs_rebuild = needs_rebuild,
            "Checked docker rebuild status for recreate default"
        );

        self.pending_recreate_select = Some(PendingRecreateSelect {
            plan: plan.clone(),
            service: service.map(|s| s.to_string()),
            force_host,
        });
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            default_recreate = Self::default_recreate_selected(needs_rebuild),
            "Showing docker recreate prompt"
        );
        self.confirm = ConfirmState {
            title: "Docker Recreate".to_string(),
            message: "Recreate containers before launch?".to_string(),
            details: vec![
                "Reuse keeps existing containers.".to_string(),
                "Recreate will rerun entrypoint.".to_string(),
                "Press 'h' to launch on host.".to_string(),
            ],
            confirm_label: "Recreate".to_string(),
            cancel_label: "Reuse".to_string(),
            selected_confirm: Self::default_recreate_selected(needs_rebuild),
            is_dangerous: false,
            ..Default::default()
        };
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::Confirm;
    }

    fn should_prompt_recreate(status: &ContainerStatus) -> bool {
        matches!(status, ContainerStatus::Stopped)
    }

    fn default_recreate_selected(needs_rebuild: bool) -> bool {
        needs_rebuild
    }

    fn quick_start_recreate_allowed(needs_rebuild: bool, requested: bool) -> bool {
        needs_rebuild && requested
    }

    fn maybe_request_build_selection(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        keep_launch_status: bool,
        force_recreate: bool,
        quick_start_keep: Option<bool>,
    ) {
        if force_host || launcher::detect_docker_environment(&plan.config.worktree_path).is_none() {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                false,
                force_recreate,
                false,
            );
            return;
        }

        let docker_file_type = match launcher::detect_docker_environment(&plan.config.worktree_path)
        {
            Some(dtype) => dtype,
            None => {
                self.launch_plan_in_tmux(
                    plan,
                    service,
                    force_host,
                    keep_launch_status,
                    None,
                    false,
                    force_recreate,
                    false,
                );
                return;
            }
        };

        let manager = DockerManager::new(
            &plan.config.worktree_path,
            &plan.config.branch_name,
            docker_file_type,
        );
        let needs_rebuild = manager.needs_rebuild();
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            needs_rebuild = needs_rebuild,
            "Checked docker build prompt requirement"
        );
        if !needs_rebuild {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Skipping docker build prompt (no changes detected)"
            );
            self.maybe_request_cleanup_selection(
                plan,
                service,
                force_host,
                keep_launch_status,
                false,
                force_recreate,
                quick_start_keep,
            );
            return;
        }

        self.pending_build_select = Some(PendingBuildSelect {
            plan: plan.clone(),
            service: service.map(|s| s.to_string()),
            force_host,
            force_recreate,
            quick_start_keep,
        });
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            "Showing docker build prompt"
        );
        self.confirm = ConfirmState {
            title: "Docker Build".to_string(),
            message: "Build Docker image before launch?".to_string(),
            details: vec![
                "No Build will skip image rebuild.".to_string(),
                "Build will run docker compose build.".to_string(),
                "Press 'h' to launch on host.".to_string(),
            ],
            confirm_label: "Build".to_string(),
            cancel_label: "No Build".to_string(),
            selected_confirm: false,
            is_dangerous: false,
            ..Default::default()
        };
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::Confirm;
    }

    #[allow(clippy::too_many_arguments)]
    fn maybe_request_cleanup_selection(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        keep_launch_status: bool,
        build: bool,
        force_recreate: bool,
        quick_start_keep: Option<bool>,
    ) {
        if let Some(keep) = quick_start_keep {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                keep = keep,
                "Quick Start docker keep setting applied"
            );
            self.maybe_request_port_selection(
                plan,
                service,
                force_host,
                keep_launch_status,
                build,
                force_recreate,
                !keep,
            );
            return;
        }
        if force_host || launcher::detect_docker_environment(&plan.config.worktree_path).is_none() {
            info!(
                category = "docker",
                branch = %plan.config.branch_name,
                "Skipping docker cleanup prompt (host launch)"
            );
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                build,
                force_recreate,
                false,
            );
            return;
        }

        self.pending_cleanup_select = Some(PendingCleanupSelect {
            plan: plan.clone(),
            service: service.map(|s| s.to_string()),
            force_host,
            force_recreate,
            build,
        });
        info!(
            category = "docker",
            branch = %plan.config.branch_name,
            "Showing docker cleanup prompt"
        );
        self.confirm = ConfirmState {
            title: "Docker Cleanup".to_string(),
            message: "Stop containers when agent exits?".to_string(),
            details: vec![
                "Keep will keep containers running.".to_string(),
                "Stop will run docker compose down.".to_string(),
                "Press 'h' to launch on host.".to_string(),
            ],
            confirm_label: "Stop".to_string(),
            cancel_label: "Keep".to_string(),
            selected_confirm: false,
            is_dangerous: false,
            ..Default::default()
        };
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::Confirm;
    }

    #[allow(clippy::too_many_arguments)]
    fn maybe_request_port_selection(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        keep_launch_status: bool,
        build: bool,
        force_recreate: bool,
        stop_on_exit: bool,
    ) -> bool {
        if force_host || launcher::detect_docker_environment(&plan.config.worktree_path).is_none() {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                build,
                force_recreate,
                stop_on_exit,
            );
            return false;
        }

        let docker_file_type = match launcher::detect_docker_environment(&plan.config.worktree_path)
        {
            Some(dtype) => dtype,
            None => {
                self.launch_plan_in_tmux(
                    plan,
                    service,
                    force_host,
                    keep_launch_status,
                    None,
                    build,
                    force_recreate,
                    stop_on_exit,
                );
                return false;
            }
        };

        if !docker_file_type.is_compose() {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                build,
                force_recreate,
                stop_on_exit,
            );
            return false;
        }

        let manager = DockerManager::new(
            &plan.config.worktree_path,
            &plan.config.branch_name,
            docker_file_type,
        );
        let defaults = manager
            .compose_port_env_defaults(service)
            .unwrap_or_default();
        if defaults.is_empty() {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                build,
                force_recreate,
                stop_on_exit,
            );
            return false;
        }

        let docker_ports = DockerManager::published_ports_in_use();
        let mut conflicts = Vec::new();

        for (env_name, default_port) in defaults {
            let current = std::env::var(&env_name)
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(default_port);
            let taken = docker_ports.contains(&current) || PortAllocator::is_port_in_use(current);
            if taken {
                conflicts.push((env_name, default_port, current));
            }
        }

        if conflicts.is_empty() {
            self.launch_plan_in_tmux(
                plan,
                service,
                force_host,
                keep_launch_status,
                None,
                build,
                force_recreate,
                stop_on_exit,
            );
            return false;
        }

        let container_name = DockerManager::generate_container_name(&plan.config.branch_name);
        self.port_select = PortSelectState::from_conflicts(conflicts, &docker_ports, |port| {
            docker_ports.contains(&port) || PortAllocator::is_port_in_use(port)
        });
        self.port_select
            .set_context(&container_name, &plan.config.branch_name, service);
        self.pending_port_select = Some(PendingPortSelect {
            plan: plan.clone(),
            service: service.map(|s| s.to_string()),
            force_host,
            build,
            force_recreate,
            stop_on_exit,
        });
        self.launch_status = None;
        self.last_mouse_click = None;
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::PortSelect;
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_plan_in_tmux(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        keep_launch_status: bool,
        docker_env_overrides: Option<&HashMap<String, String>>,
        build: bool,
        force_recreate: bool,
        stop_on_exit: bool,
    ) {
        match self.launch_plan_in_pane_with_service(
            plan,
            service,
            force_host,
            docker_env_overrides,
            build,
            force_recreate,
            stop_on_exit,
        ) {
            Ok(_) => {
                if !keep_launch_status {
                    self.launch_status = None;
                }
                // FR-088: Record agent usage to history
                let agent_id = plan.config.agent.id();
                let agent_label =
                    format!("{}@{}", plan.config.agent.label(), plan.selected_version);
                if let Err(e) = self.agent_history.record(
                    &self.repo_root,
                    &plan.config.branch_name,
                    agent_id,
                    &agent_label,
                ) {
                    warn!(category = "tui", "Failed to record agent history: {}", e);
                }
                if let Err(e) = self.agent_history.save() {
                    warn!(category = "tui", "Failed to save agent history: {}", e);
                }

                // Update branch list item directly to reflect agent usage immediately
                // (refresh_data() is not called after agent launch for optimization)
                let branch_name_for_lookup =
                    normalize_branch_name_for_history(&plan.config.branch_name);
                for item in &mut self.branch_list.branches {
                    let item_lookup = normalize_branch_name_for_history(&item.name);
                    if item_lookup == branch_name_for_lookup {
                        item.last_tool_usage = Some(agent_label.clone());
                        item.last_tool_id = Some(agent_id.to_string());
                        break;
                    }
                }
                let launch_message = if let Some(warning) = plan.session_warning.as_ref() {
                    format!(
                        "Agent launched in tmux pane for {}. {}",
                        plan.config.branch_name, warning
                    )
                } else {
                    format!(
                        "Agent launched in tmux pane for {}",
                        plan.config.branch_name
                    )
                };
                self.status_message = Some(launch_message);
                self.status_message_time = Some(Instant::now());
                self.wizard.visible = false;
                self.screen = Screen::BranchList;
            }
            Err(e) => {
                self.launch_status = None;
                gwt_core::logging::log_error_message(
                    "E4002",
                    "agent",
                    &format!("Failed to launch: {}", e),
                    None,
                );
                self.status_message = Some(format!("Failed to launch: {}", e));
                self.status_message_time = Some(Instant::now());
            }
        }
    }

    fn handle_service_select_confirm(&mut self) {
        let Some(pending) = self.pending_service_select.take() else {
            return;
        };
        let (service, force_host) = {
            let (service, force_host) = self.service_select.selected_target();
            (service.map(|s| s.to_string()), force_host)
        };
        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }
        self.last_mouse_click = None;
        let keep_launch_status = matches!(pending.plan.install_plan, InstallPlan::Install { .. });
        self.maybe_request_recreate_selection(
            &pending.plan,
            service.as_deref(),
            force_host,
            keep_launch_status,
        );
    }

    fn handle_service_select_skip(&mut self) {
        let Some(pending) = self.pending_service_select.take() else {
            return;
        };
        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }
        self.last_mouse_click = None;
        let keep_launch_status = matches!(pending.plan.install_plan, InstallPlan::Install { .. });
        self.launch_plan_in_tmux(
            &pending.plan,
            None,
            true,
            keep_launch_status,
            None,
            false,
            false,
            false,
        );
    }

    fn handle_port_select_confirm(&mut self) {
        let Some(pending) = self.pending_port_select.clone() else {
            return;
        };

        let docker_ports = DockerManager::published_ports_in_use();
        let is_taken =
            |port: u16| docker_ports.contains(&port) || PortAllocator::is_port_in_use(port);

        // Custom input confirmation
        if self.port_select.custom_input.is_some() {
            if let Err(message) = self.port_select.apply_custom_port(|port| is_taken(port)) {
                self.port_select.error = Some(message);
            }
            return;
        }

        if let Err(message) = self.port_select.validate_selected_ports(|port| {
            is_taken(port) || self.port_select.is_port_selected_elsewhere(port)
        }) {
            self.port_select.error = Some(message);
            return;
        }

        let overrides = self.port_select.build_env_overrides();
        self.pending_port_select = None;
        self.pending_cleanup_select = None;
        self.launch_status = None;
        self.last_mouse_click = None;
        self.screen_stack.clear();

        let keep_launch_status = matches!(pending.plan.install_plan, InstallPlan::Install { .. });
        self.launch_plan_in_tmux(
            &pending.plan,
            pending.service.as_deref(),
            pending.force_host,
            keep_launch_status,
            Some(&overrides),
            pending.build,
            pending.force_recreate,
            pending.stop_on_exit,
        );
    }

    /// Launch an agent in a tmux pane (multi mode)
    ///
    /// Layout strategy:
    /// - gwt is left column, agents are placed in right columns
    /// - Each agent column stacks up to 3 panes (vertical split)
    /// - When a column reaches 3 panes, a new column is added to the right
    ///
    /// Uses the same argument building logic as single mode (main.rs)
    fn launch_plan_in_pane_with_service(
        &mut self,
        plan: &LaunchPlan,
        service: Option<&str>,
        force_host: bool,
        docker_env_overrides: Option<&HashMap<String, String>>,
        build: bool,
        force_recreate: bool,
        stop_on_exit: bool,
    ) -> Result<String, String> {
        // FR-010: One Branch One Pane constraint
        // Check if an agent is already running on this branch
        if self
            .pane_list
            .panes
            .iter()
            .any(|p| p.branch_name == plan.config.branch_name)
        {
            return Err(format!(
                "Agent already running on branch '{}'",
                plan.config.branch_name
            ));
        }

        // FR-036/FR-037: Single Active Pane Constraint
        // Hide any currently active agent pane before launching new one
        self.hide_active_agent_pane();

        let working_dir = plan.config.worktree_path.to_string_lossy().to_string();
        let use_docker = !force_host
            && launcher::detect_docker_environment(&plan.config.worktree_path).is_some();
        info!(
            category = "tui",
            branch = %plan.config.branch_name,
            worktree = %plan.config.worktree_path.display(),
            use_docker = use_docker,
            service = service.unwrap_or(""),
            executable = %plan.executable,
            "Prepared launch plan"
        );

        // Build environment variables (same as single mode)
        let env_vars = plan.env.clone();

        let install_cmd = match &plan.install_plan {
            InstallPlan::Install { manager } => {
                let args = vec!["install".to_string()];
                Some(build_shell_command(manager, &args))
            }
            _ => None,
        };

        let executable = if use_docker {
            normalize_container_executable(&plan.executable)
        } else {
            plan.executable.clone()
        };
        let agent_cmd = build_shell_command(&executable, &plan.command_args);
        let full_cmd = if let Some(install_cmd) = install_cmd {
            format!("{} && {}", install_cmd, agent_cmd)
        } else {
            agent_cmd
        };

        // Build the full command string
        let command = build_tmux_command(&env_vars, &plan.config.env_remove, &full_cmd);
        let command = wrap_tmux_command_for_fast_exit(&command);

        debug!(
            category = "tui",
            gwt_pane_id = ?self.gwt_pane_id,
            working_dir = %working_dir,
            command = %command,
            agent_pane_count = self.agent_panes.len(),
            "Launching agent in tmux pane"
        );

        // FR-036: Single Active Pane Constraint
        // hide_active_agent_pane() was called above, so there are no visible agent panes.
        // Always split to the right of gwt pane.
        let target = self
            .gwt_pane_id
            .as_ref()
            .ok_or_else(|| "No gwt pane ID available".to_string())?;
        let pane_id = if use_docker {
            let docker_args = vec!["-lc".to_string(), command.clone()];
            let (pane_id, _docker_result) = launcher::launch_in_pane_with_docker(
                target,
                &plan.config.worktree_path,
                &plan.config.branch_name,
                "sh",
                &docker_args,
                service,
                docker_env_overrides,
                build,
                force_recreate,
                stop_on_exit,
            )
            .map_err(|e| e.to_string())?;
            pane_id
        } else {
            launcher::launch_in_pane(target, &working_dir, &command).map_err(|e| e.to_string())?
        };

        // Focus the new pane (FR-022)
        if let Err(e) = gwt_core::tmux::pane::select_pane(&pane_id) {
            debug!(
                category = "tui",
                pane_id = %pane_id,
                error = %e,
                "Failed to focus new pane"
            );
        }

        // Track the new pane
        self.agent_panes.push(pane_id.clone());

        // Add to pane list for display
        let agent_pane = AgentPane::new(
            pane_id.clone(),
            plan.config.branch_name.clone(),
            plan.config.agent.label().to_string(),
            SystemTime::now(),
            0, // PID is not tracked by simple launcher
        );
        let mut panes = self.pane_list.panes.clone();
        panes.push(agent_pane);
        self.pane_list.update_panes(panes);

        let docker_env = launcher::detect_docker_environment(&plan.config.worktree_path);
        let (docker_service, docker_force_host, docker_recreate, docker_keep) =
            if docker_env.is_some() {
                if force_host {
                    (None, Some(true), None, None)
                } else {
                    (
                        service.map(|value| value.to_string()),
                        Some(false),
                        Some(force_recreate),
                        Some(!stop_on_exit),
                    )
                }
            } else {
                (None, None, None, None)
            };

        // FR-071: Save session entry for tmux mode
        let session_entry = ToolSessionEntry {
            branch: plan.config.branch_name.clone(),
            worktree_path: Some(working_dir),
            tool_id: plan.config.agent.id().to_string(),
            tool_label: plan.config.agent.label().to_string(),
            session_id: plan.config.session_id.clone(),
            mode: Some(plan.config.execution_mode.label().to_string()),
            model: plan.config.model.clone(),
            reasoning_level: plan.config.reasoning_level.map(|r| r.label().to_string()),
            skip_permissions: Some(plan.config.skip_permissions),
            tool_version: Some(plan.selected_version.clone()),
            collaboration_modes: Some(plan.config.collaboration_modes),
            docker_service,
            docker_force_host,
            docker_recreate,
            docker_build: None,
            docker_keep,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        if let Err(e) = save_session_entry(&plan.config.worktree_path, session_entry) {
            debug!(
                category = "tui",
                error = %e,
                "Failed to save session entry"
            );
        }

        // Update branch list with new agent info
        self.branch_list
            .update_running_agents(&self.pane_list.panes);

        // Reflow layout (columns/rows)
        self.reflow_agent_layout(Some(&pane_id));

        Ok(pane_id)
    }

    /// FR-036: Single Active Pane Constraint - at most 1 visible agent pane
    fn desired_agent_column_count(count: usize) -> usize {
        // Under single-active constraint, count is always 0 or 1
        if count > 0 {
            1
        } else {
            0
        }
    }

    fn visible_agent_pane_ids(&self) -> Vec<String> {
        self.pane_list
            .panes
            .iter()
            .filter(|p| !p.is_background)
            .map(|p| p.pane_id.clone())
            .collect()
    }

    fn agent_layout_snapshot(&self) -> Option<(PaneGeometry, Vec<PaneColumn>)> {
        let gwt_pane_id = self.gwt_pane_id.as_deref()?;
        let geometries = list_pane_geometries(gwt_pane_id).ok()?;
        let mut geometry_map: HashMap<String, PaneGeometry> = HashMap::new();
        for geometry in geometries {
            geometry_map.insert(geometry.pane_id.clone(), geometry);
        }

        let gwt_geometry = geometry_map.get(gwt_pane_id)?.clone();
        let visible_ids: std::collections::HashSet<String> =
            self.visible_agent_pane_ids().into_iter().collect();
        if visible_ids.is_empty() {
            return Some((gwt_geometry, Vec::new()));
        }

        let agent_geometries: Vec<PaneGeometry> = visible_ids
            .iter()
            .filter_map(|id| geometry_map.get(id))
            .cloned()
            .collect();
        let columns = group_panes_by_left(&agent_geometries);
        Some((gwt_geometry, columns))
    }

    fn rebuild_agent_columns(
        &mut self,
        pane_ids: &[String],
        gwt_pane_id: &str,
    ) -> Result<HashMap<String, String>, String> {
        if pane_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut columns: Vec<Vec<String>> = Vec::new();
        for pane_id in pane_ids {
            if columns.last().map(|c| c.len() == 3).unwrap_or(true) {
                columns.push(Vec::new());
            }
            columns.last_mut().unwrap().push(pane_id.clone());
        }

        for pane_id in pane_ids {
            break_pane(pane_id).map_err(|e| e.to_string())?;
        }

        let mut id_map: HashMap<String, String> = HashMap::new();
        let mut column_roots: Vec<String> = Vec::new();
        let mut rightmost_target = gwt_pane_id.to_string();

        for column in &columns {
            let source = &column[0];
            let joined = join_pane_to_target(source, &rightmost_target, SplitDirection::Horizontal)
                .map_err(|e| e.to_string())?;
            id_map.insert(source.clone(), joined.clone());
            rightmost_target = joined.clone();
            column_roots.push(joined);
        }

        for (column, root) in columns.iter().zip(column_roots.iter()) {
            let mut row_target = root.clone();
            for pane_id in column.iter().skip(1) {
                let joined = join_pane_to_target(pane_id, &row_target, SplitDirection::Vertical)
                    .map_err(|e| e.to_string())?;
                id_map.insert(pane_id.clone(), joined.clone());
                row_target = joined;
            }
        }

        if !id_map.is_empty() {
            for pane in &mut self.pane_list.panes {
                if let Some(new_id) = id_map.get(&pane.pane_id) {
                    pane.pane_id = new_id.clone();
                }
            }
            for pane_id in &mut self.agent_panes {
                if let Some(new_id) = id_map.get(pane_id) {
                    *pane_id = new_id.clone();
                }
            }
            self.branch_list
                .update_running_agents(&self.pane_list.panes);
        }

        Ok(id_map)
    }

    fn reflow_agent_layout(&mut self, focus_pane: Option<&str>) {
        if !self.tmux_mode.is_multi() {
            return;
        }
        let Some(gwt_pane_id) = self.gwt_pane_id.clone() else {
            return;
        };

        let visible_panes = self.visible_agent_pane_ids();
        if visible_panes.is_empty() {
            return;
        }

        let desired_columns = Self::desired_agent_column_count(visible_panes.len());
        let mut focus_target = focus_pane.map(|id| id.to_string());

        let Some((mut gwt_geometry, mut columns)) = self.agent_layout_snapshot() else {
            return;
        };

        if columns.len() != desired_columns {
            match self.rebuild_agent_columns(&visible_panes, &gwt_pane_id) {
                Ok(id_map) => {
                    if let Some(target) = focus_target.as_ref() {
                        if let Some(new_id) = id_map.get(target) {
                            focus_target = Some(new_id.clone());
                        }
                    }
                }
                Err(err) => {
                    debug!(
                        category = "tui",
                        error = %err,
                        "Failed to rebuild agent layout"
                    );
                    return;
                }
            }

            if let Some((new_gwt_geometry, new_columns)) = self.agent_layout_snapshot() {
                gwt_geometry = new_gwt_geometry;
                columns = new_columns;
            }
        }

        if columns.is_empty() {
            return;
        }

        let total_width: u16 = gwt_geometry.width + columns.iter().map(|c| c.width).sum::<u16>();
        let widths = compute_equal_splits(total_width, columns.len() + 1);
        if let Some(width) = widths.first() {
            if let Err(err) = resize_pane_width(&gwt_geometry.pane_id, *width) {
                debug!(
                    category = "tui",
                    error = %err,
                    "Failed to resize gwt pane width"
                );
            }
        }

        for (column, width) in columns.iter().zip(widths.iter().skip(1)) {
            if let Some(pane_id) = column.pane_ids.first() {
                if let Err(err) = resize_pane_width(pane_id, *width) {
                    debug!(
                        category = "tui",
                        pane_id = %pane_id,
                        error = %err,
                        "Failed to resize agent column width"
                    );
                }
            }
        }

        for column in &columns {
            let heights = compute_equal_splits(column.total_height, column.pane_ids.len());
            for (pane_id, height) in column.pane_ids.iter().zip(heights.into_iter()) {
                if let Err(err) = resize_pane_height(pane_id, height) {
                    debug!(
                        category = "tui",
                        pane_id = %pane_id,
                        error = %err,
                        "Failed to resize agent row height"
                    );
                }
            }
        }

        if let Some(pane_id) = focus_target {
            let _ = gwt_core::tmux::pane::select_pane(&pane_id);
        }
    }

    /// Execute branch cleanup (FR-010)
    fn execute_cleanup(&mut self, branches: &[String]) {
        if branches.is_empty() {
            return;
        }
        if self.branch_list.cleanup_in_progress() || self.cleanup_rx.is_some() {
            return;
        }

        debug!(
            category = "tui",
            branch_count = branches.len(),
            "Starting branch cleanup"
        );

        let cleanup_items: Vec<CleanupPlanItem> = branches
            .iter()
            .map(|branch_name| {
                let branch_item = self
                    .branch_list
                    .branches
                    .iter()
                    .find(|b| &b.name == branch_name);
                let force_remove = branch_item.is_some_and(|item| {
                    item.worktree_status == WorktreeStatus::Inaccessible
                        || item.has_changes
                        || WorktreeManager::is_protected(&item.name)
                });
                CleanupPlanItem {
                    branch: branch_name.clone(),
                    force_remove,
                }
            })
            .collect();

        self.branch_list.start_cleanup_progress(cleanup_items.len());
        self.branch_list.set_cleanup_target_branches(branches);
        self.branch_list.set_cleanup_active_branch(None);

        // SPEC-a70a1ece: Use bare repo path for worktree operations in bare projects
        let repo_root = self
            .bare_repo_path
            .clone()
            .unwrap_or_else(|| self.repo_root.clone());
        let (tx, rx) = mpsc::channel();
        self.cleanup_rx = Some(rx);

        thread::spawn(move || {
            let mut deleted = 0;
            let mut errors = Vec::new();
            let manager = WorktreeManager::new(&repo_root).ok();

            for item in cleanup_items {
                let _ = tx.send(CleanupUpdate::BranchStarted {
                    branch: item.branch.clone(),
                });

                let result = if let Some(manager) = manager.as_ref() {
                    manager.cleanup_branch(&item.branch, item.force_remove, true)
                } else {
                    Branch::delete(&repo_root, &item.branch, true)
                };

                match result {
                    Ok(_) => {
                        debug!(
                            category = "tui",
                            branch = %item.branch,
                            "Branch cleanup succeeded"
                        );
                        deleted += 1;
                        let _ = tx.send(CleanupUpdate::BranchFinished {
                            branch: item.branch.clone(),
                            success: true,
                        });
                    }
                    Err(e) => {
                        error!(
                            category = "tui",
                            branch = %item.branch,
                            error = %e,
                            "Branch cleanup failed"
                        );
                        errors.push(format!("{}: {}", item.branch, e));
                        let _ = tx.send(CleanupUpdate::BranchFinished {
                            branch: item.branch.clone(),
                            success: false,
                        });
                    }
                }
            }

            let _ = tx.send(CleanupUpdate::Completed { deleted, errors });
        });
    }

    fn finish_cleanup(&mut self, deleted: usize, errors: Vec<String>) {
        self.branch_list.finish_cleanup_progress();

        if errors.is_empty() {
            info!(
                category = "tui",
                operation = "cleanup",
                deleted_count = deleted,
                "Branch cleanup completed successfully"
            );
            self.status_message = Some(format!("Deleted {} branch(es).", deleted));
        } else {
            info!(
                category = "tui",
                operation = "cleanup",
                deleted_count = deleted,
                error_count = errors.len(),
                "Branch cleanup completed with errors"
            );
            self.status_message = Some(format!(
                "Deleted {} branch(es), {} failed.",
                deleted,
                errors.len()
            ));
        }
        self.status_message_time = Some(Instant::now());

        // Refresh data to reflect changes (FR-008b)
        self.refresh_data();
    }

    /// View function (Elm Architecture)
    pub fn view(&mut self, frame: &mut Frame) {
        let base_screen = match self.screen {
            Screen::Confirm => self
                .screen_stack
                .last()
                .cloned()
                .unwrap_or(Screen::BranchList),
            Screen::PortSelect => {
                // If PortSelect is opened from Confirm, keep rendering the underlying content
                // behind Confirm so users still have context.
                if self
                    .screen_stack
                    .last()
                    .is_some_and(|s| matches!(s, Screen::Confirm))
                {
                    self.screen_stack
                        .iter()
                        .rev()
                        .nth(1)
                        .cloned()
                        .unwrap_or(Screen::BranchList)
                } else {
                    self.screen_stack
                        .last()
                        .cloned()
                        .unwrap_or(Screen::BranchList)
                }
            }
            _ => self.screen.clone(),
        };

        // Keep a consistent header across major screens.
        let needs_header = !matches!(base_screen, Screen::AISettingsWizard);
        let header_height = if needs_header { 6 } else { 0 };

        // Footer help sits above the status bar.
        let needs_footer = !matches!(base_screen, Screen::AISettingsWizard);
        let full_footer_lines = if needs_footer {
            let lines = self.footer_lines(frame.area().width);
            self.sync_footer_metrics(frame.area().width, lines.len());
            lines
        } else {
            Vec::new()
        };
        let footer_lines = if needs_footer {
            self.footer_visible_lines(&full_footer_lines, FOOTER_VISIBLE_HEIGHT)
        } else {
            Vec::new()
        };
        let footer_height = if needs_footer {
            FOOTER_VISIBLE_HEIGHT as u16
        } else {
            0
        };

        // Status bar is always the bottom-most line.
        let needs_status_bar = !matches!(base_screen, Screen::AISettingsWizard);
        let status_bar_height = if needs_status_bar { 1 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),     // Header
                Constraint::Min(0),                    // Content
                Constraint::Length(footer_height),     // Footer help
                Constraint::Length(status_bar_height), // Status bar
            ])
            .split(frame.area());

        // Header
        if needs_header {
            self.view_boxed_header(frame, chunks[0], &base_screen);
        }

        // Content
        match base_screen {
            Screen::BranchList => {
                // Use split layout (branch list takes full area, PaneList abolished)
                let split_areas = calculate_split_layout(chunks[1], &self.split_layout);

                // Render branch list (always has focus now)
                render_branch_list(
                    &mut self.branch_list,
                    frame,
                    split_areas.branch_list,
                    None,
                    true, // Branch list always has focus
                );
            }
            Screen::WorktreeCreate => {
                render_worktree_create(&self.worktree_create, frame, chunks[1])
            }
            Screen::AgentMode => {
                render_agent_mode(&self.agent_mode, frame, chunks[1], None);
            }
            Screen::Settings => render_settings(&self.settings, frame, chunks[1]),
            Screen::Logs => render_logs(&mut self.logs, frame, chunks[1]),
            Screen::Help => render_help(&self.help, frame, chunks[1]),
            Screen::Error => render_error_with_queue(&self.error_queue, frame, chunks[1]),
            Screen::Profiles => render_profiles(&mut self.profiles, frame, chunks[1]),
            Screen::Environment => render_environment(&mut self.environment, frame, chunks[1]),
            Screen::ServiceSelect => {
                render_service_select(&mut self.service_select, frame, chunks[1])
            }
            Screen::AISettingsWizard => render_ai_wizard(&mut self.ai_wizard, frame, chunks[1]),
            Screen::CloneWizard => render_clone_wizard(&self.clone_wizard, frame, chunks[1]),
            Screen::MigrationDialog => {
                render_migration_dialog(&mut self.migration_dialog, frame, chunks[1])
            }
            Screen::GitView => {
                render_git_view(&mut self.git_view, frame, chunks[1]);
            }
            Screen::Confirm => {}
            Screen::PortSelect => {}
        }

        if matches!(self.screen, Screen::Confirm)
            || (matches!(self.screen, Screen::PortSelect)
                && self
                    .screen_stack
                    .last()
                    .is_some_and(|s| matches!(s, Screen::Confirm)))
        {
            render_confirm(&mut self.confirm, frame, chunks[1]);
        }
        if matches!(self.screen, Screen::PortSelect) {
            render_port_select(&mut self.port_select, frame, chunks[1]);
        }

        // Footer help
        if needs_footer {
            self.view_footer(frame, chunks[2], &footer_lines);
        }

        // Bottom status bar
        if needs_status_bar {
            self.view_status_bar(frame, chunks[3], &base_screen);
        }

        // Wizard overlay (FR-044: popup on top of branch list)
        if self.wizard.visible {
            render_wizard(&mut self.wizard, frame, frame.area());
        }

        // Progress modal overlay (FR-041: highest z-index)
        if let Some(ref modal) = self.progress_modal {
            use super::widgets::ProgressModal;
            let widget = ProgressModal::new(modal);
            frame.render_widget(widget, frame.area());
        }
    }

    /// Boxed header shared across major screens
    fn view_boxed_header(&self, frame: &mut Frame, area: Rect, screen: &Screen) {
        let version = env!("CARGO_PKG_VERSION");
        let offline_indicator = if self.is_offline { " [OFFLINE]" } else { "" };
        let profile = self
            .branch_list
            .active_profile
            .as_deref()
            .unwrap_or("(none)");
        let working_dir = self
            .branch_list
            .working_directory
            .as_deref()
            .unwrap_or_else(|| self.repo_root.to_str().unwrap_or("."));

        let screen_title = match screen {
            Screen::BranchList => "Branch Screen",
            Screen::AgentMode => "Agent Screen",
            Screen::WorktreeCreate => "Worktree Create",
            Screen::Settings => "Settings",
            Screen::Logs => "Logs",
            Screen::Help => "Help",
            Screen::Confirm => "Confirm",
            Screen::Error => "Errors",
            Screen::Profiles => "Profiles",
            Screen::Environment => "Environment",
            Screen::ServiceSelect => "Service Select",
            Screen::PortSelect => "Port Select",
            Screen::AISettingsWizard => "AI Settings",
            Screen::CloneWizard => "Clone Repository",
            Screen::MigrationDialog => "Migration Required",
            Screen::GitView => "Git View",
        };

        let title = format!(" gwt - {} v{}{} ", screen_title, version, offline_indicator);
        let header_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );

        let inner = header_block.inner(area);
        frame.render_widget(header_block, area);

        // Inner content layout (4 lines - no remaining space needed)
        let inner_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Working Directory
                Constraint::Length(1), // Profile
                Constraint::Length(1), // Filter
                Constraint::Length(1), // Mode
            ])
            .split(inner);

        // Line 1: Working Directory with branch name (SPEC-a70a1ece FR-103) and repo type indicator
        let mut working_dir_spans = vec![
            Span::raw(" "),
            Span::styled("Working Directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(working_dir),
        ];
        // SPEC-a70a1ece: Show startup branch only for non-bare repos
        if self.repo_type != RepoType::Bare {
            if let Some(ref branch) = self.startup_branch {
                working_dir_spans.push(Span::raw(" "));
                working_dir_spans.push(Span::styled(
                    format!("[{}]", branch),
                    Style::default().fg(Color::Green),
                ));
            }
        }
        // SPEC-a70a1ece T206: Show [bare] indicator for bare repositories
        if self.repo_type == RepoType::Bare {
            working_dir_spans.push(Span::raw(" "));
            working_dir_spans.push(Span::styled("[bare]", Style::default().fg(Color::Yellow)));
        }
        // SPEC-a70a1ece T506: Show (repo.git) for worktrees in bare-based projects
        if let Some(ref name) = self.bare_name {
            working_dir_spans.push(Span::raw(" "));
            working_dir_spans.push(Span::styled(
                format!("({})", name),
                Style::default().fg(Color::DarkGray),
            ));
        }
        let working_dir_line = Line::from(working_dir_spans);
        frame.render_widget(Paragraph::new(working_dir_line), inner_chunks[0]);

        // Line 2: Profile
        let profile_line = Line::from(vec![
            Span::raw(" "),
            Span::styled("Profile(p): ", Style::default().fg(Color::DarkGray)),
            Span::raw(profile),
        ]);
        frame.render_widget(Paragraph::new(profile_line), inner_chunks[1]);

        let branch_context = matches!(screen, Screen::BranchList | Screen::AgentMode);
        if branch_context {
            // Line 3: Filter
            let filtered = self.branch_list.filtered_branches();
            let total = self.branch_list.branches.len();
            let mut filter_spans = vec![
                Span::raw(" "),
                Span::styled("Filter(f): ", Style::default().fg(Color::DarkGray)),
            ];
            if self.branch_list.filter_mode {
                if self.branch_list.filter.is_empty() {
                    filter_spans.push(Span::styled(
                        "Type to search...",
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    filter_spans.push(Span::raw(&self.branch_list.filter));
                }
                filter_spans.push(Span::styled("|", Style::default().fg(Color::White)));
            } else {
                filter_spans.push(Span::styled(
                    if self.branch_list.filter.is_empty() {
                        "(press f to filter)"
                    } else {
                        &self.branch_list.filter
                    },
                    Style::default().fg(Color::DarkGray),
                ));
            }
            if !self.branch_list.filter.is_empty() {
                filter_spans.push(Span::styled(
                    format!(" (Showing {} of {})", filtered.len(), total),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(filter_spans)), inner_chunks[2]);

            // Line 4: Mode
            let mode_spans = vec![
                Span::raw(" "),
                Span::styled("Mode(m):", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    self.branch_list.view_mode.label(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled("Sort(s):", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    self.branch_list.sort_mode.label(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            frame.render_widget(Paragraph::new(Line::from(mode_spans)), inner_chunks[3]);
        } else {
            let screen_line = Line::from(vec![
                Span::raw(" "),
                Span::styled("Screen: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    screen_title,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(screen_line), inner_chunks[2]);

            // Clear the last line to avoid stale content when switching screens.
            let blank = " ".repeat(inner_chunks[3].width as usize);
            frame.render_widget(Paragraph::new(blank), inner_chunks[3]);
        }
    }

    fn view_header(&self, frame: &mut Frame, area: Rect, screen: &Screen) {
        let version = env!("CARGO_PKG_VERSION");
        let offline_indicator = if self.is_offline { " [OFFLINE]" } else { "" };

        let profile = self
            .branch_list
            .active_profile
            .as_deref()
            .unwrap_or("default");

        // Keep header naming consistent with boxed header labels.
        let title = match screen {
            Screen::AgentMode => format!(
                " gwt - Agent Screen v{} | Profile(p): {} {}",
                version, profile, offline_indicator
            ),
            _ => format!(
                " gwt - Branch Screen v{} | Profile(p): {} {}",
                version, profile, offline_indicator
            ),
        };
        let header = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        frame.render_widget(header, area);
    }

    fn footer_lines(&self, width: u16) -> Vec<String> {
        let items = self.footer_items();
        Self::wrap_footer_items(&items, width as usize)
    }

    fn sync_footer_metrics(&mut self, width: u16, line_count: usize) {
        let width_changed = self.footer_last_width != width;
        let count_changed = self.footer_line_count != line_count;

        self.footer_last_width = width;
        self.footer_line_count = line_count;

        if width_changed || count_changed {
            self.reset_footer_scroll();
            return;
        }

        let max_offset = self.footer_line_count.saturating_sub(FOOTER_VISIBLE_HEIGHT);
        if self.footer_scroll_offset > max_offset {
            self.footer_scroll_offset = max_offset;
        }
    }

    fn footer_visible_lines(&self, full_lines: &[String], height: usize) -> Vec<String> {
        if full_lines.is_empty() || height == 0 {
            return Vec::new();
        }

        let max_offset = full_lines.len().saturating_sub(height);
        let offset = self.footer_scroll_offset.min(max_offset);
        let end = (offset + height).min(full_lines.len());
        full_lines[offset..end].to_vec()
    }

    fn footer_items(&self) -> Vec<String> {
        match self.screen {
            Screen::BranchList => {
                if self.branch_list.filter_mode {
                    vec![
                        Self::keybind_item("Esc", "Exit filter"),
                        Self::keybind_item("Enter", "Apply"),
                        Self::keybind_item("Backspace", "Delete"),
                        Self::keybind_item("Up/Down", "Move"),
                        Self::keybind_item("PgUp/PgDn", "Page"),
                        Self::keybind_item("Home/End", "Top/Bottom"),
                        "Type to search".to_string(),
                    ]
                } else {
                    let mut items = vec![
                        Self::keybind_item("Up/Down", "Move"),
                        Self::keybind_item("PgUp/PgDn", "Page"),
                        Self::keybind_item("Home/End", "Top/Bottom"),
                        Self::keybind_item("Enter", "Open/Focus"),
                        Self::keybind_item("Space", "Select"),
                        Self::keybind_item("r", "Refresh"),
                        Self::keybind_item("c", "Cleanup"),
                        Self::keybind_item("l", "Logs"),
                        Self::keybind_item("s", "Sort"),
                        Self::keybind_item("p", "Environment"),
                        Self::keybind_item("f,/", "Filter"),
                        Self::keybind_item("m", "Mode"),
                        Self::keybind_item("Tab", "Agent"),
                        Self::keybind_item("v", "GitView"),
                        Self::keybind_item("u", "Hooks"),
                        Self::keybind_item("?/h", "Help"),
                    ];
                    if self.selected_branch_has_agent() {
                        items.push(Self::keybind_item("d", "Terminate"));
                    }
                    if !self.branch_list.filter.is_empty() || self.has_active_agent_pane() {
                        items.push(Self::keybind_item("Esc", "Clear/Hide"));
                    }
                    items.push(Self::keybind_item("Ctrl+C", "Quit"));
                    items
                }
            }
            Screen::AgentMode => {
                let enter_label = if self.agent_mode.ai_ready {
                    "Send"
                } else {
                    "Configure AI"
                };
                vec![
                    Self::keybind_item("Enter", enter_label),
                    Self::keybind_item("Tab", "Settings"),
                    Self::keybind_item("Esc", "Back"),
                ]
            }
            Screen::WorktreeCreate => match self.worktree_create.step {
                WorktreeCreateStep::BranchName => vec![
                    Self::keybind_item("Enter", "Next"),
                    Self::keybind_item("Esc", "Back"),
                ],
                WorktreeCreateStep::BaseBranch => vec![
                    Self::keybind_item("Up/Down", "Select"),
                    Self::keybind_item("Enter", "Next"),
                    Self::keybind_item("Esc", "Back"),
                ],
                WorktreeCreateStep::Confirm => vec![
                    Self::keybind_item("Enter", "Create"),
                    Self::keybind_item("Esc", "Back"),
                ],
            },
            Screen::Settings => Self::split_footer_text(&self.settings.footer_keybinds()),
            Screen::Logs => vec![
                Self::keybind_item("Up/Down", "Navigate"),
                Self::keybind_item("PgUp/PgDn", "Page"),
                Self::keybind_item("Home/End", "Top/Bottom"),
                Self::keybind_item("Enter", "Detail"),
                Self::keybind_item("c", "Copy"),
                Self::keybind_item("f", "Filter"),
                Self::keybind_item("/", "Search"),
                Self::keybind_item("Esc", "Back"),
            ],
            Screen::Help => vec![
                Self::keybind_item("Up/Down", "Scroll"),
                Self::keybind_item("PgUp/PgDn", "Page"),
                Self::keybind_item("Esc", "Back"),
            ],
            Screen::Confirm => {
                let mut items = vec![
                    Self::keybind_item("Left/Right", "Select"),
                    Self::keybind_item("Enter", "Confirm"),
                ];
                if self.pending_build_select.is_some()
                    || self.pending_recreate_select.is_some()
                    || self.pending_cleanup_select.is_some()
                {
                    items.push(Self::keybind_item("h", "HostOS"));
                }
                items.push(Self::keybind_item("Esc", "Cancel"));
                items
            }
            Screen::Error => vec![
                Self::keybind_item("Enter", "Close"),
                Self::keybind_item("Esc", "Close"),
                Self::keybind_item("Up/Down", "Scroll"),
                Self::keybind_item("l", "Logs"),
                Self::keybind_item("c", "Copy"),
            ],
            Screen::Profiles => {
                if self.profiles.create_mode {
                    vec![
                        Self::keybind_item("Enter", "Save"),
                        Self::keybind_item("Esc", "Cancel"),
                    ]
                } else {
                    vec![
                        Self::keybind_item("Up/Down", "Select"),
                        Self::keybind_item("Space", "Activate"),
                        Self::keybind_item("Enter", "Edit"),
                        Self::keybind_item("n", "New"),
                        Self::keybind_item("d", "Delete"),
                        Self::keybind_item("Esc", "Back"),
                    ]
                }
            }
            Screen::Environment => {
                if self.environment.is_ai_only() {
                    if self.environment.edit_mode {
                        vec![
                            Self::keybind_item("Enter", "Save"),
                            Self::keybind_item("Tab", "Switch"),
                            Self::keybind_item("Esc", "Cancel"),
                        ]
                    } else {
                        vec![
                            Self::keybind_item("Up/Down", "Select"),
                            Self::keybind_item("PgUp/PgDn", "Page"),
                            Self::keybind_item("Home/End", "Top/Bottom"),
                            Self::keybind_item("Enter", "Edit"),
                            Self::keybind_item("Esc", "Back"),
                        ]
                    }
                } else if self.environment.edit_mode {
                    vec![
                        Self::keybind_item("Enter", "Save"),
                        Self::keybind_item("Tab", "Switch"),
                        Self::keybind_item("Esc", "Cancel"),
                    ]
                } else {
                    vec![
                        Self::keybind_item("Up/Down", "Select"),
                        Self::keybind_item("PgUp/PgDn", "Page"),
                        Self::keybind_item("Home/End", "Top/Bottom"),
                        Self::keybind_item("Enter", "Edit"),
                        Self::keybind_item("n", "New"),
                        Self::keybind_item("d", "Delete/Disable"),
                        Self::keybind_item("r", "Reset"),
                        Self::keybind_item("Esc", "Back"),
                    ]
                }
            }
            Screen::ServiceSelect => {
                vec!["[Up/Down] Select | [Enter] Launch | [s] Skip | [Esc] Cancel".to_string()]
            }
            Screen::PortSelect => vec![
                Self::keybind_item("Up/Down", "Select"),
                Self::keybind_item("Left/Right", "Change"),
                Self::keybind_item("c", "Custom"),
                Self::keybind_item("a", "Auto"),
                Self::keybind_item("Enter", "Continue"),
                Self::keybind_item("Esc", "Cancel"),
            ],
            Screen::AISettingsWizard => {
                if self.ai_wizard.show_delete_confirm {
                    vec![
                        Self::keybind_item("y", "Confirm Delete"),
                        Self::keybind_item("n", "Cancel"),
                    ]
                } else {
                    vec![self.ai_wizard.step_title().to_string()]
                }
            }
            Screen::CloneWizard => match self.clone_wizard.step {
                CloneWizardStep::UrlInput => vec![
                    Self::keybind_item("Enter", "Continue"),
                    Self::keybind_item("Esc", "Quit"),
                ],
                CloneWizardStep::TypeSelect => vec![
                    Self::keybind_item("Up/Down", "Select"),
                    Self::keybind_item("Enter", "Clone"),
                    Self::keybind_item("Backspace", "Back"),
                    Self::keybind_item("Esc", "Quit"),
                ],
                CloneWizardStep::Cloning => vec!["Cloning...".to_string()],
                CloneWizardStep::Complete => vec![Self::keybind_item("Enter", "Continue")],
                CloneWizardStep::Failed => vec![
                    Self::keybind_item("Backspace", "Try again"),
                    Self::keybind_item("Esc", "Quit"),
                ],
            },
            Screen::MigrationDialog => match self.migration_dialog.phase {
                MigrationDialogPhase::Confirmation => vec![
                    Self::keybind_item("Left/Right", "Select"),
                    Self::keybind_item("Enter", "Confirm"),
                ],
                MigrationDialogPhase::Validating | MigrationDialogPhase::InProgress => {
                    vec!["Migration in progress...".to_string()]
                }
                MigrationDialogPhase::Completed => vec![Self::keybind_item("Enter", "Continue")],
                MigrationDialogPhase::Failed => vec![Self::keybind_item("Enter", "Exit")],
                MigrationDialogPhase::Exited => Vec::new(),
            },
            // SPEC-1ea18899: GitView footer keybinds
            Screen::GitView => vec![
                Self::keybind_item("Up/Down", "Navigate"),
                Self::keybind_item("Space", "Expand"),
                Self::keybind_item("Enter", "Open PR"),
                Self::keybind_item("v/Esc", "Back"),
            ],
        }
    }

    #[cfg(test)]
    fn get_footer_keybinds(&self) -> String {
        self.footer_items().join(" ")
    }

    fn wrap_footer_items(items: &[String], width: usize) -> Vec<String> {
        if items.is_empty() {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        let mut current = String::new();

        for item in items {
            let segment = item.trim();
            if segment.is_empty() {
                continue;
            }
            let separator = if current.is_empty() { "" } else { " | " };
            let next_len = current.len() + separator.len() + segment.len();
            if !current.is_empty() && width > 0 && next_len > width {
                lines.push(current);
                current = segment.to_string();
            } else {
                if !current.is_empty() {
                    current.push_str(" | ");
                }
                current.push_str(segment);
            }
        }

        if current.is_empty() && lines.is_empty() {
            lines.push(String::new());
        } else if !current.is_empty() {
            lines.push(current);
        }

        lines
    }

    fn keybind_item(keys: &str, label: &str) -> String {
        format!("[{}] {}", keys, label)
    }

    fn split_footer_text(text: &str) -> Vec<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        trimmed
            .split(" | ")
            .map(|segment| segment.trim().to_string())
            .filter(|segment| !segment.is_empty())
            .collect()
    }

    fn reset_footer_scroll(&mut self) {
        self.footer_scroll_offset = 0;
        self.footer_scroll_dir = 1;
        self.footer_scroll_tick = 0;
        self.footer_scroll_pause = 0;
    }

    fn update_footer_scroll(&mut self) {
        if self.footer_line_count <= FOOTER_VISIBLE_HEIGHT {
            self.reset_footer_scroll();
            return;
        }

        if self.footer_scroll_pause > 0 {
            self.footer_scroll_pause -= 1;
            return;
        }

        self.footer_scroll_tick = self.footer_scroll_tick.saturating_add(1);
        if self.footer_scroll_tick < FOOTER_SCROLL_TICKS_PER_LINE {
            return;
        }
        self.footer_scroll_tick = 0;

        let max_offset = self.footer_line_count.saturating_sub(FOOTER_VISIBLE_HEIGHT);
        if self.footer_scroll_dir >= 0 {
            if self.footer_scroll_offset < max_offset {
                self.footer_scroll_offset += 1;
                if self.footer_scroll_offset >= max_offset {
                    self.footer_scroll_dir = -1;
                    self.footer_scroll_pause = FOOTER_SCROLL_PAUSE_TICKS;
                }
            } else {
                self.footer_scroll_dir = -1;
                self.footer_scroll_pause = FOOTER_SCROLL_PAUSE_TICKS;
            }
        } else if self.footer_scroll_offset > 0 {
            self.footer_scroll_offset -= 1;
            if self.footer_scroll_offset == 0 {
                self.footer_scroll_dir = 1;
                self.footer_scroll_pause = FOOTER_SCROLL_PAUSE_TICKS;
            }
        } else {
            self.footer_scroll_dir = 1;
            self.footer_scroll_pause = FOOTER_SCROLL_PAUSE_TICKS;
        }
    }

    fn view_footer(&self, frame: &mut Frame, area: Rect, lines: &[String]) {
        let style = if self.ctrl_c_count > 0 {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        let text = if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n")
        };
        let footer = Paragraph::new(text).style(style);

        frame.render_widget(footer, area);
    }

    fn view_status_bar(&self, frame: &mut Frame, area: Rect, _screen: &Screen) {
        let line = crate::tui::screens::branch_list::build_status_bar_line(
            &self.branch_list,
            self.active_status_message(),
        );
        frame.render_widget(Paragraph::new(line), area);
    }

    fn text_input_active(&self) -> bool {
        matches!(self.screen, Screen::Profiles) && self.profiles.create_mode
            || matches!(self.screen, Screen::Environment) && self.environment.edit_mode
            || matches!(self.screen, Screen::AISettingsWizard) && self.ai_wizard.is_text_input()
    }

    fn handle_text_input_key(&mut self, key: KeyEvent, enter_is_press: bool) -> Option<Message> {
        let is_env_new = matches!(self.screen, Screen::Environment)
            && self.environment.edit_mode
            && self.environment.is_new;
        let is_env_key_field = matches!(self.screen, Screen::Environment)
            && self.environment.edit_field == EditField::Key;

        match key.code {
            KeyCode::Esc => Some(Message::NavigateBack),
            KeyCode::Enter if enter_is_press => {
                if is_env_new && is_env_key_field {
                    self.environment.switch_field();
                    None
                } else {
                    Some(Message::Enter)
                }
            }
            KeyCode::Backspace if enter_is_press => Some(Message::Backspace),
            KeyCode::Left if enter_is_press => Some(Message::CursorLeft),
            KeyCode::Right if enter_is_press => Some(Message::CursorRight),
            KeyCode::Tab => {
                if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.switch_field();
                // SPEC-71f2742d US3: Settings form field navigation
                } else if matches!(self.screen, Screen::Settings) && self.settings.is_form_mode() {
                    if self.settings.is_profile_form_mode() {
                        self.settings.profile_form.next_field();
                    } else {
                        self.settings.agent_form.next_field();
                    }
                } else if matches!(self.screen, Screen::Settings)
                    && self.settings.is_env_edit_mode()
                {
                    // Tab in EnvEdit mode: switch between key and value
                    self.settings.env_state.switch_field();
                }
                None
            }
            KeyCode::BackTab => {
                if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.switch_field();
                // SPEC-71f2742d US3: Settings form field navigation (reverse)
                } else if matches!(self.screen, Screen::Settings) && self.settings.is_form_mode() {
                    if self.settings.is_profile_form_mode() {
                        self.settings.profile_form.prev_field();
                    } else {
                        self.settings.agent_form.prev_field();
                    }
                } else if matches!(self.screen, Screen::Settings)
                    && self.settings.is_env_edit_mode()
                {
                    // BackTab in EnvEdit mode: switch between key and value
                    self.settings.env_state.switch_field();
                }
                None
            }
            KeyCode::Char(c) if enter_is_press => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    None
                } else {
                    Some(Message::Char(c))
                }
            }
            _ => None,
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Message> {
        let is_key_press = key.kind == KeyEventKind::Press;

        // FR-054: Progress modal has highest priority when visible
        if let Some(ref mut modal) = self.progress_modal {
            if is_key_press {
                if modal.has_failed() && modal.waiting_for_key {
                    // FR-052: Any key dismisses error modal
                    self.progress_modal = None;
                    self.launch_in_progress = false;
                    return None;
                } else if key.code == KeyCode::Esc && !modal.completed {
                    // FR-054: ESC cancels preparation
                    modal.cancellation_requested = true;
                    self.progress_modal = None;
                    self.launch_in_progress = false;
                    self.launch_rx = None;
                    // FR-055: Cleanup is handled by the background thread
                    return None;
                }
            }
            // Block all other input while modal is visible (FR-043)
            return None;
        }

        // Wizard has priority when visible
        if self.wizard.visible {
            return match key.code {
                KeyCode::Esc => Some(Message::WizardBack),
                KeyCode::Enter if is_key_press => Some(Message::WizardConfirm),
                KeyCode::Up if is_key_press => Some(Message::WizardPrev),
                KeyCode::Down if is_key_press => Some(Message::WizardNext),
                KeyCode::Backspace => {
                    self.wizard.delete_char();
                    None
                }
                KeyCode::Left => {
                    self.wizard.cursor_left();
                    None
                }
                KeyCode::Right => {
                    self.wizard.cursor_right();
                    None
                }
                KeyCode::Char(c)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.wizard.insert_char(c);
                    None
                }
                _ => None,
            };
        }

        if matches!(self.screen, Screen::AISettingsWizard)
            && !self.ai_wizard.is_text_input()
            && is_key_press
        {
            use super::screens::ai_wizard::AIWizardStep;

            match key.code {
                KeyCode::Char('t') | KeyCode::Char('T')
                    if matches!(self.ai_wizard.step, AIWizardStep::ModelSelect) =>
                {
                    self.ai_wizard.toggle_summary_enabled();
                    return None;
                }
                KeyCode::Char('c')
                | KeyCode::Char('C')
                | KeyCode::Char('d')
                | KeyCode::Char('D')
                    if matches!(self.ai_wizard.step, AIWizardStep::ModelSelect) =>
                {
                    if self.ai_wizard.is_edit {
                        self.ai_wizard.show_delete();
                    }
                    return None;
                }
                _ => {}
            }
        }

        if self.text_input_active() {
            self.handle_text_input_key(key, is_key_press)
        } else {
            // Normal key handling
            match (key.code, key.modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) if is_key_press => Some(Message::CtrlC),
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT)
                    if is_key_press
                        && matches!(self.screen, Screen::BranchList)
                        && self.branch_list.filter_mode =>
                {
                    Some(Message::Char(c))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => {
                    // 'q' does not quit in BranchList (matches TypeScript behavior)
                    Some(Message::Char('q'))
                }
                (KeyCode::Esc, _) => {
                    // FR-095: ESC key behavior:
                    // - In filter mode: exit filter mode (handled by NavigateBack)
                    // - In BranchList with filter query: clear query
                    // - In BranchList with active agent pane: hide the pane
                    // - Otherwise: navigate back (but NOT quit from main screen)
                    if matches!(self.screen, Screen::BranchList) {
                        if self.branch_list.filter_mode {
                            // Exit filter mode (clear query if any, then exit mode)
                            Some(Message::NavigateBack)
                        } else if !self.branch_list.filter.is_empty() {
                            // Clear filter query
                            self.branch_list.clear_filter();
                            self.refresh_branch_summary();
                            None
                        } else if self.has_active_agent_pane() {
                            // FR-095: Hide active agent pane
                            Some(Message::HideActiveAgentPane)
                        } else {
                            // On main screen without filter and no active pane - do nothing
                            None
                        }
                    } else {
                        Some(Message::NavigateBack)
                    }
                }
                (KeyCode::Char('?') | KeyCode::Char('h'), KeyModifiers::NONE) => {
                    if matches!(self.screen, Screen::BranchList | Screen::Help) {
                        Some(Message::NavigateTo(Screen::Help))
                    } else {
                        Some(Message::Char(if key.code == KeyCode::Char('?') {
                            '?'
                        } else {
                            'h'
                        }))
                    }
                }
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    if matches!(self.screen, Screen::BranchList) {
                        if self.branch_list.filter_mode {
                            Some(Message::Char('n'))
                        } else {
                            None
                        }
                    } else if matches!(self.screen, Screen::Profiles) {
                        // Create new profile
                        self.profiles.enter_create_mode();
                        None
                    } else if matches!(self.screen, Screen::Environment) {
                        if self.environment.edit_mode {
                            Some(Message::Char('n'))
                        } else if self.environment.is_ai_only() {
                            self.status_message =
                                Some("AI settings only. Use Enter to edit.".to_string());
                            self.status_message_time = Some(Instant::now());
                            None
                        } else {
                            self.environment.start_new();
                            None
                        }
                    } else {
                        Some(Message::Char('n'))
                    }
                }
                (KeyCode::Char('s'), KeyModifiers::NONE) => {
                    // In filter mode, 's' goes to filter input
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        Some(Message::CycleSortMode)
                    } else {
                        Some(Message::Char('s'))
                    }
                }
                (KeyCode::Char('r'), KeyModifiers::NONE) => {
                    // In filter mode, 'r' goes to filter input
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        Some(Message::RefreshData)
                    } else if matches!(self.screen, Screen::Environment) {
                        if self.environment.edit_mode {
                            Some(Message::Char('r'))
                        } else if self.environment.is_ai_only() {
                            self.status_message =
                                Some("AI settings only. Use Enter to edit.".to_string());
                            self.status_message_time = Some(Instant::now());
                            None
                        } else {
                            self.reset_selected_env();
                            None
                        }
                    } else {
                        Some(Message::Char('r'))
                    }
                }
                (KeyCode::Char('c'), KeyModifiers::NONE) => {
                    // Copy to clipboard on Logs screen
                    if matches!(self.screen, Screen::Logs) && !self.logs.is_searching {
                        Some(Message::CopyLogToClipboard)
                    } else if matches!(self.screen, Screen::BranchList)
                        && !self.branch_list.filter_mode
                    {
                        // FR-010: Cleanup command
                        // In filter mode, 'c' goes to filter input
                        // FR-028: Check if branches are selected
                        if self.branch_list.selected_branches.is_empty() {
                            self.status_message = Some("No branches selected.".to_string());
                            self.status_message_time = Some(Instant::now());
                            None
                        } else {
                            // FR-028a-b: Filter out remote branches and current branch
                            let cleanup_branches: Vec<String> = self
                                .branch_list
                                .selected_branches
                                .iter()
                                .filter(|name| {
                                    // Find the branch in the list
                                    self.branch_list
                                        .branches
                                        .iter()
                                        .find(|b| &b.name == *name)
                                        .map(|b| {
                                            // Exclude remote branches, current branch, and no-worktree
                                            b.branch_type == BranchType::Local
                                                && !b.is_current
                                                && b.has_worktree
                                        })
                                        .unwrap_or(false)
                                })
                                .cloned()
                                .collect();

                            if cleanup_branches.is_empty() {
                                let excluded = self.branch_list.selected_branches.len();
                                self.status_message = Some(format!(
                                    "{} branch(es) excluded (remote, current, or no worktree).",
                                    excluded
                                ));
                                self.status_message_time = Some(Instant::now());
                                None
                            } else {
                                // Show cleanup confirmation dialog
                                self.confirm = ConfirmState::cleanup(&cleanup_branches);
                                self.pending_cleanup_branches = cleanup_branches;
                                self.screen_stack.push(self.screen.clone());
                                self.screen = Screen::Confirm;
                                None
                            }
                        }
                    } else {
                        Some(Message::Char('c'))
                    }
                }
                (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    if matches!(self.screen, Screen::Profiles) {
                        self.delete_selected_profile();
                        None
                    } else if matches!(self.screen, Screen::Environment) {
                        if self.environment.edit_mode {
                            Some(Message::Char('d'))
                        } else if self.environment.is_ai_only() {
                            self.status_message =
                                Some("AI settings only. Use Enter to edit.".to_string());
                            self.status_message_time = Some(Instant::now());
                            None
                        } else {
                            self.delete_selected_env();
                            None
                        }
                    } else if matches!(self.screen, Screen::BranchList)
                        && !self.branch_list.filter_mode
                        && self.selected_branch_has_agent()
                    {
                        // FR-040: d key to delete agent pane with confirmation
                        Some(Message::ConfirmAgentTermination)
                    } else {
                        Some(Message::Char('d'))
                    }
                }
                (KeyCode::Char('p'), KeyModifiers::NONE) => {
                    // In filter mode, 'p' goes to filter input
                    // 'p' now navigates to Settings → Environment tab (SPEC-71f2742d)
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        use super::screens::settings::SettingsCategory;
                        self.settings.category = SettingsCategory::Environment;
                        self.settings.load_profiles_config();
                        Some(Message::NavigateTo(Screen::Settings))
                    } else {
                        Some(Message::Char('p'))
                    }
                }
                (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    // In filter mode, 'l' goes to filter input
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        Some(Message::NavigateTo(Screen::Logs))
                    } else {
                        Some(Message::Char('l'))
                    }
                }
                (KeyCode::Char('u'), KeyModifiers::NONE) => {
                    // FR-102g: In filter mode, 'u' goes to filter input
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        Some(Message::ReregisterHooks)
                    } else {
                        Some(Message::Char('u'))
                    }
                }
                (KeyCode::Char('f'), KeyModifiers::NONE) if is_key_press => {
                    if matches!(self.screen, Screen::Logs) {
                        Some(Message::CycleFilter)
                    } else if matches!(self.screen, Screen::BranchList) {
                        Some(Message::ToggleFilterMode)
                    } else {
                        Some(Message::Char('f'))
                    }
                }
                (KeyCode::Char('/'), KeyModifiers::NONE) if is_key_press => {
                    if matches!(self.screen, Screen::Logs) {
                        Some(Message::ToggleSearch)
                    } else if matches!(self.screen, Screen::BranchList) {
                        Some(Message::ToggleFilterMode)
                    } else {
                        Some(Message::Char('/'))
                    }
                }
                (KeyCode::Char(' '), _) => {
                    if matches!(self.screen, Screen::BranchList | Screen::Profiles) {
                        Some(Message::Space)
                    } else {
                        Some(Message::Char(' '))
                    }
                }
                (KeyCode::Tab, _) => Some(Message::Tab),
                (KeyCode::Char('m'), KeyModifiers::NONE) => {
                    if matches!(self.screen, Screen::BranchList) {
                        Some(Message::CycleViewMode)
                    } else {
                        Some(Message::Char('m'))
                    }
                }
                // SPEC-1ea18899: 'v' opens GitView for selected branch
                (KeyCode::Char('v'), KeyModifiers::NONE) => {
                    if matches!(self.screen, Screen::BranchList) && !self.branch_list.filter_mode {
                        Some(Message::NavigateTo(Screen::GitView))
                    } else if matches!(self.screen, Screen::GitView) {
                        // v key to go back from GitView (FR-004)
                        Some(Message::NavigateBack)
                    } else {
                        Some(Message::Char('v'))
                    }
                }
                (KeyCode::Up, _) if is_key_press => Some(Message::SelectPrev),
                (KeyCode::Down, _) if is_key_press => Some(Message::SelectNext),
                (KeyCode::PageUp, _) if is_key_press => Some(Message::PageUp),
                (KeyCode::PageDown, _) if is_key_press => Some(Message::PageDown),
                (KeyCode::Home, _) if is_key_press => Some(Message::GoHome),
                (KeyCode::End, _) if is_key_press => Some(Message::GoEnd),
                (KeyCode::Enter, _) if is_key_press => Some(Message::Enter),
                (KeyCode::Backspace, _) if is_key_press => Some(Message::Backspace),
                (KeyCode::Left, _) if is_key_press => Some(Message::CursorLeft),
                (KeyCode::Right, _) if is_key_press => Some(Message::CursorRight),
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) if is_key_press => {
                    Some(Message::Char(c))
                }
                _ => None,
            }
        }
    }
}

/// Run the TUI application
/// Returns agent launch plan if wizard completed, None otherwise
pub fn run() -> Result<Option<LaunchPlan>, GwtError> {
    run_with_context(None)
}

/// Run the TUI application with optional entry context
/// Returns agent launch plan if wizard completed, None otherwise
pub fn run_with_context(context: Option<TuiEntryContext>) -> Result<Option<LaunchPlan>, GwtError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut model = Model::new_with_context(context);

    // Event loop
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    loop {
        // Draw
        terminal.draw(|f| model.view(f))?;

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if let Some(msg) = model.handle_key_event(key) {
                        model.update(msg);
                    }
                }
                Event::Mouse(mouse) => {
                    // Error screen handles all mouse events (click, scroll)
                    if matches!(model.screen, Screen::Error) {
                        model.handle_error_mouse(mouse);
                    } else {
                        match mouse.kind {
                            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                                if matches!(model.screen, Screen::BranchList) {
                                    model.handle_branch_list_scroll(mouse);
                                }
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                // Overlay priority: wizard > ai_wizard > confirm > screen-specific
                                if model.wizard.visible {
                                    model.handle_wizard_mouse(mouse);
                                } else if model.ai_wizard.visible {
                                    model.handle_ai_wizard_mouse(mouse);
                                } else if matches!(model.screen, Screen::Confirm) {
                                    model.handle_confirm_mouse(mouse);
                                } else {
                                    match model.screen {
                                        Screen::BranchList => model.handle_branch_list_mouse(mouse),
                                        Screen::Profiles => model.handle_profiles_mouse(mouse),
                                        Screen::Environment => {
                                            model.handle_environment_mouse(mouse)
                                        }
                                        Screen::Logs => model.handle_logs_mouse(mouse),
                                        // SPEC-1ea18899: GitView mouse handling (FR-007)
                                        Screen::GitView => model.handle_gitview_mouse(mouse),
                                        Screen::ServiceSelect => {
                                            model.handle_service_select_mouse(mouse)
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Tick
        if last_tick.elapsed() >= tick_rate {
            model.update(Message::Tick);
            last_tick = Instant::now();
        }

        // Check quit
        if model.should_quit {
            break;
        }
    }

    // Get pending agent launch before cleanup
    let pending_launch = model.pending_agent_launch.take();

    // SPEC-a70a1ece: Use bare repo path for worktree operations in bare projects
    let cleanup_path = model.bare_repo_path.as_ref().unwrap_or(&model.repo_root);
    auto_cleanup_orphans_on_exit(cleanup_path, pending_launch.is_some());

    // Cleanup agent panes on exit (tmux multi-mode)
    if model.tmux_mode.is_multi() && !model.agent_panes.is_empty() {
        debug!(
            category = "tui",
            pane_count = model.agent_panes.len(),
            "Cleaning up agent panes on exit"
        );
        for pane_id in &model.agent_panes {
            if let Err(e) = kill_pane(pane_id) {
                debug!(
                    category = "tui",
                    pane_id = %pane_id,
                    error = %e,
                    "Failed to kill agent pane"
                );
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    Ok(pending_launch)
}

fn auto_cleanup_orphans_on_exit(repo_root: &Path, pending_launch: bool) -> usize {
    if pending_launch {
        return 0;
    }

    match WorktreeManager::new(repo_root) {
        Ok(manager) => manager.auto_cleanup_orphans().unwrap_or(0),
        Err(_) => 0,
    }
}

fn canonical_tool_id(tool_id: &str) -> String {
    let lower = tool_id.trim().to_lowercase();
    if lower.contains("claude") {
        return "claude-code".to_string();
    }
    if lower.contains("codex") {
        return "codex-cli".to_string();
    }
    if lower.contains("gemini") {
        return "gemini-cli".to_string();
    }
    if lower.contains("opencode") || lower.contains("open-code") {
        return "opencode".to_string();
    }
    tool_id.trim().to_string()
}

/// Build agent-specific command line arguments for tmux mode
/// (Same logic as main.rs build_agent_args)
fn build_agent_args_for_tmux(config: &AgentLaunchConfig) -> Vec<String> {
    use gwt_core::agent::codex::{codex_default_args, codex_skip_permissions_flag};

    // SPEC-71f2742d: Handle custom agents
    if let Some(ref custom) = config.custom_agent {
        return build_custom_agent_args_for_tmux(custom, config);
    }

    let mut args = Vec::new();

    match config.agent {
        CodingAgent::ClaudeCode => {
            // Model selection
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode (FR-102) - same logic as single mode
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
                    if let Some(session_id) = &config.session_id {
                        args.push("--resume".to_string());
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("-c".to_string());
                    } else {
                        args.push("-r".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions
            if config.skip_permissions {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }
        CodingAgent::CodexCli => {
            // Execution mode - resume subcommand must come first
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
                    args.push("resume".to_string());
                    if let Some(session_id) = &config.session_id {
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("--last".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions (Codex uses versioned flag)
            let skip_flag = if config.skip_permissions {
                Some(codex_skip_permissions_flag(None))
            } else {
                None
            };
            let bypass_sandbox = matches!(
                skip_flag,
                Some("--dangerously-bypass-approvals-and-sandbox")
            );

            let reasoning_override = config.reasoning_level.map(|r| r.label());
            args.extend(codex_default_args(
                config.model.as_deref(),
                reasoning_override,
                None, // skills_flag_version
                bypass_sandbox,
                config.collaboration_modes,
            ));

            if let Some(flag) = skip_flag {
                args.push(flag.to_string());
            }
        }
        CodingAgent::GeminiCli => {
            // Model selection (Gemini uses -m or --model)
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("-m".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
                    if let Some(session_id) = &config.session_id {
                        args.push("--resume".to_string());
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("--continue".to_string());
                    } else {
                        args.push("--resume".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions
            if config.skip_permissions {
                args.push("-y".to_string());
            }
        }
        CodingAgent::OpenCode => {
            // Model selection
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
                    if let Some(session_id) = &config.session_id {
                        args.push("--resume".to_string());
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("--continue".to_string());
                    } else {
                        args.push("--resume".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }
        }
    }

    args
}

/// Build command line arguments for custom agents (SPEC-71f2742d T206)
fn build_custom_agent_args_for_tmux(
    custom: &CustomCodingAgent,
    config: &AgentLaunchConfig,
) -> Vec<String> {
    let mut args = Vec::new();

    // Add default args
    args.extend(custom.default_args.clone());

    // Add mode-specific args (T208)
    if let Some(ref mode_args) = custom.mode_args {
        match config.execution_mode {
            ExecutionMode::Normal => {
                args.extend(mode_args.normal.clone());
            }
            ExecutionMode::Continue => {
                args.extend(mode_args.continue_mode.clone());
            }
            ExecutionMode::Resume | ExecutionMode::Convert => {
                args.extend(mode_args.resume.clone());
            }
        }
    }

    // Add permission skip args if skip_permissions is true (T210)
    if config.skip_permissions && !custom.permission_skip_args.is_empty() {
        args.extend(custom.permission_skip_args.clone());
    }

    args
}

fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

fn build_shell_command(command: &str, args: &[String]) -> String {
    let mut cmd_parts = vec![shell_escape(command)];
    cmd_parts.extend(args.iter().map(|arg| shell_escape(arg)));
    cmd_parts.join(" ")
}

fn normalize_container_executable(executable: &str) -> String {
    let is_windows_drive_abs = executable.len() > 2
        && executable.as_bytes()[1] == b':'
        && (executable.as_bytes()[2] == b'\\' || executable.as_bytes()[2] == b'/');
    let is_unc_path = executable.starts_with("\\\\");

    if is_windows_drive_abs || is_unc_path {
        if let Some(name) = executable.rsplit(&['\\', '/'][..]).next() {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }

    let path = Path::new(executable);
    if path.is_absolute() {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }

    executable.to_string()
}

fn wrap_tmux_command_for_fast_exit(command: &str) -> String {
    format!(
        "start=$(date +%s); {} ; exit_status=$?; end=$(date +%s); if [ $exit_status -ne 0 ] || [ $((end-start)) -lt {} ]; then echo; echo \"[gwt] Agent exited immediately (status=$exit_status).\"; echo \"[gwt] Press Enter to close this pane.\"; read -r _; fi; exit $exit_status",
        command, FAST_EXIT_THRESHOLD_SECS
    )
}

/// Build the full tmux command string with environment variables
fn build_tmux_command(
    env_vars: &[(String, String)],
    env_remove: &[String],
    command: &str,
) -> String {
    let mut parts = Vec::new();

    // Remove environment variables
    for key in env_remove {
        if !is_valid_env_name(key) {
            continue;
        }
        parts.push(format!("unset {}", key));
    }

    // Add environment variable exports
    for (key, value) in env_vars {
        if !is_valid_env_name(key) {
            continue;
        }
        let escaped_value = shell_escape(value);
        parts.push(format!("export {}={}", key, escaped_value));
    }

    if parts.is_empty() {
        command.to_string()
    } else {
        parts.push(command.to_string());
        parts.join("; ")
    }
}

fn resolve_remote_head(repo_root: &Path, remote: &str) -> Option<String> {
    let output = Command::new("git")
        .args([
            "symbolic-ref",
            "--quiet",
            &format!("refs/remotes/{remote}/HEAD"),
        ])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let ref_name = stdout.trim();
    let ref_name = ref_name.strip_prefix("refs/remotes/")?;
    if ref_name.ends_with("/HEAD") {
        return None;
    }
    Some(ref_name.to_string())
}

fn resolve_safety_base(repo_root: &Path, base_branch: &str) -> (String, bool) {
    if Branch::exists(repo_root, base_branch).unwrap_or(false) {
        return (base_branch.to_string(), true);
    }

    let default_remote = Remote::default(repo_root).ok().flatten();
    if let Some(remote) = default_remote {
        if Branch::remote_exists(repo_root, &remote.name, base_branch).unwrap_or(false) {
            return (format!("{}/{}", remote.name, base_branch), true);
        }
        if let Some(remote_head) = resolve_remote_head(repo_root, &remote.name) {
            return (remote_head, true);
        }
    }

    (base_branch.to_string(), false)
}

fn resolve_repo_web_url(repo_root: &Path) -> Option<String> {
    let remote = Remote::get(repo_root, "origin").ok().flatten()?;
    let slug = github_repo_slug(&remote.fetch_url)?;
    Some(format!("https://github.com/{}", slug))
}

fn github_repo_slug(url: &str) -> Option<String> {
    let slug = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = url.strip_prefix("ssh://git@github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://github.com/") {
        rest
    } else if let Some(rest) = url.strip_prefix("git://github.com/") {
        rest
    } else {
        return None;
    };

    let slug = slug.trim_end_matches(".git").trim_end_matches('/');
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::branch_list::{SafetyStatus, WorktreeStatus};
    use crate::tui::screens::environment::EnvItem;
    use crate::tui::screens::settings::{ProfileMode, SettingsCategory};
    use crate::tui::screens::wizard::WizardStep;
    use crate::tui::screens::{BranchItem, BranchListState, BranchType};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use gwt_core::config::{AISettings, Profile, ProfilesConfig, Settings};
    use gwt_core::git::Branch;
    use gwt_core::git::BranchSummary;
    use gwt_core::git::DivergenceStatus;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashMap;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::TempDir;

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git execution failed");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        temp
    }

    fn create_test_repo_with_branch(branch: &str) -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init", "-b", branch]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        std::fs::write(temp.path().join("test.txt"), "hello").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        temp
    }

    fn create_repo_with_remote_main_only() -> (TempDir, TempDir) {
        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare", "-b", "main"]);

        let repo = create_test_repo_with_branch("main");
        let origin_path = origin.path().to_string_lossy().to_string();
        run_git(repo.path(), &["remote", "add", "origin", &origin_path]);
        run_git(repo.path(), &["push", "-u", "origin", "main"]);
        run_git(repo.path(), &["checkout", "-b", "develop"]);
        run_git(repo.path(), &["branch", "-D", "main"]);
        run_git(repo.path(), &["fetch", "origin"]);
        run_git(repo.path(), &["remote", "set-head", "origin", "-a"]);

        (repo, origin)
    }

    fn create_repo_with_local_and_remote_main() -> (TempDir, TempDir) {
        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare", "-b", "main"]);

        let repo = create_test_repo_with_branch("main");
        let origin_path = origin.path().to_string_lossy().to_string();
        run_git(repo.path(), &["remote", "add", "origin", &origin_path]);
        run_git(repo.path(), &["push", "-u", "origin", "main"]);
        run_git(repo.path(), &["fetch", "origin"]);

        (repo, origin)
    }

    fn worktree_list_output(repo_root: &Path) -> String {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo_root)
            .output()
            .expect("git worktree list execution failed");
        assert!(
            output.status.success(),
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn sample_tool_entry(tool_id: &str) -> ToolSessionEntry {
        ToolSessionEntry {
            branch: "feature/test".to_string(),
            worktree_path: Some("/tmp/worktree".to_string()),
            tool_id: tool_id.to_string(),
            tool_label: "Codex".to_string(),
            session_id: None,
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: None,
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            timestamp: 0,
        }
    }

    #[test]
    fn test_default_recreate_selected_for_rebuild() {
        assert!(Model::default_recreate_selected(true));
        assert!(!Model::default_recreate_selected(false));
    }

    #[test]
    fn test_quick_start_recreate_allowed_requires_change() {
        assert!(!Model::quick_start_recreate_allowed(false, true));
        assert!(!Model::quick_start_recreate_allowed(false, false));
        assert!(Model::quick_start_recreate_allowed(true, true));
        assert!(!Model::quick_start_recreate_allowed(true, false));
    }

    #[test]
    fn test_resolve_safety_base_uses_remote_branch_when_local_missing() {
        let (repo, _origin) = create_repo_with_remote_main_only();
        let (base_ref, exists) = resolve_safety_base(repo.path(), "main");

        assert!(exists);
        assert_eq!(base_ref, "origin/main");
    }

    #[test]
    fn test_resolve_safety_base_falls_back_to_remote_head() {
        let (repo, _origin) = create_repo_with_remote_main_only();
        let (base_ref, exists) = resolve_safety_base(repo.path(), "trunk");

        assert!(exists);
        assert_eq!(base_ref, "origin/main");
    }

    #[test]
    fn test_auto_cleanup_orphans_on_exit_does_not_prune() {
        let temp = create_test_repo();
        let manager = WorktreeManager::new(temp.path()).unwrap();

        let wt = manager
            .create_new_branch("feature/orphan-exit", None)
            .unwrap();
        let wt_path = wt.path.clone();
        std::fs::remove_dir_all(&wt_path).unwrap();

        let before = worktree_list_output(temp.path());
        assert!(before.contains(wt_path.to_string_lossy().as_ref()));

        let cleaned = auto_cleanup_orphans_on_exit(temp.path(), false);
        assert_eq!(cleaned, 0);

        let after = worktree_list_output(temp.path());
        assert!(after.contains(wt_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn test_quick_start_reuses_existing_worktree_path_for_remote_branch_name() {
        let repo = create_test_repo_with_branch("main");

        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare", "-b", "main"]);
        let origin_path = origin.path().to_string_lossy().to_string();
        run_git(repo.path(), &["remote", "add", "origin", &origin_path]);
        run_git(repo.path(), &["push", "-u", "origin", "main"]);

        let worktree_path = repo.path().join(".worktrees/feature-existing");
        run_git(
            repo.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_string_lossy().as_ref(),
                "-b",
                "feature/existing",
            ],
        );
        run_git(repo.path(), &["push", "-u", "origin", "feature/existing"]);
        let worktree_path = std::fs::canonicalize(&worktree_path).unwrap_or(worktree_path);

        let context = TuiEntryContext::success("".to_string()).with_repo_root(repo.path().into());
        let mut model = Model::new_with_context(Some(context));

        let request = LaunchRequest {
            branch_name: "remotes/origin/feature/existing".to_string(),
            create_new_branch: false,
            base_branch: None,
            agent: CodingAgent::ClaudeCode,
            custom_agent: None,
            model: None,
            reasoning_level: None,
            version: "latest".to_string(),
            execution_mode: ExecutionMode::Normal,
            session_id: None,
            skip_permissions: false,
            collaboration_modes: false,
            env: Vec::new(),
            env_remove: Vec::new(),
            auto_install_deps: false,
            selected_issue: None,
            existing_worktree_path: Some(worktree_path.clone()),
        };

        model.start_launch_preparation(request);

        let rx = model.launch_rx.take().expect("launch rx not set");
        let mut worktree_ready = None;
        let mut failure = None;
        for _ in 0..40 {
            match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(LaunchUpdate::WorktreeReady { path, .. }) => {
                    worktree_ready = Some(path);
                    break;
                }
                Ok(LaunchUpdate::Failed(message)) => {
                    failure = Some(message);
                    break;
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if let Some(message) = failure {
            panic!("expected WorktreeReady, got failure: {}", message);
        }
        let ready_path = worktree_ready.expect("expected WorktreeReady update");
        assert_eq!(ready_path, worktree_path);
    }

    #[test]
    fn test_background_window_name_uses_branch_name_only() {
        let branch = "feature/clean-name";
        assert_eq!(background_window_name(branch), branch);
    }

    #[test]
    fn test_normalize_branch_name_for_history_strips_remote_prefix() {
        let normalized = normalize_branch_name_for_history("remotes/origin/feature/test");
        assert_eq!(normalized.as_ref(), "feature/test");
    }

    #[test]
    fn test_normalize_branch_name_for_history_keeps_local_name() {
        let normalized = normalize_branch_name_for_history("feature/test");
        assert_eq!(normalized.as_ref(), "feature/test");
    }

    #[test]
    fn test_resolve_existing_worktree_path_uses_normalized_remote_name() {
        let local = sample_branch_with_session("feature/test");
        let mut remote = local.clone();
        remote.name = "remotes/origin/feature/test".to_string();
        remote.branch_type = BranchType::Remote;
        remote.has_worktree = false;
        remote.worktree_path = None;
        remote.worktree_status = WorktreeStatus::None;

        let resolved =
            resolve_existing_worktree_path("remotes/origin/feature/test", &[remote, local], false);

        assert_eq!(resolved, Some(PathBuf::from("/tmp/worktree")));
    }

    #[test]
    fn test_apply_last_tool_usage_falls_back_to_agent_history_for_remote_branch() {
        let branch = Branch::new("remotes/origin/feature/test", "deadbeef");
        let mut item = BranchItem::from_branch_minimal(&branch, &[]);
        let mut history = AgentHistoryStore::new();
        history
            .record(
                Path::new("/tmp/repo"),
                "feature/test",
                "codex-cli",
                "Codex@latest",
            )
            .unwrap();
        let tool_usage_map: HashMap<String, ToolSessionEntry> = HashMap::new();

        apply_last_tool_usage(&mut item, Path::new("/tmp/repo"), &tool_usage_map, &history);

        assert_eq!(item.last_tool_usage.as_deref(), Some("Codex@latest"));
        assert_eq!(item.last_tool_id.as_deref(), Some("codex-cli"));
    }

    #[test]
    fn test_branch_list_refresh_includes_remote_branch_with_local_counterpart() {
        let (repo, _origin) = create_repo_with_local_and_remote_main();
        let context = TuiEntryContext {
            status_message: None,
            error_message: None,
            repo_root: Some(repo.path().to_path_buf()),
        };
        let mut model = Model::new_with_context(Some(context));
        model.repo_root = repo.path().to_path_buf();
        model.repo_type = RepoType::Normal;
        model.bare_repo_path = None;

        model.start_branch_list_refresh(Settings::default());
        let rx = model.branch_list_rx.take().expect("branch_list_rx");
        let update = rx
            // Some CI environments can be slow; avoid flaky timeouts here.
            .recv_timeout(Duration::from_secs(10))
            .expect("branch list update");

        assert!(
            update
                .branches
                .iter()
                .any(|b| b.name == "remotes/origin/main" && b.branch_type == BranchType::Remote),
            "remote branch should be included even when local counterpart exists"
        );
    }

    fn sample_branch_with_session(name: &str) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            branch_type: BranchType::Local,
            is_current: false,
            has_worktree: true,
            worktree_path: Some("/tmp/worktree".to_string()),
            worktree_status: WorktreeStatus::Active,
            has_changes: false,
            has_unpushed: false,
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: true,
            remote_name: None,
            safe_to_cleanup: Some(true),
            safety_status: SafetyStatus::Safe,
            is_unmerged: false,
            last_commit_timestamp: None,
            last_tool_usage: None,
            last_tool_id: Some("codex-cli".to_string()),
            last_session_id: Some("sess-1".to_string()),
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
        }
    }

    fn sample_branch_with_usage(name: &str, usage: &str, tool_id: Option<&str>) -> BranchItem {
        let mut branch = sample_branch_with_session(name);
        branch.last_tool_usage = Some(usage.to_string());
        branch.last_tool_id = tool_id.map(|id| id.to_string());
        branch
    }

    fn sample_launch_plan() -> LaunchPlan {
        let config = AgentLaunchConfig {
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_path: PathBuf::from("/tmp/worktree"),
            branch_name: "feature/test".to_string(),
            agent: CodingAgent::ClaudeCode,
            custom_agent: None,
            model: None,
            reasoning_level: None,
            version: "latest".to_string(),
            execution_mode: ExecutionMode::Continue,
            session_id: None,
            skip_permissions: false,
            env: Vec::new(),
            env_remove: Vec::new(),
            auto_install_deps: false,
            collaboration_modes: false,
        };

        LaunchPlan {
            config,
            executable: "claude".to_string(),
            command_args: Vec::new(),
            log_lines: Vec::new(),
            session_warning: None,
            selected_version: "latest".to_string(),
            install_plan: InstallPlan::None,
            env: Vec::new(),
            repo_root: PathBuf::from("/tmp/repo"),
        }
    }

    fn sample_mouse_click() -> MouseClick {
        MouseClick {
            index: 0,
            at: Instant::now(),
        }
    }

    fn render_model_lines(model: &mut Model, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal init");
        terminal.draw(|f| model.view(f)).expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn footer_lines_for(model: &mut Model, width: u16) -> Vec<String> {
        let lines = model.footer_lines(width);
        model.sync_footer_metrics(width, lines.len());
        lines
    }

    #[test]
    fn test_branchlist_footer_and_status_bar_present_without_agents() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/layout")];
        model.branch_list = BranchListState::new().with_branches(branches);
        // Force BranchList screen (normal repos now show migration dialog by default)
        model.screen = Screen::BranchList;

        let height = 24;
        let lines = render_model_lines(&mut model, 80, height);
        let footer_text = footer_lines_for(&mut model, 80).join(" ");
        let status_line = &lines[(height - 1) as usize];

        assert!(
            footer_text.contains("[r] Refresh"),
            "Footer help should be visible on BranchList"
        );
        assert!(
            status_line.contains("Agents:") && status_line.contains("none"),
            "Status bar should show Agents: none when no agents are running"
        );
    }

    #[test]
    fn test_logs_screen_renders_header_working_directory() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Logs;

        let lines = render_model_lines(&mut model, 80, 24);
        let has_working_dir = lines.iter().any(|line| line.contains("Working Directory:"));
        assert!(
            has_working_dir,
            "Header should be present on Logs screen and include Working Directory"
        );
    }

    #[test]
    fn test_branch_screen_header_label() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/branch-screen")];
        model.branch_list = BranchListState::new().with_branches(branches);
        model.screen = Screen::BranchList;

        let lines = render_model_lines(&mut model, 80, 24);
        assert!(
            lines.iter().any(|line| line.contains("Branch Screen")),
            "Header should label BranchList as Branch Screen"
        );
    }

    #[test]
    fn test_agent_screen_header_label() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::AgentMode;

        let lines = render_model_lines(&mut model, 80, 24);
        assert!(
            lines.iter().any(|line| line.contains("Agent Screen")),
            "Header should label AgentMode as Agent Screen"
        );
    }

    #[test]
    fn test_settings_footer_has_context_keybinds_without_duplicate_instructions() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Settings;
        model.settings = SettingsState::new().with_settings(Settings::default());

        let width = 120;
        let height = 24;
        let lines = render_model_lines(&mut model, width, height);
        let footer_text = footer_lines_for(&mut model, width).join(" ");
        let instruction_key = "[Left/Right] Category";

        assert!(
            footer_text.contains(instruction_key),
            "Settings footer help should include category navigation"
        );

        let occurrences = lines
            .iter()
            .filter(|line| line.contains(instruction_key))
            .count();
        assert_eq!(
            occurrences, 1,
            "Settings instructions should appear once in the footer help"
        );
    }

    #[test]
    fn test_branchlist_footer_includes_all_shortcuts() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/shortcuts")];
        model.branch_list = BranchListState::new().with_branches(branches);
        model.screen = Screen::BranchList;

        let footer_text = footer_lines_for(&mut model, 140).join(" ");
        let expected = [
            "[Up/Down] Move",
            "[PgUp/PgDn] Page",
            "[Home/End] Top/Bottom",
            "[Enter] Open/Focus",
            "[Space] Select",
            "[r] Refresh",
            "[c] Cleanup",
            "[l] Logs",
            "[s] Sort",
            "[p] Environment",
            "[f,/] Filter",
            "[m] Mode",
            "[Tab] Agent",
            "[v] GitView",
            "[u] Hooks",
            "[?/h] Help",
            "[Ctrl+C] Quit",
        ];

        for key in expected {
            assert!(
                footer_text.contains(key),
                "BranchList footer should include {}",
                key
            );
        }
    }

    #[test]
    fn test_branchlist_filter_footer_includes_controls() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/filter")];
        let mut state = BranchListState::new().with_branches(branches);
        state.filter_mode = true;
        model.branch_list = state;
        model.screen = Screen::BranchList;

        let footer_text = footer_lines_for(&mut model, 120).join(" ");
        let expected = [
            "[Esc] Exit filter",
            "[Enter] Apply",
            "[Backspace] Delete",
            "[Up/Down] Move",
            "[PgUp/PgDn] Page",
            "[Home/End] Top/Bottom",
            "Type to search",
        ];

        for key in expected {
            assert!(
                footer_text.contains(key),
                "Filter-mode footer should include {}",
                key
            );
        }
    }

    #[test]
    fn test_logs_footer_includes_paging_shortcuts() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Logs;

        let footer_text = footer_lines_for(&mut model, 120).join(" ");
        let expected = ["[PgUp/PgDn] Page", "[Home/End] Top/Bottom"];

        for key in expected {
            assert!(
                footer_text.contains(key),
                "Logs footer should include {}",
                key
            );
        }
    }

    #[test]
    fn test_help_footer_includes_paging_shortcuts() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Help;

        let footer_text = footer_lines_for(&mut model, 120).join(" ");
        assert!(
            footer_text.contains("[PgUp/PgDn] Page"),
            "Help footer should include page navigation"
        );
    }

    #[test]
    fn test_error_footer_includes_actions() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Error;

        let footer_text = footer_lines_for(&mut model, 120).join(" ");
        let expected = [
            "[Enter] Close",
            "[Esc] Close",
            "[Up/Down] Scroll",
            "[l] Logs",
            "[c] Copy",
        ];

        for key in expected {
            assert!(
                footer_text.contains(key),
                "Error footer should include {}",
                key
            );
        }
    }

    #[test]
    fn test_footer_scroll_disabled_when_single_line() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/scroll")];
        model.branch_list = BranchListState::new().with_branches(branches);
        model.screen = Screen::BranchList;

        let lines = footer_lines_for(&mut model, 1000);
        assert_eq!(lines.len(), 1, "Footer should fit in one line");

        for _ in 0..10 {
            model.update(Message::Tick);
        }

        assert_eq!(model.footer_scroll_offset, 0);
    }

    #[test]
    fn test_footer_scroll_advances_and_reverses() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/scroll")];
        model.branch_list = BranchListState::new().with_branches(branches);
        model.screen = Screen::BranchList;

        let lines = footer_lines_for(&mut model, 40);
        assert!(lines.len() > 1, "Footer should overflow for scrolling");
        let max_offset = lines.len().saturating_sub(1);

        let advance_ticks = FOOTER_SCROLL_TICKS_PER_LINE as usize * max_offset.max(1);
        for _ in 0..advance_ticks {
            model.update(Message::Tick);
        }

        assert_eq!(model.footer_scroll_offset, max_offset);
        assert_eq!(model.footer_scroll_pause, 0);

        for _ in 0..FOOTER_SCROLL_TICKS_PER_LINE {
            model.update(Message::Tick);
        }

        assert_eq!(model.footer_scroll_offset, max_offset.saturating_sub(1));
    }

    #[test]
    fn test_footer_scroll_resets_on_navigation() {
        let mut model = Model::new_with_context(None);
        let branches = vec![sample_branch_with_session("feature/scroll")];
        model.branch_list = BranchListState::new().with_branches(branches);
        model.screen = Screen::BranchList;

        let lines = footer_lines_for(&mut model, 40);
        assert!(lines.len() > 1, "Footer should overflow for scrolling");

        for _ in 0..FOOTER_SCROLL_TICKS_PER_LINE {
            model.update(Message::Tick);
        }
        assert!(model.footer_scroll_offset > 0);

        model.update(Message::NavigateTo(Screen::Logs));
        assert_eq!(model.footer_scroll_offset, 0);
    }

    #[test]
    fn test_resolve_orphaned_agent_name_prefers_session_entry() {
        let entry = sample_tool_entry("codex-cli");
        let resolved = resolve_orphaned_agent_name("bash", Some(&entry));
        assert_eq!(resolved, "codex-cli");
    }

    #[test]
    fn test_wrap_tmux_command_for_fast_exit_uses_exit_status_variable() {
        let wrapped = wrap_tmux_command_for_fast_exit("echo ok");
        assert!(wrapped.contains("exit_status=$?"));
        assert!(wrapped.contains("status=$exit_status"));
        assert!(wrapped.contains("exit $exit_status"));
        assert!(!wrapped.contains("; status=$?;"));
    }

    #[test]
    fn test_normalize_container_executable_strips_path() {
        assert_eq!(
            normalize_container_executable("/usr/local/bin/bunx"),
            "bunx"
        );
        assert_eq!(
            normalize_container_executable("C:\\tools\\codex.exe"),
            "codex.exe"
        );
        assert_eq!(normalize_container_executable("codex"), "codex");
        assert_eq!(
            normalize_container_executable("./scripts/agent.sh"),
            "./scripts/agent.sh"
        );
        assert_eq!(
            normalize_container_executable("scripts\\agent.exe"),
            "scripts\\agent.exe"
        );
    }

    #[test]
    fn test_resolve_orphaned_agent_name_fallbacks() {
        let resolved = resolve_orphaned_agent_name("bash", None);
        assert_eq!(resolved, "bash");
        let resolved = resolve_orphaned_agent_name("  ", None);
        assert_eq!(resolved, "unknown");
    }

    #[test]
    fn test_extract_tool_version_from_usage_matches_tool_id() {
        let branch = sample_branch_with_usage("feature/version", "Codex@2.1.0", Some("codex-cli"));
        let version = Model::extract_tool_version_from_usage(&branch, "codex-cli");
        assert_eq!(version.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn test_extract_tool_version_from_usage_matches_label_when_id_missing() {
        let branch = sample_branch_with_usage("feature/version", "Codex@2.1.0", None);
        let version = Model::extract_tool_version_from_usage(&branch, "codex-cli");
        assert_eq!(version.as_deref(), Some("2.1.0"));
    }

    #[test]
    fn test_extract_tool_version_from_usage_rejects_mismatch() {
        let branch = sample_branch_with_usage("feature/version", "Codex@2.1.0", Some("codex-cli"));
        let version = Model::extract_tool_version_from_usage(&branch, "gemini-cli");
        assert!(version.is_none());
    }

    #[test]
    fn test_extract_tool_version_from_usage_requires_version_suffix() {
        let branch = sample_branch_with_usage("feature/version", "Codex", Some("codex-cli"));
        let version = Model::extract_tool_version_from_usage(&branch, "codex-cli");
        assert!(version.is_none());
        let branch = sample_branch_with_usage("feature/version", "Codex@", Some("codex-cli"));
        let version = Model::extract_tool_version_from_usage(&branch, "codex-cli");
        assert!(version.is_none());
    }

    #[test]
    fn test_apply_launch_updates_sets_status() {
        let mut model = Model::new_with_context(None);
        let (tx, rx) = mpsc::channel();
        model.launch_rx = Some(rx);

        tx.send(LaunchUpdate::Progress(LaunchProgress::BuildingCommand))
            .unwrap();
        model.apply_launch_updates();

        let expected = LaunchProgress::BuildingCommand.message();
        assert_eq!(model.launch_status.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn test_confirm_cancel_clears_plugin_setup_state() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Confirm;
        model.screen_stack.push(Screen::BranchList);
        model.pending_plugin_setup = true;
        model.pending_plugin_setup_launch = Some(sample_launch_plan());
        model.launch_status = Some("Launching...".to_string());

        model.update(Message::NavigateBack);

        assert!(!model.pending_plugin_setup);
        assert!(model.pending_plugin_setup_launch.is_none());
        assert!(model.launch_status.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_prepare_docker_service_selection_prompts_for_dockerfile_target() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM alpine:3.20").unwrap();

        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let mut plan = sample_launch_plan();
        plan.config.worktree_path = temp_dir.path().to_path_buf();

        let decision = model
            .prepare_docker_service_selection(&plan)
            .expect("prepare_docker_service_selection");
        assert!(matches!(decision, ServiceSelectionDecision::AwaitSelection));
        assert!(matches!(model.screen, Screen::ServiceSelect));
        assert!(model.pending_service_select.is_some());
        assert_eq!(model.service_select.items.len(), 2);
        assert_eq!(model.service_select.items[0].label, "HostOS");
        assert_eq!(model.service_select.items[1].label, "Docker");
        assert_eq!(model.service_select.selected, 1);
        assert_eq!(model.screen_stack.len(), 1);
    }

    #[test]
    fn test_service_select_cancel_clears_double_click_state() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::ServiceSelect;
        model.screen_stack.push(Screen::BranchList);
        model.pending_service_select = Some(PendingServiceSelect {
            plan: sample_launch_plan(),
            services: vec!["app".to_string()],
        });
        model.last_mouse_click = Some(sample_mouse_click());

        model.update(Message::NavigateBack);

        assert!(model.last_mouse_click.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_service_select_confirm_clears_double_click_state() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::ServiceSelect;
        model.screen_stack.push(Screen::BranchList);
        model.pending_service_select = Some(PendingServiceSelect {
            plan: sample_launch_plan(),
            services: vec!["app".to_string()],
        });
        model.service_select = ServiceSelectState::with_services(vec!["app".to_string()]);
        model.last_mouse_click = Some(sample_mouse_click());

        model.handle_service_select_confirm();

        assert!(model.last_mouse_click.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_service_select_skip_clears_double_click_state() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::ServiceSelect;
        model.screen_stack.push(Screen::BranchList);
        model.pending_service_select = Some(PendingServiceSelect {
            plan: sample_launch_plan(),
            services: vec!["app".to_string()],
        });
        model.last_mouse_click = Some(sample_mouse_click());

        model.handle_service_select_skip();

        assert!(model.last_mouse_click.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_docker_confirm_host_shortcut_closes_build_prompt() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Confirm;
        model.screen_stack.push(Screen::BranchList);
        model.pending_build_select = Some(PendingBuildSelect {
            plan: sample_launch_plan(),
            service: Some("app".to_string()),
            force_host: false,
            force_recreate: false,
            quick_start_keep: None,
        });
        model.last_mouse_click = Some(sample_mouse_click());

        model.update(Message::Char('h'));

        assert!(model.pending_build_select.is_none());
        assert!(model.last_mouse_click.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_confirm_footer_keybinds_include_host_shortcut_when_docker_prompt_active() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Confirm;
        model.pending_build_select = Some(PendingBuildSelect {
            plan: sample_launch_plan(),
            service: Some("app".to_string()),
            force_host: false,
            force_recreate: false,
            quick_start_keep: None,
        });

        let keybinds = model.get_footer_keybinds();

        assert!(keybinds.contains("[h] HostOS"));
    }

    #[test]
    fn test_prepare_docker_service_selection_respects_force_host_setting() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("Dockerfile"), "FROM scratch\n").unwrap();

        let mut model = Model::new_with_context(None);
        let mut settings = Settings::default();
        settings.docker.force_host = true;
        model.settings = SettingsState::new().with_settings(settings);

        let mut plan = sample_launch_plan();
        plan.config.worktree_path = temp.path().to_path_buf();

        let decision = model.prepare_docker_service_selection(&plan).unwrap();
        match decision {
            ServiceSelectionDecision::Proceed {
                service,
                force_host,
            } => {
                assert!(force_host);
                assert!(service.is_none());
            }
            ServiceSelectionDecision::AwaitSelection => {
                panic!("expected Proceed, got AwaitSelection");
            }
        }
    }

    #[test]
    fn test_version_select_single_enter_advances() {
        let mut model = Model::new_with_context(None);
        model.wizard.visible = true;
        model.wizard.step = WizardStep::ModelSelect;
        model.wizard.agent = CodingAgent::ClaudeCode;
        model.wizard.agent_index = 0;
        model.wizard.versions_fetched = true;

        model.update(Message::WizardConfirm);
        assert_eq!(model.wizard.step, WizardStep::VersionSelect);

        model.update(Message::WizardConfirm);
        assert_eq!(model.wizard.step, WizardStep::ExecutionMode);
    }

    #[test]
    fn test_space_ignores_branch_without_worktree() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branch = Branch::new("feature/no-worktree", "deadbeef");
        let item = BranchItem::from_branch(&branch, &[]);
        model.branch_list = BranchListState::new().with_branches(vec![item]);

        model.update(Message::Space);

        assert!(model.branch_list.selected_branches.is_empty());
        assert!(model.pending_unsafe_selection.is_none());
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_profile_input_disables_shortcuts() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Profiles;
        model.profiles.enter_create_mode();

        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        let msg = model.handle_text_input_key(key, true);
        assert!(matches!(msg, Some(Message::Char('n'))));
    }

    #[test]
    fn test_filter_mode_char_overrides_shortcuts() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        model.branch_list.enter_filter_mode();

        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        let msg = model.handle_key_event(key);
        assert!(matches!(msg, Some(Message::Char('r'))));

        let key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE);
        let msg = model.handle_key_event(key);
        assert!(matches!(msg, Some(Message::Char('f'))));

        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        let msg = model.handle_key_event(key);
        assert!(matches!(msg, Some(Message::Char('s'))));
    }

    #[test]
    fn test_branchlist_s_key_cycles_sort_mode() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branches = vec![sample_branch_with_session("feature/sort")];
        model.branch_list = BranchListState::new().with_branches(branches);

        let initial = model.branch_list.sort_mode;
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        let msg = model.handle_key_event(key);
        assert!(matches!(msg, Some(Message::CycleSortMode)));

        model.update(Message::CycleSortMode);
        assert_ne!(model.branch_list.sort_mode, initial);
    }

    #[test]
    fn test_filter_toggle_works_when_not_in_filter_mode() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE);
        let msg = model.handle_key_event(key);
        assert!(matches!(msg, Some(Message::ToggleFilterMode)));
    }

    #[test]
    fn test_environment_input_switches_field_on_tab() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Environment;
        model.environment.start_new();
        assert_eq!(model.environment.edit_field, EditField::Key);

        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        let msg = model.handle_text_input_key(key, true);
        assert!(msg.is_none());
        assert_eq!(model.environment.edit_field, EditField::Value);
    }

    #[test]
    fn test_environment_input_enter_moves_to_value_field() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Environment;
        model.environment.start_new();
        assert_eq!(model.environment.edit_field, EditField::Key);

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let msg = model.handle_text_input_key(key, true);
        assert!(msg.is_none());
        assert_eq!(model.environment.edit_field, EditField::Value);
    }

    #[test]
    fn test_settings_env_edit_enter_switches_to_value_field() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Settings;
        model.settings.category = SettingsCategory::Environment;
        model.settings.profile_mode = ProfileMode::EnvEdit("dev".to_string());
        model.settings.env_state.start_new();

        assert_eq!(model.settings.env_state.edit_field, EditField::Key);
        model.update(Message::Enter);
        assert_eq!(model.settings.env_state.edit_field, EditField::Value);
        assert!(model.settings.env_state.edit_mode);
    }

    #[test]
    fn test_settings_env_edit_updates_existing_variable() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Settings;
        model.settings.category = SettingsCategory::Environment;
        model.settings.profile_mode = ProfileMode::EnvEdit("dev".to_string());
        model.settings.env_state = EnvironmentState::new()
            .with_variables(vec![EnvItem {
                key: "MY_VAR".to_string(),
                value: "old".to_string(),
                is_secret: false,
            }])
            .with_hide_ai(true);
        model.settings.env_state.selected = 0;
        model.settings.env_state.start_edit_selected();
        model.settings.env_state.edit_value = "new".to_string();

        model.update(Message::Enter);

        assert_eq!(model.settings.env_state.variables[0].value, "new");
        assert!(!model.settings.env_state.edit_mode);
    }

    #[test]
    fn test_settings_env_edit_deletes_added_variable_with_d() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::Settings;
        model.settings.category = SettingsCategory::Environment;
        model.settings.profile_mode = ProfileMode::EnvEdit("dev".to_string());
        model.settings.env_state = EnvironmentState::new()
            .with_variables(vec![EnvItem {
                key: "MY_VAR".to_string(),
                value: "value".to_string(),
                is_secret: false,
            }])
            .with_hide_ai(true);
        model.settings.env_state.selected = 0;

        model.update(Message::Char('d'));

        assert!(model.settings.env_state.variables.is_empty());
    }

    #[test]
    fn test_refresh_data_starts_branch_list_loading() {
        let mut model = Model::new_with_context(None);

        assert!(model.branch_list.is_loading);
        assert!(model.branch_list.branches.is_empty());

        model.update(Message::SelectNext);
        assert_eq!(model.branch_list.selected, 0);
    }

    #[test]
    fn test_status_progress_does_not_block_branch_navigation() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: Some("/path".to_string()),
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: gwt_core::git::DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: true,
                worktree_path: Some("/path2".to_string()),
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: gwt_core::git::DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
        ];

        model.branch_list = BranchListState::new().with_branches(branches);
        model.branch_list.reset_status_progress(2);
        model.branch_list.increment_status_progress();
        model.pr_title_rx = None;
        model.safety_rx = None;
        model.worktree_status_rx = None;

        model.update(Message::SelectNext);
        assert_eq!(model.branch_list.selected, 1);
    }

    #[test]
    fn test_cleanup_allows_navigation_and_skips_target_branch() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let branches = vec![
            sample_branch_with_session("feature/a"),
            sample_branch_with_session("feature/b"),
            sample_branch_with_session("feature/c"),
        ];

        model.branch_list = BranchListState::new().with_branches(branches);
        model.branch_list.start_cleanup_progress(3);
        model
            .branch_list
            .set_cleanup_target_branches(&["feature/b".to_string()]);

        model.update(Message::SelectNext);
        assert_eq!(
            model
                .branch_list
                .selected_branch()
                .map(|branch| branch.name.as_str()),
            Some("feature/c")
        );
    }

    #[test]
    fn test_refresh_data_keeps_cleanup_state() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let branches = vec![
            sample_branch_with_session("feature/a"),
            sample_branch_with_session("feature/b"),
        ];

        model.branch_list = BranchListState::new().with_branches(branches);
        model.branch_list.start_cleanup_progress(2);
        model
            .branch_list
            .set_cleanup_target_branches(&["feature/b".to_string()]);
        model
            .branch_list
            .set_cleanup_active_branch(Some("feature/b".to_string()));
        model.branch_list.increment_cleanup_progress();

        model.update(Message::RefreshData);

        assert!(model.branch_list.cleanup_in_progress());
        assert_eq!(model.branch_list.cleanup_progress_total, 2);
        assert_eq!(model.branch_list.cleanup_progress_done, 1);
        assert_eq!(model.branch_list.cleanup_active_branch(), Some("feature/b"));
    }

    #[test]
    fn test_branch_list_update_preserves_cleanup_targets() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let branches = vec![
            sample_branch_with_session("feature/a"),
            sample_branch_with_session("feature/b"),
        ];

        model.branch_list = BranchListState::new().with_branches(branches);
        model.branch_list.start_cleanup_progress(2);
        model
            .branch_list
            .set_cleanup_target_branches(&["feature/b".to_string()]);
        model
            .branch_list
            .set_cleanup_active_branch(Some("feature/b".to_string()));
        model.branch_list.increment_cleanup_progress();

        let (tx, rx) = mpsc::channel();
        model.branch_list_rx = Some(rx);

        let update = BranchListUpdate {
            branches: vec![
                sample_branch_with_session("feature/a"),
                sample_branch_with_session("feature/b"),
            ],
            branch_names: Vec::new(),
            worktree_targets: Vec::new(),
            safety_targets: Vec::new(),
            base_branches: Vec::new(),
            base_branch: "main".to_string(),
            base_branch_exists: true,
            total_count: 2,
            active_count: 2,
        };
        tx.send(update).unwrap();

        model.apply_branch_list_updates();

        assert!(model.branch_list.cleanup_in_progress());
        assert_eq!(model.branch_list.cleanup_progress_total, 2);
        assert_eq!(model.branch_list.cleanup_progress_done, 1);
        assert_eq!(model.branch_list.cleanup_active_branch(), Some("feature/b"));

        let target_index = model
            .branch_list
            .filtered_indices
            .iter()
            .position(|&idx| model.branch_list.branches[idx].name == "feature/b")
            .expect("cleanup branch index");
        assert!(model.branch_list.is_cleanup_target_index(target_index));
    }

    #[test]
    fn test_session_file_is_quiet_threshold() {
        let now = SystemTime::now();
        let recent =
            now - Duration::from_secs(SESSION_SUMMARY_QUIET_PERIOD.as_secs().saturating_sub(1));
        assert!(!session_file_is_quiet(recent, now));

        let old = now - Duration::from_secs(SESSION_SUMMARY_QUIET_PERIOD.as_secs() + 1);
        assert!(session_file_is_quiet(old, now));
    }

    #[test]
    fn test_defer_poll_for_quiet_schedules_next_check() {
        let now = Instant::now();
        let deferred = defer_poll_for_quiet(now);
        let delta = now.duration_since(deferred);
        assert_eq!(delta, SESSION_POLL_INTERVAL - SESSION_SUMMARY_QUIET_PERIOD);
    }

    #[test]
    fn test_poll_session_summary_defers_when_inflight() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let item = sample_branch_with_session("feature/poll");
        model.branch_list = BranchListState::new().with_branches(vec![item]);
        model.branch_list.ai_enabled = true;
        model.branch_list.session_summary_enabled = true;

        let branch_name = model
            .branch_list
            .selected_branch()
            .map(|b| b.name.clone())
            .unwrap();
        model
            .branch_list
            .mark_session_summary_inflight(&branch_name);

        let previous = Instant::now() - SESSION_POLL_INTERVAL - Duration::from_secs(1);
        model.last_session_poll = Some(previous);

        model.poll_session_summary_if_needed();

        assert!(model.session_poll_deferred);
        assert_eq!(model.last_session_poll, Some(previous));
    }

    #[test]
    fn test_active_session_summary_enabled_prefers_profile() {
        let mut model = Model::new_with_context(None);
        let mut config = ProfilesConfig::default();
        let mut profile = Profile::new("dev");
        profile.ai = Some(AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            summary_enabled: false,
        });
        config.profiles.insert("dev".to_string(), profile);
        config.active = Some("dev".to_string());
        config.default_ai = Some(AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            summary_enabled: true,
        });
        model.profiles_config = config;

        assert!(!model.active_session_summary_enabled());

        if let Some(profile) = model.profiles_config.profiles.get_mut("dev") {
            if let Some(ai) = profile.ai.as_mut() {
                ai.summary_enabled = true;
            }
        }

        assert!(model.active_session_summary_enabled());
    }

    // FR-020: Tab cycles BranchList → AgentMode → Settings → BranchList
    #[test]
    fn test_tab_cycles_three_screens() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        // BranchList → AgentMode
        model.update(Message::Tab);
        assert!(matches!(model.screen, Screen::AgentMode));

        // AgentMode → Settings
        model.update(Message::Tab);
        assert!(matches!(model.screen, Screen::Settings));

        // Settings → BranchList
        model.update(Message::Tab);
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_tab_ignored_when_filter_mode() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        model.branch_list.enter_filter_mode();

        model.update(Message::Tab);
        assert!(matches!(model.screen, Screen::BranchList));
    }

    #[test]
    fn test_agent_mode_input_accepts_char_and_backspace() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::AgentMode;
        model.agent_mode.ai_ready = true;

        model.update(Message::Char('h'));
        model.update(Message::Char('i'));
        assert_eq!(model.agent_mode.input, "hi");

        model.update(Message::Backspace);
        assert_eq!(model.agent_mode.input, "h");
    }

    #[test]
    fn test_agent_mode_enter_opens_ai_wizard_when_disabled() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::AgentMode;
        model.agent_mode.ai_ready = false;

        model.update(Message::Enter);
        assert!(matches!(model.screen, Screen::AISettingsWizard));
    }

    #[test]
    fn test_branch_summary_update_ignores_non_selected_branch() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;

        let branches = vec![
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: Some("/path".to_string()),
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: gwt_core::git::DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
            BranchItem {
                name: "feature/two".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: true,
                worktree_path: Some("/path2".to_string()),
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: gwt_core::git::DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
        ];

        model.branch_list = BranchListState::new().with_branches(branches);
        model.branch_list.selected = 0;

        let mut existing = BranchSummary::new("feature/one");
        existing.errors.commits = Some("keep".to_string());
        model.branch_list.branch_summary = Some(existing);

        let (tx, rx) = mpsc::channel();
        model.branch_summary_rx = Some(rx);

        let mut ignored = BranchSummary::new("feature/two");
        ignored.errors.commits = Some("ignored".to_string());
        tx.send(BranchSummaryUpdate {
            branch: "feature/two".to_string(),
            summary: ignored,
        })
        .unwrap();

        model.apply_branch_summary_updates();

        let current = model.branch_list.branch_summary.as_ref().expect("summary");
        assert_eq!(current.branch_name, "feature/one");
        assert_eq!(current.errors.commits.as_deref(), Some("keep"));

        let (tx2, rx2) = mpsc::channel();
        model.branch_summary_rx = Some(rx2);

        let mut applied = BranchSummary::new("feature/one");
        applied.errors.commits = Some("applied".to_string());
        tx2.send(BranchSummaryUpdate {
            branch: "feature/one".to_string(),
            summary: applied,
        })
        .unwrap();

        model.apply_branch_summary_updates();
        let current = model.branch_list.branch_summary.as_ref().expect("summary");
        assert_eq!(current.errors.commits.as_deref(), Some("applied"));
    }

    #[test]
    fn test_mouse_single_click_selects_branch() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branches = [
            Branch::new("feature/one", "deadbeef"),
            Branch::new("feature/two", "deadbeef"),
        ];
        let items = branches
            .iter()
            .map(|branch| BranchItem::from_branch(branch, &[]))
            .collect();
        model.branch_list = BranchListState::new().with_branches(items);
        model.branch_list.update_list_area(Rect::new(0, 0, 20, 5));

        assert_eq!(model.branch_list.selected, 0);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        model.handle_branch_list_mouse(mouse);

        // Single click should select the branch (index 1, since row 2 maps to second item)
        assert_eq!(model.branch_list.selected, 1);
        // Wizard should NOT open on single click
        assert!(!model.wizard.visible);
    }

    #[test]
    fn test_mouse_double_click_opens_wizard() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branches = [
            Branch::new("feature/one", "deadbeef"),
            Branch::new("feature/two", "deadbeef"),
        ];
        let items = branches
            .iter()
            .map(|branch| BranchItem::from_branch(branch, &[]))
            .collect();
        model.branch_list = BranchListState::new().with_branches(items);
        model.branch_list.update_list_area(Rect::new(0, 0, 20, 5));

        assert_eq!(model.branch_list.selected, 0);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };

        // First click: select branch
        model.handle_branch_list_mouse(mouse);
        assert_eq!(model.branch_list.selected, 1);
        assert!(!model.wizard.visible);

        // Second click (double click): open wizard
        model.handle_branch_list_mouse(mouse);
        assert_eq!(model.branch_list.selected, 1);
        assert!(model.wizard.visible);
    }

    #[test]
    fn test_mouse_click_different_branch_resets_double_click() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branches = [
            Branch::new("feature/one", "deadbeef"),
            Branch::new("feature/two", "deadbeef"),
            Branch::new("feature/three", "deadbeef"),
        ];
        let items = branches
            .iter()
            .map(|branch| BranchItem::from_branch(branch, &[]))
            .collect();
        model.branch_list = BranchListState::new().with_branches(items);
        model.branch_list.update_list_area(Rect::new(0, 0, 20, 5));

        // Click on first branch (row 1)
        let mouse1 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        model.handle_branch_list_mouse(mouse1);
        assert_eq!(model.branch_list.selected, 0);

        // Click on different branch (row 2) - should NOT trigger double click
        let mouse2 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        model.handle_branch_list_mouse(mouse2);
        assert_eq!(model.branch_list.selected, 1);
        // Wizard should NOT open because it's a different branch
        assert!(!model.wizard.visible);
    }

    #[test]
    fn test_mouse_click_ignores_cleanup_target_branch() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::BranchList;
        let branches = [
            Branch::new("feature/one", "deadbeef"),
            Branch::new("feature/two", "deadbeef"),
        ];
        let items = branches
            .iter()
            .map(|branch| BranchItem::from_branch(branch, &[]))
            .collect();
        model.branch_list = BranchListState::new().with_branches(items);
        model.branch_list.update_list_area(Rect::new(0, 0, 20, 5));
        model.branch_list.start_cleanup_progress(2);
        model
            .branch_list
            .set_cleanup_target_branches(&["feature/two".to_string()]);

        assert_eq!(model.branch_list.selected, 0);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        model.handle_branch_list_mouse(mouse);

        assert_eq!(model.branch_list.selected, 0);
        assert!(!model.wizard.visible);
    }

    #[test]
    fn test_service_select_footer_keybinds() {
        let mut model = Model::new_with_context(None);
        model.screen = Screen::ServiceSelect;
        let keybinds = model.get_footer_keybinds();
        assert!(keybinds.contains("Skip"));
        assert!(keybinds.contains("Cancel"));
    }

    #[test]
    fn test_should_prompt_recreate() {
        assert!(!Model::should_prompt_recreate(&ContainerStatus::Running));
        assert!(Model::should_prompt_recreate(&ContainerStatus::Stopped));
        assert!(!Model::should_prompt_recreate(&ContainerStatus::NotFound));
    }
}
