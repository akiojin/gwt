//! Error overlay screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the error overlay.
#[derive(Debug, Default)]
pub struct ErrorState;

/// Messages specific to the error overlay.
#[derive(Debug, Clone)]
pub enum ErrorMessage {}

/// Render the error overlay.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title("Error");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::Red));
    frame.render_widget(text, area);
}
