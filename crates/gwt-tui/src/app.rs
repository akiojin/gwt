//! TUI application: integrates PaneManager, PTY I/O, key handling, and rendering

use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwt_core::terminal::{
    manager::PaneManager, AgentColor, BuiltinLaunchConfig,
};

use crate::{
    event::PtyOutputSender,
    state::{AppMode, ScrollState, TabInfo, TabStatus, TuiState},
};

/// Key actions produced by the key mapper.
#[derive(Debug)]
pub enum KeyAction {
    /// Quit the application.
    Quit,
    /// Create a new shell tab.
    NewShellWindow,
    /// Pass key through to the active PTY.
    Passthrough(KeyEvent),
    /// Switch to next tab.
    NextTab,
    /// Switch to previous tab.
    PrevTab,
    /// Enter scroll mode.
    EnterScrollMode,
    /// Scroll up one page (scroll mode only).
    ScrollUp,
    /// Scroll down one page (scroll mode only).
    ScrollDown,
    /// Scroll to top (scroll mode only).
    ScrollHome,
    /// Scroll to bottom / exit scroll mode.
    ScrollEnd,
    /// Exit scroll mode without scrolling.
    ExitScrollMode,
    /// No action.
    None,
}

/// Per-pane vt100 parser for terminal emulation.
pub struct PaneParser {
    pub parser: vt100::Parser,
}

impl PaneParser {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 1000),
        }
    }
}

/// The main TUI application.
pub struct App {
    pub state: TuiState,
    pub pane_manager: PaneManager,
    pub repo_root: PathBuf,
    pub pty_tx: PtyOutputSender,
    /// Per-pane vt100 parsers keyed by pane_id.
    pub parsers: HashMap<String, PaneParser>,
    /// Terminal dimensions (rows, cols).
    pub term_size: (u16, u16),
    /// Whether the app should quit.
    pub should_quit: bool,
}

impl App {
    /// Create a new App instance.
    pub fn new(repo_root: PathBuf, pty_tx: PtyOutputSender, rows: u16, cols: u16) -> Self {
        Self {
            state: TuiState::new(),
            pane_manager: PaneManager::new(),
            repo_root,
            pty_tx,
            parsers: HashMap::new(),
            term_size: (rows, cols),
            should_quit: false,
        }
    }

    /// Map a key event to a KeyAction based on current mode.
    pub fn map_key(&self, key: KeyEvent) -> KeyAction {
        // Ctrl-C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return KeyAction::Quit;
        }

