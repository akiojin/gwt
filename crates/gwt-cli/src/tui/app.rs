//! TUI Application with Elm Architecture

#![allow(dead_code)] // TUI application components for future expansion

use crate::{prepare_launch_plan, InstallPlan, LaunchPlan, LaunchProgress};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::ai::{
    summarize_session, AIClient, AIError, AgentType, ClaudeSessionParser, CodexSessionParser,
    GeminiSessionParser, OpenCodeSessionParser, SessionParseError, SessionParser,
};
use gwt_core::config::get_branch_tool_history;
use gwt_core::config::{
    get_claude_settings_path, is_gwt_hooks_registered, register_gwt_hooks, save_session_entry,
    AISettings, Profile, ProfilesConfig, ResolvedAISettings, ToolSessionEntry,
};
use gwt_core::error::GwtError;
use gwt_core::git::{Branch, PrCache, Remote, Repository};
use gwt_core::tmux::{
    break_pane, compute_equal_splits, get_current_session, group_panes_by_left,
    join_pane_to_target, kill_pane, launcher, list_pane_geometries, resize_pane_height,
    resize_pane_width, AgentPane, PaneColumn, PaneGeometry, SplitDirection,
};
use gwt_core::worktree::WorktreeManager;
use gwt_core::TmuxMode;
use ratatui::{prelude::*, widgets::*};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, error, info};

const BRANCH_LIST_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);

use super::screens::branch_list::{PrInfo, WorktreeStatus};
use super::screens::environment::EditField;
use super::screens::pane_list::PaneListState;
use super::screens::split_layout::{calculate_split_layout, SplitLayoutState};
use super::screens::{
    collect_os_env, render_ai_wizard, render_branch_list, render_confirm, render_environment,
    render_error, render_help, render_logs, render_profiles, render_settings, render_wizard,
    render_worktree_create, AIWizardState, BranchItem, BranchListState, BranchType, CodingAgent,
    ConfirmState, DetailPanelTab, EnvironmentState, ErrorState, ExecutionMode, HelpState, LogsState,
    ProfilesState, QuickStartEntry, ReasoningLevel, SettingsState, WizardConfirmResult, WizardState,
    WorktreeCreateState,
};

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

fn format_ai_error(err: AIError) -> String {
    err.to_string()
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
    /// Worktree path where agent should run
    pub worktree_path: PathBuf,
    /// Branch name
    pub branch_name: String,
    /// Coding agent to launch
    pub agent: CodingAgent,
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
}

#[derive(Debug, Clone)]
pub struct TuiEntryContext {
    status_message: Option<String>,
    error_message: Option<String>,
}

impl TuiEntryContext {
    pub fn success(message: String) -> Self {
        Self {
            status_message: Some(message),
            error_message: None,
        }
    }

