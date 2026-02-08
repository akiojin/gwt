//! Agent pane rendering for the TUI (FR-047)
//!
//! Renders agent pane contents including tab bar, VT100 output area,
//! and status bar using the gwt-core terminal infrastructure.

#![allow(dead_code)]

use gwt_core::terminal::manager::PaneManager;
use gwt_core::terminal::pane::PaneStatus;
use ratatui::{prelude::*, widgets::*};

/// State for copy mode (FR-070: scrollback / text selection).
#[derive(Debug, Clone, Default)]
pub struct CopyModeState {
    /// Whether copy mode is currently active.
    pub active: bool,
    /// Cursor row position within the terminal buffer.
    pub cursor_row: u16,
    /// Cursor column position within the terminal buffer.
    pub cursor_col: u16,
    /// Selection start position (row, col). None if selection has not started.
    pub selection_start: Option<(u16, u16)>,
}

/// View data for rendering an agent pane area (FR-047).
pub struct AgentPaneView<'a> {
    pub pane_manager: &'a PaneManager,
    pub is_focused: bool,
}

/// Format the elapsed time as mm:ss.
fn format_elapsed(started_at: chrono::DateTime<chrono::Utc>) -> String {
    let elapsed = chrono::Utc::now() - started_at;
    let total_secs = elapsed.num_seconds().max(0);
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}m{secs:02}s")
}

/// Get the status icon for a pane status (ASCII only per CLAUDE.md).
fn status_icon(status: &PaneStatus) -> &'static str {
    match status {
        PaneStatus::Running => "*",
        PaneStatus::Completed(0) => "o",
        PaneStatus::Completed(_) => "x",
        PaneStatus::Error(_) => "!",
    }
}

/// Get the status label for a pane status.
fn status_label(status: &PaneStatus) -> &'static str {
    match status {
        PaneStatus::Running => "Running",
        PaneStatus::Completed(0) => "Completed",
        PaneStatus::Completed(_) => "Failed",
        PaneStatus::Error(_) => "Error",
    }
}

/// Format tab label for a pane: `{index+1}:{agent_name}`.
fn format_tab_label(index: usize, agent_name: &str) -> String {
    format!("{}:{}", index + 1, agent_name)
}

/// FR-045: Get the agent pane block title.
/// Returns the active agent name, or "No Agent" if no panes exist.
fn agent_pane_title(manager: &PaneManager) -> String {
    match manager.active_pane() {
        Some(pane) => format!(" {} ", pane.agent_name()),
        None => " No Agent ".to_string(),
    }
}

/// Render the agent pane (tab bar + VT100 content + status bar).
pub fn render_agent_pane(view: &AgentPaneView, frame: &mut Frame, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    let border_color = if view.is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    // FR-045: Dynamic title based on active agent name
    let title = agent_pane_title(view.pane_manager);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 || view.pane_manager.is_empty() {
        // FR-046: Show "No agent running" when no panes
        let msg = Paragraph::new("No agent running")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    }

    // Layout: tab_bar (1 line) | terminal output (remaining) | status bar (1 line)
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let tab_area = chunks[0];
    let terminal_area = chunks[1];
    let status_area = chunks[2];

    // --- Tab bar ---
    render_tab_bar(view.pane_manager, frame, tab_area);

    // --- Terminal output ---
    if let Some(pane) = view.pane_manager.active_pane() {
        pane.render(terminal_area, frame.buffer_mut());
    }

    // --- Status bar ---
    render_status_bar(view.pane_manager, frame, status_area);
}

