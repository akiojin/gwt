//! TUI Application with Elm Architecture

#![allow(dead_code)] // TUI application components for future expansion

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::config::get_branch_tool_history;
use gwt_core::config::{Profile, ProfilesConfig};
use gwt_core::error::GwtError;
use gwt_core::git::{Branch, PrCache, Repository};
use gwt_core::tmux::{get_current_session, kill_pane, launcher, AgentPane};
use gwt_core::worktree::WorktreeManager;
use gwt_core::TmuxMode;
use ratatui::{prelude::*, widgets::*};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, error, info};

use super::screens::branch_list::WorktreeStatus;
use super::screens::environment::EditField;
use super::screens::pane_list::{render_pane_list, PaneListState};
use super::screens::split_layout::{calculate_split_layout, SplitLayoutState};
use super::screens::{
    collect_os_env, render_branch_list, render_confirm, render_environment, render_error,
    render_help, render_logs, render_profiles, render_settings, render_wizard,
    render_worktree_create, BranchItem, BranchListState, BranchType, CodingAgent, ConfirmState,
    EnvironmentState, ErrorState, ExecutionMode, HelpState, LogsState, ProfilesState,
    QuickStartEntry, ReasoningLevel, SettingsState, WizardConfirmResult, WizardState,
    WorktreeCreateState,
};

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

struct PrTitleUpdate {
    titles: HashMap<String, String>,
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

struct BranchListUpdate {
    branches: Vec<BranchItem>,
    branch_names: Vec<String>,
    worktree_targets: Vec<WorktreeStatusTarget>,
    safety_targets: Vec<SafetyCheckTarget>,
    base_branches: Vec<String>,
    base_branch: String,
    total_count: usize,
    active_count: usize,
}

/// Application state (Model in Elm Architecture)
pub struct Model {
    /// Whether the app should quit
    should_quit: bool,
    /// Ctrl+C press count
    ctrl_c_count: u8,
    /// Last Ctrl+C press time
    last_ctrl_c: Option<Instant>,
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
    /// Status message
    status_message: Option<String>,
    /// Status message timestamp (for auto-clear)
    status_message_time: Option<Instant>,
    /// Is offline
    is_offline: bool,
    /// Active worktree count
    active_count: usize,
    /// Total branch count
    total_count: usize,
    /// Pending agent launch configuration (set when wizard completes)
    pending_agent_launch: Option<AgentLaunchConfig>,
    /// Pending unsafe branch selection (FR-029b)
    pending_unsafe_selection: Option<String>,
    /// Pending cleanup branches (FR-010)
    pending_cleanup_branches: Vec<String>,
    /// Branch list update receiver
    branch_list_rx: Option<Receiver<BranchListUpdate>>,
    /// PR title update receiver
    pr_title_rx: Option<Receiver<PrTitleUpdate>>,
    /// Safety check update receiver
    safety_rx: Option<Receiver<SafetyUpdate>>,
    /// Worktree status update receiver
    worktree_status_rx: Option<Receiver<WorktreeStatusUpdate>>,
    /// Tmux mode (Single or Multi)
    tmux_mode: TmuxMode,
    /// Tmux session name (when in multi mode)
    tmux_session: Option<String>,
    /// The pane ID where gwt is running (for splitting)
    gwt_pane_id: Option<String>,
    /// Launched agent pane IDs (for horizontal layout management)
    agent_panes: Vec<String>,
    /// Pane list state for tmux multi-mode
    pane_list: PaneListState,
    /// Split layout state for tmux multi-mode
    split_layout: SplitLayoutState,
    /// Last time pane list was updated (for 1-second polling)
    last_pane_update: Option<Instant>,
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
            status_message: None,
            status_message_time: None,
            is_offline: false,
            active_count: 0,
            total_count: 0,
            pending_agent_launch: None,
            pending_unsafe_selection: None,
            pending_cleanup_branches: Vec::new(),
            branch_list_rx: None,
            pr_title_rx: None,
            safety_rx: None,
            worktree_status_rx: None,
            tmux_mode: TmuxMode::detect(),
            tmux_session: None,
            gwt_pane_id: None,
            agent_panes: Vec::new(),
            pane_list: PaneListState::new(),
            split_layout: SplitLayoutState::new(),
            last_pane_update: None,
        };