    pub fn warning(message: String) -> Self {
        Self {
            status_message: Some(message),
            error_message: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            status_message: None,
            error_message: Some(message),
        }
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
    mtime: Option<std::time::SystemTime>,
    missing: bool,
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

struct LaunchRequest {
    branch_name: String,
    create_new_branch: bool,
    base_branch: Option<String>,
    agent: CodingAgent,
    model: Option<String>,
    reasoning_level: Option<ReasoningLevel>,
    version: String,
    execution_mode: ExecutionMode,
    session_id: Option<String>,
    skip_permissions: bool,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    auto_install_deps: bool,
}

enum LaunchUpdate {
    Progress(LaunchProgress),
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
    /// Confirm dialog state
    confirm: ConfirmState,
    /// Error display state
    error: ErrorState,
    /// Profiles state
    profiles: ProfilesState,
    /// Profiles configuration
    profiles_config: ProfilesConfig,
    /// Environment variables state
    environment: EnvironmentState,
    /// Wizard popup state
    wizard: WizardState,
    /// AI settings wizard state (FR-100)
    ai_wizard: AIWizardState,
    /// Status message
    status_message: Option<String>,
    /// Status message timestamp (for auto-clear)
    status_message_time: Option<Instant>,
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
    /// Branch list update receiver
    branch_list_rx: Option<Receiver<BranchListUpdate>>,
    /// PR title update receiver
    pr_title_rx: Option<Receiver<PrTitleUpdate>>,
    /// Safety check update receiver
    safety_rx: Option<Receiver<SafetyUpdate>>,
    /// Worktree status update receiver
    worktree_status_rx: Option<Receiver<WorktreeStatusUpdate>>,
    /// Session summary update sender
    session_summary_tx: Option<Sender<SessionSummaryUpdate>>,
    /// Session summary update receiver
    session_summary_rx: Option<Receiver<SessionSummaryUpdate>>,
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
    /// Last time session polling ran (30s interval)
    last_session_poll: Option<Instant>,
}

/// Screen types
#[derive(Clone, Debug)]
pub enum Screen {
    BranchList,
    WorktreeCreate,
    Settings,
    Logs,
    Help,
    Confirm,
    Error,
    Profiles,
    Environment,
    /// AI settings wizard (FR-100)
    AISettingsWizard,
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
    /// Repair worktrees (x key)
    RepairWorktrees,
    /// Copy selected log to clipboard
    CopyLogToClipboard,
    /// FR-095: Hide active agent pane (ESC key in branch list)
    HideActiveAgentPane,
    /// FR-040: Confirm agent termination (d key)
    ConfirmAgentTermination,
    /// Execute agent termination after confirmation
    ExecuteAgentTermination,
}

impl Model {
    /// Create a new model
    pub fn new() -> Self {
        Self::new_with_context(None)
    }

    pub fn new_with_context(context: Option<TuiEntryContext>) -> Self {
        let repo_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        debug!(
            category = "tui",
            repo_root = %repo_root.display(),
            "Initializing TUI model"
        );

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
            confirm: ConfirmState::new(),
            error: ErrorState::new(),
            profiles: ProfilesState::new(),
            profiles_config: ProfilesConfig::default(),
            environment: EnvironmentState::new(),
            wizard: WizardState::new(),
            ai_wizard: AIWizardState::new(),
            status_message: None,
            status_message_time: None,
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
            branch_list_rx: None,
            pr_title_rx: None,
            safety_rx: None,
            worktree_status_rx: None,
            session_summary_tx: None,
            session_summary_rx: None,
            tmux_mode: TmuxMode::detect(),
            tmux_session: None,
            gwt_pane_id: None,
            agent_panes: Vec::new(),
            pane_list: PaneListState::new(),
            split_layout: SplitLayoutState::new(),
            last_pane_update: None,
            last_spinner_update: None,
            last_session_poll: None,
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

        // SPEC-861d8cdf T-104: Check if hook setup is needed on first startup
        if model.tmux_mode.is_multi() {
            if let Some(settings_path) = get_claude_settings_path() {
                if !is_gwt_hooks_registered(&settings_path) {
                    model.pending_hook_setup = true;
                    model.confirm = ConfirmState::hook_setup();
                    model.screen_stack.push(model.screen.clone());
                    model.screen = Screen::Confirm;
                }
            }
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
        self.start_branch_list_refresh(settings);
    }

    fn start_branch_list_refresh(&mut self, settings: gwt_core::config::Settings) {
        self.pr_title_rx = None;
        self.safety_rx = None;
        self.worktree_status_rx = None;
        self.branch_list_rx = None;
        self.total_count = 0;
        self.active_count = 0;

        let mut branch_list = BranchListState::new();
        branch_list.active_profile = self.profiles_config.active.clone();
        branch_list.ai_enabled = self.active_ai_enabled();
        branch_list.working_directory = Some(self.repo_root.display().to_string());
        branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
        branch_list.set_loading(true);
        self.branch_list = branch_list;

        let repo_root = self.repo_root.clone();
        let base_branch = settings.default_base_branch.clone();
        let (tx, rx) = mpsc::channel();
        self.branch_list_rx = Some(rx);

        thread::spawn(move || {
            let base_branch_exists = Branch::exists(&repo_root, &base_branch).unwrap_or(false);
            let worktrees = WorktreeManager::new(&repo_root)
                .ok()
                .and_then(|manager| manager.list_basic().ok())
                .unwrap_or_default();
            let branches = Branch::list_basic(&repo_root).unwrap_or_default();
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

                    // Set tool usage from TypeScript session history (FR-070)
                    if let Some(entry) = tool_usage_map.get(&b.name) {
                        item.last_tool_usage = Some(entry.format_tool_usage());
                        item.last_tool_id = Some(entry.tool_id.clone());
                        item.last_session_id = entry.session_id.clone();
                        // FR-041: Compare git commit timestamp and session timestamp,
                        // use the newer one
                        let session_timestamp = entry.timestamp / 1000; // Convert ms to seconds
                        let git_timestamp = item.last_commit_timestamp.unwrap_or(0);
                        item.last_commit_timestamp = Some(session_timestamp.max(git_timestamp));
                    }

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

            // Sort branches by timestamp for those with sessions
            branch_items.iter_mut().for_each(|item| {
                if item.last_commit_timestamp.is_none() {
                    // Try to get timestamp from git (fallback)
                    // For now, leave as None - the sort will handle it
                }
            });

            let total_count = branch_items.len();
            let active_count = branch_items.iter().filter(|b| b.has_worktree).count();
            let base_branches: Vec<String> = branches
                .iter()
                .filter(|b| !b.name.starts_with("remotes/"))
                .map(|b| b.name.clone())
                .collect();
            let branch_names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();

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

        let repo_root = self.repo_root.clone();
        let (tx, rx) = mpsc::channel();
        self.pr_title_rx = Some(rx);

        thread::spawn(move || {
            let mut cache = PrCache::new();
            cache.populate(&repo_root);

            let mut info = HashMap::new();
            for name in branch_names {
                if let Some(pr) = cache.get(&name) {
                    info.insert(
                        name,
                        PrInfo {
                            title: pr.title.clone(),
                            number: pr.number,
                            url: pr.url.clone(),
                        },
                    );
                }
            }

            let _ = tx.send(PrTitleUpdate { info });
        });
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

        let repo_root = self.repo_root.clone();
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
        self.branch_list.update_branch_summary(&self.repo_root);
        if self.branch_list.detail_panel_tab == DetailPanelTab::Session {
            self.maybe_request_session_summary_for_selected(false);
        }
    }

    fn maybe_request_session_summary_for_selected(&mut self, force: bool) {
        if !self.branch_list.ai_enabled {
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

    fn persist_detected_session(&self, branch: &BranchItem, tool_id: &str, session_id: &str) {
        if branch.worktree_path.is_none() {
            return;
        }
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
            tool_version: None,
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
                            mtime,
                            missing: false,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(SessionSummaryUpdate {
                            branch: task.branch,
                            session_id: task.session_id,
                            summary: None,
                            error: Some(format_ai_error(err)),
                            mtime,
                            missing: false,
                        });
                    }
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
                    } else if let Some(error) = update.error {
                        self.branch_list.apply_session_error(&update.branch, error);
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

    fn poll_session_summary_if_needed(&mut self) {
        if self.branch_list.detail_panel_tab != DetailPanelTab::Session {
            self.last_session_poll = None;
            return;
        }

        if !self.branch_list.ai_enabled {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_session_poll {
            if now.duration_since(last) < Duration::from_secs(30) {
                return;
            }
        }
        self.last_session_poll = Some(now);

        let (branch_name, session_id, tool_id) = match self.branch_list.selected_branch() {
            Some(branch) => (
                branch.name.clone(),
                branch.last_session_id.clone(),
                branch.last_tool_id.clone(),
            ),
            None => return,
        };

        let Some(session_id) = session_id else {
            self.branch_list.mark_session_missing(&branch_name);
            return;
        };
        let Some(tool_id) = tool_id else {
            self.branch_list.mark_session_missing(&branch_name);
            return;
        };

        if self.branch_list.session_summary_inflight(&branch_name) {
            return;
        }

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
        let worktrees: Vec<(String, std::path::PathBuf)> =
            match WorktreeManager::new(&self.repo_root) {
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
                self.error = ErrorState::from_error(&message);
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
                let session_cache = self.branch_list.clone_session_cache();
                let session_inflight = self.branch_list.clone_session_inflight();
                let session_missing = self.branch_list.clone_session_missing();
                let detail_tab = self.branch_list.detail_panel_tab;
                let mut branch_list = BranchListState::new().with_branches(update.branches);
                branch_list.detail_panel_tab = detail_tab;
                branch_list.active_profile = self.profiles_config.active.clone();
                branch_list.ai_enabled = self.active_ai_enabled();
                branch_list.set_session_cache(session_cache);
                branch_list.set_session_inflight(session_inflight);
                branch_list.set_session_missing(session_missing);
                branch_list.set_repo_web_url(self.branch_list.repo_web_url().cloned());
                branch_list.working_directory = Some(self.repo_root.display().to_string());
                branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
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
                self.pr_title_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pr_title_rx = None;
            }
        }
    }

    fn apply_launch_updates(&mut self) {
        let Some(rx) = &self.launch_rx else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => match update {
                    LaunchUpdate::Progress(progress) => {
                        self.launch_status = Some(progress.message());
                    }
                    LaunchUpdate::Ready(plan) => {
                        self.launch_in_progress = false;
                        self.launch_rx = None;
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
        let window_name = format!(
            "gwt-agent-{}",
            pane.branch_name.replace('/', "-").replace(' ', "_")
        );

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

        if let Some(url) = self.branch_list.link_at_point(mouse.column, mouse.row) {
            self.open_url(&url);
            return;
        }

        let Some(index) = self
            .branch_list
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
            if self.branch_list.select_index(index) {
                self.refresh_branch_summary();
            }
            self.handle_branch_enter();
        } else {
            self.last_mouse_click = Some(MouseClick { index, at: now });
        }
    }

    fn open_url(&mut self, url: &str) {
        if url.trim().is_empty() {
            self.status_message = Some("Failed to open URL: empty URL".to_string());
            self.status_message_time = Some(Instant::now());
            return;
        }

        let mut last_error: Option<std::io::Error> = None;
        #[cfg(target_os = "macos")]
        {
            match std::process::Command::new("open").arg(url).spawn() {
                Ok(_) => return,
                Err(err) => last_error = Some(err),
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
                match std::process::Command::new(cmd).args(args).spawn() {
                    Ok(_) => return,
                    Err(err) => last_error = Some(err),
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            match std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
            {
                Ok(_) => return,
                Err(err) => last_error = Some(err),
            }

            match std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", "Start-Process", url])
                .spawn()
            {
                Ok(_) => return,
                Err(err) => last_error = Some(err),
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

        let err = last_error.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::Other));
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
        let (vars, disabled_keys, ai_enabled, ai_endpoint, ai_api_key, ai_model) = self
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
                let (ai_enabled, ai_endpoint, ai_api_key, ai_model) = match &profile.ai {
                    Some(ai) => (
                        true,
                        ai.endpoint.clone(),
                        ai.api_key.clone(),
                        ai.model.clone(),
                    ),
                    None => {
                        let defaults = AISettings::default();
                        (false, defaults.endpoint, String::new(), defaults.model)
                    }
                };
                (
                    items,
                    profile.disabled_env.clone(),
                    ai_enabled,
                    ai_endpoint,
                    ai_api_key,
                    ai_model,
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
                )
            });

        self.environment = EnvironmentState::new()
            .with_profile(profile_name)
            .with_variables(vars)
            .with_disabled_keys(disabled_keys)
            .with_ai_settings(ai_enabled, ai_endpoint, ai_api_key, ai_model)
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
                );
            } else {
                // Create new settings
                self.ai_wizard.open_new(false, Some(profile_name.to_string()));
            }
            self.screen_stack.push(self.screen.clone());
            self.screen = Screen::AISettingsWizard;
        }
    }

    /// Handle Enter key in AI settings wizard
    fn handle_ai_wizard_enter(&mut self) {
        use super::screens::ai_wizard::AIWizardStep;

        match self.ai_wizard.step {
            AIWizardStep::Endpoint => {
                self.ai_wizard.next_step();
            }
            AIWizardStep::ApiKey => {
                // Start fetching models
                self.ai_wizard.step = AIWizardStep::FetchingModels;
                self.ai_wizard.loading_message = Some("Fetching models...".to_string());

                // Fetch models (blocking)
                match self.ai_wizard.fetch_models() {
                    Ok(()) => {
                        self.ai_wizard.fetch_complete();
                    }
                    Err(e) => {
                        self.ai_wizard.fetch_failed(&e);
                    }
                }
            }
            AIWizardStep::FetchingModels => {
                // Do nothing while fetching
            }
            AIWizardStep::ModelSelect => {
                // Save AI settings
                self.save_ai_wizard_settings();
                self.ai_wizard.close();
                if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                }
                self.load_profiles();
            }
        }
    }

    /// Save AI settings from wizard
    fn save_ai_wizard_settings(&mut self) {
        let model = self
            .ai_wizard
            .current_model()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let settings = AISettings {
            endpoint: self.ai_wizard.endpoint.trim().to_string(),
            api_key: self.ai_wizard.api_key.trim().to_string(),
            model,
        };

        if self.ai_wizard.is_default_ai {
            self.profiles_config.default_ai = Some(settings);
        } else if let Some(profile_name) = &self.ai_wizard.profile_name {
            if let Some(profile) = self.profiles_config.profiles.get_mut(profile_name) {
                profile.ai = Some(settings);
            }
        }
        self.save_profiles();
    }

    /// Delete AI settings from wizard
    fn delete_ai_wizard_settings(&mut self) {
        if self.ai_wizard.is_default_ai {
            self.profiles_config.default_ai = None;
        } else if let Some(profile_name) = &self.ai_wizard.profile_name {
            if let Some(profile) = self.profiles_config.profiles.get_mut(profile_name) {
                profile.ai = None;
            }
        }
        self.save_profiles();
        self.ai_wizard.close();
        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }
        self.load_profiles();
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
                });
            }
        } else {
            profile.ai = None;
        }
        self.save_profiles();
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
                self.screen_stack.push(self.screen.clone());
                self.screen = screen;
                self.status_message = None;
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
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029d: Cancel confirm dialog without executing action
                    self.pending_unsafe_selection = None;
                    self.pending_cleanup_branches.clear();
                    self.pending_hook_setup = false;
                    if let Some(prev_screen) = self.screen_stack.pop() {
                        self.screen = prev_screen;
                    }
                } else if matches!(self.screen, Screen::AISettingsWizard) {
                    // Go back in AI wizard or close if at first step
                    if self.ai_wizard.show_delete_confirm {
                        self.ai_wizard.cancel_delete();
                    } else {
                        self.ai_wizard.prev_step();
                        if !self.ai_wizard.visible {
                            // Wizard was closed
                            if let Some(prev_screen) = self.screen_stack.pop() {
                                self.screen = prev_screen;
                            }
                        }
                    }
                } else if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                }
                self.status_message = None;
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
                self.apply_branch_list_updates();
                self.apply_pr_title_updates();
                self.apply_safety_updates();
                self.apply_worktree_updates();
                self.apply_launch_updates();
                self.apply_session_summary_updates();
                self.poll_session_summary_if_needed();
                // FR-033: Update pane list every 1 second in tmux multi mode
                self.update_pane_list();
            }
            Message::SelectNext => match self.screen {
                Screen::BranchList => {
                    self.branch_list.select_next();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::WorktreeCreate => self.worktree_create.select_next_base(),
                Screen::Settings => self.settings.select_next(),
                Screen::Logs => self.logs.select_next(),
                Screen::Help => self.help.scroll_down(),
                Screen::Error => self.error.scroll_down(),
                Screen::Profiles => self.profiles.select_next(),
                Screen::Environment => self.environment.select_next(),
                Screen::AISettingsWizard => self.ai_wizard.select_next_model(),
                Screen::Confirm => {}
            },
            Message::SelectPrev => match self.screen {
                Screen::BranchList => {
                    self.branch_list.select_prev();
                    // SPEC-4b893dae: Update branch summary on selection change
                    self.refresh_branch_summary();
                }
                Screen::WorktreeCreate => self.worktree_create.select_prev_base(),
                Screen::Settings => self.settings.select_prev(),
                Screen::Logs => self.logs.select_prev(),
                Screen::Help => self.help.scroll_up(),
                Screen::Error => self.error.scroll_up(),
                Screen::Profiles => self.profiles.select_prev(),
                Screen::Environment => self.environment.select_prev(),
                Screen::AISettingsWizard => self.ai_wizard.select_prev_model(),
                Screen::Confirm => {}
            },
            Message::PageUp => match self.screen {
                Screen::BranchList => {
                    if self.branch_list.detail_panel_tab == DetailPanelTab::Session {
                        self.branch_list.scroll_session_page_up();
                    } else {
                        self.branch_list.page_up(10);
                        // SPEC-4b893dae: Update branch summary on selection change
                        self.refresh_branch_summary();
                    }
                }
                Screen::Logs => self.logs.page_up(10),
                Screen::Help => self.help.page_up(),
                Screen::Environment => self.environment.page_up(),
                _ => {}
            },
            Message::PageDown => match self.screen {
                Screen::BranchList => {
                    if self.branch_list.detail_panel_tab == DetailPanelTab::Session {
                        self.branch_list.scroll_session_page_down();
                    } else {
                        self.branch_list.page_down(10);
                        // SPEC-4b893dae: Update branch summary on selection change
                        self.refresh_branch_summary();
                    }
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
                Screen::WorktreeCreate => {
                    if self.worktree_create.is_confirm_step() {
                        // Create the worktree
                        self.create_worktree();
                    } else {
                        self.worktree_create.next_step();
                    }
                }
                Screen::Confirm => {
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
                    }
                    // Clear pending state and return to previous screen
                    self.pending_unsafe_selection = None;
                    self.pending_agent_termination = None;
                    self.pending_cleanup_branches.clear();
                    self.pending_hook_setup = false;
                    if let Some(prev_screen) = self.screen_stack.pop() {
                        self.screen = prev_screen;
                    }
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
                    self.update(Message::NavigateBack);
                }
                Screen::Logs => {
                    self.logs.toggle_detail();
                }
                Screen::AISettingsWizard => {
                    self.handle_ai_wizard_enter();
                }
                _ => {}
            },
            Message::Char(c) => {
                if matches!(self.screen, Screen::WorktreeCreate) {
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
                } else if matches!(self.screen, Screen::AISettingsWizard) {
                    if self.ai_wizard.show_delete_confirm {
                        // Handle delete confirmation
                        if c == 'y' || c == 'Y' {
                            self.delete_ai_wizard_settings();
                        } else if c == 'n' || c == 'N' {
                            self.ai_wizard.cancel_delete();
                        }
                    } else if c == 'd' || c == 'D' {
                        // Show delete confirmation (only in edit mode)
                        if self.ai_wizard.is_edit {
                            self.ai_wizard.show_delete();
                        }
                    } else if self.ai_wizard.is_text_input() {
                        self.ai_wizard.insert_char(c);
                    }
                }
            }
            Message::Backspace => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.delete_char();
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode
                {
                    self.branch_list.filter_pop();
                    self.refresh_branch_summary();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.delete_char();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.delete_char();
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Log search mode - delete character
                    self.logs.search.pop();
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
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029c: Left/Right toggle selection in confirm dialog
                    self.confirm.toggle_selection();
                } else if matches!(self.screen, Screen::AISettingsWizard)
                    && self.ai_wizard.is_text_input()
                {
                    self.ai_wizard.cursor_left();
                }
            }
            Message::CursorRight => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_right();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.cursor_right();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.cursor_right();
                } else if matches!(self.screen, Screen::Confirm) {
                    // FR-029c: Left/Right toggle selection in confirm dialog
                    self.confirm.toggle_selection();
                } else if matches!(self.screen, Screen::AISettingsWizard)
                    && self.ai_wizard.is_text_input()
                {
                    self.ai_wizard.cursor_right();
                }
            }
            Message::RefreshData => {
                self.refresh_data();
            }
            Message::RepairWorktrees => {
                // Run git worktree repair
                match WorktreeManager::new(&self.repo_root) {
                    Ok(manager) => match manager.repair() {
                        Ok(()) => {
                            self.status_message =
                                Some("Worktrees repaired successfully".to_string());
                            // Refresh data after repair
                            self.refresh_data();
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Repair failed: {}", e));
                        }
                    },
                    Err(e) => {
                        self.status_message = Some(format!("Failed to open repository: {}", e));
                    }
                }
                self.status_message_time = Some(Instant::now());
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
            Message::Tab => match self.screen {
                Screen::Settings => self.settings.next_category(),
                Screen::BranchList => {
                    if !self.branch_list.filter_mode {
                        self.branch_list.detail_panel_tab.toggle();
                        self.refresh_branch_summary();
                        if self.branch_list.detail_panel_tab == DetailPanelTab::Session {
                            self.maybe_request_session_summary_for_selected(false);
                        } else {
                            self.last_session_poll = None;
                        }
                    }
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
                        WizardConfirmResult::Advance => {}
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

        let auto_install_deps = self
            .settings
            .settings
            .as_ref()
            .map(|settings| settings.agent.auto_install_deps)
            .unwrap_or(false);

        let request = LaunchRequest {
            branch_name: branch,
            create_new_branch: self.worktree_create.create_new_branch,
            base_branch: base,
            agent: self.wizard.agent,
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
            env: self.active_env_overrides(),
            env_remove: self.active_env_removals(),
            auto_install_deps,
        };

        self.start_launch_preparation(request);
    }

    fn start_launch_preparation(&mut self, request: LaunchRequest) {
        if self.launch_in_progress {
            return;
        }

        self.launch_in_progress = true;
        self.launch_status = Some(LaunchProgress::ResolvingWorktree.message());

        let repo_root = self.repo_root.clone();
        let (tx, rx) = mpsc::channel();
        self.launch_rx = Some(rx);

        thread::spawn(move || {
            let send = |update: LaunchUpdate| {
                let _ = tx.send(update);
            };

            send(LaunchUpdate::Progress(LaunchProgress::ResolvingWorktree));

            let manager = match WorktreeManager::new(&repo_root) {
                Ok(manager) => manager,
                Err(e) => {
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };

            let existing_wt = manager.get_by_branch(&request.branch_name).ok().flatten();
            let result = if let Some(wt) = existing_wt {
                Ok(wt)
            } else if request.create_new_branch {
                manager.create_new_branch(&request.branch_name, request.base_branch.as_deref())
            } else {
                manager.create_for_branch(&request.branch_name)
            };

            let worktree = match result {
                Ok(wt) => wt,
                Err(e) => {
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };

            let config = AgentLaunchConfig {
                worktree_path: worktree.path.clone(),
                branch_name: request.branch_name.clone(),
                agent: request.agent,
                model: request.model.clone(),
                reasoning_level: request.reasoning_level,
                version: request.version.clone(),
                execution_mode: request.execution_mode,
                session_id: request.session_id.clone(),
                skip_permissions: request.skip_permissions,
                env: request.env.clone(),
                env_remove: request.env_remove.clone(),
                auto_install_deps: request.auto_install_deps,
            };

            let plan = match prepare_launch_plan(config, |progress| {
                send(LaunchUpdate::Progress(progress))
            }) {
                Ok(plan) => plan,
                Err(e) => {
                    send(LaunchUpdate::Failed(e.to_string()));
                    return;
                }
            };

            send(LaunchUpdate::Ready(Box::new(plan)));
        });
    }

    fn handle_launch_plan(&mut self, plan: LaunchPlan) {
        // Refresh data to reflect branch/worktree changes (FR-008b)
        self.refresh_data();

        if let InstallPlan::Skip { message } = &plan.install_plan {
            self.status_message = Some(message.clone());
            self.status_message_time = Some(Instant::now());
        }

        let keep_launch_status = matches!(plan.install_plan, InstallPlan::Install { .. });

        if self.tmux_mode.is_multi() && self.gwt_pane_id.is_some() {
            match self.launch_plan_in_pane(&plan) {
                Ok(_) => {
                    if !keep_launch_status {
                        self.launch_status = None;
                    }
                    self.status_message = Some(format!(
                        "Agent launched in tmux pane for {}",
                        plan.config.branch_name
                    ));
                    self.status_message_time = Some(Instant::now());
                    self.wizard.visible = false;
                    self.screen = Screen::BranchList;
                }
                Err(e) => {
                    self.launch_status = None;
                    self.status_message = Some(format!("Failed to launch: {}", e));
                    self.status_message_time = Some(Instant::now());
                }
            }
        } else {
            self.pending_agent_launch = Some(plan);
            self.should_quit = true;
        }
    }

    /// Launch an agent in a tmux pane (multi mode)
    ///
    /// Layout strategy:
    /// - gwt is left column, agents are placed in right columns
    /// - Each agent column stacks up to 3 panes (vertical split)
    /// - When a column reaches 3 panes, a new column is added to the right
    ///
    /// Uses the same argument building logic as single mode (main.rs)
    fn launch_plan_in_pane(&mut self, plan: &LaunchPlan) -> Result<String, String> {
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

        // Build environment variables (same as single mode)
        let env_vars = plan.env.clone();

        let install_cmd = match &plan.install_plan {
            InstallPlan::Install { manager } => {
                let args = vec!["install".to_string()];
                Some(build_shell_command(manager, &args))
            }
            _ => None,
        };

        let agent_cmd = build_shell_command(&plan.executable, &plan.command_args);
        let full_cmd = if let Some(install_cmd) = install_cmd {
            format!("{} && {}", install_cmd, agent_cmd)
        } else {
            agent_cmd
        };

        // Build the full command string
        let command = build_tmux_command(&env_vars, &plan.config.env_remove, &full_cmd);

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
        let pane_id =
            launcher::launch_in_pane(target, &working_dir, &command).map_err(|e| e.to_string())?;

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
        debug!(
            category = "tui",
            branch_count = branches.len(),
            "Starting branch cleanup"
        );
        let mut deleted = 0;
        let mut errors = Vec::new();
        let manager = WorktreeManager::new(&self.repo_root).ok();

        for branch_name in branches {
            let branch_item = self
                .branch_list
                .branches
                .iter()
                .find(|b| &b.name == branch_name);

            if let (Some(manager), Some(item)) = (manager.as_ref(), branch_item) {
                if let Some(path) = item.worktree_path.as_deref() {
                    let force_remove = item.worktree_status == WorktreeStatus::Inaccessible
                        || item.has_changes
                        || WorktreeManager::is_protected(&item.name);
                    let path_buf = PathBuf::from(path);
                    if let Err(e) = manager.remove(&path_buf, force_remove) {
                        errors.push(format!("{}: {}", branch_name, e));
                        continue;
                    }
                }
            }

            // Try to delete the branch
            match Branch::delete(&self.repo_root, branch_name, true) {
                Ok(_) => {
                    debug!(
                        category = "tui",
                        branch = %branch_name,
                        "Branch deleted successfully"
                    );
                    deleted += 1;
                    // Remove from selection
                    self.branch_list.selected_branches.remove(branch_name);
                }
                Err(e) => {
                    error!(
                        category = "tui",
                        branch = %branch_name,
                        error = %e,
                        "Failed to delete branch"
                    );
                    errors.push(format!("{}: {}", branch_name, e));
                }
            }
        }

        // Show result message
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
        let base_screen = if matches!(self.screen, Screen::Confirm) {
            self.screen_stack
                .last()
                .cloned()
                .unwrap_or(Screen::BranchList)
        } else {
            self.screen.clone()
        };

        // Profiles, Environment, and Logs screens don't need header
        let needs_header = !matches!(
            base_screen,
            Screen::Profiles | Screen::Environment | Screen::Logs | Screen::AISettingsWizard
        );
        let header_height = if needs_header { 6 } else { 0 };

        // BranchList screen doesn't need footer (shortcut legend removed)
        let needs_footer = !matches!(base_screen, Screen::BranchList);
        let footer_height = if needs_footer {
            // Calculate footer height dynamically based on text length
            let keybinds = self.get_footer_keybinds();
            let status = self.active_status_message().unwrap_or("");
            let footer_text_len = if status.is_empty() {
                keybinds.len() + 2 // " {} " format adds 2 spaces
            } else {
                keybinds.len() + status.len() + 5 // " {} | {} " format adds 5 chars
            };
            let inner_width = frame.area().width.saturating_sub(2) as usize; // borders
            if footer_text_len > inner_width {
                4
            } else {
                3
            }
        } else {
            0
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height), // Header (0 for Profiles/Environment)
                Constraint::Min(0),                // Content
                Constraint::Length(footer_height), // Footer (0 for BranchList)
            ])
            .split(frame.area());

        // Header (for branch list screen, render boxed header)
        if needs_header {
            if matches!(base_screen, Screen::BranchList) {
                self.view_boxed_header(frame, chunks[0]);
            } else {
                self.view_header(frame, chunks[0]);
            }
        }

        // Content
        match base_screen {
            Screen::BranchList => {
                // Use split layout (branch list takes full area, PaneList abolished)
                let split_areas = calculate_split_layout(chunks[1], &self.split_layout);
                let status_message = self
                    .active_status_message()
                    .map(|message| message.to_string());

                // Render branch list (always has focus now)
                render_branch_list(
                    &mut self.branch_list,
                    frame,
                    split_areas.branch_list,
                    status_message.as_deref(),
                    true, // Branch list always has focus
                );
            }
            Screen::WorktreeCreate => {
                render_worktree_create(&self.worktree_create, frame, chunks[1])
            }
            Screen::Settings => render_settings(&self.settings, frame, chunks[1]),
            Screen::Logs => render_logs(&self.logs, frame, chunks[1]),
            Screen::Help => render_help(&self.help, frame, chunks[1]),
            Screen::Error => render_error(&self.error, frame, chunks[1]),
            Screen::Profiles => render_profiles(&self.profiles, frame, chunks[1]),
            Screen::Environment => render_environment(&mut self.environment, frame, chunks[1]),
            Screen::AISettingsWizard => render_ai_wizard(&self.ai_wizard, frame, chunks[1]),
            Screen::Confirm => {}
        }

        if matches!(self.screen, Screen::Confirm) {
            render_confirm(&self.confirm, frame, chunks[1]);
        }

        // Footer (not for BranchList screen)
        if needs_footer {
            self.view_footer(frame, chunks[2]);
        }

        // Wizard overlay (FR-044: popup on top of branch list)
        if self.wizard.visible {
            render_wizard(&self.wizard, frame, frame.area());
        }
    }

    /// Boxed header for branch list screen
    fn view_boxed_header(&self, frame: &mut Frame, area: Rect) {
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

        // Title for the box
        let title = format!(" gwt - Branch Selection v{}{} ", version, offline_indicator);
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

        // Line 1: Working Directory
        let working_dir_line = Line::from(vec![
            Span::raw(" "),
            Span::styled("Working Directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(working_dir),
        ]);
        frame.render_widget(Paragraph::new(working_dir_line), inner_chunks[0]);

        // Line 2: Profile
        let profile_line = Line::from(vec![
            Span::raw(" "),
            Span::styled("Profile(p): ", Style::default().fg(Color::DarkGray)),
            Span::raw(profile),
        ]);
        frame.render_widget(Paragraph::new(profile_line), inner_chunks[1]);

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
        ];
        frame.render_widget(Paragraph::new(Line::from(mode_spans)), inner_chunks[3]);
    }

    fn view_header(&self, frame: &mut Frame, area: Rect) {
        let version = env!("CARGO_PKG_VERSION");
        let offline_indicator = if self.is_offline { " [OFFLINE]" } else { "" };

        let profile = self
            .branch_list
            .active_profile
            .as_deref()
            .unwrap_or("default");

        // Match TypeScript format: gwt - Branch Selection v{version} | Profile(p): {name}
        let title = format!(
            " gwt - Branch Selection v{} | Profile(p): {} {}",
            version, profile, offline_indicator
        );
        let header = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        frame.render_widget(header, area);
    }

    fn get_footer_keybinds(&self) -> &'static str {
        match self.screen {
            Screen::BranchList => {
                if self.branch_list.filter_mode {
                    "[Esc] Exit filter | Type to search"
                } else {
                    "[r] Refresh | [c] Cleanup | [x] Repair | [l] Logs"
                }
            }
            Screen::WorktreeCreate => "[Enter] Next | [Esc] Back",
            Screen::Settings => "[Tab] Category | [Esc] Back",
            Screen::Logs => "[Up/Down] Navigate | [Enter] Detail | [c] Copy | [f] Filter | [/] Search | [Esc] Back",
            Screen::Help => "[Esc] Close | [Up/Down] Scroll",
            Screen::Confirm => "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel",
            Screen::Error => "[Enter/Esc] Close | [Up/Down] Scroll",
            Screen::Profiles => {
                if self.profiles.create_mode {
                    "[Enter] Save | [Esc] Cancel"
                } else {
                    "[Space] Activate | [Enter] Edit AI/env | [n] New | [d] Delete | [Esc] Back"
                }
            }
            Screen::Environment => {
                if self.environment.is_ai_only() {
                    if self.environment.edit_mode {
                        "[Enter] Save | [Tab] Switch | [Esc] Cancel"
                    } else {
                        "[Enter] Edit | [Esc] Back"
                    }
                } else if self.environment.edit_mode {
                    "[Enter] Save | [Tab] Switch | [Esc] Cancel"
                } else {
                    "[Enter] Edit | [n] New | [d] Delete (profile)/Disable (OS) | [r] Reset (override) | [Esc] Back"
                }
            }
            Screen::AISettingsWizard => {
                if self.ai_wizard.show_delete_confirm {
                    "[y] Confirm Delete | [n] Cancel"
                } else {
                    self.ai_wizard.step_title()
                }
            }
        }
    }

    fn view_footer(&self, frame: &mut Frame, area: Rect) {
        let keybinds = self.get_footer_keybinds();

        let status = self.active_status_message().unwrap_or("");
        let footer_text = if status.is_empty() {
            format!(" {} ", keybinds)
        } else {
            format!(" {} | {} ", keybinds, status)
        };

        let style = if self.ctrl_c_count > 0 {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        // Calculate if wrap is needed based on text length and available width
        let inner_width = area.width.saturating_sub(2); // borders
        let needs_wrap = footer_text.len() > inner_width as usize;

        let mut footer = Paragraph::new(footer_text)
            .style(style)
            .block(Block::default().borders(Borders::ALL));

        if needs_wrap {
            footer = footer.wrap(Wrap { trim: true });
        }

        frame.render_widget(footer, area);
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
                }
                None
            }
            KeyCode::BackTab => {
                if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.switch_field();
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
                    let is_key_press = key.kind == KeyEventKind::Press;
                    // Wizard has priority when visible
                    let msg = if model.wizard.visible {
                        match key.code {
                            KeyCode::Esc => Some(Message::WizardBack),
                            KeyCode::Enter if is_key_press => Some(Message::WizardConfirm),
                            KeyCode::Up if is_key_press => Some(Message::WizardPrev),
                            KeyCode::Down if is_key_press => Some(Message::WizardNext),
                            KeyCode::Backspace => {
                                model.wizard.delete_char();
                                None
                            }
                            KeyCode::Left => {
                                model.wizard.cursor_left();
                                None
                            }
                            KeyCode::Right => {
                                model.wizard.cursor_right();
                                None
                            }
                            KeyCode::Char(c)
                                if key.modifiers.is_empty()
                                    || key.modifiers == KeyModifiers::SHIFT =>
                            {
                                model.wizard.insert_char(c);
                                None
                            }
                            _ => None,
                        }
                    } else if model.text_input_active() {
                        model.handle_text_input_key(key, is_key_press)
                    } else {
                        // Normal key handling
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) if is_key_press => {
                                Some(Message::CtrlC)
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
                                if matches!(model.screen, Screen::BranchList) {
                                    if model.branch_list.filter_mode {
                                        // Exit filter mode (clear query if any, then exit mode)
                                        Some(Message::NavigateBack)
                                    } else if !model.branch_list.filter.is_empty() {
                                        // Clear filter query
                                        model.branch_list.clear_filter();
                                        model.refresh_branch_summary();
                                        None
                                    } else if model.has_active_agent_pane() {
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
                                if matches!(model.screen, Screen::BranchList | Screen::Help) {
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
                                if matches!(model.screen, Screen::BranchList) {
                                    if model.branch_list.filter_mode {
                                        Some(Message::Char('n'))
                                    } else {
                                        None
                                    }
                                } else if matches!(model.screen, Screen::Profiles) {
                                    // Create new profile
                                    model.profiles.enter_create_mode();
                                    None
                                } else if matches!(model.screen, Screen::Environment) {
                                    if model.environment.edit_mode {
                                        Some(Message::Char('n'))
                                    } else if model.environment.is_ai_only() {
                                        model.status_message = Some(
                                            "AI settings only. Use Enter to edit.".to_string(),
                                        );
                                        model.status_message_time = Some(Instant::now());
                                        None
                                    } else {
                                        model.environment.start_new();
                                        None
                                    }
                                } else {
                                    Some(Message::Char('n'))
                                }
                            }
                            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                                // In filter mode, 's' goes to filter input
                                if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    Some(Message::NavigateTo(Screen::Settings))
                                } else {
                                    Some(Message::Char('s'))
                                }
                            }
                            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                                // In filter mode, 'r' goes to filter input
                                if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    Some(Message::RefreshData)
                                } else if matches!(model.screen, Screen::Environment) {
                                    if model.environment.edit_mode {
                                        Some(Message::Char('r'))
                                    } else if model.environment.is_ai_only() {
                                        model.status_message = Some(
                                            "AI settings only. Use Enter to edit.".to_string(),
                                        );
                                        model.status_message_time = Some(Instant::now());
                                        None
                                    } else {
                                        model.reset_selected_env();
                                        None
                                    }
                                } else {
                                    Some(Message::Char('r'))
                                }
                            }
                            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                                // Copy to clipboard on Logs screen
                                if matches!(model.screen, Screen::Logs) && !model.logs.is_searching
                                {
                                    Some(Message::CopyLogToClipboard)
                                } else if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    // FR-010: Cleanup command
                                    // In filter mode, 'c' goes to filter input
                                    // FR-028: Check if branches are selected
                                    if model.branch_list.selected_branches.is_empty() {
                                        model.status_message =
                                            Some("No branches selected.".to_string());
                                        model.status_message_time = Some(Instant::now());
                                        None
                                    } else {
                                        // FR-028a-b: Filter out remote branches and current branch
                                        let cleanup_branches: Vec<String> = model
                                            .branch_list
                                            .selected_branches
                                            .iter()
                                            .filter(|name| {
                                                // Find the branch in the list
                                                model
                                                    .branch_list
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
                                            let excluded =
                                                model.branch_list.selected_branches.len();
                                            model.status_message = Some(format!(
                                            "{} branch(es) excluded (remote, current, or no worktree).",
                                            excluded
                                        ));
                                            model.status_message_time = Some(Instant::now());
                                            None
                                        } else {
                                            // Show cleanup confirmation dialog
                                            model.confirm =
                                                ConfirmState::cleanup(&cleanup_branches);
                                            model.pending_cleanup_branches = cleanup_branches;
                                            model.screen_stack.push(model.screen.clone());
                                            model.screen = Screen::Confirm;
                                            None
                                        }
                                    }
                                } else {
                                    Some(Message::Char('c'))
                                }
                            }
                            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                                if matches!(model.screen, Screen::Profiles) {
                                    model.delete_selected_profile();
                                    None
                                } else if matches!(model.screen, Screen::Environment) {
                                    if model.environment.edit_mode {
                                        Some(Message::Char('d'))
                                    } else if model.environment.is_ai_only() {
                                        model.status_message = Some(
                                            "AI settings only. Use Enter to edit.".to_string(),
                                        );
                                        model.status_message_time = Some(Instant::now());
                                        None
                                    } else {
                                        model.delete_selected_env();
                                        None
                                    }
                                } else if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                    && model.selected_branch_has_agent()
                                {
                                    // FR-040: d key to delete agent pane with confirmation
                                    Some(Message::ConfirmAgentTermination)
                                } else {
                                    Some(Message::Char('d'))
                                }
                            }
                            (KeyCode::Char('x'), KeyModifiers::NONE) => {
                                // Repair worktrees command
                                // In filter mode, 'x' goes to filter input
                                if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    Some(Message::RepairWorktrees)
                                } else {
                                    Some(Message::Char('x'))
                                }
                            }
                            (KeyCode::Char('p'), KeyModifiers::NONE) => {
                                // In filter mode, 'p' goes to filter input
                                if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    Some(Message::NavigateTo(Screen::Profiles))
                                } else {
                                    Some(Message::Char('p'))
                                }
                            }
                            (KeyCode::Char('l'), KeyModifiers::NONE) => {
                                // In filter mode, 'l' goes to filter input
                                if matches!(model.screen, Screen::BranchList)
                                    && !model.branch_list.filter_mode
                                {
                                    Some(Message::NavigateTo(Screen::Logs))
                                } else {
                                    Some(Message::Char('l'))
                                }
                            }
                            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                                if matches!(model.screen, Screen::Logs) {
                                    Some(Message::CycleFilter)
                                } else if matches!(model.screen, Screen::BranchList) {
                                    Some(Message::ToggleFilterMode)
                                } else {
                                    Some(Message::Char('f'))
                                }
                            }
                            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                                if matches!(model.screen, Screen::Logs) {
                                    Some(Message::ToggleSearch)
                                } else if matches!(model.screen, Screen::BranchList) {
                                    Some(Message::ToggleFilterMode)
                                } else {
                                    Some(Message::Char('/'))
                                }
                            }
                            (KeyCode::Char(' '), _) => {
                                if matches!(model.screen, Screen::BranchList | Screen::Profiles) {
                                    Some(Message::Space)
                                } else {
                                    Some(Message::Char(' '))
                                }
                            }
                            (KeyCode::Tab, _) => Some(Message::Tab),
                            (KeyCode::Char('m'), KeyModifiers::NONE) => {
                                if matches!(model.screen, Screen::BranchList) {
                                    Some(Message::CycleViewMode)
                                } else {
                                    None
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
                            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT)
                                if is_key_press =>
                            {
                                Some(Message::Char(c))
                            }
                            _ => None,
                        }
                    };

                    if let Some(msg) = msg {
                        model.update(msg);
                    }
                }
                Event::Mouse(mouse) => {
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                        model.handle_branch_list_mouse(mouse);
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

    // Cleanup on exit - check for orphaned worktrees (only if not launching agent)
    if pending_launch.is_none() {
        if let Ok(manager) = WorktreeManager::new(&model.repo_root) {
            let orphans = manager.detect_orphans();
            if !orphans.is_empty() {
                // Attempt to prune automatically
                let _ = manager.prune();
            }
        }
    }

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
                ExecutionMode::Continue | ExecutionMode::Resume => {
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
                ExecutionMode::Continue | ExecutionMode::Resume => {
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
                ExecutionMode::Continue | ExecutionMode::Resume => {
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
                ExecutionMode::Continue | ExecutionMode::Resume => {
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
    use crate::tui::screens::branch_list::SafetyStatus;
    use crate::tui::screens::wizard::WizardStep;
    use crate::tui::screens::{BranchItem, BranchListState};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use gwt_core::git::Branch;
    use std::sync::mpsc;

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
            timestamp: 0,
        }
    }

    #[test]
    fn test_resolve_orphaned_agent_name_prefers_session_entry() {
        let entry = sample_tool_entry("codex-cli");
        let resolved = resolve_orphaned_agent_name("bash", Some(&entry));
        assert_eq!(resolved, "codex-cli");
    }

    #[test]
    fn test_resolve_orphaned_agent_name_fallbacks() {
        let resolved = resolve_orphaned_agent_name("bash", None);
        assert_eq!(resolved, "bash");
        let resolved = resolve_orphaned_agent_name("  ", None);
        assert_eq!(resolved, "unknown");
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
    fn test_mouse_double_click_selects_branch_and_opens_wizard() {
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

        assert_eq!(model.branch_list.selected, 0);
        assert!(!model.wizard.visible);
        model.handle_branch_list_mouse(mouse);

        assert_eq!(model.branch_list.selected, 1);
        assert!(model.wizard.visible);
    }
}
