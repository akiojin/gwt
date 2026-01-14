//! Error Display Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Error severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorSeverity {
    #[default]
    Error,
    Warning,
    Info,
}

/// Error display state
#[derive(Debug, Default)]
pub struct ErrorState {
    /// Error title
    pub title: String,
    /// Error message
    pub message: String,
    /// Error code (optional)
    pub code: Option<String>,
    /// Error details/stack trace (optional)
    pub details: Vec<String>,
    /// Suggested actions
    pub suggestions: Vec<String>,
    /// Severity level
    pub severity: ErrorSeverity,
    /// Scroll offset for details
    pub scroll_offset: usize,
}

impl ErrorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an error state from message
    pub fn from_error(message: &str) -> Self {
        Self {
            title: "Error".to_string(),
            message: message.to_string(),
            severity: ErrorSeverity::Error,
            ..Default::default()
        }
    }

    /// Create an error state with code
    pub fn with_code(mut self, code: &str) -> Self {
        self.code = Some(code.to_string());
        self
    }

    /// Add details
    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    /// Add suggestions
    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// Set severity to warning
    pub fn with_warning_severity(mut self) -> Self {
        self.severity = ErrorSeverity::Warning;
        self.title = "Warning".to_string();
        self
    }

    /// Set severity to info
    pub fn with_info_severity(mut self) -> Self {
        self.severity = ErrorSeverity::Info;
        self.title = "Information".to_string();
        self
    }

    /// Scroll down
    pub fn scroll_down(&mut self) {
        if !self.details.is_empty() && self.scroll_offset < self.details.len().saturating_sub(1) {
            self.scroll_offset += 1;
        }
    }

    /// Scroll up
    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }
}

/// Render error screen
pub fn render_error(state: &ErrorState, frame: &mut Frame, area: Rect) {
    const H_PADDING: u16 = 2;
    // Calculate dialog size
    let dialog_width = 70.min(area.width.saturating_sub(4));
    let base_height = 8;
    let details_height = state.details.len().min(10) as u16;
    let suggestions_height = if state.suggestions.is_empty() {
        0
    } else {
        state.suggestions.len() as u16 + 2
    };
    let dialog_height =
        (base_height + details_height + suggestions_height).min(area.height.saturating_sub(4));

    // Center the dialog
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // Clear the background
    frame.render_widget(Clear, dialog_area);

    // Border color based on severity
    let border_color = match state.severity {
        ErrorSeverity::Error => Color::Red,
        ErrorSeverity::Warning => Color::Yellow,
        ErrorSeverity::Info => Color::Cyan,
    };

    // Icon based on severity
    let icon = match state.severity {
        ErrorSeverity::Error => "X",
        ErrorSeverity::Warning => "!",
        ErrorSeverity::Info => "i",
    };

    let title = if let Some(code) = &state.code {
        format!(" {} {} [{}] ", icon, state.title, code)
    } else {
        format!(" {} {} ", icon, state.title)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    let content_area = Rect::new(
        inner_area.x + H_PADDING,
        inner_area.y,
        inner_area.width.saturating_sub(H_PADDING.saturating_mul(2)),
        inner_area.height,
    );
    frame.render_widget(block, dialog_area);

    // Layout
    let mut constraints = vec![
        Constraint::Length(3), // Message
    ];
    if !state.details.is_empty() {
        constraints.push(Constraint::Length(details_height + 2));
    }
    if !state.suggestions.is_empty() {
        constraints.push(Constraint::Length(suggestions_height));
    }
    constraints.push(Constraint::Length(2)); // Footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(content_area);

    let mut chunk_idx = 0;

    // Message
    let message_style = Style::default().fg(border_color);
    let message = Paragraph::new(state.message.clone())
        .style(message_style)
        .wrap(Wrap { trim: true });
    frame.render_widget(message, chunks[chunk_idx]);
    chunk_idx += 1;

    // Details
    if !state.details.is_empty() {
        let details_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Details ")
            .title_style(Style::default().fg(Color::DarkGray));

        let details_inner = details_block.inner(chunks[chunk_idx]);
        frame.render_widget(details_block, chunks[chunk_idx]);

        let visible_details: Vec<Line> = state
            .details
            .iter()
            .skip(state.scroll_offset)
            .take(details_inner.height as usize)
            .map(|d| Line::from(d.as_str()).style(Style::default().fg(Color::DarkGray)))
            .collect();

        let details = Paragraph::new(visible_details);
        frame.render_widget(details, details_inner);
        chunk_idx += 1;
    }

    // Suggestions
    if !state.suggestions.is_empty() {
        let suggestions_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Suggestions ")
            .title_style(Style::default().fg(Color::Green));

        let suggestions_inner = suggestions_block.inner(chunks[chunk_idx]);
        frame.render_widget(suggestions_block, chunks[chunk_idx]);

        let suggestions_text: Vec<Line> = state
            .suggestions
            .iter()
            .map(|s| {
                Line::from(vec![
                    Span::styled("-> ", Style::default().fg(Color::Green)),
                    Span::raw(s.as_str()),
                ])
            })
            .collect();

        let suggestions = Paragraph::new(suggestions_text);
        frame.render_widget(suggestions, suggestions_inner);
        chunk_idx += 1;
    }

    // Footer
    let footer = Paragraph::new("[Enter/Esc] Close")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[chunk_idx]);
}

/// Helper function to create a centered rect
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_state_creation() {
        let state = ErrorState::from_error("Something went wrong");
        assert_eq!(state.message, "Something went wrong");
        assert_eq!(state.severity, ErrorSeverity::Error);
    }

    #[test]
    fn test_error_with_code() {
        let state = ErrorState::from_error("Git error").with_code("E1001");
        assert_eq!(state.code, Some("E1001".to_string()));
    }

    #[test]
    fn test_severity_variants() {
        let error = ErrorState::from_error("Error");
        assert_eq!(error.title, "Error");

        let warning = ErrorState::from_error("Warning").with_warning_severity();
        assert_eq!(warning.title, "Warning");
        assert_eq!(warning.severity, ErrorSeverity::Warning);

        let info = ErrorState::from_error("Info").with_info_severity();
        assert_eq!(info.title, "Information");
        assert_eq!(info.severity, ErrorSeverity::Info);
    }

    #[test]
    fn test_scroll() {
        let mut state = ErrorState::from_error("Error").with_details(vec![
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
        ]);

        assert_eq!(state.scroll_offset, 0);

        state.scroll_down();
        assert_eq!(state.scroll_offset, 1);

        state.scroll_down();
        assert_eq!(state.scroll_offset, 2);

        state.scroll_down(); // Should not go beyond last item
        assert_eq!(state.scroll_offset, 2);

        state.scroll_up();
        assert_eq!(state.scroll_offset, 1);
    }
}
