//! Error overlay screen — shows errors from the model's error_queue.

use std::collections::VecDeque;

use gwt_core::logging::LogEvent as Notification;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

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

    let width = (area.width * 60 / 100).max(40);
    // Inner width available for text (excluding left/right borders).
    let inner_width = width.saturating_sub(2).max(1) as usize;

    let summary = if notification.source == "app" && notification.detail.is_none() {
        notification.message.clone()
    } else {
        format!("{}: {}", notification.source, notification.message)
    };

    let mut text = vec![Line::from(Span::styled(
        summary.clone(),
        theme::style::error_text(),
    ))];
    let mut content_lines: u16 = wrapped_line_count(&summary, inner_width);

    if let Some(detail) = notification
        .detail
        .as_deref()
        .filter(|detail| !detail.is_empty())
    {
        text.push(Line::from(Span::styled(
            detail.to_string(),
            Style::default().fg(theme::color::TEXT_PRIMARY),
        )));
        content_lines += wrapped_line_count(detail, inner_width);
    }
    text.push(Line::from(""));
    text.push(Line::from(Span::styled(
        "Press Enter or Esc to dismiss",
        theme::style::muted_text(),
    )));
    // +2 for the empty line and dismiss hint, +2 for top/bottom borders.
    let height = (content_lines + 2 + 2).min(area.height * 60 / 100).max(5);

    let overlay = super::centered_rect(width, height, area);
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

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(theme::color::ERROR));
    frame.render_widget(paragraph, overlay);
}

/// Calculate how many display lines a string occupies when wrapped to `max_width`.
fn wrapped_line_count(text: &str, max_width: usize) -> u16 {
    if max_width == 0 {
        return 1;
    }
    let width = UnicodeWidthStr::width(text);
    if width == 0 {
        return 1;
    }
    width.div_ceil(max_width) as u16
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
    fn wrapped_line_count_basic() {
        assert_eq!(wrapped_line_count("hello", 10), 1);
        assert_eq!(wrapped_line_count("hello world!!", 10), 2);
        assert_eq!(wrapped_line_count("", 10), 1);
        assert_eq!(wrapped_line_count("x", 0), 1);
        // Exact boundary: 20 chars in width 10 → 2 lines
        assert_eq!(wrapped_line_count("12345678901234567890", 10), 2);
        // 21 chars → 3 lines
        assert_eq!(wrapped_line_count("123456789012345678901", 10), 3);
    }

    #[test]
    fn render_long_message_wraps_without_truncation() {
        let long_msg = "A".repeat(120);
        let errors: VecDeque<Notification> =
            vec![Notification::new(Severity::Error, "app", &long_msg)].into();
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Collect text from each row separately so we can join wrapped content.
        let mut all_text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                all_text.push_str(buf[(x, y)].symbol());
            }
        }
        // The full 120-char message should appear (possibly across wrapped lines).
        let a_count = all_text.chars().filter(|ch| *ch == 'A').count();
        assert!(
            a_count >= 120,
            "Expected at least 120 'A' chars in buffer, found {a_count}"
        );
    }

    #[test]
    fn render_long_detail_expands_height() {
        let long_detail = "D".repeat(200);
        let errors: VecDeque<Notification> =
            vec![Notification::new(Severity::Error, "core", "Short").with_detail(&long_detail)]
                .into();
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&errors, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut all_text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                all_text.push_str(buf[(x, y)].symbol());
            }
        }
        let d_count = all_text.chars().filter(|ch| *ch == 'D').count();
        assert!(
            d_count >= 200,
            "Expected at least 200 'D' chars in buffer, found {d_count}"
        );
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