        match self.state.mode {
            AppMode::ScrollMode => self.map_scroll_key(key),
            AppMode::Normal => self.map_normal_key(key),
            AppMode::Management => {
                // Management mode: pass through for now
                KeyAction::Passthrough(key)
            }
        }
    }

    fn map_normal_key(&self, key: KeyEvent) -> KeyAction {
        // Ctrl-B + c => new shell, Ctrl-B + n => next tab, Ctrl-B + p => prev tab
        // For simplicity, use Ctrl-T for new tab, Ctrl-N/Ctrl-P for tab switching
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('t') => return KeyAction::NewShellWindow,
                KeyCode::Char('n') => return KeyAction::NextTab,
                KeyCode::Char('p') => return KeyAction::PrevTab,
                _ => {}
            }
        }

        // PageUp enters scroll mode
        if key.code == KeyCode::PageUp {
            return KeyAction::EnterScrollMode;
        }

        KeyAction::Passthrough(key)
    }

    fn map_scroll_key(&self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::PageUp => KeyAction::ScrollUp,
            KeyCode::PageDown => KeyAction::ScrollDown,
            KeyCode::Home => KeyAction::ScrollHome,
            KeyCode::End => KeyAction::ScrollEnd,
            KeyCode::Esc => KeyAction::ExitScrollMode,
            _ => KeyAction::None,
        }
    }

    /// Handle a mapped key action. Returns true if the app should continue.
    pub fn handle_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Quit => {
                self.should_quit = true;
            }
            KeyAction::NewShellWindow => {
                let _ = self.spawn_shell_tab();
            }
            KeyAction::Passthrough(key) => {
                self.handle_passthrough(key);
            }
            KeyAction::NextTab => {
                self.pane_manager.next_tab();
                if !self.state.tabs.is_empty() {
                    self.state.active_tab =
                        (self.state.active_tab + 1) % self.state.tabs.len();
                }
            }
            KeyAction::PrevTab => {
                self.pane_manager.prev_tab();
                if !self.state.tabs.is_empty() {
                    self.state.active_tab = (self.state.active_tab + self.state.tabs.len() - 1)
                        % self.state.tabs.len();
                }
            }
            KeyAction::EnterScrollMode => {
                self.state.mode = AppMode::ScrollMode;
                // Start with one page up
                self.scroll_up();
            }
            KeyAction::ScrollUp => {
                self.scroll_up();
            }
            KeyAction::ScrollDown => {
                self.scroll_down();
            }
            KeyAction::ScrollHome => {
                self.scroll_home();
            }
            KeyAction::ScrollEnd | KeyAction::ExitScrollMode => {
                self.state.scroll_state = ScrollState::Live;
                self.state.mode = AppMode::Normal;
            }
            KeyAction::None => {}
        }
    }

    /// Spawn a new shell tab. Returns the pane_id on success.
    pub fn spawn_shell_tab(&mut self) -> Result<String, gwt_core::terminal::TerminalError> {
        let (rows, cols) = self.term_size;
        let config = BuiltinLaunchConfig {
            command: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
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

        let pane_id = self.pane_manager.spawn_shell(&self.repo_root, config, rows, cols)?;

        // Register vt100 parser for this pane
        self.parsers
            .insert(pane_id.clone(), PaneParser::new(rows, cols));

        // Add tab info
        self.state.tabs.push(TabInfo {
            pane_id: pane_id.clone(),
            name: "shell".to_string(),
            color: AgentColor::White,
            status: TabStatus::Running,
            branch: "shell".to_string(),
        });
        self.state.active_tab = self.state.tabs.len() - 1;

        // Spawn PTY reader thread: take the reader from the pane we just created
        let pane = self
            .pane_manager
            .pane_mut_by_id(&pane_id)
            .ok_or_else(|| gwt_core::terminal::TerminalError::PtyIoError {
                details: format!("pane {pane_id} not found after spawn"),
            })?;
        let reader = pane.take_reader()?;
        let tx = self.pty_tx.clone();
        let id = pane_id.clone();
        std::thread::spawn(move || {
            pty_reader_loop(reader, &id, &tx);
        });

        Ok(pane_id)
    }

    /// Handle PTY output data: feed to vt100 parser and process_bytes on pane.
    pub fn handle_pty_output(&mut self, pane_id: &str, data: &[u8]) {
        // Feed data to the vt100 parser
        if let Some(pp) = self.parsers.get_mut(pane_id) {
            pp.parser.process(data);
        }

        // Also write to pane's scrollback
        if let Some(pane) = self.pane_manager.pane_mut_by_id(pane_id) {
            let _ = pane.process_bytes(data);
        }
    }

    /// Handle terminal resize.
    pub fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.term_size = (rows, cols);

        // Resize all PTY panes
        let _ = self.pane_manager.resize_all(rows, cols);

        // Resize all vt100 parsers
        for pp in self.parsers.values_mut() {
            pp.parser.set_size(rows, cols);
        }
    }

    /// Convert a KeyEvent to bytes for PTY input.
    fn handle_passthrough(&mut self, key: KeyEvent) {
        let bytes = key_event_to_bytes(&key);
        if bytes.is_empty() {
            return;
        }

        let pane_id = match self.state.active_pane_id() {
            Some(id) => id.to_string(),
            None => return,
        };

        if let Some(pane) = self.pane_manager.pane_mut_by_id(&pane_id) {
            let _ = pane.write_input(&bytes);
        }
    }

    // -- Scroll helpers --

    fn scroll_up(&mut self) {
        let page_size = self.term_size.0 as usize;
        let current = self.state.scroll_offset();
        self.state.scroll_state = ScrollState::Scrolled {
            offset: current.saturating_add(page_size),
        };
    }

    fn scroll_down(&mut self) {
        let page_size = self.term_size.0 as usize;
        let current = self.state.scroll_offset();
        let new_offset = current.saturating_sub(page_size);
        if new_offset == 0 {
            self.state.scroll_state = ScrollState::Live;
            self.state.mode = AppMode::Normal;
        } else {
            self.state.scroll_state = ScrollState::Scrolled { offset: new_offset };
        }
    }

    fn scroll_home(&mut self) {
        // Scroll to maximum offset (beginning of scrollback).
        // Use a large value; rendering will clamp to actual scrollback size.
        self.state.scroll_state = ScrollState::Scrolled { offset: usize::MAX };
    }

    /// Get the vt100 screen content for the active pane, accounting for scroll offset.
    pub fn active_screen_rows(&self) -> Vec<String> {
        let pane_id = match self.state.active_pane_id() {
            Some(id) => id,
            None => return vec!["No active pane. Press Ctrl-T to create a shell.".to_string()],
        };

        let parser = match self.parsers.get(pane_id) {
            Some(pp) => &pp.parser,
            None => return vec![],
        };

        let screen = parser.screen();
        let rows = screen.rows(0, screen.size().1);
        let all_rows: Vec<String> = rows.map(|row| row.to_string()).collect();

        let offset = self.state.scroll_offset();
        if offset == 0 {
            // Live mode: show the tail
            all_rows
        } else {
            // Scrolled: shift view up by offset
            let total = all_rows.len();
            let end = total.saturating_sub(offset);
            let start = end.saturating_sub(self.term_size.0 as usize);
            all_rows[start..end].to_vec()
        }
    }
}

