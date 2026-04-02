//! Status bar widget — footer with session info, branch, and help hint.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{ActiveLayer, Model, SessionLayout};

/// Render the status bar.
pub fn render(model: &Model, frame: &mut Frame, area: Rect) {
    let session_name = model
        .active_session_tab()
        .map(|s| s.name.as_str())
        .unwrap_or("No session");

    let layout_icon = match model.session_layout {
        SessionLayout::Tab => "\u{25A3}",
        SessionLayout::Grid => "\u{25A6}",
    };

    let layer = match model.active_layer {
        ActiveLayer::Main => "Main",
        ActiveLayer::Management => "Mgmt",
    };

    let status = Line::from(vec![
        Span::styled(
            format!(" {layout_icon} {session_name} "),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!(" [{layer}] "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", model.repo_path.display()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" Ctrl+G,? Help ", Style::default().fg(Color::DarkGray)),
    ]);

    let bar = Paragraph::new(status).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(bar, area);
}
