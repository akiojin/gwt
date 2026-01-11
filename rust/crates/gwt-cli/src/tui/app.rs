//! TUI Application with Elm Architecture

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_core::error::GwtError;
use ratatui::{prelude::*, widgets::*};
use std::io;
use std::time::{Duration, Instant};

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
}

/// Screen types
#[derive(Clone, Debug)]
pub enum Screen {
    BranchList,
    WorktreeCreate,
    Settings,
    Help,
}

/// Messages (Events in Elm Architecture)
#[derive(Debug)]
pub enum Message {
    Quit,
    CtrlC,
    NavigateTo(Screen),
    NavigateBack,
    Tick,
}

impl Model {
    /// Create a new model
    pub fn new() -> Self {
        Self {
            should_quit: false,
            ctrl_c_count: 0,
            last_ctrl_c: None,
            screen: Screen::BranchList,
            screen_stack: Vec::new(),
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
            }
            Message::NavigateTo(screen) => {
                self.screen_stack.push(self.screen.clone());
                self.screen = screen;
            }
            Message::NavigateBack => {
                if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                }
            }
            Message::Tick => {
                // Reset Ctrl+C counter after timeout
                if let Some(last) = self.last_ctrl_c {
                    if Instant::now().duration_since(last) > Duration::from_secs(2) {
                        self.ctrl_c_count = 0;
                    }
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
        let header = Block::default()
            .borders(Borders::ALL)
            .title(" gwt - Git Worktree Manager ");
        frame.render_widget(header, chunks[0]);

        // Content
        match self.screen {
            Screen::BranchList => self.view_branch_list(frame, chunks[1]),
            Screen::WorktreeCreate => self.view_worktree_create(frame, chunks[1]),
            Screen::Settings => self.view_settings(frame, chunks[1]),
            Screen::Help => self.view_help(frame, chunks[1]),
        }

        // Footer
        let ctrl_c_hint = if self.ctrl_c_count > 0 {
            " | Press Ctrl+C again to quit"
        } else {
            ""
        };
        let footer_text = format!(
            " [q] Quit | [?] Help | [Enter] Select | [Esc] Back{}",
            ctrl_c_hint
        );
        let footer = Paragraph::new(footer_text).block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }

    fn view_branch_list(&self, frame: &mut Frame, area: Rect) {
        let items = vec![
            ListItem::new("  main"),
            ListItem::new("  develop"),
            ListItem::new("* feature/current-branch"),
        ];
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Branches "))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_widget(list, area);
    }

    fn view_worktree_create(&self, frame: &mut Frame, area: Rect) {
        let text = Paragraph::new("Create Worktree Wizard")
            .block(Block::default().borders(Borders::ALL).title(" Create "));
        frame.render_widget(text, area);
    }

    fn view_settings(&self, frame: &mut Frame, area: Rect) {
        let text = Paragraph::new("Settings")
            .block(Block::default().borders(Borders::ALL).title(" Settings "));
        frame.render_widget(text, area);
    }

    fn view_help(&self, frame: &mut Frame, area: Rect) {
        let help_text = vec![
            "Keyboard Shortcuts:",
            "",
            "  Arrow Up/Down  - Navigate",
            "  Enter          - Select/Confirm",
            "  Esc            - Go back",
            "  q              - Quit",
            "  h or ?         - Show this help",
            "  PageUp/Down    - Scroll page",
            "  Home/End       - Jump to start/end",
            "",
            "Press any key to close this help.",
        ];
        let text = Paragraph::new(help_text.join("\n"))
            .block(Block::default().borders(Borders::ALL).title(" Help "));
        frame.render_widget(text, area);
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
                let msg = match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Message::CtrlC),
                    (KeyCode::Char('q'), _) => Some(Message::Quit),
                    (KeyCode::Esc, _) => Some(Message::NavigateBack),
                    (KeyCode::Char('?') | KeyCode::Char('h'), _) => {
                        Some(Message::NavigateTo(Screen::Help))
                    }
                    _ => None,
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
