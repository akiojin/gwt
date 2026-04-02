//! Versions screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the versions screen.
#[derive(Debug, Default)]
pub struct VersionsState;

/// Messages specific to the versions screen.
#[derive(Debug, Clone)]
pub enum VersionsMessage {}

/// Render the versions screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Versions");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
