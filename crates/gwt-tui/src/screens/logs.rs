//! Logs viewer screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the logs screen.
#[derive(Debug, Default)]
pub struct LogsState;

/// Messages specific to the logs screen.
#[derive(Debug, Clone)]
pub enum LogsMessage {}

/// Render the logs screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Logs");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
