//! Session tab bar widget.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Tabs,
    Frame,
};

use crate::model::{Model, SessionTabType};

/// Render the session tab bar.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    let titles: Vec<Line> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let icon = match &s.tab_type {
                SessionTabType::Shell => "\u{25B6}",
                SessionTabType::Agent { .. } => "\u{2B50}",
            };
            let style = if i == model.active_session {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(format!(" {icon} {} ", s.name), style))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(model.active_session)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}
