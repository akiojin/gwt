//! App — Update and View functions for the Elm Architecture.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame,
};

use crate::{
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
                ActiveLayer::Main => {
                    model.management_visible = true;
                    ActiveLayer::Management
                }
                ActiveLayer::Management => {
                    model.management_visible = false;
                    ActiveLayer::Main
                }
            };
        }
        Message::SwitchManagementTab(tab) => {
            model.management_tab = tab;
            model.active_layer = ActiveLayer::Management;
            model.management_visible = true;
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
            model.error_queue.push(err);
        }
        Message::DismissError => {
            if !model.error_queue.is_empty() {
                model.error_queue.remove(0);
            }
        }
        Message::KeyInput(_) | Message::MouseInput(_) | Message::Tick => {
            // Phase 2: forward to active pane / tick logic
        }
    }
}

/// Render the full UI (Elm: view).
pub fn view(model: &Model, frame: &mut Frame) {
    let size = frame.area();

    if model.management_visible {
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

    // Error overlay on top
    if let Some(err) = model.error_queue.first() {
        render_error_overlay(err, frame, size);
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
        ManagementTab::Branches => screens::branches::render(frame, area),
        ManagementTab::Specs => screens::specs::render(frame, area),
        ManagementTab::Issues => screens::issues::render(frame, area),
        ManagementTab::Profiles => screens::profiles::render(frame, area),
        ManagementTab::GitView => screens::git_view::render(frame, area),
        ManagementTab::Versions => screens::versions::render(frame, area),
        ManagementTab::Settings => screens::settings::render(frame, area),
        ManagementTab::Logs => screens::logs::render(frame, area),
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

/// Render an error overlay.
fn render_error_overlay(err: &str, frame: &mut Frame, area: Rect) {
    let width = (area.width / 2).max(40).min(area.width);
    let height = 5_u16.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let overlay_area = Rect::new(x, y, width, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title("Error");

    let text = Paragraph::new(err.to_string())
        .block(block)
        .style(Style::default().fg(Color::Red));

    frame.render_widget(text, overlay_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test"))
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
        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert!(!model.management_visible);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert!(model.management_visible);

        update(&mut model, Message::ToggleLayer);
        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert!(!model.management_visible);
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
        assert!(model.management_visible);
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
        assert_eq!(model.error_queue[0], "e2");
    }

    #[test]
    fn update_dismiss_empty_error_queue_is_noop() {
        let mut model = test_model();
        update(&mut model, Message::DismissError);
        assert!(model.error_queue.is_empty());
    }
}
