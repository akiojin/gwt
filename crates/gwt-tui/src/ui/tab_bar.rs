use gwt_core::terminal::AgentColor;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

use crate::state::{PaneStatusIndicator, TuiState};

/// Convert AgentColor to ratatui Color.
pub fn agent_color_to_ratatui(color: &AgentColor) -> Color {
    match color {
        AgentColor::Green => Color::Green,
        AgentColor::Blue => Color::Blue,
        AgentColor::Cyan => Color::Cyan,
        AgentColor::Red => Color::Red,
        AgentColor::Yellow => Color::Yellow,
        AgentColor::Magenta => Color::Magenta,
        AgentColor::White => Color::White,
        AgentColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
        AgentColor::Indexed(i) => Color::Indexed(*i),
    }
}

/// Status indicator character.
fn status_indicator(status: &PaneStatusIndicator) -> &'static str {
    match status {
        PaneStatusIndicator::Running => "*",
        PaneStatusIndicator::Idle => "o",
        PaneStatusIndicator::Completed(_) => "-",
        PaneStatusIndicator::Error(_) => "!",
    }
}

/// Render the tab bar into the given area.
pub fn render(buf: &mut Buffer, area: Rect, state: &TuiState) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let mut spans: Vec<Span> = Vec::new();

    for (i, tab) in state.tabs.iter().enumerate() {
        let is_active = i == state.active_tab;
        let indicator = status_indicator(&tab.status);
        let pane_info = if tab.pane_count > 1 {
            format!("split({})", tab.pane_count)
        } else {
            tab.name.clone()
        };
        let label = format!("W{}: {} {}", i + 1, indicator, pane_info);

        let color = agent_color_to_ratatui(&tab.color);
        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };

        spans.push(Span::styled(format!(" [{label}] "), style));
    }

    // Add [+] button
    spans.push(Span::styled(" [+] ", Style::default().fg(Color::DarkGray)));

    let line = Line::from(spans);
    let line_widget =
        ratatui::widgets::Paragraph::new(line).style(Style::default().bg(Color::Black));
    line_widget.render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TabInfo;

    fn make_tab(name: &str, color: AgentColor, active: bool) -> TabInfo {
        TabInfo {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            color,
            status: if active {
                PaneStatusIndicator::Running
            } else {
                PaneStatusIndicator::Idle
            },
            branch: None,
            spec_id: None,
            pane_count: 1,
        }
    }

    #[test]
    fn test_agent_color_conversion() {
        assert_eq!(agent_color_to_ratatui(&AgentColor::Green), Color::Green);
        assert_eq!(agent_color_to_ratatui(&AgentColor::Blue), Color::Blue);
        assert_eq!(
            agent_color_to_ratatui(&AgentColor::Rgb(255, 0, 128)),
            Color::Rgb(255, 0, 128)
        );
        assert_eq!(
            agent_color_to_ratatui(&AgentColor::Indexed(42)),
            Color::Indexed(42)
        );
    }

    #[test]
    fn test_render_empty_state() {
        let state = TuiState::new();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
        // Should render at least the [+] button
        let content: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(content.contains("[+]"));
    }

    #[test]
    fn test_render_with_tabs() {
        let mut state = TuiState::new();
        state.add_tab(make_tab("claude", AgentColor::Green, true));
        state.add_tab(make_tab("shell", AgentColor::White, false));
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
        let content: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(content.contains("W1"));
        assert!(content.contains("W2"));
    }

    #[test]
    fn test_render_zero_area() {
        let state = TuiState::new();
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
        // Should not panic
    }

    #[test]
    fn test_status_indicator_chars() {
        assert_eq!(status_indicator(&PaneStatusIndicator::Running), "*");
        assert_eq!(status_indicator(&PaneStatusIndicator::Idle), "o");
        assert_eq!(status_indicator(&PaneStatusIndicator::Completed(0)), "-");
        assert_eq!(
            status_indicator(&PaneStatusIndicator::Error("err".into())),
            "!"
        );
    }
}
