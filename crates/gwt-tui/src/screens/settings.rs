//! Settings management screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the settings screen.
#[derive(Debug, Default)]
pub struct SettingsState;

/// Messages specific to the settings screen.
#[derive(Debug, Clone)]
pub enum SettingsMessage {}

/// Render the settings screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Settings");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
