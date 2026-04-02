//! Issues management screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the issues screen.
#[derive(Debug, Default)]
pub struct IssuesState;

/// Messages specific to the issues screen.
#[derive(Debug, Clone)]
pub enum IssuesMessage {}

/// Render the issues screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Issues");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
