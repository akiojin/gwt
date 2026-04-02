//! App — Update and View functions for the Elm Architecture.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame,
};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    input::voice::VoiceInputMessage,
    message::Message,
    model::{ActiveLayer, ManagementTab, Model, SessionLayout, SessionTabType},
    screens,
};

/// Process a message and update the model (Elm: update).
pub fn update(model: &mut Model, msg: Message) {
    match msg {
        Message::Quit => {
            model.quit = true;
        }
        Message::ToggleLayer => {
            model.active_layer = match model.active_layer {
                ActiveLayer::Main => ActiveLayer::Management,
                ActiveLayer::Management => ActiveLayer::Main,
            };
        }
        Message::SwitchManagementTab(tab) => {
            model.management_tab = tab;
            model.active_layer = ActiveLayer::Management;
        }
        Message::NextSession => {
            if !model.sessions.is_empty() {
                model.active_session = (model.active_session + 1) % model.sessions.len();
            }
        }
        Message::PrevSession => {
            if !model.sessions.is_empty() {
                model.active_session = if model.active_session == 0 {
                    model.sessions.len() - 1
                } else {
                    model.active_session - 1
                };
            }
        }
        Message::SwitchSession(idx) => {
            if idx < model.sessions.len() {
                model.active_session = idx;
            }
        }
        Message::ToggleSessionLayout => {
            model.session_layout = match model.session_layout {
                SessionLayout::Tab => SessionLayout::Grid,
                SessionLayout::Grid => SessionLayout::Tab,
            };
        }
        Message::NewShell => {
            let idx = model.sessions.len();
            let session = crate::model::SessionTab {
                id: format!("shell-{idx}"),
                name: format!("Shell {}", idx + 1),
                tab_type: SessionTabType::Shell,
                vt: crate::model::VtState::new(24, 80),
            };
            model.sessions.push(session);
            model.active_session = idx;
        }
        Message::CloseSession => {
            if model.sessions.len() > 1 {
                model.sessions.remove(model.active_session);
                if model.active_session >= model.sessions.len() {
                    model.active_session = model.sessions.len() - 1;
                }
            }
        }
        Message::Resize(w, h) => {
            model.terminal_size = (w, h);
        }
        Message::PtyOutput(_pane_id, _data) => {
            // Phase 2: feed data into vt100 parser for the matching pane
        }
        Message::PushError(err) => {
            model.error_queue.push_back(err);
        }
        Message::DismissError => {
            model.error_queue.pop_front();
        }
        Message::Tick => {
            // Forward tick to wizard (AI suggest spinner) when active
            if let Some(ref mut wizard) = model.wizard {
                if wizard.ai_suggest.loading {
                    screens::wizard::update(wizard, screens::wizard::WizardMessage::Tick);
                }
            }
            // Forward tick to voice input when recording/transcribing
            if model.voice.is_active() {
                crate::input::voice::update(&mut model.voice, VoiceInputMessage::Tick);
            }
        }
        Message::KeyInput(key) => {
            if model.active_layer == ActiveLayer::Management {
                route_key_to_management(model, key);
            } else {
                forward_key_to_active_session(model, key);
            }
        }
        Message::MouseInput(_) => {
            // Phase 2: mouse routing
        }
        Message::Branches(msg) => {
            screens::branches::update(&mut model.branches, msg);
        }
        Message::Profiles(msg) => {
            screens::profiles::update(&mut model.profiles, msg);
        }
        Message::Issues(msg) => {
            screens::issues::update(&mut model.issues, msg);
        }
        Message::GitView(msg) => {
            screens::git_view::update(&mut model.git_view, msg);
        }
        Message::PrDashboard(msg) => {
            screens::pr_dashboard::update(&mut model.pr_dashboard, msg);
        }
        Message::Specs(msg) => {
            screens::specs::update(&mut model.specs, msg);
        }
        Message::Settings(msg) => {
            screens::settings::update(&mut model.settings, msg);
        }
        Message::Logs(msg) => {
            screens::logs::update(&mut model.logs, msg);
        }
        Message::Versions(msg) => {
            screens::versions::update(&mut model.versions, msg);
        }
        Message::Wizard(msg) => {
            if let Some(ref mut wizard) = model.wizard {
                screens::wizard::update(wizard, msg);
                if wizard.completed || wizard.cancelled {
                    model.wizard = None;
                }
            }
        }
        Message::DockerProgress(msg) => {
            if let Some(ref mut state) = model.docker_progress {
                screens::docker_progress::update(state, msg);
            }
        }
        Message::ServiceSelect(msg) => {
            if let Some(ref mut state) = model.service_select {
                screens::service_select::update(state, msg);
                if !state.visible {
                    model.service_select = None;
                }
            }
        }
        Message::PortSelect(msg) => {
            if let Some(ref mut state) = model.port_select {
                screens::port_select::update(state, msg);
                if !state.visible {
                    model.port_select = None;
                }
            }
        }
        Message::Confirm(msg) => {
            screens::confirm::update(&mut model.confirm, msg);
        }
        Message::Voice(msg) => {
            crate::input::voice::update(&mut model.voice, msg);
        }
        Message::PasteFiles => {
            // Placeholder: actual clipboard access is handled by gwt-clipboard crate.
            // TUI triggers the action; the event loop will dispatch to clipboard integration.
        }
        Message::OpenWizard => {
            model.wizard = Some(crate::screens::wizard::WizardState::default());
        }
        Message::OpenWizardWithSpec(spec_id, title) => {
            let wizard = crate::screens::wizard::WizardState {
                branch_name: format!("feature/{}", spec_id.to_lowercase()),
                spec_context: Some(crate::screens::wizard::SpecContext { spec_id, title }),
                ..Default::default()
            };
            model.wizard = Some(wizard);
        }
        Message::CloseWizard => {
            model.wizard = None;
        }
    }
}