/// Background PTY reader loop. Reads from the PTY reader and sends data to the channel.
fn pty_reader_loop(
    mut reader: Box<dyn Read + Send>,
    pane_id: &str,
    tx: &PtyOutputSender,
) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send((pane_id.to_string(), buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Convert a crossterm KeyEvent to raw bytes for PTY input.
pub fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    // Handle Ctrl+key combinations
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            // Ctrl+A = 0x01, Ctrl+B = 0x02, ..., Ctrl+Z = 0x1A
            let ctrl_byte = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
            if ctrl_byte <= 26 {
                return vec![ctrl_byte];
            }
        }
    }

    match key.code {
        KeyCode::Char(c) => {
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
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(1) => b"\x1bOP".to_vec(),
        KeyCode::F(2) => b"\x1bOQ".to_vec(),
        KeyCode::F(3) => b"\x1bOR".to_vec(),
        KeyCode::F(4) => b"\x1bOS".to_vec(),
        KeyCode::F(n) if n >= 5 => {
            // F5-F12 use CSI sequences
            let code = match n {
                5 => "15",
                6 => "17",
                7 => "18",
                8 => "19",
                9 => "20",
                10 => "21",
                11 => "23",
                12 => "24",
                _ => return vec![],
            };
            format!("\x1b[{code}~").into_bytes()
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn make_ctrl_key(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // -- key_event_to_bytes tests --

    #[test]
    fn test_char_to_bytes() {
        let bytes = key_event_to_bytes(&make_key(KeyCode::Char('a')));
        assert_eq!(bytes, vec![b'a']);
    }

    #[test]
    fn test_enter_to_bytes() {
        let bytes = key_event_to_bytes(&make_key(KeyCode::Enter));
        assert_eq!(bytes, vec![b'\r']);
    }

    #[test]
    fn test_backspace_to_bytes() {
        let bytes = key_event_to_bytes(&make_key(KeyCode::Backspace));
        assert_eq!(bytes, vec![0x7f]);
    }

    #[test]
    fn test_tab_to_bytes() {
        let bytes = key_event_to_bytes(&make_key(KeyCode::Tab));
        assert_eq!(bytes, vec![b'\t']);
    }

    #[test]
    fn test_escape_to_bytes() {
        let bytes = key_event_to_bytes(&make_key(KeyCode::Esc));
        assert_eq!(bytes, vec![0x1b]);
    }

    #[test]
    fn test_arrow_keys_to_bytes() {
        assert_eq!(
            key_event_to_bytes(&make_key(KeyCode::Up)),
            b"\x1b[A".to_vec()
        );
        assert_eq!(
            key_event_to_bytes(&make_key(KeyCode::Down)),
            b"\x1b[B".to_vec()
        );
        assert_eq!(
            key_event_to_bytes(&make_key(KeyCode::Right)),
            b"\x1b[C".to_vec()
        );
        assert_eq!(
            key_event_to_bytes(&make_key(KeyCode::Left)),
            b"\x1b[D".to_vec()
        );
    }

    #[test]
    fn test_ctrl_c_to_bytes() {
        let bytes = key_event_to_bytes(&make_ctrl_key('c'));
        assert_eq!(bytes, vec![0x03]);
    }

    #[test]
    fn test_ctrl_d_to_bytes() {
        let bytes = key_event_to_bytes(&make_ctrl_key('d'));
        assert_eq!(bytes, vec![0x04]);
    }

    #[test]
    fn test_unicode_char_to_bytes() {
        let key = make_key(KeyCode::Char('\u{3042}')); // hiragana 'a'
        let bytes = key_event_to_bytes(&key);
        assert_eq!(bytes, "\u{3042}".as_bytes());
    }

    // -- App key mapping tests --

    #[test]
    fn test_ctrl_c_maps_to_quit() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        let action = app.map_key(make_ctrl_key('c'));
        assert!(matches!(action, KeyAction::Quit));
    }

    #[test]
    fn test_ctrl_t_maps_to_new_shell() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        let action = app.map_key(make_ctrl_key('t'));
        assert!(matches!(action, KeyAction::NewShellWindow));
    }

    #[test]
    fn test_regular_key_maps_to_passthrough() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        let action = app.map_key(make_key(KeyCode::Char('x')));
        assert!(matches!(action, KeyAction::Passthrough(_)));
    }

    #[test]
    fn test_pageup_enters_scroll_mode() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        let action = app.map_key(make_key(KeyCode::PageUp));
        assert!(matches!(action, KeyAction::EnterScrollMode));
    }

    #[test]
    fn test_scroll_mode_keys() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.state.mode = AppMode::ScrollMode;

        assert!(matches!(
            app.map_key(make_key(KeyCode::PageUp)),
            KeyAction::ScrollUp
        ));
        assert!(matches!(
            app.map_key(make_key(KeyCode::PageDown)),
            KeyAction::ScrollDown
        ));
        assert!(matches!(
            app.map_key(make_key(KeyCode::Home)),
            KeyAction::ScrollHome
        ));
        assert!(matches!(
            app.map_key(make_key(KeyCode::End)),
            KeyAction::ScrollEnd
        ));
        assert!(matches!(
            app.map_key(make_key(KeyCode::Esc)),
            KeyAction::ExitScrollMode
        ));
    }

    // -- Scroll state tests --

    #[test]
    fn test_scroll_up_increases_offset() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.state.mode = AppMode::ScrollMode;
        app.state.scroll_state = ScrollState::Live;
        app.scroll_up();
        assert_eq!(app.state.scroll_offset(), 24); // page_size = rows
    }

    #[test]
    fn test_scroll_down_decreases_offset() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.state.scroll_state = ScrollState::Scrolled { offset: 48 };
        app.state.mode = AppMode::ScrollMode;
        app.scroll_down();
        assert_eq!(app.state.scroll_offset(), 24);
    }

    #[test]
    fn test_scroll_down_to_zero_returns_to_live() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.state.scroll_state = ScrollState::Scrolled { offset: 10 };
        app.state.mode = AppMode::ScrollMode;
        app.scroll_down();
        assert_eq!(app.state.scroll_state, ScrollState::Live);
        assert_eq!(app.state.mode, AppMode::Normal);
    }

    #[test]
    fn test_scroll_home_sets_max_offset() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.scroll_home();
        assert_eq!(app.state.scroll_offset(), usize::MAX);
    }

    #[test]
    fn test_handle_resize() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.handle_resize(120, 40);
        assert_eq!(app.term_size, (40, 120));
    }

    #[test]
    fn test_active_screen_rows_no_pane() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        let rows = app.active_screen_rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("No active pane"));
    }

    #[test]
    fn test_handle_pty_output_feeds_parser() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        // Add a parser manually (without spawning a real PTY)
        app.parsers
            .insert("test-pane".to_string(), PaneParser::new(24, 80));
        app.handle_pty_output("test-pane", b"Hello, world!");

        let screen = app.parsers["test-pane"].parser.screen();
        let row0 = screen.rows(0, 80).next().unwrap().to_string();
        assert!(row0.contains("Hello, world!"));
    }

    #[test]
    fn test_handle_resize_updates_parsers() {
        let (tx, _rx) = crate::event::pty_output_channel();
        let mut app = App::new(PathBuf::from("/tmp"), tx, 24, 80);
        app.parsers
            .insert("test-pane".to_string(), PaneParser::new(24, 80));

        app.handle_resize(120, 40);

        let size = app.parsers["test-pane"].parser.screen().size();
        assert_eq!(size, (40, 120));
    }
}
