//! Help Overlay Screen

use ratatui::{prelude::*, widgets::*};

/// Help state
#[derive(Debug, Default)]
pub struct HelpState {
    pub scroll_offset: usize,
}

impl HelpState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scroll up
    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll down
    pub fn scroll_down(&mut self) {
        self.scroll_offset += 1;
    }

    /// Page up
    pub fn page_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(10);
    }

    /// Page down
    pub fn page_down(&mut self) {
        self.scroll_offset += 10;
    }
}

/// Help content sections
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("Up/k", "Move selection up"),
            ("Down/j", "Move selection down"),
            ("PageUp", "Page up"),
            ("PageDown", "Page down"),
            ("Home/g", "Go to first item"),
            ("End/G", "Go to last item"),
        ],
    ),
    (
        "Actions",
        &[
            ("Enter", "Select/Confirm"),
            ("n", "New worktree"),
            ("d", "Delete worktree"),
            ("s", "Switch to worktree"),
            ("r", "Refresh data"),
        ],
    ),
    (
        "Screens",
        &[
            ("1", "Branch list"),
            ("2", "Worktree list"),
            ("3", "Settings"),
            ("4", "Logs"),
        ],
    ),
    (
        "General",
        &[
            ("?/F1", "Show this help"),
            ("/", "Search/Filter"),
            ("Tab", "Next section/tab"),
            ("Esc", "Close/Cancel/Back"),
            ("q", "Quit application"),
        ],
    ),
];

/// Render help overlay
pub fn render_help(state: &HelpState, frame: &mut Frame, area: Rect) {
    // Calculate centered area (80% width, 80% height)
    let popup_area = centered_rect(80, 80, area);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Build help content
    let mut lines: Vec<Line> = Vec::new();

    for (section_title, items) in HELP_SECTIONS {
        // Section header
        lines.push(Line::from(vec![Span::styled(
            format!(" {} ", section_title),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        // Items
        for (key, desc) in *items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:12}", key), Style::default().fg(Color::Yellow)),
                Span::raw(" - "),
                Span::raw(*desc),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Footer
    lines.push(Line::from(vec![Span::styled(
        " Press Esc or ? to close ",
        Style::default().fg(Color::DarkGray),
    )]));

    // Apply scroll offset
    let visible_lines: Vec<Line> = lines.into_iter().skip(state.scroll_offset).collect();

    let help = Paragraph::new(visible_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Help ")
                .title_alignment(Alignment::Center),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(help, popup_area);
}

/// Helper to create centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll() {
        let mut state = HelpState::new();
        assert_eq!(state.scroll_offset, 0);

        state.scroll_down();
        assert_eq!(state.scroll_offset, 1);

        state.scroll_up();
        assert_eq!(state.scroll_offset, 0);

        state.scroll_up(); // Should not go negative
        assert_eq!(state.scroll_offset, 0);

        state.page_down();
        assert_eq!(state.scroll_offset, 10);

        state.page_up();
        assert_eq!(state.scroll_offset, 0);
    }
}
