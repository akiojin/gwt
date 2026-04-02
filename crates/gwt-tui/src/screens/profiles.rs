//! Profiles management screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the profiles screen.
#[derive(Debug, Default)]
pub struct ProfilesState;

/// Messages specific to the profiles screen.
#[derive(Debug, Clone)]
pub enum ProfilesMessage {}

/// Render the profiles screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Profiles");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
