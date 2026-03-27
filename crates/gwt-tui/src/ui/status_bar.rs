use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Widget,
};

use crate::state::{PaneStatusIndicator, TuiState};

/// Render the status bar into the given area.
pub fn render(buf: &mut Buffer, area: Rect, state: &TuiState) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let spans = if let Some(tab) = state.active_tab_info() {
        let status_str = match &tab.status {
            PaneStatusIndicator::Running => "running",
            PaneStatusIndicator::Idle => "idle",
            PaneStatusIndicator::Completed(code) => {
                if *code == 0 {
                    "completed"
                } else {
                    "failed"
                }
            }
            PaneStatusIndicator::Error(_) => "error",
        };

        let mut parts = vec![
            Span::styled(
                format!(
                    " W{} | Pane 1/{} ",
                    state.active_tab + 1,
                    tab.pane_count
                ),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::styled(
                format!("| {status_str} "),
                Style::default().fg(Color::Yellow).bg(Color::DarkGray),
            ),
        ];

        if let Some(ref branch) = tab.branch {
            parts.push(Span::styled(
                format!("| {branch} "),
                Style::default().fg(Color::Cyan).bg(Color::DarkGray),
            ));
        }

        if let Some(ref spec_id) = tab.spec_id {
            parts.push(Span::styled(
                format!("| {spec_id} "),
                Style::default().fg(Color::Magenta).bg(Color::DarkGray),
            ));
        }

        parts
    } else {
        vec![Span::styled(
            " gwt | No active windows ",
            Style::default().fg(Color::DarkGray).bg(Color::Black),
        )]
    };

    let line = Line::from(spans);
    let paragraph = ratatui::widgets::Paragraph::new(line)
        .style(Style::default().bg(Color::DarkGray));
    paragraph.render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TabInfo, TuiState};
    use gwt_core::terminal::AgentColor;

    #[test]
    fn test_render_no_tabs() {
        let state = TuiState::new();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
        let content: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(content.contains("No active windows"));
    }

    #[test]
    fn test_render_with_active_tab() {
        let mut state = TuiState::new();
        state.add_tab(TabInfo {
            pane_id: "p1".into(),
            name: "claude".into(),
            color: AgentColor::Green,
            status: PaneStatusIndicator::Running,
            branch: Some("feature/test".into()),
            spec_id: Some("SPEC-42".into()),
            pane_count: 1,
        });
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
        let content: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(content.contains("W1"));
        assert!(content.contains("running"));
        assert!(content.contains("feature/test"));
        assert!(content.contains("SPEC-42"));
    }

    #[test]
    fn test_render_zero_area() {
        let state = TuiState::new();
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
    }
}
