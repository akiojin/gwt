//! Specs management screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the specs screen.
#[derive(Debug, Default)]
pub struct SpecsState;

/// Messages specific to the specs screen.
#[derive(Debug, Clone)]
pub enum SpecsMessage {}

/// Render the specs screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Specs");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