        // Initialize tmux session if in multi mode
        if model.tmux_mode.is_multi() {
            // Use the current tmux session, not a generated one
            model.tmux_session = get_current_session();
            // Capture the gwt pane ID for splitting
            model.gwt_pane_id = gwt_core::tmux::get_current_pane_id();
            // Enable split layout for pane list
            model.split_layout.enable_tmux_mode();
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
        model.apply_entry_context(context);
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

            let mut titles = HashMap::new();
            for name in branch_names {
                if let Some(title) = cache.get_title(&name) {
                    titles.insert(name, title.to_string());
                }
            }

            let _ = tx.send(PrTitleUpdate { titles });
        });
    }

    fn spawn_safety_checks(&mut self, targets: Vec<SafetyCheckTarget>, base_branch: String) {
        if targets.is_empty() {
            self.safety_rx = None;
            return;
        }

        let repo_root = self.repo_root.clone();
        let (tx, rx) = mpsc::channel();
        self.safety_rx = Some(rx);

        thread::spawn(move || {
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

                // Check if branch is merged into base using merge-base --is-ancestor
                // This correctly handles cases where base_branch has advanced beyond the merge point
                if let Ok(is_merged) =
                    Branch::is_merged_into(&repo_root, &target.branch, &base_branch)
                {
                    is_unmerged = !is_merged;
                    safe_to_cleanup = is_merged;
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

    fn apply_branch_list_updates(&mut self) {
        let Some(rx) = &self.branch_list_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                let mut branch_list = BranchListState::new().with_branches(update.branches);
                branch_list.active_profile = self.profiles_config.active.clone();
                branch_list.working_directory = Some(self.repo_root.display().to_string());
                branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
                self.branch_list = branch_list;

                self.total_count = update.total_count;
                self.active_count = update.active_count;

                let total_updates = update.safety_targets.len() + update.worktree_targets.len();
                self.branch_list.reset_status_progress(total_updates);
                self.spawn_safety_checks(update.safety_targets, update.base_branch);
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
                self.branch_list.apply_pr_titles(&update.titles);
                self.pr_title_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pr_title_rx = None;
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
    fn update_pane_list(&mut self) {
        // Only in tmux multi mode with panes
        if !self.tmux_mode.is_multi() || self.pane_list.panes.is_empty() {
            return;
        }

        // Check if 1 second has passed since last update
        let now = Instant::now();
        if let Some(last) = self.last_pane_update {
            if now.duration_since(last) < Duration::from_secs(1) {
                return;
            }
        }
        self.last_pane_update = Some(now);

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

        let remaining_panes: Vec<_> = self
            .pane_list
            .panes
            .iter()
            .filter(|p| current_pane_ids.contains(p.pane_id.as_str()))
            .cloned()
            .collect();

        // Also update agent_panes list
        self.agent_panes
            .retain(|id| current_pane_ids.contains(id.as_str()));

        // Update if any panes were removed
        if remaining_panes.len() != self.pane_list.panes.len() {
            self.pane_list.update_panes(remaining_panes);
        }
    }

    fn load_profiles(&mut self) {
        let profiles_config = ProfilesConfig::load().unwrap_or_default();
        self.profiles_config = profiles_config.clone();

        let mut names: Vec<String> = profiles_config.profiles.keys().cloned().collect();
        names.sort();

        let profiles = names
            .into_iter()
            .filter_map(|name| {
                profiles_config.profiles.get(&name).map(|profile| {
                    super::screens::profiles::ProfileItem {
                        name: name.clone(),
                        is_active: profiles_config.active.as_deref() == Some(name.as_str()),
                        env_count: profile.env.len(),
                        description: if profile.description.is_empty() {
                            None
                        } else {
                            Some(profile.description.clone())
                        },
                    }
                })
            })
            .collect();

        self.profiles = ProfilesState::new().with_profiles(profiles);
        self.branch_list.active_profile = self.profiles_config.active.clone();
    }

    fn save_profiles(&mut self) {
        if let Err(e) = self.profiles_config.save() {
            self.status_message = Some(format!("Failed to save profiles: {}", e));
            self.status_message_time = Some(Instant::now());
            return;
        }
        self.load_profiles();
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
        let (vars, disabled_keys) = self
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
                (items, profile.disabled_env.clone())
            })
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        self.environment = EnvironmentState::new()
            .with_profile(profile_name)
            .with_variables(vars)
            .with_disabled_keys(disabled_keys)
            .with_os_variables(collect_os_env());
        self.screen_stack.push(self.screen.clone());
        self.screen = Screen::Environment;
    }

    fn persist_environment(&mut self) {
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
        self.save_profiles();
    }

    fn delete_selected_profile(&mut self) {
        let selected = match self.profiles.selected_profile() {
            Some(item) => item.name.clone(),
            None => return,
        };

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
                    if let Some(prev_screen) = self.screen_stack.pop() {
                        self.screen = prev_screen;
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
                // FR-033: Update pane list every 1 second in tmux multi mode
                self.update_pane_list();
            }
            Message::SelectNext => match self.screen {
                Screen::BranchList => {
                    if self.split_layout.pane_list_has_focus() {
                        self.pane_list.select_next();
                    } else {
                        self.branch_list.select_next();
                    }
                }
                Screen::WorktreeCreate => self.worktree_create.select_next_base(),
                Screen::Settings => self.settings.select_next(),
                Screen::Logs => self.logs.select_next(),
                Screen::Help => self.help.scroll_down(),
                Screen::Error => self.error.scroll_down(),
                Screen::Profiles => self.profiles.select_next(),
                Screen::Environment => self.environment.select_next(),
                Screen::Confirm => {}
            },
            Message::SelectPrev => match self.screen {
                Screen::BranchList => {
                    if self.split_layout.pane_list_has_focus() {
                        self.pane_list.select_prev();
                    } else {
                        self.branch_list.select_prev();
                    }
                }
                Screen::WorktreeCreate => self.worktree_create.select_prev_base(),
                Screen::Settings => self.settings.select_prev(),
                Screen::Logs => self.logs.select_prev(),
                Screen::Help => self.help.scroll_up(),
                Screen::Error => self.error.scroll_up(),
                Screen::Profiles => self.profiles.select_prev(),
                Screen::Environment => self.environment.select_prev(),
                Screen::Confirm => {}
            },
            Message::PageUp => match self.screen {
                Screen::BranchList => self.branch_list.page_up(10),
                Screen::Logs => self.logs.page_up(10),
                Screen::Help => self.help.page_up(),
                Screen::Environment => self.environment.page_up(),
                _ => {}
            },
            Message::PageDown => match self.screen {
                Screen::BranchList => self.branch_list.page_down(10),
                Screen::Logs => self.logs.page_down(10),
                Screen::Help => self.help.page_down(),
                Screen::Environment => self.environment.page_down(),
                _ => {}
            },
            Message::GoHome => match self.screen {
                Screen::BranchList => self.branch_list.go_home(),
                Screen::Logs => self.logs.go_home(),
                Screen::Environment => self.environment.go_home(),
                _ => {}
            },
            Message::GoEnd => match self.screen {
                Screen::BranchList => self.branch_list.go_end(),
                Screen::Logs => self.logs.go_end(),
                Screen::Environment => self.environment.go_end(),
                _ => {}
            },
            Message::Enter => match &self.screen {
                Screen::BranchList => {
                    if self.split_layout.pane_list_has_focus() {
                        // FR-040: Enter on pane list focuses the selected pane
                        if let Some(pane) = self.pane_list.panes.get(self.pane_list.selected) {
                            if let Err(e) = gwt_core::tmux::pane::select_pane(&pane.pane_id) {
                                self.status_message = Some(format!("Failed to focus pane: {}", e));
                                self.status_message_time = Some(Instant::now());
                            }
                        }
                    } else if self.branch_list.filter_mode {
                        // FR-020: Enter in filter mode exits filter mode
                        self.branch_list.exit_filter_mode();
                    } else {
                        // Open wizard for selected branch (FR-007)
                        self.update(Message::OpenWizard);
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
                    }
                    // Clear pending state and return to previous screen
                    self.pending_unsafe_selection = None;
                    self.pending_cleanup_branches.clear();
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
                        let name = item.name.clone();
                        self.open_environment_editor(&name);
                    }
                }
                Screen::Environment => {
                    if self.environment.edit_mode {
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
                                    if let Some(var) = self.environment.variables.get_mut(index) {
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
                _ => {}
            },
            Message::Char(c) => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.insert_char(c);
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode
                {
                    // Filter mode - add character to filter
                    self.branch_list.filter_push(c);
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    // Profile create mode - add character to name
                    self.profiles.insert_char(c);
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.insert_char(c);
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Log search mode - add character to search
                    self.logs.search.push(c);
                }
            }
            Message::Backspace => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.delete_char();
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode
                {
                    self.branch_list.filter_pop();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.delete_char();
                } else if matches!(self.screen, Screen::Environment) && self.environment.edit_mode {
                    self.environment.delete_char();
                } else if matches!(self.screen, Screen::Logs) && self.logs.is_searching {
                    // Log search mode - delete character
                    self.logs.search.pop();
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
            Message::Tab => {
                if let Screen::Settings = self.screen {
                    self.settings.next_category()
                } else if let Screen::BranchList = self.screen {
                    // Toggle focus between branch list and pane list in tmux multi-mode
                    self.split_layout.toggle_focus();
                }
            }
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
                    // FR-050: Load session history for Quick Start feature
                    let ts_history = get_branch_tool_history(&self.repo_root, &branch.name);
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
                    self.wizard.open_for_branch(&branch.name, history);
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
        debug!(
            category = "tui",
            branch = %self.worktree_create.branch_name,
            create_new_branch = self.worktree_create.create_new_branch,
            "Creating worktree from wizard state"
        );
        if let Ok(manager) = WorktreeManager::new(&self.repo_root) {
            let branch = self.worktree_create.branch_name.clone();
            let base = if self.worktree_create.create_new_branch {
                self.wizard
                    .base_branch_override
                    .as_deref()
                    .or_else(|| self.worktree_create.selected_base_branch())
            } else {
                None
            };

            // First try to get existing worktree for this branch
            let existing_wt = manager.get_by_branch(&branch).ok().flatten();

            let result = if let Some(wt) = existing_wt {
                // Worktree already exists, just use it
                Ok(wt)
            } else if self.worktree_create.create_new_branch {
                manager.create_new_branch(&branch, base)
            } else {
                manager.create_for_branch(&branch)
            };

            match result {
                Ok(wt) => {
                    info!(
                        category = "tui",
                        operation = "create_worktree",
                        branch = %branch,
                        worktree_path = %wt.path.display(),
                        "Worktree created successfully"
                    );
                    // Create agent launch configuration
                    let auto_install_deps = self
                        .settings
                        .settings
                        .as_ref()
                        .map(|settings| settings.agent.auto_install_deps)
                        .unwrap_or(false);

                    let launch_config = AgentLaunchConfig {
                        worktree_path: wt.path.clone(),
                        branch_name: branch.clone(),
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

                    // In multi mode, launch in tmux pane without quitting TUI
                    if self.tmux_mode.is_multi() && self.gwt_pane_id.is_some() {
                        match self.launch_agent_in_pane(&launch_config) {
                            Ok(_) => {
                                self.status_message =
                                    Some(format!("Agent launched in tmux pane for {}", branch));
                                self.status_message_time = Some(Instant::now());
                                // Close wizard and return to branch list
                                self.wizard.visible = false;
                                self.screen = Screen::BranchList;
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Failed to launch: {}", e));
                                self.status_message_time = Some(Instant::now());
                            }
                        }
                    } else {
                        // Single mode: store launch config and quit TUI
                        self.pending_agent_launch = Some(launch_config);
                        self.should_quit = true;
                    }
                }
                Err(e) => {
                    error!(
                        category = "tui",
                        operation = "create_worktree",
                        branch = %branch,
                        error = %e,
                        "Failed to create worktree"
                    );
                    self.worktree_create.error_message = Some(e.to_string());
                    self.status_message = Some(format!("Error: {}", e));
                    self.status_message_time = Some(Instant::now());
                }
            }
        }
    }

    /// Launch an agent in a tmux pane (multi mode)
    ///
    /// Layout strategy:
    /// - First agent: vertical split below gwt pane
    /// - Additional agents: horizontal split beside last agent pane
    ///
    /// Uses the same argument building logic as single mode (main.rs)
    fn launch_agent_in_pane(&mut self, config: &AgentLaunchConfig) -> Result<String, String> {
        let working_dir = config.worktree_path.to_string_lossy().to_string();

        // Build environment variables (same as single mode)
        let mut env_vars = config.env.clone();
        // Add IS_SANDBOX=1 for Claude Code with skip_permissions (same as single mode)
        if config.skip_permissions && config.agent == CodingAgent::ClaudeCode {
            env_vars.push(("IS_SANDBOX".to_string(), "1".to_string()));
        }

        // Determine the agent command based on version (FR-063)
        // - "installed": use direct binary name
        // - "latest" or specific version: use bunx (or npx fallback) with package spec
        let (base_cmd, base_args) = if config.version == "installed" {
            // Use direct binary name for installed version
            (config.agent.command_name().to_string(), vec![])
        } else {
            // Try bunx first, then npx as fallback (same logic as single mode)
            let runner = which::which("bunx")
                .or_else(|_| which::which("npx"))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "bunx".to_string());

            let package_spec = if config.version == "latest" {
                format!("{}@latest", config.agent.npm_package())
            } else {
                format!("{}@{}", config.agent.npm_package(), config.version)
            };

            (runner, vec![package_spec])
        };

        // Build agent-specific arguments (same logic as main.rs build_agent_args)
        let agent_args = build_agent_args_for_tmux(config);

        // Build the full command string
        let command = build_tmux_command(&env_vars, &base_cmd, &base_args, &agent_args);

        debug!(
            category = "tui",
            gwt_pane_id = ?self.gwt_pane_id,
            working_dir = %working_dir,
            command = %command,
            agent_pane_count = self.agent_panes.len(),
            "Launching agent in tmux pane"
        );

        // Determine how to split based on existing agent panes
        let pane_id = if self.agent_panes.is_empty() {
            // First agent: vertical split below gwt pane
            // Use gwt_pane_id as the target for splitting
            let target = self
                .gwt_pane_id
                .as_ref()
                .ok_or_else(|| "No gwt pane ID available".to_string())?;
            launcher::launch_in_pane(target, &working_dir, &command)
        } else {
            // Additional agents: horizontal split beside last agent pane
            let last_pane = self.agent_panes.last().unwrap();
            launcher::launch_in_pane_beside(last_pane, &working_dir, &command)
        }
        .map_err(|e| e.to_string())?;

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
        let branch_name = self
            .branch_list
            .selected_branch()
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let agent_pane = AgentPane::new(
            pane_id.clone(),
            branch_name,
            config.agent.label().to_string(),
            SystemTime::now(),
            0, // PID is not tracked by simple launcher
        );
        let mut panes = self.pane_list.panes.clone();
        panes.push(agent_pane);
        self.pane_list.update_panes(panes);

        Ok(pane_id)
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

        // Calculate footer height dynamically based on text length
        let keybinds = self.get_footer_keybinds();
        let status = self.status_message.as_deref().unwrap_or("");
        let footer_text_len = if status.is_empty() {
            keybinds.len() + 2 // " {} " format adds 2 spaces
        } else {
            keybinds.len() + status.len() + 5 // " {} | {} " format adds 5 chars
        };
        let inner_width = frame.area().width.saturating_sub(2) as usize; // borders
        let footer_height = if footer_text_len > inner_width { 4 } else { 3 };

        // Profiles, Environment, and Logs screens don't need header
        let needs_header = !matches!(
            base_screen,
            Screen::Profiles | Screen::Environment | Screen::Logs
        );
        let header_height = if needs_header { 6 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height), // Header (0 for Profiles/Environment)
                Constraint::Min(0),                // Content
                Constraint::Length(footer_height), // Footer (dynamic)
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
                // Use split layout for tmux multi-mode
                let split_areas = calculate_split_layout(chunks[1], &self.split_layout);

                // Update focus state for pane list
                self.pane_list.has_focus = self.split_layout.pane_list_has_focus();

                // Render branch list
                let branch_list_has_focus = !self.pane_list.has_focus;
                render_branch_list(
                    &mut self.branch_list,
                    frame,
                    split_areas.branch_list,
                    self.status_message.as_deref(),
                    branch_list_has_focus,
                );

                // Render pane list (only visible in tmux multi mode)
                if self.split_layout.pane_list_visible {
                    render_pane_list(&mut self.pane_list, frame, split_areas.pane_list);
                }
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
            Screen::Confirm => {}
        }

        if matches!(self.screen, Screen::Confirm) {
            render_confirm(&self.confirm, frame, chunks[1]);
        }

        // Footer
        self.view_footer(frame, chunks[2]);

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
                    "[Space] Activate | [Enter] Edit env | [n] New | [d] Delete | [Esc] Back"
                }
            }
            Screen::Environment => {
                if self.environment.edit_mode {
                    "[Enter] Save | [Tab] Switch | [Esc] Cancel"
                } else {
                    "[Enter] Edit | [n] New | [d] Delete (profile)/Disable (OS) | [r] Reset (override) | [Esc] Back"
                }
            }
        }
    }

    fn view_footer(&self, frame: &mut Frame, area: Rect) {
        let keybinds = self.get_footer_keybinds();

        let status = self.status_message.as_deref().unwrap_or("");
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
/// Returns agent launch configuration if wizard completed, None otherwise
pub fn run() -> Result<Option<AgentLaunchConfig>, GwtError> {
    run_with_context(None)
}

/// Run the TUI application with optional entry context
/// Returns agent launch configuration if wizard completed, None otherwise
pub fn run_with_context(
    context: Option<TuiEntryContext>,
) -> Result<Option<AgentLaunchConfig>, GwtError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
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
            if let Event::Key(key) = event::read()? {
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
                            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
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
                            // Esc behavior matches TypeScript:
                            // - In filter mode: exit filter mode (handled by NavigateBack)
                            // - In BranchList with filter query: clear query
                            // - Otherwise: navigate back (but NOT quit from main screen)
                            if matches!(model.screen, Screen::BranchList) {
                                if model.branch_list.filter_mode {
                                    // Exit filter mode (clear query if any, then exit mode)
                                    Some(Message::NavigateBack)
                                } else if !model.branch_list.filter.is_empty() {
                                    // Clear filter query
                                    model.branch_list.clear_filter();
                                    None
                                } else {
                                    // On main screen without filter - do nothing (TypeScript doesn't quit here)
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
                            if matches!(model.screen, Screen::Logs) && !model.logs.is_searching {
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
                                        let excluded = model.branch_list.selected_branches.len();
                                        model.status_message = Some(format!(
                                            "{} branch(es) excluded (remote, current, or no worktree).",
                                            excluded
                                        ));
                                        model.status_message_time = Some(Instant::now());
                                        None
                                    } else {
                                        // Show cleanup confirmation dialog
                                        model.confirm = ConfirmState::cleanup(&cleanup_branches);
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
                                } else {
                                    model.delete_selected_env();
                                    None
                                }
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(pending_launch)
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

/// Build the full tmux command string with environment variables
fn build_tmux_command(
    env_vars: &[(String, String)],
    base_cmd: &str,
    base_args: &[String],
    agent_args: &[String],
) -> String {
    let mut parts = Vec::new();

    // Add environment variable exports
    for (key, value) in env_vars {
        let escaped_value = value.replace('\'', "'\\''");
        parts.push(format!("export {}='{}'", key, escaped_value));
    }

    // Build the command with all arguments
    let mut cmd_parts = vec![base_cmd.to_string()];
    cmd_parts.extend(base_args.iter().cloned());
    cmd_parts.extend(agent_args.iter().cloned());
    let full_cmd = cmd_parts.join(" ");

    if parts.is_empty() {
        full_cmd
    } else {
        parts.push(full_cmd);
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::screens::branch_list::SafetyStatus;
    use crate::tui::screens::wizard::WizardStep;
    use crate::tui::screens::{BranchItem, BranchListState};
    use gwt_core::git::Branch;

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
                is_selected: false,
                pr_title: None,
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
                is_selected: false,
                pr_title: None,
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
}