/// Render the tab bar showing all panes.
fn render_tab_bar(manager: &PaneManager, frame: &mut Frame, area: Rect) {
    let active_idx = manager.active_index();
    let mut spans: Vec<Span> = Vec::new();

    for (i, pane) in manager.panes().iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let label = format_tab_label(i, pane.agent_name());
        let is_active = i == active_idx;

        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let display = if is_active {
            format!("[{label}]")
        } else {
            format!(" {label} ")
        };
        spans.push(Span::styled(display, style));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

/// Render the status bar for the active pane.
fn render_status_bar(manager: &PaneManager, frame: &mut Frame, area: Rect) {
    let Some(pane) = manager.active_pane() else {
        return;
    };

    let branch = pane.branch_name();
    let agent = pane.agent_name();
    let agent_color = pane.agent_color();
    let status = pane.status();
    let elapsed = format_elapsed(pane.started_at());
    let icon = status_icon(status);
    let label = status_label(status);

    let spans = vec![
        Span::styled(format!(" {branch}"), Style::default().fg(Color::White)),
        Span::raw(" | "),
        Span::styled(agent, Style::default().fg(agent_color)),
        Span::raw(" "),
        Span::styled(
            format!("{icon} {label}"),
            Style::default().fg(match status {
                PaneStatus::Running => Color::Green,
                PaneStatus::Completed(0) => Color::Cyan,
                _ => Color::Red,
            }),
        ),
        Span::raw(" "),
        Span::styled(elapsed, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
    ];

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

/// FR-048: Convert mouse scroll event to terminal escape bytes.
/// Uses X10 mouse protocol: ESC [ M button x y
/// ScrollUp = button 64 (0x40), ScrollDown = button 65 (0x41)
/// Coordinates are offset by 33 (0x21) per X10 protocol.
pub fn mouse_scroll_to_bytes(is_scroll_up: bool, col: u16, row: u16, pane_area: Rect) -> Vec<u8> {
    // Convert host terminal coordinates to pane-local coordinates
    // Account for border (1px on each side)
    let local_col = col.saturating_sub(pane_area.x + 1);
    let local_row = row.saturating_sub(pane_area.y + 1);

    // X10 protocol: button byte, x+33, y+33 (capped at 255)
    let button: u8 = if is_scroll_up { 64 } else { 65 };
    let x = ((local_col as u32) + 33).min(255) as u8;
    let y = ((local_row as u32) + 33).min(255) as u8;

    vec![0x1b, b'[', b'M', button, x, y]
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- format_elapsed tests ---

    #[test]
    fn test_format_elapsed_zero() {
        let now = chrono::Utc::now();
        let result = format_elapsed(now);
        // Should be 0m00s (or 0m01s depending on timing)
        assert!(
            result == "0m00s" || result == "0m01s",
            "Expected 0m00s or 0m01s, got: {result}"
        );
    }

    #[test]
    fn test_format_elapsed_5_minutes() {
        let started = chrono::Utc::now() - chrono::Duration::seconds(300);
        let result = format_elapsed(started);
        assert_eq!(result, "5m00s");
    }

    #[test]
    fn test_format_elapsed_1h_23m_45s() {
        let started = chrono::Utc::now() - chrono::Duration::seconds(5025);
        let result = format_elapsed(started);
        assert_eq!(result, "83m45s");
    }

    #[test]
    fn test_format_elapsed_future_clamps_to_zero() {
        let future = chrono::Utc::now() + chrono::Duration::seconds(100);
        let result = format_elapsed(future);
        assert_eq!(result, "0m00s");
    }

    // --- status_icon tests ---

    #[test]
    fn test_status_icon_running() {
        assert_eq!(status_icon(&PaneStatus::Running), "*");
    }

    #[test]
    fn test_status_icon_completed_success() {
        assert_eq!(status_icon(&PaneStatus::Completed(0)), "o");
    }

    #[test]
    fn test_status_icon_completed_failure() {
        assert_eq!(status_icon(&PaneStatus::Completed(1)), "x");
    }

    #[test]
    fn test_status_icon_error() {
        assert_eq!(status_icon(&PaneStatus::Error("fail".into())), "!");
    }

    // --- status_label tests ---

    #[test]
    fn test_status_label_running() {
        assert_eq!(status_label(&PaneStatus::Running), "Running");
    }

    #[test]
    fn test_status_label_completed_success() {
        assert_eq!(status_label(&PaneStatus::Completed(0)), "Completed");
    }

    #[test]
    fn test_status_label_completed_failure() {
        assert_eq!(status_label(&PaneStatus::Completed(1)), "Failed");
    }

    #[test]
    fn test_status_label_error() {
        assert_eq!(status_label(&PaneStatus::Error("x".into())), "Error");
    }

    // --- format_tab_label tests ---

    #[test]
    fn test_format_tab_label_first() {
        assert_eq!(format_tab_label(0, "claude"), "1:claude");
    }

    #[test]
    fn test_format_tab_label_third() {
        assert_eq!(format_tab_label(2, "gemini"), "3:gemini");
    }

    // --- CopyModeState tests (SPEC-1d6dd9fc FR-070) ---

    #[test]
    fn test_copy_mode_state_default_inactive() {
        let state = CopyModeState::default();
        assert!(!state.active);
    }

    #[test]
    fn test_copy_mode_state_default_cursor_at_origin() {
        let state = CopyModeState::default();
        assert_eq!(state.cursor_row, 0);
        assert_eq!(state.cursor_col, 0);
    }

    #[test]
    fn test_copy_mode_state_default_no_selection() {
        let state = CopyModeState::default();
        assert!(state.selection_start.is_none());
    }

    // --- agent_pane_title tests (FR-045) ---

    #[test]
    fn test_block_title_no_agent() {
        let manager = PaneManager::new();
        let title = agent_pane_title(&manager);
        assert_eq!(title, " No Agent ");
    }

    #[test]
    fn test_block_title_shows_agent_name() {
        use gwt_core::terminal::pane::{PaneConfig, TerminalPane};
        use std::collections::HashMap;
        let mut manager = PaneManager::new();
        let pane = TerminalPane::new(PaneConfig {
            pane_id: "p1".to_string(),
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "main".to_string(),
            agent_name: "claude".to_string(),
            agent_color: Color::Green,
            rows: 24,
            cols: 80,
            env_vars: HashMap::new(),
        })
        .unwrap();
        manager.add_pane(pane).unwrap();
        let title = agent_pane_title(&manager);
        assert_eq!(title, " claude ");
    }

    // --- FR-048: mouse_scroll_to_bytes tests ---

    #[test]
    fn test_mouse_scroll_to_bytes_up() {
        let pane_area = Rect::new(80, 0, 80, 40);
        let bytes = mouse_scroll_to_bytes(true, 90, 10, pane_area);
        // button=64, x=(90-80-1)+33=42, y=(10-0-1)+33=42
        assert_eq!(bytes[0], 0x1b);
        assert_eq!(bytes[1], b'[');
        assert_eq!(bytes[2], b'M');
        assert_eq!(bytes[3], 64); // ScrollUp button
        assert_eq!(bytes[4], 42); // x = 9 + 33
        assert_eq!(bytes[5], 42); // y = 9 + 33
    }

    #[test]
    fn test_mouse_scroll_to_bytes_down() {
        let pane_area = Rect::new(80, 0, 80, 40);
        let bytes = mouse_scroll_to_bytes(false, 90, 10, pane_area);
        assert_eq!(bytes[3], 65); // ScrollDown button
    }

    #[test]
    fn test_mouse_scroll_to_bytes_origin() {
        let pane_area = Rect::new(0, 0, 80, 40);
        let bytes = mouse_scroll_to_bytes(true, 1, 1, pane_area);
        // local_col = 1 - 0 - 1 = 0, local_row = 1 - 0 - 1 = 0
        assert_eq!(bytes[4], 33); // x = 0 + 33
        assert_eq!(bytes[5], 33); // y = 0 + 33
    }
}
