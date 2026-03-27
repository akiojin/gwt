use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
    Terminal,
};

use crate::event::{self, EventLoop, PtyOutputSender, TuiEvent};
use crate::input::keybind::{self, KeyAction};
use crate::state::{AppMode, PaneStatusIndicator, TabInfo, TabType, TuiState};
use crate::ui;
use gwt_core::terminal::{manager::PaneManager, AgentColor, BuiltinLaunchConfig};

pub struct App {
    state: TuiState,
    pane_manager: PaneManager,
    vt_parsers: HashMap<String, vt100::Parser>,
    pty_tx: PtyOutputSender,
    should_quit: bool,
    repo_root: PathBuf,
    last_ctrl_c: Option<Instant>,
    terminal_rows: u16,
    terminal_cols: u16,
}

impl App {
    pub fn new(repo_root: PathBuf) -> Self {
        let (pty_tx, _) = event::pty_output_channel();
        Self {
            state: TuiState::new(),
            pane_manager: PaneManager::new(),
            vt_parsers: HashMap::new(),
            pty_tx,
            should_quit: false,
            repo_root,
            last_ctrl_c: None,
            terminal_rows: 24,
            terminal_cols: 80,
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (pty_tx, pty_rx) = event::pty_output_channel();
        self.pty_tx = pty_tx;
        let mut event_loop = EventLoop::new(pty_rx);

        // Store initial terminal size
        let size = terminal.size()?;
        self.terminal_rows = size.height.saturating_sub(2); // minus tab bar + status bar
        self.terminal_cols = size.width;

        // Auto-open first shell tab
        self.spawn_shell_tab()?;

        loop {
            self.render(terminal)?;

            let evt = event_loop.next()?;
            match evt {
                TuiEvent::Key(key) => self.handle_key(key)?,
                TuiEvent::Resize(w, h) => self.handle_resize(w, h)?,
                TuiEvent::PtyOutput { pane_id, data } => self.handle_pty_output(&pane_id, &data),
                TuiEvent::Tick => {}
            }

            if self.should_quit {
                let _ = self.pane_manager.kill_all();
                return Ok(());
            }
        }
    }

    fn spawn_shell_tab(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let config = BuiltinLaunchConfig {
            command: shell,
            args: vec!["-l".to_string()],
            working_dir: self.repo_root.clone(),
            branch_name: "shell".to_string(),
            agent_name: "shell".to_string(),
            agent_color: AgentColor::White,
            env_vars: HashMap::new(),
            terminal_shell: None,
            interactive: true,
            windows_force_utf8: false,
        };

        let pane_id = self.pane_manager.spawn_shell(
            &self.repo_root,
            config,
            self.terminal_rows,
            self.terminal_cols,
        )?;

        // Start PTY reader thread
        self.start_pty_reader(&pane_id)?;

        // Register vt100 parser
        self.vt_parsers.insert(
            pane_id.clone(),
            vt100::Parser::new(self.terminal_rows, self.terminal_cols, 1000),
        );

        // Add tab to state
        self.state.add_tab(TabInfo {
            pane_id,
            name: "shell".to_string(),
            tab_type: TabType::Shell,
            color: AgentColor::White,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });

        Ok(())
    }

    fn start_pty_reader(&self, pane_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let pane = self
            .pane_manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or("pane not found")?;
        let mut reader = pane.take_reader()?;
        let tx = self.pty_tx.clone();
        let id = pane_id.to_string();

        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send((id.clone(), buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(())
    }

    // Need &mut self for pane_manager, but also need &self for pane_id lookup.
    // Use index-based access instead.
    fn start_pty_reader_by_index(
        pane_manager: &PaneManager,
        pane_id: &str,
        tx: &PtyOutputSender,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pane = pane_manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or("pane not found")?;
        let mut reader = pane.take_reader()?;
        let tx = tx.clone();
        let id = pane_id.to_string();

        std::thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send((id.clone(), buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(())
    }

    fn render(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

            let buf = frame.buffer_mut();
            ui::tab_bar::render(buf, layout[0], &self.state);

            if self.state.tabs.is_empty() {
                render_welcome(buf, layout[1]);
            } else if let Some(tab) = self.state.active_tab_info() {
                if let Some(parser) = self.vt_parsers.get(&tab.pane_id) {
                    ui::terminal_view::render(buf, layout[1], parser.screen());
                }
            }

            ui::status_bar::render(buf, layout[2], &self.state);
        })?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
        // Ctrl+C handling depends on active tab type
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let is_agent_tab = self
                .state
                .active_tab_info()
                .map(|t| t.tab_type == TabType::Agent)
                .unwrap_or(false);

            if is_agent_tab {
                // Agent tab: always forward Ctrl+C to PTY (never quit)
                self.write_to_active_pty(&[0x03])?;
                self.last_ctrl_c = None;
                return Ok(());
            }

            // Shell tab or no tab: double-tap within 500ms quits
            if let Some(last) = self.last_ctrl_c {
                if last.elapsed().as_millis() < 500 {
                    self.should_quit = true;
                    return Ok(());
                }
            }
            self.last_ctrl_c = Some(Instant::now());
            self.write_to_active_pty(&[0x03])?;
            return Ok(());
        }

        // Reset Ctrl+C timer on any other key
        self.last_ctrl_c = None;

        let action = keybind::process_key(&mut self.state.prefix_state, key);
        match action {
            KeyAction::Quit => self.should_quit = true,
            KeyAction::NewShellWindow => self.spawn_shell_tab()?,
            KeyAction::NextWindow => self.state.next_tab(),
            KeyAction::PrevWindow => self.state.prev_tab(),
            KeyAction::SwitchTab(n) => self.state.set_active_tab(n),
            KeyAction::CloseWindow => {
                if let Some(tab) = self.state.active_tab_info() {
                    let pane_id = tab.pane_id.clone();
                    self.vt_parsers.remove(&pane_id);
                    self.pane_manager.close_pane(self.state.active_tab);
                    self.state.remove_tab(self.state.active_tab);
                }
            }
            KeyAction::ToggleManagement => {
                self.state.mode = match self.state.mode {
                    AppMode::Management => AppMode::Normal,
                    _ => AppMode::Management,
                };
            }
            KeyAction::ScrollMode => {
                self.state.mode = AppMode::ScrollMode;
            }
            KeyAction::Passthrough(key) => {
                let bytes = key_event_to_bytes(&key);
                if !bytes.is_empty() {
                    self.write_to_active_pty(&bytes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn write_to_active_pty(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(tab) = self.state.active_tab_info() {
            let pane_id = tab.pane_id.clone();
            if let Some(pane) = self.pane_manager.pane_mut_by_id(&pane_id) {
                pane.write_input(data)?;
            }
        }
        Ok(())
    }

    fn handle_resize(&mut self, width: u16, height: u16) -> Result<(), Box<dyn std::error::Error>> {
        let rows = height.saturating_sub(2); // minus tab bar + status bar
        self.terminal_rows = rows;
        self.terminal_cols = width;
        self.pane_manager.resize_all(rows, width)?;
        // Resize vt100 parsers too
        for parser in self.vt_parsers.values_mut() {
            parser.set_size(rows, width);
        }
        Ok(())
    }

    fn handle_pty_output(&mut self, pane_id: &str, data: &[u8]) {
        // Feed to vt100 parser for rendering
        if let Some(parser) = self.vt_parsers.get_mut(pane_id) {
            parser.process(data);
        }
        // Feed to scrollback
        if let Some(pane) = self.pane_manager.pane_mut_by_id(pane_id) {
            let _ = pane.process_bytes(data);
        }
    }
}

/// Convert a crossterm KeyEvent to bytes for PTY input.
fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
                let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                if ctrl_byte <= 26 {
                    return vec![ctrl_byte];
                }
            }
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    }
}

fn render_welcome(buf: &mut Buffer, area: Rect) {
    if area.height < 8 || area.width < 40 {
        return;
    }

    let center_y = area.y + area.height / 2 - 4;
    let max_width = 40u16.min(area.width);
    let text_x = area.x + area.width / 2 - max_width / 2;
    let text_area = Rect::new(text_x, center_y, max_width, 9);

    let lines = vec![
        Line::from(Span::styled(
            "Welcome to gwt",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "No agents running. Get started:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Ctrl+G, c  ", Style::default().fg(Color::Yellow)),
            Span::styled("Open shell", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+G, n  ", Style::default().fg(Color::Yellow)),
            Span::styled("Launch agent", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+G, q  ", Style::default().fg(Color::Yellow)),
            Span::styled("Quit", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Ctrl+C x2  Quit immediately",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    Paragraph::new(lines).render(text_area, buf);
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
    fn test_ctrl_c_double_tap_quits() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        // No PTY, so write_to_active_pty is a no-op
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_c.clone()).unwrap();
        assert!(!app.should_quit); // first tap doesn't quit
        app.handle_key(ctrl_c).unwrap();
        assert!(app.should_quit); // second tap within 500ms quits
    }

    #[test]
    fn test_ctrl_c_single_tap_does_not_quit() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_c).unwrap();
        assert!(!app.should_quit);
    }

    #[test]
    fn test_quit_via_prefix() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap();
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
    fn test_key_event_to_bytes_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"a");
    }

    #[test]
    fn test_key_event_to_bytes_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"\r");
    }

    #[test]
    fn test_key_event_to_bytes_ctrl_a() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(&key), vec![0x01]);
    }

    #[test]
    fn test_key_event_to_bytes_arrow_up() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(&key), b"\x1b[A");
    }

    #[test]
    fn test_handle_tab_navigation() {
        let mut app = App::new(PathBuf::from("/tmp/test"));

        app.state.add_tab(TabInfo {
            pane_id: "p1".into(),
            name: "tab1".into(),
            tab_type: TabType::Shell,
            color: AgentColor::Green,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });
        app.state.add_tab(TabInfo {
            pane_id: "p2".into(),
            name: "tab2".into(),
            tab_type: TabType::Shell,
            color: AgentColor::Blue,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });

        assert_eq!(app.state.active_tab, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.state.active_tab, 0);
    }
}
