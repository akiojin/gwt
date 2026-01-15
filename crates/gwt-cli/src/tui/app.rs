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
use gwt_core::git::{Branch, PrCache};
use gwt_core::worktree::WorktreeManager;
use ratatui::{prelude::*, widgets::*};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use super::screens::branch_list::WorktreeStatus;
use super::screens::environment::EditField;
use super::screens::{
    render_branch_list, render_confirm, render_environment, render_error, render_help, render_logs,
    render_profiles, render_settings, render_wizard, render_worktree_create, BranchItem,
    BranchListState, BranchType, CodingAgent, ConfirmState, EnvironmentState, ErrorState,
    ExecutionMode, HelpState, LogsState, ProfilesState, QuickStartEntry, ReasoningLevel,
    SettingsState, WizardConfirmResult, WizardState, WorktreeCreateState,
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
    upstream: String,
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
    /// PR title update receiver
    pr_title_rx: Option<Receiver<PrTitleUpdate>>,
    /// Safety check update receiver
    safety_rx: Option<Receiver<SafetyUpdate>>,
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
}

impl Model {
    /// Create a new model
    pub fn new() -> Self {
        Self::new_with_context(None)
    }

    pub fn new_with_context(context: Option<TuiEntryContext>) -> Self {
        let repo_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

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
            pr_title_rx: None,
            safety_rx: None,
        };

