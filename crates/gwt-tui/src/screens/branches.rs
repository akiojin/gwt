//! Branches management screen (stub).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// State for the branches screen.
#[derive(Debug, Default)]
pub struct BranchesState;

/// Messages specific to the branches screen.
#[derive(Debug, Clone)]
pub enum BranchesMessage {}

/// Render the branches screen.
pub fn render(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Branches");
    let text = Paragraph::new("Not yet implemented")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(text, area);
}
