//! Confirmation dialog overlay (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the confirmation dialog.
#[derive(Debug, Default)]
pub struct ConfirmState;

/// Messages specific to the confirmation dialog.
#[derive(Debug, Clone)]
pub enum ConfirmMessage {}

/// Render the confirmation dialog.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Confirm");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