/// Route a key event to the active management tab's screen message.
fn route_key_to_management(model: &mut Model, key: crossterm::event::KeyEvent) {
    use screens::branches::BranchesMessage;
    use screens::git_view::GitViewMessage;
    use screens::issues::IssuesMessage;
    use screens::logs::LogsMessage;
    use screens::pr_dashboard::PrDashboardMessage;
    use screens::profiles::ProfilesMessage;
    use screens::settings::SettingsMessage;
    use screens::specs::SpecsMessage;
    use screens::versions::VersionsMessage;

    // Global management keys: tab switching with Left/Right arrows
    // (only when not in text input mode like search or edit)
    if !is_in_text_input_mode(model) {
        let tab_count = ManagementTab::ALL.len();
        let idx = ManagementTab::ALL
            .iter()
            .position(|t| *t == model.management_tab)
            .unwrap_or(0);

        match key.code {
            KeyCode::Right => {
                model.management_tab = ManagementTab::ALL[(idx + 1) % tab_count];
                return;
            }
            KeyCode::Left => {
                model.management_tab =
                    ManagementTab::ALL[if idx == 0 { tab_count - 1 } else { idx - 1 }];
                return;
            }
            _ => {}
        }
    }

    // Tab-specific key routing
    match model.management_tab {
        ManagementTab::Branches => {
            if model.branches.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(BranchesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(BranchesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::branches::update(&mut model.branches, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(BranchesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(BranchesMessage::MoveUp),
                KeyCode::Enter => Some(BranchesMessage::Select),
                KeyCode::Char('s') => Some(BranchesMessage::ToggleSort),
                KeyCode::Char('v') => Some(BranchesMessage::ToggleView),
                KeyCode::Char('/') => Some(BranchesMessage::SearchStart),
                KeyCode::Char('r') => Some(BranchesMessage::Refresh),
                KeyCode::Esc => Some(BranchesMessage::SearchClear),
                _ => None,
            };
            if let Some(m) = msg {
                screens::branches::update(&mut model.branches, m);
            }
        }
        ManagementTab::Issues => {
            if model.issues.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(IssuesMessage::SearchBackspace),
                    _ => search_input_char(&key).map(IssuesMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::issues::update(&mut model.issues, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(IssuesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(IssuesMessage::MoveUp),
                KeyCode::Enter => Some(IssuesMessage::ToggleDetail),
                KeyCode::Char('/') => Some(IssuesMessage::SearchStart),
                KeyCode::Char('r') => Some(IssuesMessage::Refresh),
                KeyCode::Esc => Some(IssuesMessage::SearchClear),
                _ => None,
            };
            if let Some(m) = msg {
                screens::issues::update(&mut model.issues, m);
            }
        }
        ManagementTab::Specs => {
            if model.specs.search_active {
                let msg = match key.code {
                    KeyCode::Backspace => Some(SpecsMessage::SearchBackspace),
                    _ => search_input_char(&key).map(SpecsMessage::SearchInput),
                };
                if let Some(m) = msg {
                    screens::specs::update(&mut model.specs, m);
                    return;
                }
            }

            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(SpecsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(SpecsMessage::MoveUp),
                KeyCode::Enter => Some(SpecsMessage::ToggleDetail),
                KeyCode::Tab => Some(SpecsMessage::NextSection),
                KeyCode::BackTab => Some(SpecsMessage::PrevSection),
                KeyCode::Char('/') => Some(SpecsMessage::SearchStart),
                KeyCode::Char('r') => Some(SpecsMessage::Refresh),
                KeyCode::Esc => Some(SpecsMessage::SearchClear),
                _ => None,
            };
            if let Some(m) = msg {
                screens::specs::update(&mut model.specs, m);
            }
        }
        ManagementTab::Settings => {
            if model.settings.editing {
                let msg = match key.code {
                    KeyCode::Enter => Some(SettingsMessage::EndEdit),
                    KeyCode::Esc => Some(SettingsMessage::CancelEdit),
                    KeyCode::Backspace => Some(SettingsMessage::Backspace),
                    KeyCode::Char(ch) => Some(SettingsMessage::InputChar(ch)),
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                }
            } else {
                let msg = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Some(SettingsMessage::MoveDown),
                    KeyCode::Char('k') | KeyCode::Up => Some(SettingsMessage::MoveUp),
                    KeyCode::Enter => Some(SettingsMessage::StartEdit),
                    KeyCode::Char(' ') => Some(SettingsMessage::ToggleBool),
                    KeyCode::Tab => Some(SettingsMessage::NextCategory),
                    KeyCode::BackTab => Some(SettingsMessage::PrevCategory),
                    KeyCode::Char('S') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        Some(SettingsMessage::Save)
                    }
                    _ => None,
                };
                if let Some(m) = msg {
                    screens::settings::update(&mut model.settings, m);
                }
            }
        }
        ManagementTab::Logs => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(LogsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(LogsMessage::MoveUp),
                KeyCode::Enter => Some(LogsMessage::ToggleDetail),
                KeyCode::Char('r') => Some(LogsMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::logs::update(&mut model.logs, m);
            }
        }
        ManagementTab::Versions => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(VersionsMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(VersionsMessage::MoveUp),
                KeyCode::Char('r') => Some(VersionsMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::versions::update(&mut model.versions, m);
            }
        }
        ManagementTab::GitView => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(GitViewMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(GitViewMessage::MoveUp),
                KeyCode::Enter => Some(GitViewMessage::ToggleExpand),
                KeyCode::Char('r') => Some(GitViewMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::git_view::update(&mut model.git_view, m);
            }
        }
        ManagementTab::PrDashboard => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(PrDashboardMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(PrDashboardMessage::MoveUp),
                KeyCode::Enter => Some(PrDashboardMessage::ToggleDetail),
                KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
                _ => None,
            };
            if let Some(m) = msg {
                screens::pr_dashboard::update(&mut model.pr_dashboard, m);
            }
        }
        ManagementTab::Profiles => {
            let msg = match key.code {
                KeyCode::Char('j') | KeyCode::Down => Some(ProfilesMessage::MoveDown),
                KeyCode::Char('k') | KeyCode::Up => Some(ProfilesMessage::MoveUp),
                KeyCode::Enter => Some(ProfilesMessage::ToggleActive),
                KeyCode::Char('n') => Some(ProfilesMessage::StartCreate),
                KeyCode::Char('e') => Some(ProfilesMessage::StartEdit),
                KeyCode::Char('d') => Some(ProfilesMessage::StartDelete),
                KeyCode::Esc => Some(ProfilesMessage::Cancel),
                _ => None,
            };
            if let Some(m) = msg {
                screens::profiles::update(&mut model.profiles, m);
            }
        }
    }
}

fn search_input_char(key: &crossterm::event::KeyEvent) -> Option<char> {
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }

    match key.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn forward_key_to_active_session(model: &mut Model, key: crossterm::event::KeyEvent) {
    let Some(bytes) = key_event_to_bytes(key) else {
        return;
    };
    let Some(session_id) = model.active_session_tab().map(|session| session.id.clone()) else {
        return;
    };

    model
        .pending_pty_inputs
        .push_back(crate::model::PendingPtyInput { session_id, bytes });
}

fn key_event_to_bytes(key: crossterm::event::KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            control_char_bytes(ch)
        }
        KeyCode::Char(ch) => Some(ch.to_string().into_bytes()),
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        _ => None,
    }
}

