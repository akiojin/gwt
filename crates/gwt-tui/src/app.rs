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
use crate::model::{ActiveLayer, ErrorEntry, ErrorSeverity, ManagementTab, Model, OverlayMode};
use crate::screens::{self, LogsMessage, SettingsMessage};
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
            if let Err(e) = spawn_shell_session(model) {
                model.push_error(ErrorEntry {
                    message: format!("Failed to spawn shell: {e}"),
                    severity: ErrorSeverity::Critical,
                });
            }
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
            // Management layer: Tab key cycles management tabs
            if model.active_layer == ActiveLayer::Management
                && key.code == crossterm::event::KeyCode::Tab
            {
                model.management_tab = match model.management_tab {
                    ManagementTab::Branches => ManagementTab::Issues,
                    ManagementTab::Issues => ManagementTab::Settings,
                    ManagementTab::Settings => ManagementTab::Logs,
                    ManagementTab::Logs => ManagementTab::Branches,
                };
                return;
            }
            // Forward to active screen handler
            match model.active_layer {
                ActiveLayer::Main => {
                    // Forward to active PTY pane
                    if let Some(session) = model.session_tabs.get(model.active_session) {
                        let pane_id = session.pane_id.clone();
                        if let Some(pane) = model.pane_manager.pane_mut_by_id(&pane_id) {
                            let bytes = key_event_to_bytes(&key);
                            if !bytes.is_empty() {
                                let _ = pane.write_input(&bytes);
                            }
                        }
                    }
                }
                ActiveLayer::Management => {
                    let sub_msg = match model.management_tab {
                        ManagementTab::Branches => {
                            crate::screens::branches::handle_key(&model.branches_state, &key)
                                .map(Message::BranchesMsg)
                        }
                        ManagementTab::Issues => {
                            crate::screens::issues::handle_key(&model.issues_state, &key)
                                .map(Message::IssuesMsg)
                        }
                        ManagementTab::Settings => {
                            crate::screens::settings::handle_key(&model.settings_state, &key)
                                .map(Message::SettingsMsg)
                        }
                        ManagementTab::Logs => {
                            crate::screens::logs::handle_key(&model.logs_state, &key)
                                .map(Message::LogsMsg)
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
        // -- Overlay / dialog messages ------------------------------------------
        Message::OpenCloneWizard => {
            model.clone_wizard = Some(screens::clone_wizard::CloneWizardState::new());
            model.overlay_mode = OverlayMode::CloneWizard;
        }
        Message::CloseCloneWizard => {
            model.clone_wizard = None;
            model.overlay_mode = OverlayMode::None;
        }
        Message::OpenMigrationDialog { source, target } => {
            model.migration_dialog = Some(screens::migration_dialog::MigrationDialogState::new(
                &source, &target,
            ));
            model.overlay_mode = OverlayMode::MigrationDialog;
        }
        Message::CloseMigrationDialog => {
            model.migration_dialog = None;
            model.overlay_mode = OverlayMode::None;
        }
        Message::OpenSpecKitWizard => {
            model.speckit_wizard.open();
            model.overlay_mode = OverlayMode::SpecKitWizard;
        }
        Message::CloseSpecKitWizard => {
            model.speckit_wizard.close();
            model.overlay_mode = OverlayMode::None;
        }
        Message::ConfirmAccepted => {
            model.confirm = None;
            model.overlay_mode = OverlayMode::None;
        }
        Message::ConfirmCancelled => {
            model.confirm = None;
            model.overlay_mode = OverlayMode::None;
        }
        Message::ProgressAdvance => {
            if let Some(ref mut progress) = model.progress {
                progress.advance();
            }
        }
        Message::ProgressError(msg) => {
            if let Some(ref mut progress) = model.progress {
                progress.set_error(msg);
            }
        }

        // Screen-specific messages
        Message::BranchesMsg(msg) => {
            crate::screens::branches::update(&mut model.branches_state, msg);
        }
        Message::IssuesMsg(msg) => {
            crate::screens::issues::update(&mut model.issues_state, msg);
        }
        Message::SettingsMsg(msg) => {
            handle_settings_msg(model, msg);
        }
        Message::LogsMsg(msg) => {
            handle_logs_msg(model, msg);
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
            ManagementTab::Settings => {
                crate::screens::settings::render(&model.settings_state, buf, layout[1]);
            }
            ManagementTab::Logs => {
                crate::screens::logs::render(&model.logs_state, buf, layout[1]);
            }
        },
    }

    // Status bar
    widgets::status_bar::render(model, buf, layout[2]);

    // Overlays (on top of everything, priority order)
    // Wizard overlay
    if let Some(ref wizard) = model.wizard {
        crate::screens::wizard::render(buf, area, wizard);
    }

    // Error overlay (v2 queue)
    if !model.error_queue_v2.is_empty() {
        screens::error::render_error_with_queue(&model.error_queue_v2, buf, area);
    } else if !model.error_queue.is_empty() {
        // Legacy error overlay
        render_error_overlay(buf, area, &model.error_queue[0]);
    }

    // Confirm dialog
    if let Some(ref confirm) = model.confirm {
        screens::confirm::render_confirm(confirm, buf, area);
    }

    // Progress modal
    if let Some(ref progress) = model.progress {
        widgets::progress_modal::render(buf, area, progress);
    }

    // Clone wizard
    if let Some(ref clone_wiz) = model.clone_wizard {
        screens::clone_wizard::render_clone_wizard(clone_wiz, buf, area);
    }

    // Migration dialog
    if let Some(ref migration) = model.migration_dialog {
        screens::migration_dialog::render_migration_dialog(migration, buf, area);
    }

    // SpecKit wizard
    screens::speckit_wizard::render_speckit_wizard(&model.speckit_wizard, buf, area);
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
// Settings message handler
// ---------------------------------------------------------------------------

fn handle_settings_msg(model: &mut Model, msg: SettingsMessage) {
    let state = &mut model.settings_state;
    match msg {
        SettingsMessage::Refresh => {
            state.load_settings();
        }
        SettingsMessage::NextCategory => state.next_category(),
        SettingsMessage::PrevCategory => state.prev_category(),
        SettingsMessage::SelectNext => state.select_next(),
        SettingsMessage::SelectPrev => state.select_prev(),
        SettingsMessage::Edit => {
            if state.category == crate::screens::settings::SettingsCategory::CustomAgents {
                if state.is_add_agent_selected() {
                    state.enter_add_mode();
                } else {
                    state.enter_edit_mode();
                }
            }
        }
        SettingsMessage::Delete => {
            if state.category == crate::screens::settings::SettingsCategory::CustomAgents {
                state.enter_delete_mode();
            }
        }
        SettingsMessage::Save => {
            if matches!(
                state.custom_agent_mode,
                crate::screens::settings::CustomAgentMode::Add
                    | crate::screens::settings::CustomAgentMode::Edit(_)
            ) {
                if let Err(e) = state.save_agent() {
                    state.error_message = Some(e.to_string());
                }
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                if let Err(e) = state.save_profile() {
                    state.error_message = Some(e.to_string());
                }
            }
        }
        SettingsMessage::Cancel => {
            if state.is_form_mode() || state.is_delete_mode() {
                state.cancel_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                state.cancel_profile_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                // Save env edits before leaving
                let _ = state.persist_env_edit();
                state.cancel_profile_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.cancel_profile_mode();
            }
        }
        SettingsMessage::FormChar(c) => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.insert_char(c);
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                handle_env_edit_char(state, c);
            } else {
                state.agent_form.insert_char(c);
            }
        }
        SettingsMessage::FormBackspace => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.delete_char();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                handle_env_edit_backspace(state);
            } else {
                state.agent_form.delete_char();
            }
        }
        SettingsMessage::FormNextField => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.next_field();
            } else {
                state.agent_form.next_field();
            }
        }
        SettingsMessage::FormPrevField => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.prev_field();
            } else {
                state.agent_form.prev_field();
            }
        }
        SettingsMessage::FormCycleType => {
            state.agent_form.cycle_type();
        }
        SettingsMessage::ToggleDeleteConfirm => {
            if state.is_delete_mode() {
                state.delete_confirm = !state.delete_confirm;
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                state.profile_delete_confirm = !state.profile_delete_confirm;
            }
        }
        SettingsMessage::ConfirmDelete => {
            if state.is_delete_mode() {
                if state.delete_confirm {
                    state.delete_agent();
                } else {
                    state.cancel_mode();
                }
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                if state.profile_delete_confirm {
                    state.delete_profile();
                } else {
                    state.cancel_profile_mode();
                }
            }
        }
        SettingsMessage::Activate => {}
        SettingsMessage::ProfileAdd => state.enter_profile_add_mode(),
        SettingsMessage::ProfileEdit => state.enter_profile_edit_mode(),
        SettingsMessage::ProfileDelete => state.enter_profile_delete_mode(),
        SettingsMessage::ProfileToggleActive => state.toggle_active_profile(),
        SettingsMessage::ProfileEnvEdit => state.enter_env_edit_mode(),
        SettingsMessage::EnvNew => state.env_state.add_new_var(),
        SettingsMessage::EnvDelete => state.env_state.delete_selected(),
        SettingsMessage::EnvToggleKeyValue => state.env_state.toggle_key_value(),
        SettingsMessage::EnvStartEdit => {
            if !state.env_state.vars.is_empty() {
                let idx = state.env_state.selected_index;
                let key_len = state.env_state.vars[idx].0.len();
                state.env_state.editing = Some(crate::screens::settings::EnvEditMode::Key(key_len));
            }
        }
        SettingsMessage::EnvConfirm => {
            state.env_state.editing = None;
        }
    }
}

