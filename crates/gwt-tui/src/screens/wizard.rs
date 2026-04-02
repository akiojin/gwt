//! Wizard overlay screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the wizard overlay.
#[derive(Debug, Default)]
pub struct WizardState;

/// Messages specific to the wizard overlay.
#[derive(Debug, Clone)]
pub enum WizardMessage {}

/// Render the wizard overlay.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Wizard");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
