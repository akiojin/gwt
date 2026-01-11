//! TUI Application with Elm Architecture

#![allow(dead_code)] // TUI application components for future expansion

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::error::GwtError;
use gwt_core::git::Branch;
use gwt_core::worktree::WorktreeManager;
use ratatui::{prelude::*, widgets::*};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::screens::{
    BranchItem, BranchListState, HelpState, LogsState, SettingsState, WorktreeCreateState,
    ConfirmState, ErrorState, ProfilesState, EnvironmentState, WizardState,
    render_branch_list, render_help, render_logs, render_settings, render_worktree_create,
    render_confirm, render_error, render_profiles, render_environment, render_wizard,
};

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
    /// Is offline
    is_offline: bool,
    /// Active worktree count
    active_count: usize,
    /// Total branch count
    total_count: usize,
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
            is_offline: false,
            active_count: 0,
            total_count: 0,
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
                let branch_items: Vec<BranchItem> = branches
                    .iter()
                    .map(|b| BranchItem::from_branch(b, &worktrees))
                    .collect();

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
        if let Ok(settings) = gwt_core::config::Settings::load(&self.repo_root) {
            self.settings = SettingsState::new().with_settings(settings);
        }

        // Load logs
        let log_dir = self.repo_root.join(".gwt").join("logs");
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
                        self.status_message = None;
                    }
                }
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
                    // Open wizard for selected branch (FR-007)
                    self.update(Message::OpenWizard);
                }
                Screen::WorktreeCreate => {
                    if self.worktree_create.is_confirm_step() {
                        // Create the worktree
                        self.create_worktree();
                    } else {
                        self.worktree_create.next_step();
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
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode {
                    // Filter mode - add character to filter
                    self.branch_list.filter_push(c);
                }
            }
            Message::Backspace => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.delete_char();
                } else if matches!(self.screen, Screen::BranchList) && self.branch_list.filter_mode {
                    self.branch_list.filter_pop();
                }
            }
            Message::CursorLeft => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_left();
                }
            }
            Message::CursorRight => {
                if matches!(self.screen, Screen::WorktreeCreate) {
                    self.worktree_create.cursor_right();
                }
            }
            Message::RefreshData => {
                self.refresh_data();
            }
            Message::Tab => if let Screen::Settings = self.screen { self.settings.next_category() },
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
                if matches!(self.screen, Screen::BranchList) {
                    self.branch_list.cycle_view_mode();
                }
            }
            Message::ToggleSelection | Message::Space => {
                if matches!(self.screen, Screen::BranchList) {
                    self.branch_list.toggle_selection();
                }
            }
            Message::OpenWizard => {
                // Open wizard for selected branch (FR-044)
                if let Some(branch) = self.branch_list.selected_branch() {
                    if branch.has_worktree {
                        self.status_message = Some(format!(
                            "Worktree already exists: {}",
                            branch.worktree_path.as_deref().unwrap_or("")
                        ));
                    } else {
                        self.wizard.open_for_branch(&branch.name);
                    }
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
                    if self.wizard.is_complete() {
                        // Start worktree creation with wizard settings
                        let branch_name = if self.wizard.is_new_branch {
                            self.wizard.full_branch_name()
                        } else {
                            self.wizard.branch_name.clone()
                        };
                        self.worktree_create.branch_name = branch_name;
                        self.worktree_create.branch_name_cursor = self.worktree_create.branch_name.len();
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

    /// Create worktree from wizard state
    fn create_worktree(&mut self) {
        if let Ok(manager) = WorktreeManager::new(&self.repo_root) {
            let branch = &self.worktree_create.branch_name;
            let base = self.worktree_create.selected_base_branch();

            let result = if self.worktree_create.create_new_branch {
                manager.create_new_branch(branch, base)
            } else {
                manager.create_for_branch(branch)
            };

            match result {
                Ok(wt) => {
                    self.status_message = Some(format!("Created worktree: {}", wt.path.display()));
                    self.refresh_data();
                    self.screen = Screen::BranchList;
                    self.screen_stack.clear();
                }
                Err(e) => {
                    self.worktree_create.error_message = Some(e.to_string());
                }
            }
        }
    }

    /// View function (Elm Architecture)
    pub fn view(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Content
                Constraint::Length(3), // Footer
            ])
            .split(frame.area());

        // Header
        self.view_header(frame, chunks[0]);

        // Content
        match self.screen {
            Screen::BranchList => render_branch_list(&self.branch_list, frame, chunks[1]),
            Screen::WorktreeCreate => render_worktree_create(&self.worktree_create, frame, chunks[1]),
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

    fn view_header(&self, frame: &mut Frame, area: Rect) {
        let version = env!("CARGO_PKG_VERSION");
        let offline_indicator = if self.is_offline { " [OFFLINE]" } else { "" };

        let profile = self.branch_list.active_profile.as_deref().unwrap_or("default");

        // Match TypeScript format: gwt - Branch Selection v{version} | Profile(p): {name}
        let title = format!(
            " gwt - Branch Selection v{} | Profile(p): {} {}",
            version,
            profile,
            offline_indicator
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
                    // Match TypeScript: [r] Refresh | [c] Cleanup | [x] Repair | [l] Logs
                    "[r] Refresh | [c] Cleanup | [x] Repair | [l] Logs"
                }
            }
            Screen::WorktreeCreate => "[Enter] Next | [Esc] Back",
            Screen::Settings => "[Tab] Category | [Esc] Back",
            Screen::Logs => "[f] Filter | [/] Search | [Esc] Back",
            Screen::Help => "[Esc] Close | [Up/Down] Scroll",
            Screen::Confirm => "[Left/Right] Select | [Enter] Confirm | [Esc] Cancel",
            Screen::Error => "[Enter/Esc] Close | [Up/Down] Scroll",
            Screen::Profiles => "[Enter] Activate | [n] New | [d] Delete | [e] Edit env | [Esc] Back",
            Screen::Environment => "[n] New | [e] Edit | [d] Delete | [v] Toggle visibility | [Esc] Back",
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
pub fn run() -> Result<(), GwtError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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
                        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
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
                            Some(Message::Char(if key.code == KeyCode::Char('?') { '?' } else { 'h' }))
                        }
                    }
                    (KeyCode::Char('n'), KeyModifiers::NONE) => {
                        // In filter mode, 'n' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            // Open wizard for new branch (FR-008)
                            Some(Message::OpenWizardNewBranch)
                        } else {
                            Some(Message::Char('n'))
                        }
                    }
                    (KeyCode::Char('s'), KeyModifiers::NONE) => {
                        // In filter mode, 's' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            Some(Message::NavigateTo(Screen::Settings))
                        } else {
                            Some(Message::Char('s'))
                        }
                    }
                    (KeyCode::Char('r'), KeyModifiers::NONE) => {
                        // In filter mode, 'r' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            Some(Message::RefreshData)
                        } else {
                            Some(Message::Char('r'))
                        }
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => {
                        // Cleanup command - not yet implemented fully
                        // In filter mode, 'c' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            // TODO: Show cleanup dialog
                            model.status_message = Some("Cleanup not yet implemented".to_string());
                            None
                        } else {
                            Some(Message::Char('c'))
                        }
                    }
                    (KeyCode::Char('x'), KeyModifiers::NONE) => {
                        // Repair worktrees command
                        // In filter mode, 'x' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            // TODO: Show repair dialog
                            model.status_message = Some("Repair not yet implemented".to_string());
                            None
                        } else {
                            Some(Message::Char('x'))
                        }
                    }
                    (KeyCode::Char('p'), KeyModifiers::NONE) => {
                        // In filter mode, 'p' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
                            Some(Message::NavigateTo(Screen::Profiles))
                        } else {
                            Some(Message::Char('p'))
                        }
                    }
                    (KeyCode::Char('l'), KeyModifiers::NONE) => {
                        // In filter mode, 'l' goes to filter input
                        if matches!(model.screen, Screen::BranchList) && !model.branch_list.filter_mode {
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

    // Cleanup on exit - check for orphaned worktrees
    if let Ok(manager) = WorktreeManager::new(&model.repo_root) {
        let orphans = manager.detect_orphans();
        if !orphans.is_empty() {
            // Attempt to prune automatically
            let _ = manager.prune();
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