fn handle_env_edit_char(state: &mut crate::screens::SettingsState, c: char) {
    let idx = state.env_state.selected_index;
    if idx >= state.env_state.vars.len() {
        return;
    }
    if let Some(ref mode) = state.env_state.editing.clone() {
        match mode {
            crate::screens::settings::EnvEditMode::Key(cursor) => {
                let cursor = *cursor;
                state.env_state.vars[idx].0.insert(cursor, c);
                state.env_state.editing =
                    Some(crate::screens::settings::EnvEditMode::Key(cursor + 1));
            }
            crate::screens::settings::EnvEditMode::Value(cursor) => {
                let cursor = *cursor;
                state.env_state.vars[idx].1.insert(cursor, c);
                state.env_state.editing =
                    Some(crate::screens::settings::EnvEditMode::Value(cursor + 1));
            }
        }
    }
}

fn handle_env_edit_backspace(state: &mut crate::screens::SettingsState) {
    let idx = state.env_state.selected_index;
    if idx >= state.env_state.vars.len() {
        return;
    }
    if let Some(ref mode) = state.env_state.editing.clone() {
        match mode {
            crate::screens::settings::EnvEditMode::Key(cursor) => {
                if *cursor > 0 {
                    let new_cursor = cursor - 1;
                    state.env_state.vars[idx].0.remove(new_cursor);
                    state.env_state.editing =
                        Some(crate::screens::settings::EnvEditMode::Key(new_cursor));
                }
            }
            crate::screens::settings::EnvEditMode::Value(cursor) => {
                if *cursor > 0 {
                    let new_cursor = cursor - 1;
                    state.env_state.vars[idx].1.remove(new_cursor);
                    state.env_state.editing =
                        Some(crate::screens::settings::EnvEditMode::Value(new_cursor));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Logs message handler
// ---------------------------------------------------------------------------

fn handle_logs_msg(model: &mut Model, msg: LogsMessage) {
    let state = &mut model.logs_state;
    match msg {
        LogsMessage::Refresh => {
            let entries = crate::screens::logs::load_log_entries();
            *state = crate::screens::LogsState::new().with_entries(entries);
        }
        LogsMessage::SelectPrev => state.select_prev(),
        LogsMessage::SelectNext => state.select_next(),
        LogsMessage::PageUp => state.page_up(10),
        LogsMessage::PageDown => state.page_down(10),
        LogsMessage::GoHome => state.go_home(),
        LogsMessage::GoEnd => state.go_end(),
        LogsMessage::CycleFilter => state.cycle_filter(),
        LogsMessage::ToggleSearch => state.toggle_search(),
        LogsMessage::ToggleDetail => state.toggle_detail(),
        LogsMessage::CloseDetail => state.close_detail(),
        LogsMessage::SearchChar(c) => {
            state.search.push(c);
            state.selected = 0;
            state.offset = 0;
        }
        LogsMessage::SearchBackspace => {
            state.search.pop();
            state.selected = 0;
            state.offset = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Shell session spawning
// ---------------------------------------------------------------------------

fn spawn_shell_session(model: &mut Model) -> Result<(), Box<dyn std::error::Error>> {
    use gwt_core::agent::launch::ShellLaunchBuilder;
    use gwt_core::terminal::AgentColor;

    let config = ShellLaunchBuilder::new(&model.repo_root).build();
    let rows = model.terminal_rows.saturating_sub(2);
    let cols = model.terminal_cols;

    let pane_id = model
        .pane_manager
        .spawn_shell(&model.repo_root, config, rows, cols)?;

    // Start PTY reader thread
    let pane = model
        .pane_manager
        .panes()
        .iter()
        .find(|p| p.pane_id() == pane_id)
        .ok_or("pane not found")?;
    let mut reader = pane.take_reader()?;
    let tx = model.pty_tx.as_ref().ok_or("pty_tx not initialized")?.clone();
    let id = pane_id.clone();
    std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        use crate::event::PtyOutputMsg;
                        if tx
                            .send(PtyOutputMsg {
                                pane_id: id.clone(),
                                data: buf[..n].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })?;

    // Register VT100 parser
    model
        .vt_parsers
        .insert(pane_id.clone(), vt100::Parser::new(rows, cols, 1000));

    // Add session tab
    model.add_session(crate::model::SessionTab {
        pane_id,
        name: "shell".to_string(),
        tab_type: crate::model::SessionTabType::Shell,
        color: AgentColor::White,
        status: crate::model::SessionStatus::Running,
        branch: None,
        spec_id: None,
    });

    // Switch to Main layer
    model.active_layer = ActiveLayer::Main;

    Ok(())
}

// ---------------------------------------------------------------------------
// Key → bytes conversion (for PTY input)
// ---------------------------------------------------------------------------

fn key_event_to_bytes(key: &crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};
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
        _ => vec![],
    }
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
    let (pty_tx, pty_rx) = event::pty_output_channel();
    model.pty_tx = Some(pty_tx);

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
                // Only handle key Press events (ignore Release/Repeat/IME)
                if key.kind != crossterm::event::KeyEventKind::Press {
                    None
                }
                // When wizard is open, intercept all keys
                else if model.wizard.is_some() {
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
        model.progress = Some(crate::widgets::progress_modal::ProgressState::simple(
            "Loading...",
            Some("step 1"),
        ));
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }
}
