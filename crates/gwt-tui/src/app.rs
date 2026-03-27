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
use crate::input::keybind::{self, Direction, KeyAction};
use crate::state::{AppMode, PaneStatusIndicator, TabInfo, TabType, TuiState};
use crate::ui;
use crate::ui::split_layout::{LayoutTree, SplitDirection};
use gwt_core::agent::launch::{AgentLaunchBuilder, ShellLaunchBuilder};
use gwt_core::terminal::{manager::PaneManager, AgentColor};

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

        let size = terminal.size()?;
        self.terminal_rows = size.height.saturating_sub(2);
        self.terminal_cols = size.width;

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

    // --- Pane lifecycle ---

    fn spawn_shell_pane(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        let config = ShellLaunchBuilder::new(&self.repo_root).build();

        let pane_id = self.pane_manager.spawn_shell(
            &self.repo_root,
            config,
            self.terminal_rows,
            self.terminal_cols,
        )?;

        self.start_pty_reader(&pane_id)?;
        self.vt_parsers.insert(
            pane_id.clone(),
            vt100::Parser::new(self.terminal_rows, self.terminal_cols, 1000),
        );

        Ok(pane_id)
    }

    fn spawn_shell_tab(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let pane_id = self.spawn_shell_pane()?;
        let tree = LayoutTree::new(&pane_id);

        self.state.add_tab(TabInfo {
            pane_id: pane_id.clone(),
            name: "shell".to_string(),
            tab_type: TabType::Shell,
            color: AgentColor::White,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });

        self.state.layout_trees.insert(pane_id, tree);
        Ok(())
    }

    fn split_active_pane(
        &mut self,
        direction: SplitDirection,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tab_pane_id = match self.state.active_tab_info() {
            Some(tab) => tab.pane_id.clone(),
            None => return Ok(()),
        };

        let new_pane_id = self.spawn_shell_pane()?;

        if let Some(tree) = self.state.layout_trees.get_mut(&tab_pane_id) {
            tree.split(direction, &new_pane_id);
        }

        // Update pane count on tab
        if let Some(tab) = self.state.tabs.get_mut(self.state.active_tab) {
            if let Some(tree) = self.state.layout_trees.get(&tab.pane_id) {
                tab.pane_count = tree.pane_count();
            }
        }

        Ok(())
    }

    fn close_active_pane(&mut self) {
        let Some(focused_id) = self.state.focused_pane_id() else {
            return;
        };
        let Some(tab) = self.state.active_tab_info() else {
            return;
        };
        let tab_pane_id = tab.pane_id.clone();

        self.vt_parsers.remove(&focused_id);
        // Find and kill the pane in PaneManager
        if let Some(idx) = self
            .pane_manager
            .panes()
            .iter()
            .position(|p| p.pane_id() == focused_id)
        {
            self.pane_manager.close_pane(idx);
        }

        if let Some(tree) = self.state.layout_trees.get_mut(&tab_pane_id) {
            if tree.pane_count() <= 1 {
                // Last pane in tab — close the whole tab
                self.state.layout_trees.remove(&tab_pane_id);
                self.state.remove_tab(self.state.active_tab);
            } else {
                tree.remove(&focused_id);
                if let Some(tab) = self.state.tabs.get_mut(self.state.active_tab) {
                    tab.pane_count = tree.pane_count();
                }
            }
        }
    }

    fn close_active_tab(&mut self) {
        let Some(tab) = self.state.active_tab_info() else {
            return;
        };
        let tab_pane_id = tab.pane_id.clone();

        // Kill all panes in the tab's layout tree
        if let Some(tree) = self.state.layout_trees.get(&tab_pane_id) {
            for id in tree.pane_ids() {
                self.vt_parsers.remove(&id);
                if let Some(idx) = self
                    .pane_manager
                    .panes()
                    .iter()
                    .position(|p| p.pane_id() == id)
                {
                    self.pane_manager.close_pane(idx);
                }
            }
        }
        self.state.layout_trees.remove(&tab_pane_id);
        self.state.remove_tab(self.state.active_tab);
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

    // --- Rendering ---

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

            match self.state.mode {
                AppMode::Management => {
                    ui::tab_bar::render(buf, layout[0], &self.state);
                    ui::management::render(buf, layout[1], &self.state.management);
                    ui::status_bar::render(buf, layout[2], &self.state);
                }
                AppMode::LaunchDialog => {
                    ui::tab_bar::render(buf, layout[0], &self.state);
                    self.render_terminal_area(buf, layout[1]);
                    ui::status_bar::render(buf, layout[2], &self.state);
                    // Overlay launch dialog
                    ui::management::launch_dialog::render(
                        buf,
                        centered_rect(60, 40, area),
                        &self.state.management.launch_dialog,
                    );
                }
                _ => {
                    ui::tab_bar::render(buf, layout[0], &self.state);
                    self.render_terminal_area(buf, layout[1]);
                    ui::status_bar::render(buf, layout[2], &self.state);
                }
            }
        })?;
        Ok(())
    }

    fn render_terminal_area(&self, buf: &mut Buffer, area: Rect) {
        if self.state.tabs.is_empty() {
            render_welcome(buf, area);
            return;
        }

        let Some(tab) = self.state.active_tab_info() else {
            return;
        };

        if self.state.zoomed {
            // Zoomed: render only focused pane
            if let Some(focused_id) = self.state.focused_pane_id() {
                if let Some(parser) = self.vt_parsers.get(&focused_id) {
                    ui::terminal_view::render(buf, area, parser.screen());
                }
            }
            return;
        }

        if let Some(tree) = self.state.layout_trees.get(&tab.pane_id) {
            let areas = tree.calculate_areas(area);
            let focused = tree.focused_pane().to_string();
            for (pane_id, pane_area) in &areas {
                if let Some(parser) = self.vt_parsers.get(pane_id) {
                    ui::terminal_view::render(buf, *pane_area, parser.screen());
                }
                // Draw focus indicator (thin border highlight for focused pane)
                if *pane_id == focused && areas.len() > 1 {
                    let border_style = Style::default().fg(Color::Cyan);
                    // Top border
                    for x in pane_area.left()..pane_area.right() {
                        if let Some(cell) = buf.cell_mut((x, pane_area.top())) {
                            cell.set_style(border_style);
                        }
                    }
                }
            }
        } else if let Some(parser) = self.vt_parsers.get(&tab.pane_id) {
            ui::terminal_view::render(buf, area, parser.screen());
        }
    }

    // --- Key handling ---

    fn handle_key(&mut self, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
        // Mode-specific key handling
        match self.state.mode {
            AppMode::Management => return self.handle_management_key(key),
            AppMode::LaunchDialog => return self.handle_launch_dialog_key(key),
            _ => {}
        }

        // Ctrl+C handling
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let is_agent_tab = self
                .state
                .active_tab_info()
                .map(|t| t.tab_type == TabType::Agent)
                .unwrap_or(false);

            if is_agent_tab {
                self.write_to_focused_pty(&[0x03])?;
                self.last_ctrl_c = None;
                return Ok(());
            }

            if let Some(last) = self.last_ctrl_c {
                if last.elapsed().as_millis() < 500 {
                    self.should_quit = true;
                    return Ok(());
                }
            }
            self.last_ctrl_c = Some(Instant::now());
            self.write_to_focused_pty(&[0x03])?;
            return Ok(());
        }

        self.last_ctrl_c = None;

        let action = keybind::process_key(&mut self.state.prefix_state, key);
        match action {
            KeyAction::Quit => self.should_quit = true,
            KeyAction::NewShellWindow => self.spawn_shell_tab()?,
            KeyAction::NewAgentWindow => self.state.mode = AppMode::LaunchDialog,
            KeyAction::NextWindow => self.state.next_tab(),
            KeyAction::PrevWindow => self.state.prev_tab(),
            KeyAction::SwitchTab(n) => self.state.set_active_tab(n),
            KeyAction::CloseWindow => self.close_active_tab(),
            KeyAction::VerticalSplit => self.split_active_pane(SplitDirection::Vertical)?,
            KeyAction::HorizontalSplit => self.split_active_pane(SplitDirection::Horizontal)?,
            KeyAction::FocusPane(dir) => {
                if let Some(tab) = self.state.active_tab_info() {
                    let tab_id = tab.pane_id.clone();
                    if let Some(tree) = self.state.layout_trees.get_mut(&tab_id) {
                        let (split_dir, first) = match dir {
                            Direction::Left => (SplitDirection::Horizontal, true),
                            Direction::Right => (SplitDirection::Horizontal, false),
                            Direction::Up => (SplitDirection::Vertical, true),
                            Direction::Down => (SplitDirection::Vertical, false),
                        };
                        tree.focus_direction(split_dir, first);
                    }
                }
            }
            KeyAction::ClosePane => self.close_active_pane(),
            KeyAction::ZoomPane => self.state.zoomed = !self.state.zoomed,
            KeyAction::ToggleManagement => {
                self.state.mode = AppMode::Management;
                self.sync_management_state();
            }
            KeyAction::ScrollMode => {
                self.state.mode = AppMode::ScrollMode;
            }
            KeyAction::Passthrough(key) => {
                let bytes = key_event_to_bytes(&key);
                if !bytes.is_empty() {
                    self.write_to_focused_pty(&bytes)?;
                }
            }
            KeyAction::None => {}
        }
        Ok(())
    }

    fn handle_management_key(&mut self, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.state.mode = AppMode::Normal,
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.mode = AppMode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => self.state.management.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.state.management.select_next(),
            KeyCode::Enter => {
                // Switch to selected agent's tab
                if let Some(agent) = self.state.management.selected_agent() {
                    let pane_id = agent.pane_id.clone();
                    if let Some(idx) = self.state.tabs.iter().position(|t| t.pane_id == pane_id) {
                        self.state.set_active_tab(idx);
                    }
                }
                self.state.mode = AppMode::Normal;
            }
            KeyCode::Char('n') => {
                self.state.mode = AppMode::LaunchDialog;
            }
            KeyCode::Char('s') => {
                self.spawn_shell_tab()?;
                self.state.mode = AppMode::Normal;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_launch_dialog_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::ui::management::launch_dialog::DialogField;

        let field = &self.state.management.launch_dialog.focused_field;

        match key.code {
            KeyCode::Esc => {
                self.state.management.launch_dialog = Default::default();
                self.state.mode = AppMode::Normal;
            }
            KeyCode::Tab => self.state.management.launch_dialog.focus_next(),
            KeyCode::Enter => {
                match field {
                    DialogField::CancelButton => {
                        self.state.management.launch_dialog = Default::default();
                        self.state.mode = AppMode::Normal;
                    }
                    DialogField::LaunchButton => {
                        self.launch_agent_from_dialog()?;
                        self.state.management.launch_dialog = Default::default();
                        self.state.mode = AppMode::Normal;
                    }
                    DialogField::Agent => {
                        // Enter on agent field cycles agent
                        self.state.management.launch_dialog.next_agent();
                    }
                    DialogField::Branch => {
                        // Enter on branch field moves to Launch button
                        self.state.management.launch_dialog.focus_next();
                    }
                }
            }
            // Agent field: Left/Right or Space to cycle agent
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if *field == DialogField::Agent =>
            {
                self.state.management.launch_dialog.next_agent();
            }
            // Branch field: character input
            KeyCode::Char(c) if *field == DialogField::Branch => {
                self.state.management.launch_dialog.branch_input.push(c);
            }
            KeyCode::Backspace if *field == DialogField::Branch => {
                self.state.management.launch_dialog.branch_input.pop();
            }
            _ => {}
        }
        Ok(())
    }

    fn launch_agent_from_dialog(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let dialog = &self.state.management.launch_dialog;

        // Map dialog selection to agent_id
        let agent_id = match dialog.selected_agent {
            0 => "claude",
            1 => "codex",
            2 => "gemini",
            _ => "claude",
        };

        let branch = if dialog.branch_input.is_empty() {
            "main".to_string()
        } else {
            dialog.branch_input.clone()
        };

        let config = AgentLaunchBuilder::new(agent_id, &self.repo_root)
            .branch_name(&branch)
            .interactive(true)
            .build()?;

        let agent_name = config.agent_name.clone();
        let color = config.agent_color;

        let pane_id = self.pane_manager.launch_agent(
            &self.repo_root,
            config,
            self.terminal_rows,
            self.terminal_cols,
        )?;

        self.start_pty_reader(&pane_id)?;
        self.vt_parsers.insert(
            pane_id.clone(),
            vt100::Parser::new(self.terminal_rows, self.terminal_cols, 1000),
        );

        let tree = LayoutTree::new(&pane_id);
        self.state.add_tab(TabInfo {
            pane_id: pane_id.clone(),
            name: agent_name,
            tab_type: TabType::Agent,
            color,
            status: PaneStatusIndicator::Running,
            branch: Some(branch),
            spec_id: None,
            pane_count: 1,
        });
        self.state.layout_trees.insert(pane_id, tree);

        Ok(())
    }

    fn sync_management_state(&mut self) {
        use crate::ui::management::{AgentEntry, AgentStatus};

        self.state.management.agents = self
            .state
            .tabs
            .iter()
            .map(|tab| AgentEntry {
                pane_id: tab.pane_id.clone(),
                agent_name: tab.name.clone(),
                agent_type: match tab.tab_type {
                    TabType::Shell => "shell".to_string(),
                    TabType::Agent => "agent".to_string(),
                },
                branch: tab.branch.clone(),
                status: match &tab.status {
                    PaneStatusIndicator::Running => AgentStatus::Running,
                    PaneStatusIndicator::Idle => AgentStatus::Idle,
                    PaneStatusIndicator::Completed(c) => AgentStatus::Completed(*c),
                    PaneStatusIndicator::Error(e) => AgentStatus::Error(e.clone()),
                },
                uptime: None,
                pr_url: None,
                spec_id: tab.spec_id.clone(),
            })
            .collect();
    }

    // --- PTY I/O ---

    fn write_to_focused_pty(&mut self, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(pane_id) = self.state.focused_pane_id() {
            if let Some(pane) = self.pane_manager.pane_mut_by_id(&pane_id) {
                pane.write_input(data)?;
            }
        }
        Ok(())
    }

    fn handle_resize(&mut self, width: u16, height: u16) -> Result<(), Box<dyn std::error::Error>> {
        let rows = height.saturating_sub(2);
        self.terminal_rows = rows;
        self.terminal_cols = width;
        self.pane_manager.resize_all(rows, width)?;
        for parser in self.vt_parsers.values_mut() {
            parser.set_size(rows, width);
        }
        Ok(())
    }

    fn handle_pty_output(&mut self, pane_id: &str, data: &[u8]) {
        if let Some(parser) = self.vt_parsers.get_mut(pane_id) {
            parser.process(data);
        }
        if let Some(pane) = self.pane_manager.pane_mut_by_id(pane_id) {
            let _ = pane.process_bytes(data);
        }
    }
}

