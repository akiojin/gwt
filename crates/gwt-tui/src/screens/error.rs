//! Error overlay screen — shows errors from the model's error_queue.

use std::collections::VecDeque;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Messages specific to the error overlay.
#[derive(Debug, Clone)]
pub enum ErrorMessage {
    Dismiss,
}

/// Render the error overlay.
///
/// Takes the error_queue from the model directly.
/// Shows the first (oldest) error with a dismiss hint.
pub fn render(error_queue: &VecDeque<String>, frame: &mut Frame, area: Rect) {
    let err = match error_queue.front() {
        Some(e) => e,
        None => return, // Nothing to show
    };

    let queue_count = error_queue.len();

    // Centered overlay
    let width = (area.width * 60 / 100).max(40);
    let overlay = super::centered_rect(width, 7, area);

    frame.render_widget(Clear, overlay);

    let title = if queue_count > 1 {
        format!("Error (1 of {})", queue_count)
    } else {
        "Error".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Red));

    let text = vec![
        Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter or Esc to dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::Red));
    frame.render_widget(paragraph, overlay);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_with_error_does_not_panic() {
        let errors: VecDeque<String> = vec!["Something went wrong".to_string()].into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(full_text.contains("Error"));
    }

    #[test]
    fn render_empty_queue_is_noop() {
        let errors: VecDeque<String> = VecDeque::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        // Should not render "Error" anywhere
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(!full_text.contains("Error"));
    }

    #[test]
    fn render_multiple_errors_shows_count() {
        let errors: VecDeque<String> = vec![
            "Error 1".to_string(),
            "Error 2".to_string(),
            "Error 3".to_string(),
        ]
        .into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(full_text.contains("1 of 3"));
    }

    #[test]
    fn render_single_error_no_count() {
        let errors: VecDeque<String> = vec!["Only one".to_string()].into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        // Should just say "Error", not "Error (1 of 1)"
        assert!(full_text.contains("Error"));
        assert!(!full_text.contains("1 of 1"));
    }

    #[test]
    fn error_message_dismiss_variant_exists() {
        let msg = ErrorMessage::Dismiss;
        // Just verify it compiles and can be matched
        match msg {
            ErrorMessage::Dismiss => {}
        }
    }
}