fn control_char_bytes(ch: char) -> Option<Vec<u8>> {
    let ch = ch.to_ascii_lowercase();
    match ch {
        '@' | ' ' => Some(vec![0x00]),
        'a'..='z' => Some(vec![(ch as u8) & 0x1f]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' => Some(vec![0x1f]),
        _ => None,
    }
}

/// Check if the active management screen is in a text input mode (search, edit).
fn is_in_text_input_mode(model: &Model) -> bool {
    match model.management_tab {
        ManagementTab::Branches => model.branches.search_active,
        ManagementTab::Issues => model.issues.search_active,
        ManagementTab::Specs => model.specs.search_active || model.specs.editing,
        ManagementTab::Settings => model.settings.editing,
        _ => false,
    }
}

/// Render the full UI (Elm: view).
pub fn view(model: &Model, frame: &mut Frame) {
    let size = frame.area();

    if model.active_layer == ActiveLayer::Management {
        // Split: left = management panel, right = sessions
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(size);

        render_management_panel(model, frame, chunks[0]);
        render_sessions_area(model, frame, chunks[1]);
    } else {
        render_sessions_area(model, frame, size);
    }

    // Confirm dialog overlay
    screens::confirm::render(&model.confirm, frame, size);

    // Docker progress overlay
    if let Some(ref docker) = model.docker_progress {
        screens::docker_progress::render(docker, frame, size);
    }

    // Service selection overlay
    if let Some(ref svc) = model.service_select {
        screens::service_select::render(svc, frame, size);
    }

    // Port selection overlay
    if let Some(ref port) = model.port_select {
        screens::port_select::render(port, frame, size);
    }

    // Wizard overlay (on top of everything except errors)
    if let Some(ref wizard) = model.wizard {
        screens::wizard::render(wizard, frame, size);
    }

    // Error overlay on top of everything
    if !model.error_queue.is_empty() {
        screens::error::render(&model.error_queue, frame, size);
    }
}

/// Render the management panel (left side).
fn render_management_panel(model: &Model, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Tab bar
    let titles: Vec<Line> = ManagementTab::ALL
        .iter()
        .map(|t| Line::from(t.label()))
        .collect();

    let active_idx = ManagementTab::ALL
        .iter()
        .position(|t| *t == model.management_tab)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Management"))
        .select(active_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, chunks[0]);

    // Tab content
    render_management_tab_content(model, frame, chunks[1]);
}

/// Render the content of the active management tab.
fn render_management_tab_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.management_tab {
        ManagementTab::Branches => screens::branches::render(&model.branches, frame, area),
        ManagementTab::Specs => screens::specs::render(&model.specs, frame, area),
        ManagementTab::Issues => screens::issues::render(&model.issues, frame, area),
        ManagementTab::PrDashboard => {
            screens::pr_dashboard::render(&model.pr_dashboard, frame, area)
        }
        ManagementTab::Profiles => screens::profiles::render(&model.profiles, frame, area),
        ManagementTab::GitView => screens::git_view::render(&model.git_view, frame, area),
        ManagementTab::Versions => screens::versions::render(&model.versions, frame, area),
        ManagementTab::Settings => screens::settings::render(&model.settings, frame, area),
        ManagementTab::Logs => screens::logs::render(&model.logs, frame, area),
    }
}

