//! PR Dashboard screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the PR dashboard screen.
#[derive(Debug, Default)]
pub struct PrDashboardState;

/// Messages specific to the PR dashboard screen.
#[derive(Debug, Clone)]
pub enum PrDashboardMessage {}

/// Render the PR dashboard screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("PR Dashboard");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
