use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use crossterm::event::KeyEvent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    Terminal,
};

use crate::event::{self, EventLoop, TuiEvent};
use crate::input::keybind::{self, KeyAction};
use crate::state::{AppMode, TuiState};
use crate::ui;

/// Main TUI application.
pub struct App {
    state: TuiState,
    vt_parsers: HashMap<String, vt100::Parser>,
    should_quit: bool,
    #[allow(dead_code)]
    repo_root: PathBuf,
}

impl App {
    /// Create a new App instance.
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            state: TuiState::new(),
            vt_parsers: HashMap::new(),
            should_quit: false,
            repo_root,
        }
    }

    /// Run the main event loop.
    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_pty_tx, pty_rx) = event::pty_output_channel();
        let mut event_loop = EventLoop::new(pty_rx);

        loop {
            self.render(terminal)?;

            let evt = event_loop.next()?;
            match evt {
                TuiEvent::Key(key) => self.handle_key(key),
                TuiEvent::Resize(w, h) => self.handle_resize(w, h),
                TuiEvent::PtyOutput { pane_id, data } => self.handle_pty_output(&pane_id, &data),
                TuiEvent::Tick => {} // UI refresh handled by render
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn render(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = Layout::vertical([
                Constraint::Length(1), // Tab bar
                Constraint::Min(1),   // Terminal view
                Constraint::Length(1), // Status bar
            ])
            .split(area);

            let mut tab_buf = frame.buffer_mut().clone();
            ui::tab_bar::render(&mut tab_buf, layout[0], &self.state);

            // Render terminal view for active pane
            if let Some(tab) = self.state.active_tab_info() {
                if let Some(parser) = self.vt_parsers.get(&tab.pane_id) {
                    ui::terminal_view::render(&mut tab_buf, layout[1], parser.screen());
                }
            }

            ui::status_bar::render(&mut tab_buf, layout[2], &self.state);
        })?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let action = keybind::process_key(&mut self.state.prefix_state, key);
        match action {
            KeyAction::Quit => self.should_quit = true,
            KeyAction::NextWindow => self.state.next_tab(),
            KeyAction::PrevWindow => self.state.prev_tab(),
            KeyAction::SwitchTab(n) => self.state.set_active_tab(n),
            KeyAction::ToggleManagement => {
                self.state.mode = match self.state.mode {
                    AppMode::Management => AppMode::Normal,
                    _ => AppMode::Management,
                };
            }
            KeyAction::ScrollMode => {
                self.state.mode = AppMode::ScrollMode;
            }
            KeyAction::Passthrough(_key) => {
                // TODO: wire to PaneManager::write_input
            }
            _ => {} // Other actions to be wired in later phases
        }
    }

    fn handle_resize(&mut self, _width: u16, _height: u16) {
        // TODO: wire to PaneManager::resize_all
    }

    fn handle_pty_output(&mut self, pane_id: &str, data: &[u8]) {
        if let Some(parser) = self.vt_parsers.get_mut(pane_id) {
            parser.process(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_creation() {
        let app = App::new(PathBuf::from("/tmp/test"));
        assert!(!app.should_quit);
        assert_eq!(app.state.tab_count(), 0);
    }

    #[test]
    fn test_handle_quit() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        assert!(!app.should_quit);
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('g'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(app.should_quit);
    }

    #[test]
    fn test_handle_pty_output() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        app.vt_parsers
            .insert("pane-1".to_string(), vt100::Parser::new(24, 80, 0));
        app.handle_pty_output("pane-1", b"Hello");
        let screen = app.vt_parsers.get("pane-1").unwrap().screen();
        let cell = screen.cell(0, 0).unwrap();
        assert_eq!(cell.contents(), "H");
    }

    #[test]
    fn test_handle_tab_navigation() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        use crate::state::{PaneStatusIndicator, TabInfo};
        use gwt_core::terminal::AgentColor;

        app.state.add_tab(TabInfo {
            pane_id: "p1".into(),
            name: "tab1".into(),
            color: AgentColor::Green,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });
        app.state.add_tab(TabInfo {
            pane_id: "p2".into(),
            name: "tab2".into(),
            color: AgentColor::Blue,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });

        assert_eq!(app.state.active_tab, 1);

        // Ctrl+G, [ -> prev window
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('g'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('['),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.state.active_tab, 0);
    }
}