/// Render the sessions area (right side, or full screen).
fn render_sessions_area(model: &Model, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Tab bar
            Constraint::Min(0),    // Terminal content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Session tab bar — delegate to widget
    crate::widgets::tab_bar::render(model, frame, chunks[0]);

    // Session content
    render_session_content(model, frame, chunks[1]);

    // Status bar — delegate to widget
    crate::widgets::status_bar::render(model, frame, chunks[2]);
}

/// Render session content (Tab mode = single, Grid mode = tiled).
fn render_session_content(model: &Model, frame: &mut Frame, area: Rect) {
    match model.session_layout {
        SessionLayout::Tab => {
            if let Some(session) = model.active_session_tab() {
                render_single_session(session, frame, area);
            }
        }
        SessionLayout::Grid => {
            render_grid_sessions(model, frame, area);
        }
    }
}

/// Render a single session pane.
fn render_single_session(session: &crate::model::SessionTab, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(session.name.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Phase 2: render vt100 screen buffer here
    let placeholder = Paragraph::new(format!(
        "Session: {} ({}x{})",
        session.name,
        session.vt.cols(),
        session.vt.rows()
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(placeholder, inner);
}

/// Render sessions in a grid layout.
fn render_grid_sessions(model: &Model, frame: &mut Frame, area: Rect) {
    let count = model.sessions.len();
    if count == 0 {
        return;
    }

    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);

    let row_constraints: Vec<Constraint> = (0..rows)
        .map(|_| Constraint::Ratio(1, rows as u32))
        .collect();

    let row_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);

    for (row_idx, row_area) in row_chunks.iter().enumerate() {
        let start = row_idx * cols;
        let end = (start + cols).min(count);
        let n = end - start;

        let col_constraints: Vec<Constraint> =
            (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

        let col_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(*row_area);

        for (col_idx, col_area) in col_chunks.iter().enumerate() {
            let session_idx = start + col_idx;
            if let Some(session) = model.sessions.get(session_idx) {
                let is_active = session_idx == model.active_session;
                let border_style = if is_active {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(session.name.as_str());
                frame.render_widget(block, *col_area);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState};
    use std::path::PathBuf;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test"))
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn update_quit_sets_flag() {
        let mut model = test_model();
        update(&mut model, Message::Quit);
        assert!(model.quit);
    }

    #[test]
    fn update_toggle_layer() {
        let mut model = test_model();
        assert_eq!(model.active_layer, ActiveLayer::Management);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Main);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_switch_management_tab() {
        let mut model = test_model();
        update(
            &mut model,
            Message::SwitchManagementTab(ManagementTab::Settings),
        );
        assert_eq!(model.management_tab, ManagementTab::Settings);
        assert_eq!(model.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn update_next_prev_session() {
        let mut model = test_model();
        // Add a second session
        update(&mut model, Message::NewShell);
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::PrevSession);
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::PrevSession);
        // Wraps to last
        assert_eq!(model.active_session, 1);

        update(&mut model, Message::NextSession);
        assert_eq!(model.active_session, 0);
    }

    #[test]
    fn update_switch_session_by_index() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);

        update(&mut model, Message::SwitchSession(0));
        assert_eq!(model.active_session, 0);

        update(&mut model, Message::SwitchSession(2));
        assert_eq!(model.active_session, 2);

        // Out of range — no change
        update(&mut model, Message::SwitchSession(99));
        assert_eq!(model.active_session, 2);
    }

    #[test]
    fn update_toggle_session_layout() {
        let mut model = test_model();
        assert_eq!(model.session_layout, SessionLayout::Tab);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Grid);

        update(&mut model, Message::ToggleSessionLayout);
        assert_eq!(model.session_layout, SessionLayout::Tab);
    }

    #[test]
    fn update_new_shell_adds_session() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
        assert_eq!(model.sessions[1].name, "Shell 2");
    }

    #[test]
    fn update_close_session_removes_active() {
        let mut model = test_model();
        update(&mut model, Message::NewShell);
        update(&mut model, Message::NewShell);
        assert_eq!(model.session_count(), 3);
        assert_eq!(model.active_session, 2);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 2);
        assert_eq!(model.active_session, 1);
    }

    #[test]
    fn update_close_session_wont_remove_last() {
        let mut model = test_model();
        assert_eq!(model.session_count(), 1);

        update(&mut model, Message::CloseSession);
        assert_eq!(model.session_count(), 1);
    }

    #[test]
    fn update_resize() {
        let mut model = test_model();
        update(&mut model, Message::Resize(120, 40));
        assert_eq!(model.terminal_size, (120, 40));
    }

    #[test]
    fn update_error_queue() {
        let mut model = test_model();
        update(&mut model, Message::PushError("e1".into()));
        update(&mut model, Message::PushError("e2".into()));
        assert_eq!(model.error_queue.len(), 2);

        update(&mut model, Message::DismissError);
        assert_eq!(model.error_queue.len(), 1);
        assert_eq!(model.error_queue.front().unwrap(), "e2");
    }

    #[test]
    fn update_dismiss_empty_error_queue_is_noop() {
        let mut model = test_model();
        update(&mut model, Message::DismissError);
        assert!(model.error_queue.is_empty());
    }

    #[test]
    fn update_open_wizard_with_spec_prefills() {
        let mut model = test_model();
        update(
            &mut model,
            Message::OpenWizardWithSpec("SPEC-42".into(), "My Feature".into()),
        );
        let wizard = model.wizard.as_ref().unwrap();
        assert_eq!(wizard.branch_name, "feature/spec-42");
        let ctx = wizard.spec_context.as_ref().unwrap();
        assert_eq!(ctx.spec_id, "SPEC-42");
        assert_eq!(ctx.title, "My Feature");
    }

    #[test]
    fn update_open_wizard_without_spec_has_no_context() {
        let mut model = test_model();
        update(&mut model, Message::OpenWizard);
        let wizard = model.wizard.as_ref().unwrap();
        assert!(wizard.spec_context.is_none());
        assert!(wizard.branch_name.is_empty());
    }

    #[test]
    fn update_key_input_in_main_layer_queues_pty_bytes() {
        let mut model = test_model();
        model.active_layer = ActiveLayer::Main;

        update(
            &mut model,
            Message::KeyInput(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );

        let forwarded = model.pending_pty_inputs().back().unwrap();
        assert_eq!(forwarded.session_id, "shell-0");
        assert_eq!(forwarded.bytes, vec![0x03]);
    }

    #[test]
    fn route_key_to_management_routes_search_input_for_issues() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.issues.search_active = true;

        route_key_to_management(&mut model, key(KeyCode::Char('b'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('u'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(model.issues.search_query, "b");
    }

    #[test]
    fn route_key_to_management_routes_search_input_for_specs() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Specs;
        model.specs.search_active = true;

        route_key_to_management(&mut model, key(KeyCode::Char('s'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Char('p'), KeyModifiers::NONE));
        route_key_to_management(&mut model, key(KeyCode::Backspace, KeyModifiers::NONE));

        assert_eq!(model.specs.search_query, "s");
    }
}
