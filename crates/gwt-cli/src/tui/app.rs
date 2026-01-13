//! TUI Application with Elm Architecture

#![allow(dead_code)] // TUI application components for future expansion

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::config::get_branch_tool_history;
use gwt_core::error::GwtError;
use gwt_core::git::Branch;
use gwt_core::worktree::WorktreeManager;
use ratatui::{prelude::*, widgets::*};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::screens::{
    render_branch_list, render_confirm, render_environment, render_error, render_help, render_logs,
    render_profiles, render_settings, render_wizard, render_worktree_create, BranchItem,
    BranchListState, BranchType, CodingAgent, ConfirmState, EnvironmentState, ErrorState,
    ExecutionMode, HelpState, LogsState, ProfilesState, QuickStartEntry, ReasoningLevel,
    SettingsState, WizardState, WorktreeCreateState,
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
    /// Skip permission prompts
    pub skip_permissions: bool,
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
        };

        // Load initial data
        model.refresh_data();
        model
    }

    /// Refresh data from repository
    fn refresh_data(&mut self) {
        if let Ok(manager) = WorktreeManager::new(&self.repo_root) {
            // Get branches
            if let Ok(branches) = Branch::list(&self.repo_root) {
                let worktrees = manager.list().unwrap_or_default();

                // Load tool usage from TypeScript session file (FR-070)
                let tool_usage_map = gwt_core::config::get_last_tool_usage_map(&self.repo_root);

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
            }

            // Get base branches for worktree create
            if let Ok(branches) = Branch::list(&self.repo_root) {
                let base_branches: Vec<String> = branches
                    .iter()
                    .filter(|b| !b.name.starts_with("remotes/"))
                    .map(|b| b.name.clone())
                    .collect();
                self.worktree_create = WorktreeCreateState::new().with_base_branches(base_branches);
            }
        }

        // Load settings
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

        // Load profiles (initialize with default profile if none exist)
        // For now, create a simple default profile until a full profile manager is implemented
        use super::screens::profiles::ProfileItem;
        let profiles = vec![ProfileItem {
            name: "default".to_string(),
            is_active: true,
            env_count: 0,
            description: Some("Default profile".to_string()),
        }];
        self.profiles = super::screens::ProfilesState::new().with_profiles(profiles);
        self.branch_list.active_profile = Some("default".to_string());
        self.branch_list.working_directory = Some(self.repo_root.display().to_string());
        self.branch_list.version = Some(env!("CARGO_PKG_VERSION").to_string());
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
                Screen::Help => {
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
                }
            }
            Message::CursorLeft => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_left();
                } else if matches!(self.screen, Screen::Profiles) && self.profiles.create_mode {
                    self.profiles.cursor_left();
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
                        let is_selected = self.branch_list.selected_branches.contains(&branch.name);

                        // Only show warning when selecting (not deselecting)
                        if !is_selected {
                            // Check if branch is unsafe (FR-029b/FR-029e)
                            let is_unsafe = branch.has_changes
                                || branch.has_unpushed
                                || branch.is_unmerged
                                || branch.safe_to_cleanup.is_none(); // Unknown = treat as potentially unsafe

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
                    // FR-074: Block first Enter in VersionSelect to prevent auto-advance
                    if self.wizard.block_next_enter {
                        self.wizard.block_next_enter = false;
                        return;
                    }
                    if self.wizard.is_complete() {
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
                    } else {
                        self.wizard.next_step();
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
            let base = self.worktree_create.selected_base_branch();

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
                        skip_permissions: self.wizard.skip_permissions,
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

        for branch_name in branches {
            // Try to delete the branch
            match Branch::delete(&self.repo_root, branch_name, false) {
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

        // Header (for branch list screen, render boxed header)
        if matches!(self.screen, Screen::BranchList) {
            self.view_boxed_header(frame, chunks[0]);
        } else {
            self.view_header(frame, chunks[0]);
        }

        // Content
        match self.screen {
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
            Screen::Confirm => render_confirm(&self.confirm, frame, chunks[1]),
            Screen::Error => render_error(&self.error, frame, chunks[1]),
            Screen::Profiles => render_profiles(&self.profiles, frame, chunks[1]),
            Screen::Environment => render_environment(&self.environment, frame, chunks[1]),
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
            .unwrap_or("default");
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
                Constraint::Length(1), // Stats
            ])
            .split(inner);

        // Line 1: Working Directory
        let working_dir_line = Line::from(vec![
            Span::styled("Working Directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(working_dir),
        ]);
        frame.render_widget(Paragraph::new(working_dir_line), inner_chunks[0]);

        // Line 2: Profile
        let profile_line = Line::from(vec![
            Span::styled("Profile(p): ", Style::default().fg(Color::DarkGray)),
            Span::raw(profile),
        ]);
        frame.render_widget(Paragraph::new(profile_line), inner_chunks[1]);

        // Line 3: Filter
        let filtered = self.branch_list.filtered_branches();
        let total = self.branch_list.branches.len();
        let mut filter_spans = vec![Span::styled(
            "Filter(f): ",
            Style::default().fg(Color::DarkGray),
        )];
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

        // Line 4: Stats
        let stats = &self.branch_list.stats;
        let relative_time = self.branch_list.format_relative_time();
        let mut stats_spans = vec![
            Span::styled("Mode(tab): ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                self.branch_list.view_mode.label(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("Local: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.local_count.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("Remote: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.remote_count.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("Worktrees: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.worktree_count.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("Changes: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.changes_count.to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !relative_time.is_empty() {
            stats_spans.push(Span::styled("  ", Style::default()));
            stats_spans.push(Span::styled(
                "Updated: ",
                Style::default().fg(Color::DarkGray),
            ));
            stats_spans.push(Span::styled(
                relative_time,
                Style::default().fg(Color::DarkGray),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(stats_spans)), inner_chunks[3]);
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
                    "[Esc] Exit filter | [Enter] Apply | Type to search"
                } else {
                    // FR-004: Enter: Select, n: New, r: Refresh, c: Cleanup, x: Repair, l: Logs
                    "[Enter] Select | [n] New | [r] Refresh | [c] Cleanup | [x] Repair | [l] Logs"
                }
            }
            Screen::WorktreeCreate => "[Enter] Next | [Esc] Back",
            Screen::Settings => "[Tab] Category | [Esc] Back",
            Screen::Logs => "[f] Filter | [/] Search | [Esc] Back",
            Screen::Help => "[Esc] Close | [Up/Down] Scroll",
            Screen::Confirm => "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel",
            Screen::Error => "[Enter/Esc] Close | [Up/Down] Scroll",
            Screen::Profiles => {
                "[Enter] Activate | [n] New | [d] Delete | [e] Edit env | [Esc] Back"
            }
            Screen::Environment => {
                "[n] New | [e] Edit | [d] Delete | [v] Toggle visibility | [Esc] Back"
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
}

/// Run the TUI application
/// Returns agent launch configuration if wizard completed, None otherwise
pub fn run() -> Result<Option<AgentLaunchConfig>, GwtError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut model = Model::new();

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
                // Wizard has priority when visible
                let msg = if model.wizard.visible {
                    match key.code {
                        KeyCode::Esc => Some(Message::WizardBack),
                        KeyCode::Enter => Some(Message::WizardConfirm),
                        KeyCode::Up => Some(Message::WizardPrev),
                        KeyCode::Down => Some(Message::WizardNext),
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
                } else {
                    // Normal key handling
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Message::CtrlC),
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
                            // In filter mode, 'n' goes to filter input
                            if matches!(model.screen, Screen::BranchList)
                                && !model.branch_list.filter_mode
                            {
                                // Open wizard for new branch (FR-008)
                                Some(Message::OpenWizardNewBranch)
                            } else if matches!(model.screen, Screen::Profiles) {
                                // Create new profile
                                model.profiles.enter_create_mode();
                                None
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
                                                    // Exclude remote branches and current branch
                                                    b.branch_type == BranchType::Local
                                                        && !b.is_current
                                                })
                                                .unwrap_or(false)
                                        })
                                        .cloned()
                                        .collect();

                                    if cleanup_branches.is_empty() {
                                        let excluded = model.branch_list.selected_branches.len();
                                        model.status_message = Some(format!(
                                            "{} branch(es) excluded (remote or current).",
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
                        (KeyCode::Up, _) => Some(Message::SelectPrev),
                        (KeyCode::Down, _) => Some(Message::SelectNext),
                        (KeyCode::PageUp, _) => Some(Message::PageUp),
                        (KeyCode::PageDown, _) => Some(Message::PageDown),
                        (KeyCode::Home, _) => Some(Message::GoHome),
                        (KeyCode::End, _) => Some(Message::GoEnd),
                        (KeyCode::Enter, _) => Some(Message::Enter),
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
