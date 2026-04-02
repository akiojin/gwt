//! Git View screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the git view screen.
#[derive(Debug, Default)]
pub struct GitViewState;

/// Messages specific to the git view screen.
#[derive(Debug, Clone)]
pub enum GitViewMessage {}

/// Render the git view screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Git View");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
