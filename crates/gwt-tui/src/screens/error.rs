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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_does_not_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Error"));
    }
}
