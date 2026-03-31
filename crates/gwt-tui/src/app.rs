//! TUI Application with Elm Architecture (Model / View / Update)

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::Terminal;

use crate::event::{self, EventLoop, TuiEvent};
use crate::input::keybind::{self, KeyAction, PrefixState};
use crate::message::Message;
use crate::model::{ActiveLayer, ErrorEntry, ErrorSeverity, ManagementTab, Model};
use crate::widgets;

/// Tick interval for background polling.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Apply a message to the model (Elm Architecture update function).
pub fn update(model: &mut Model, msg: Message) {
    match msg {
        Message::Quit => {
            model.should_quit = true;
        }
        Message::ToggleLayer => {
            model.toggle_layer();
        }
        Message::SwitchManagementTab(tab) => {
            model.management_tab = tab;
            model.active_layer = ActiveLayer::Management;
        }
        Message::NextSession => {
            model.next_session();
            if !model.session_tabs.is_empty() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::PrevSession => {
            model.prev_session();
            if !model.session_tabs.is_empty() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::SwitchSession(index) => {
            // 1-based → 0-based
            let idx = index.saturating_sub(1);
            model.switch_session(idx);
            if idx < model.session_tabs.len() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::CloseSession => {
            model.close_active_session();
        }
        Message::NewShell => {
            // Phase 2: spawn shell PTY and add session tab
        }
        Message::OpenWizard => {
            // Open wizard for current branch or default
            let branch = model
                .session_tabs
                .get(model.active_session)
                .and_then(|t| t.branch.clone())
                .unwrap_or_default();
            if branch.is_empty() {
                model.wizard = Some(crate::screens::wizard::WizardState::new());
            } else {
                model.wizard = Some(crate::screens::wizard::WizardState::open_for_branch(
                    &branch,
                    vec![],
                ));
            }
        }
        Message::WizardKey(key) => {
            use crossterm::event::KeyCode;
            if let Some(ref mut wiz) = model.wizard {
                match key.code {
                    KeyCode::Up => wiz.select_prev(),
                    KeyCode::Down => wiz.select_next(),
                    KeyCode::Enter => {
                        let action = wiz.confirm();
                        match action {
                            crate::screens::wizard::WizardAction::Complete => {
                                // Build config and launch (Phase 3+)
                                model.wizard = None;
                            }
                            crate::screens::wizard::WizardAction::Cancel => {
                                model.wizard = None;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Esc => {
                        let action = wiz.cancel();
                        if action == crate::screens::wizard::WizardAction::Cancel {
                            model.wizard = None;
                        }
                    }
                    KeyCode::Backspace => wiz.input_backspace(),
                    KeyCode::Char(ch) => wiz.input_char(ch),
                    _ => {}
                }
            }
        }
        Message::KeyInput(key) => {
            // Forward to active screen handler
            match model.active_layer {
                ActiveLayer::Main => {
                    // Phase 2: forward to active pane
                }
                ActiveLayer::Management => {
                    let sub_msg = match model.management_tab {
                        ManagementTab::Branches => {
<<<<<<< HEAD
                            crate::screens::branches::handle_key(&key).map(Message::BranchesMsg)
                        }
                        ManagementTab::Issues => {
                            crate::screens::issues::handle_key(&key).map(Message::IssuesMsg)
=======
                            crate::screens::branches::handle_key(&model.branches_state, &key)
                                .map(Message::BranchesMsg)
                        }
                        ManagementTab::Issues => {
                            crate::screens::issues::handle_key(&model.issues_state, &key)
                                .map(Message::IssuesMsg)
>>>>>>> origin/feature/feature-1776
                        }
                        ManagementTab::Settings => {
                            crate::screens::settings::handle_key(&key).map(Message::SettingsMsg)
                        }
                        ManagementTab::Logs => {
                            crate::screens::logs::handle_key(&key).map(Message::LogsMsg)
                        }
                    };
                    // Recursively apply sub-message if any
                    if let Some(sub_msg) = sub_msg {
                        update(model, sub_msg);
                    }
                }
            }
        }
        Message::MouseInput(_mouse) => {
            // Phase 2: mouse handling
        }
        Message::Resize(w, h) => {
            model.terminal_cols = w;
            model.terminal_rows = h;
        }
        Message::PtyOutput { pane_id, data } => {
            // Feed data to VT100 parser
            if let Some(parser) = model.vt_parsers.get_mut(&pane_id) {
                parser.process(&data);
            }
        }
        Message::Tick => {
            model.apply_background_updates();
        }
        Message::PushError(entry) => {
            model.push_error(entry);
        }
        Message::DismissError => {
            model.dismiss_error();
        }
        // Screen-specific messages
        Message::BranchesMsg(msg) => {
            crate::screens::branches::update(&mut model.branches_state, msg);
        }
        Message::IssuesMsg(msg) => {
            crate::screens::issues::update(&mut model.issues_state, msg);
        }
        Message::SettingsMsg(_msg) => {
            // Phase 3: handle settings messages
        }
        Message::LogsMsg(_msg) => {
            // Phase 3: handle logs messages
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Render the model to the terminal frame (Elm Architecture view function).
pub fn view(model: &Model, frame: &mut Frame) {
    let area = frame.area();
    let layout = Layout::vertical([
        Constraint::Length(1), // Tab bar
        Constraint::Min(1),    // Main area
        Constraint::Length(1), // Status bar
    ])
    .split(area);

    let buf = frame.buffer_mut();

    // Tab bar
    widgets::tab_bar::render(model, buf, layout[0]);

    // Main content area
    match model.active_layer {
        ActiveLayer::Main => {
            if model.session_tabs.is_empty() {
                // Placeholder when no sessions
                let center =
                    centered_text("No sessions. Press Ctrl+G, c for shell or Ctrl+G, n for agent.");
                let text_area = centered_rect(60, 3, layout[1]);
                ratatui::widgets::Widget::render(center, text_area, buf);
            } else {
                // Phase 2: render active session terminal
                let pane_id = &model.session_tabs[model.active_session].pane_id;
                let parser = model.vt_parsers.get(pane_id);
                crate::screens::agent_pane::render(buf, layout[1], parser);
            }
        }
        ActiveLayer::Management => match model.management_tab {
            ManagementTab::Branches => {
                crate::screens::branches::render(&model.branches_state, buf, layout[1]);
            }
            ManagementTab::Issues => {
                crate::screens::issues::render(&model.issues_state, buf, layout[1]);
            }
            ManagementTab::Settings => crate::screens::settings::render(buf, layout[1]),
            ManagementTab::Logs => crate::screens::logs::render(buf, layout[1]),
        },
    }

    // Status bar
    widgets::status_bar::render(model, buf, layout[2]);

    // Overlays (on top of everything)
    if let Some(ref wizard) = model.wizard {
        crate::screens::wizard::render(buf, area, wizard);
    }
    if let Some(ref progress) = model.progress {
        widgets::progress_modal::render(buf, area, progress);
    }
    if !model.error_queue.is_empty() {
        render_error_overlay(buf, area, &model.error_queue[0]);
    }
}

/// Render a simple error overlay.
fn render_error_overlay(buf: &mut Buffer, area: Rect, entry: &ErrorEntry) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

    let modal_width = 60.min(area.width.saturating_sub(4));
    let modal_height = 5.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    Clear.render(modal_area, buf);

    let border_color = match entry.severity {
        ErrorSeverity::Critical => Color::Red,
        ErrorSeverity::Minor => Color::Yellow,
    };

    let para = Paragraph::new(entry.message.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(" Error (Enter to dismiss) "),
        )
        .wrap(Wrap { trim: true });

    ratatui::widgets::Widget::render(para, modal_area, buf);
}

/// Helper: create a centered Paragraph.
fn centered_text(text: &str) -> ratatui::widgets::Paragraph<'_> {
    ratatui::widgets::Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray))
}

/// Helper: create a centered rect within `area`.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).max(1);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

// ---------------------------------------------------------------------------
// Key → Message conversion
// ---------------------------------------------------------------------------

/// Convert a KeyAction to an optional Message.
fn action_to_message(action: KeyAction, key: crossterm::event::KeyEvent) -> Option<Message> {
    match action {
        KeyAction::None => None,
        KeyAction::Forward(k) => Some(Message::KeyInput(k)),
        KeyAction::ToggleLayer => Some(Message::ToggleLayer),
        KeyAction::NextSession => Some(Message::NextSession),
        KeyAction::PrevSession => Some(Message::PrevSession),
        KeyAction::SwitchSession(n) => Some(Message::SwitchSession(n)),
        KeyAction::CloseSession => Some(Message::CloseSession),
        KeyAction::NewShell => Some(Message::NewShell),
        KeyAction::OpenWizard => Some(Message::OpenWizard),
        KeyAction::ShowHelp => {
            // Phase 2: open help screen
            let _ = key;
            None
        }
        KeyAction::Quit => Some(Message::Quit),
    }
}

// ---------------------------------------------------------------------------
// Run (event loop)
// ---------------------------------------------------------------------------

/// Run the TUI application.
pub fn run(repo_root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize model
    let mut model = Model::new(repo_root);

    // PTY output channel
    let (_pty_tx, pty_rx) = event::pty_output_channel();

    // Event loop
    let event_loop = EventLoop::new(pty_rx);
    let mut prefix_state = PrefixState::default();
    let mut last_tick = Instant::now();

    loop {
        // View
        terminal.draw(|f| view(&model, f))?;

        // Event → Message
        let evt = event_loop.next()?;
        let msg = match evt {
            TuiEvent::Key(key) => {
                // When wizard is open, intercept all keys
                if model.wizard.is_some() {
                    Some(Message::WizardKey(key))
                } else if keybind::is_ctrl_c(&key) {
                    if model.handle_ctrl_c() {
                        Some(Message::Quit)
                    } else {
                        // Single Ctrl+C: forward to active pane in Main layer
                        if model.active_layer == ActiveLayer::Main {
                            Some(Message::KeyInput(key))
                        } else {
                            None
                        }
                    }
                } else {
                    let action = keybind::process_key(&mut prefix_state, key);
                    action_to_message(action, key)
                }
            }
            TuiEvent::Mouse(mouse) => Some(Message::MouseInput(mouse)),
            TuiEvent::Resize(w, h) => Some(Message::Resize(w, h)),
            TuiEvent::PtyOutput { pane_id, data } => Some(Message::PtyOutput { pane_id, data }),
            TuiEvent::Tick => {
                if last_tick.elapsed() >= TICK_INTERVAL {
                    last_tick = Instant::now();
                    Some(Message::Tick)
                } else {
                    None
                }
            }
        };

        // Update
        if let Some(msg) = msg {
            update(&mut model, msg);
        }

        // Quit check
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ActiveLayer, ErrorEntry, ErrorSeverity, ManagementTab, SessionStatus, SessionTab,
        SessionTabType,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use gwt_core::terminal::AgentColor;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test"))
    }

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn test_session(name: &str) -> SessionTab {
        SessionTab {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        }
    }

    // -- Update tests ---------------------------------------------------------

    #[test]
    fn update_quit_sets_should_quit() {
        let mut m = test_model();
        update(&mut m, Message::Quit);
        assert!(m.should_quit);
    }

    #[test]
    fn update_toggle_layer() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Management);
        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Main);
    }

    #[test]
    fn update_switch_management_tab() {
        let mut m = test_model();
        update(&mut m, Message::SwitchManagementTab(ManagementTab::Logs));
        assert_eq!(m.management_tab, ManagementTab::Logs);
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_session_navigation() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        m.add_session(test_session("s2"));
        m.add_session(test_session("s3"));

        update(&mut m, Message::SwitchSession(1)); // 1-based
        assert_eq!(m.active_session, 0);

        update(&mut m, Message::NextSession);
        assert_eq!(m.active_session, 1);

        update(&mut m, Message::PrevSession);
        assert_eq!(m.active_session, 0);
    }

    #[test]
    fn update_close_session() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        assert_eq!(m.session_tabs.len(), 1);
        update(&mut m, Message::CloseSession);
        assert!(m.session_tabs.is_empty());
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_resize() {
        let mut m = test_model();
        update(&mut m, Message::Resize(120, 40));
        assert_eq!(m.terminal_cols, 120);
        assert_eq!(m.terminal_rows, 40);
    }

    #[test]
    fn update_pty_output_feeds_parser() {
        let mut m = test_model();
        m.vt_parsers
            .insert("pane-1".to_string(), vt100::Parser::new(24, 80, 0));
        update(
            &mut m,
            Message::PtyOutput {
                pane_id: "pane-1".into(),
                data: b"hello".to_vec(),
            },
        );
        let screen = m.vt_parsers["pane-1"].screen();
        let row = screen.contents_between(0, 0, 0, 5);
        assert_eq!(row, "hello");
    }

    #[test]
    fn update_tick_increments() {
        let mut m = test_model();
        update(&mut m, Message::Tick);
        assert_eq!(m.tick_count, 1);
    }

    #[test]
    fn update_error_push_and_dismiss() {
        let mut m = test_model();
        update(
            &mut m,
            Message::PushError(ErrorEntry {
                message: "fail".into(),
                severity: ErrorSeverity::Critical,
            }),
        );
        assert_eq!(m.error_queue.len(), 1);
        update(&mut m, Message::DismissError);
        assert!(m.error_queue.is_empty());
    }

    // -- Key → Message conversion tests ----------------------------------------

    #[test]
    fn action_to_message_maps_correctly() {
        let dummy_key = make_key(KeyCode::Char('x'), KeyModifiers::NONE);

        assert!(action_to_message(KeyAction::None, dummy_key).is_none());
        assert!(matches!(
            action_to_message(KeyAction::Quit, dummy_key),
            Some(Message::Quit)
        ));
        assert!(matches!(
            action_to_message(KeyAction::ToggleLayer, dummy_key),
            Some(Message::ToggleLayer)
        ));
        assert!(matches!(
            action_to_message(KeyAction::NextSession, dummy_key),
            Some(Message::NextSession)
        ));
        assert!(matches!(
            action_to_message(KeyAction::NewShell, dummy_key),
            Some(Message::NewShell)
        ));
        assert!(matches!(
            action_to_message(KeyAction::SwitchSession(3), dummy_key),
            Some(Message::SwitchSession(3))
        ));
    }

    #[test]
    fn action_forward_produces_key_input() {
        let key = make_key(KeyCode::Char('a'), KeyModifiers::NONE);
        let msg = action_to_message(KeyAction::Forward(key), key);
        assert!(matches!(msg, Some(Message::KeyInput(_))));
    }

    // -- View smoke test -------------------------------------------------------

    #[test]
    fn view_renders_without_panic() {
        let model = test_model();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_with_sessions_renders_without_panic() {
        let mut model = test_model();
        model.add_session(test_session("shell-1"));
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_with_error_overlay_renders() {
        let mut model = test_model();
        model.push_error(ErrorEntry {
            message: "Something went wrong".into(),
            severity: ErrorSeverity::Critical,
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_with_progress_renders() {
        let mut model = test_model();
        model.progress = Some(crate::model::ProgressState {
            title: "Loading...".into(),
            detail: Some("step 1".into()),
            percent: Some(50),
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }
}
