//! Error overlay screen — shows errors from the model's error_queue.

use std::collections::VecDeque;

use gwt_core::logging::LogEvent as Notification;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme;

/// Messages specific to the error overlay.
#[derive(Debug, Clone)]
pub enum ErrorMessage {
    Dismiss,
}

/// Render the error overlay.
///
/// Takes the error_queue from the model directly.
/// Shows the first (oldest) error with a dismiss hint.
pub fn render(error_queue: &VecDeque<Notification>, frame: &mut Frame, area: Rect) {
    let notification = match error_queue.front() {
        Some(notification) => notification,
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
        .border_type(theme::border::modal())
        .border_style(Style::default().fg(theme::color::ERROR));

    let summary = if notification.source == "app" && notification.detail.is_none() {
        notification.message.clone()
    } else {
        format!("{}: {}", notification.source, notification.message)
    };

    let mut text = vec![Line::from(Span::styled(
        summary,
        theme::style::error_text(),
    ))];
    if let Some(detail) = notification
        .detail
        .as_deref()
        .filter(|detail| !detail.is_empty())
    {
        text.push(Line::from(Span::styled(
            detail.to_string(),
            Style::default().fg(theme::color::TEXT_PRIMARY),
        )));
    }
    text.push(Line::from(""));
    text.push(Line::from(Span::styled(
        "Press Enter or Esc to dismiss",
        theme::style::muted_text(),
    )));

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(theme::color::ERROR));
    frame.render_widget(paragraph, overlay);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::logging::{LogEvent as Notification, LogLevel as Severity};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_with_error_does_not_panic() {
        let errors: VecDeque<Notification> = vec![Notification::new(
            Severity::Error,
            "pty",
            "Something went wrong",
        )]
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
        assert!(full_text.contains("Error"));
    }

    #[test]
    fn render_empty_queue_is_noop() {
        let errors: VecDeque<Notification> = VecDeque::new();
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
        let errors: VecDeque<Notification> = vec![
            Notification::new(Severity::Error, "core", "Error 1"),
            Notification::new(Severity::Error, "core", "Error 2"),
            Notification::new(Severity::Error, "core", "Error 3"),
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
        let errors: VecDeque<Notification> =
            vec![Notification::new(Severity::Error, "core", "Only one")].into();
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

    #[test]
    fn render_structured_error_includes_source_and_detail() {
        let errors: VecDeque<Notification> =
            vec![Notification::new(Severity::Error, "pty", "Crash")
                .with_detail("stack trace line 1")]
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
        assert!(full_text.contains("pty: Crash"));
        assert!(full_text.contains("stack trace line 1"));
    }
}