// --- Helpers ---

fn key_event_to_bytes(key: &KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                if ctrl_byte <= 26 {
                    return vec![ctrl_byte];
                }
            }
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    let h = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
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
        Line::from(Span::styled("Welcome to gwt", Style::default().fg(Color::Cyan))),
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
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_c.clone()).unwrap();
        assert!(!app.should_quit);
        app.handle_key(ctrl_c).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_c_agent_tab_never_quits() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        app.state.add_tab(TabInfo {
            pane_id: "p1".into(),
            name: "claude".into(),
            tab_type: TabType::Agent,
            color: AgentColor::Green,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        app.handle_key(ctrl_c.clone()).unwrap();
        app.handle_key(ctrl_c).unwrap();
        assert!(!app.should_quit); // Agent tab: never quits via Ctrl+C
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
    fn test_launch_dialog_mode() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        // Ctrl+G, n -> LaunchDialog mode
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.state.mode, AppMode::LaunchDialog);
        // Esc -> back to Normal
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.state.mode, AppMode::Normal);
    }

    #[test]
    fn test_management_mode() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        // Ctrl+G, Ctrl+G -> Management mode
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.state.mode, AppMode::Management);
        // Esc -> back to Normal
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.state.mode, AppMode::Normal);
    }

    #[test]
    fn test_zoom_toggle() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        assert!(!app.state.zoomed);
        // Ctrl+G, z -> zoom
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.state.zoomed);
        // Ctrl+G, z -> unzoom
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .unwrap();
        assert!(!app.state.zoomed);
    }

    #[test]
    fn test_handle_pty_output() {
        let mut app = App::new(PathBuf::from("/tmp/test"));
        app.vt_parsers
            .insert("pane-1".to_string(), vt100::Parser::new(24, 80, 0));
        app.handle_pty_output("pane-1", b"Hello");
        let screen = app.vt_parsers.get("pane-1").unwrap().screen();
        assert_eq!(screen.cell(0, 0).unwrap().contents(), "H");
    }

    #[test]
    fn test_key_event_to_bytes() {
        assert_eq!(
            key_event_to_bytes(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            b"a"
        );
        assert_eq!(
            key_event_to_bytes(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            b"\r"
        );
        assert_eq!(
            key_event_to_bytes(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            vec![0x01]
        );
        assert_eq!(
            key_event_to_bytes(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            b"\x1b[A"
        );
    }

    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let r = centered_rect(60, 40, area);
        assert_eq!(r.width, 60);
        assert_eq!(r.height, 20);
        assert_eq!(r.x, 20);
        assert_eq!(r.y, 15);
    }

    #[test]
    fn test_focused_pane_id_no_tabs() {
        let state = TuiState::new();
        assert!(state.focused_pane_id().is_none());
    }

    #[test]
    fn test_focused_pane_id_with_layout() {
        let mut state = TuiState::new();
        state.add_tab(TabInfo {
            pane_id: "tab1".into(),
            name: "shell".into(),
            tab_type: TabType::Shell,
            color: AgentColor::White,
            status: PaneStatusIndicator::Running,
            branch: None,
            spec_id: None,
            pane_count: 1,
        });
        let tree = LayoutTree::new("tab1");
        state.layout_trees.insert("tab1".to_string(), tree);
        assert_eq!(state.focused_pane_id(), Some("tab1".to_string()));
    }
}