        // Load initial data
        model.refresh_data();
        model.apply_entry_context(context);
        model
    }

    /// Refresh data from repository
    fn refresh_data(&mut self) {
        let settings = gwt_core::config::Settings::load(&self.repo_root).unwrap_or_default();
        let base_branch = settings.default_base_branch.clone();
        let base_branch_exists = Branch::exists(&self.repo_root, &base_branch).unwrap_or(false);

        if let Ok(manager) = WorktreeManager::new(&self.repo_root) {
            // Get branches
            if let Ok(branches) = Branch::list_basic(&self.repo_root) {
                let worktrees = manager.list().unwrap_or_default();

                // Load tool usage from TypeScript session file (FR-070)
                let tool_usage_map = gwt_core::config::get_last_tool_usage_map(&self.repo_root);
                let mut safety_targets = Vec::new();

                let mut branch_items: Vec<BranchItem> = branches
                    .iter()
                    .map(|b| {
                        let mut item = BranchItem::from_branch(b, &worktrees);

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
                            if item.has_changes
                                || item.has_unpushed
                                || !b.has_remote
                                || !base_branch_exists
                            {
                                item.safe_to_cleanup = Some(false);
                            } else if let Some(upstream) = b.upstream.clone() {
                                item.safe_to_cleanup = None;
                                safety_targets.push(SafetyCheckTarget {
                                    branch: b.name.clone(),
                                    upstream,
                                });
                            } else {
                                item.safe_to_cleanup = Some(false);
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

                self.total_count = branch_items.len();
                self.active_count = branch_items.iter().filter(|b| b.has_worktree).count();
                self.branch_list = BranchListState::new().with_branches(branch_items);
                self.spawn_safety_checks(safety_targets, base_branch.clone());
                self.spawn_pr_title_fetch(&branches);

                // Get base branches for worktree create
                let base_branches: Vec<String> = branches
                    .iter()
                    .filter(|b| !b.name.starts_with("remotes/"))
                    .map(|b| b.name.clone())
                    .collect();
                self.worktree_create = WorktreeCreateState::new().with_base_branches(base_branches);
            }
        }

        self.settings = SettingsState::new().with_settings(settings.clone());

        // Load logs from configured log directory
        let log_dir = settings.log_dir(&self.repo_root);
        if log_dir.exists() {
            let reader = gwt_core::logging::LogReader::new(&log_dir);
            if let Ok(entries) = reader.read_latest(100) {
                // Convert gwt_core LogEntry to TUI LogEntry
                let tui_entries: Vec<super::screens::logs::LogEntry> = entries
                    .into_iter()
                    .map(|e| super::screens::logs::LogEntry {
                        timestamp: e.timestamp,
                        level: e.level,
                        message: e.message,
                        target: e.target,
                    })
                    .collect();
                self.logs = LogsState::new().with_entries(tui_entries);
            }
        }

        self.load_profiles();
        self.branch_list.working_directory = Some(self.repo_root.display().to_string());
        self.branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
    }

    fn spawn_pr_title_fetch(&mut self, branches: &[Branch]) {
        if branches.is_empty() {
            self.pr_title_rx = None;
            return;
        }

        let repo_root = self.repo_root.clone();
        let branch_names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
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

                let unpushed_result =
                    Branch::divergence_between(&repo_root, &target.branch, &target.upstream);
                match unpushed_result {
                    Ok((ahead, _)) => {
                        if ahead > 0 {
                            has_unpushed = true;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(SafetyUpdate {
                            branch: target.branch,
                            has_unpushed,
                            is_unmerged,
                            safe_to_cleanup,
                        });
                        continue;
                    }
                }

                if has_unpushed {
                    let _ = tx.send(SafetyUpdate {
                        branch: target.branch,
                        has_unpushed,
                        is_unmerged,
                        safe_to_cleanup,
                    });
                    continue;
                }

                if let Ok((ahead, _)) =
                    Branch::divergence_between(&repo_root, &target.branch, &base_branch)
                {
                    is_unmerged = ahead > 0;
                    safe_to_cleanup = !is_unmerged;
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
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.safety_rx = None;
                    break;
                }
            }
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

    fn open_environment_editor(&mut self, profile_name: &str) {
        let vars = self
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
                items
            })
            .unwrap_or_default();

        self.environment = EnvironmentState::new()
            .with_profile(profile_name)
            .with_variables(vars);
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

    fn delete_selected_env(&mut self) {
        if self.environment.variables.is_empty() {
            return;
        }
        if self.environment.selected < self.environment.variables.len() {
            self.environment.variables.remove(self.environment.selected);
            if self.environment.selected >= self.environment.variables.len() {
                self.environment.selected = self.environment.variables.len().saturating_sub(1);
            }
            self.persist_environment();
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
                self.apply_pr_title_updates();
                self.apply_safety_updates();
            }
            Message::SelectNext => match self.screen {
                Screen::BranchList => self.branch_list.select_next(),
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
                Screen::BranchList => self.branch_list.select_prev(),
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
                _ => {}
            },
            Message::PageDown => match self.screen {
                Screen::BranchList => self.branch_list.page_down(10),
                Screen::Logs => self.logs.page_down(10),
                Screen::Help => self.help.page_down(),
                _ => {}
            },
            Message::GoHome => match self.screen {
                Screen::BranchList => self.branch_list.go_home(),
                Screen::Logs => self.logs.go_home(),
                _ => {}
            },
            Message::GoEnd => match self.screen {
                Screen::BranchList => self.branch_list.go_end(),
                Screen::Logs => self.logs.go_end(),
                _ => {}
            },
            Message::Enter => match &self.screen {
                Screen::BranchList => {
                    if self.branch_list.filter_mode {
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
                        self.profiles_config.set_active(Some(item.name.clone()));
                        self.save_profiles();
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
                                } else if let Some(var) = self
                                    .environment
                                    .variables
                                    .get_mut(self.environment.selected)
                                {
                                    var.key = key;
                                    var.value = value;
                                }
                                self.environment.cancel_edit();
                                self.persist_environment();
                            }
                            Err(msg) => {
                                self.environment.error = Some(msg.to_string());
                            }
                        }
                    }
                }
                Screen::Help => {
                    self.update(Message::NavigateBack);
                }
                Screen::Error => {
                    self.update(Message::NavigateBack);
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
            Message::Tab => {
                if let Screen::Settings = self.screen {
                    self.settings.next_category()
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
                // FR-036: Tab disabled in filter mode
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
        if let Ok(manager) = WorktreeManager::new(&self.repo_root) {
            let branch = &self.worktree_create.branch_name;
            let base = if self.worktree_create.create_new_branch {
                self.wizard
                    .base_branch_override
                    .as_deref()
                    .or_else(|| self.worktree_create.selected_base_branch())
            } else {
                None
            };

            // First try to get existing worktree for this branch
            let existing_wt = manager.get_by_branch(branch).ok().flatten();

            let result = if let Some(wt) = existing_wt {
                // Worktree already exists, just use it
                Ok(wt)
            } else if self.worktree_create.create_new_branch {
                manager.create_new_branch(branch, base)
            } else {
                manager.create_for_branch(branch)
            };

            match result {
                Ok(wt) => {
                    // Create agent launch configuration
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
                    };

                    // Store the launch config and quit TUI
                    self.pending_agent_launch = Some(launch_config);
                    self.should_quit = true;
                }
                Err(e) => {
                    self.worktree_create.error_message = Some(e.to_string());
                    self.status_message = Some(format!("Error: {}", e));
                    self.status_message_time = Some(Instant::now());
                }
            }
        }
    }

    /// Execute branch cleanup (FR-010)
    fn execute_cleanup(&mut self, branches: &[String]) {
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
                    deleted += 1;
                    // Remove from selection
                    self.branch_list.selected_branches.remove(branch_name);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", branch_name, e));
                }
            }
        }

        // Show result message
        if errors.is_empty() {
            self.status_message = Some(format!("Deleted {} branch(es).", deleted));
        } else {
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
    pub fn view(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // Boxed header (title + 4 lines + borders)
                Constraint::Min(0),    // Content
                Constraint::Length(3), // Footer
            ])
            .split(frame.area());

        let base_screen = if matches!(self.screen, Screen::Confirm) {
            self.screen_stack
                .last()
                .cloned()
                .unwrap_or(Screen::BranchList)
        } else {
            self.screen.clone()
        };

        // Header (for branch list screen, render boxed header)
        if matches!(base_screen, Screen::BranchList) {
            self.view_boxed_header(frame, chunks[0]);
        } else {
            self.view_header(frame, chunks[0]);
        }

        // Content
        match base_screen {
            Screen::BranchList => render_branch_list(
                &self.branch_list,
                frame,
                chunks[1],
                self.status_message.as_deref(),
            ),
            Screen::WorktreeCreate => {
                render_worktree_create(&self.worktree_create, frame, chunks[1])
            }
            Screen::Settings => render_settings(&self.settings, frame, chunks[1]),
            Screen::Logs => render_logs(&self.logs, frame, chunks[1]),
            Screen::Help => render_help(&self.help, frame, chunks[1]),
            Screen::Error => render_error(&self.error, frame, chunks[1]),
            Screen::Profiles => render_profiles(&self.profiles, frame, chunks[1]),
            Screen::Environment => render_environment(&self.environment, frame, chunks[1]),
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
            Span::styled("Mode(tab): ", Style::default().fg(Color::DarkGray)),
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

    fn view_footer(&self, frame: &mut Frame, area: Rect) {
        let keybinds = match self.screen {
            Screen::BranchList => {
                if self.branch_list.filter_mode {
                    "[Esc] Exit filter | Type to search"
                } else {
                    // FR-004: r: Refresh, c: Cleanup, x: Repair, l: Logs
                    "[r] Refresh | [c] Cleanup | [x] Repair | [l] Logs"
                }
            }
            Screen::WorktreeCreate => "[Enter] Next | [Esc] Back",
            Screen::Settings => "[Tab] Category | [Esc] Back",
            Screen::Logs => "[f] Filter | [/] Search | [Esc] Back",
            Screen::Help => "[Esc] Close | [Up/Down] Scroll",
            Screen::Confirm => "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel",
            Screen::Error => "[Enter/Esc] Close | [Up/Down] Scroll",
            Screen::Profiles => {
                if self.profiles.create_mode {
                    "[Enter] Save | [Esc] Cancel"
                } else {
                    "[Enter] Activate | [n] New | [d] Delete | [e] Edit env | [Esc] Back"
                }
            }
            Screen::Environment => {
                if self.environment.edit_mode {
                    "[Enter] Save | [Tab] Switch | [Esc] Cancel"
                } else {
                    "[n] New | [e] Edit | [d] Delete | [v] Toggle visibility | [Esc] Back"
                }
            }
        };

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

        let footer = Paragraph::new(footer_text)
            .style(style)
            .block(Block::default().borders(Borders::ALL));
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
            KeyCode::Backspace => Some(Message::Backspace),
            KeyCode::Left => Some(Message::CursorLeft),
            KeyCode::Right => Some(Message::CursorRight),
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
            KeyCode::Char(c) => {
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
                            } else {
                                Some(Message::Char('r'))
                            }
                        }
                        (KeyCode::Char('c'), KeyModifiers::NONE) => {
                            // FR-010: Cleanup command
                            // In filter mode, 'c' goes to filter input
                            if matches!(model.screen, Screen::BranchList)
                                && !model.branch_list.filter_mode
                            {
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
                        (KeyCode::Char('e'), KeyModifiers::NONE) => {
                            if matches!(model.screen, Screen::Profiles) {
                                if let Some(item) = model.profiles.selected_profile() {
                                    let name = item.name.clone();
                                    model.open_environment_editor(&name);
                                }
                                None
                            } else if matches!(model.screen, Screen::Environment) {
                                if model.environment.edit_mode {
                                    Some(Message::Char('e'))
                                } else {
                                    model.environment.start_edit();
                                    None
                                }
                            } else {
                                Some(Message::Char('e'))
                            }
                        }
                        (KeyCode::Char('v'), KeyModifiers::NONE) => {
                            if matches!(model.screen, Screen::Environment) {
                                if model.environment.edit_mode {
                                    Some(Message::Char('v'))
                                } else {
                                    model.environment.toggle_visibility();
                                    None
                                }
                            } else {
                                Some(Message::Char('v'))
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
                            if matches!(model.screen, Screen::BranchList) {
                                Some(Message::Space)
                            } else {
                                Some(Message::Char(' '))
                            }
                        }
                        (KeyCode::Tab, _) => {
                            if matches!(model.screen, Screen::BranchList) {
                                Some(Message::CycleViewMode)
                            } else {
                                Some(Message::Tab)
                            }
                        }
                        (KeyCode::Up, _) if is_key_press => Some(Message::SelectPrev),
                        (KeyCode::Down, _) if is_key_press => Some(Message::SelectNext),
                        (KeyCode::PageUp, _) if is_key_press => Some(Message::PageUp),
                        (KeyCode::PageDown, _) if is_key_press => Some(Message::PageDown),
                        (KeyCode::Home, _) if is_key_press => Some(Message::GoHome),
                        (KeyCode::End, _) if is_key_press => Some(Message::GoEnd),
                        (KeyCode::Enter, _) if is_key_press => Some(Message::Enter),
                        (KeyCode::Backspace, _) => Some(Message::Backspace),
                        (KeyCode::Left, _) => Some(Message::CursorLeft),
                        (KeyCode::Right, _) => Some(Message::CursorRight),
                        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
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

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(pending_launch)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
