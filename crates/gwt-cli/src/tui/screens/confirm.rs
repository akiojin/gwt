//! Confirmation Dialog Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Confirmation dialog state
#[derive(Debug, Default)]
pub struct ConfirmState {
    /// Dialog title
    pub title: String,
    /// Dialog message
    pub message: String,
    /// Additional details (optional)
    pub details: Vec<String>,
    /// Confirm button label
    pub confirm_label: String,
    /// Cancel button label
    pub cancel_label: String,
    /// Currently selected button (true = confirm, false = cancel)
    pub selected_confirm: bool,
    /// Is this a dangerous action (shows in red)
    pub is_dangerous: bool,
}

impl ConfirmState {
    pub fn new() -> Self {
        Self {
            title: "Confirm".to_string(),
            message: "Are you sure?".to_string(),
            details: Vec::new(),
            confirm_label: "Confirm".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false, // Default to cancel for safety
            is_dangerous: false,
        }
    }

    /// Create a delete confirmation dialog
    pub fn delete(item_name: &str) -> Self {
        Self {
            title: "Delete Confirmation".to_string(),
            message: format!("Are you sure you want to delete '{}'?", item_name),
            details: vec!["This action cannot be undone.".to_string()],
            confirm_label: "Delete".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
        }
    }

    /// Create a cleanup confirmation dialog
    pub fn cleanup(branches: &[String]) -> Self {
        Self {
            title: "Cleanup Confirmation".to_string(),
            message: format!("Delete {} branch(es)?", branches.len()),
            details: branches.iter().map(|b| format!("  - {}", b)).collect(),
            confirm_label: "Cleanup".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
        }
    }

    /// Create an unsafe branch selection warning dialog (FR-029b)
    pub fn unsafe_selection_warning(
        branch_name: &str,
        has_uncommitted: bool,
        has_unpushed: bool,
        is_unmerged: bool,
    ) -> Self {
        let mut reasons = Vec::new();
        if has_uncommitted {
            reasons.push("- Has uncommitted changes".to_string());
        }
        if has_unpushed {
            reasons.push("- Has unpushed commits".to_string());
        }
        if is_unmerged {
            reasons.push("- Is unmerged with main branch".to_string());
        }

        Self {
            title: "Warning: Unsafe Branch".to_string(),
            message: format!("Branch '{}' may have unsaved work:", branch_name),
            details: reasons,
            confirm_label: "OK".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
        }
    }

    /// Toggle selection
    pub fn toggle_selection(&mut self) {
        self.selected_confirm = !self.selected_confirm;
    }

    /// Select confirm
    pub fn select_confirm(&mut self) {
        self.selected_confirm = true;
    }

    /// Select cancel
    pub fn select_cancel(&mut self) {
        self.selected_confirm = false;
    }

    /// Check if confirm is selected
    pub fn is_confirmed(&self) -> bool {
        self.selected_confirm
    }
}

/// Render confirmation dialog
pub fn render_confirm(state: &ConfirmState, frame: &mut Frame, area: Rect) {
    // Calculate dialog size
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let content_lines = 4 + state.details.len() as u16; // title + message + details + spacer + buttons
    let dialog_height = (content_lines + 4).min(area.height.saturating_sub(4));

    // Center the dialog
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // Clear the background
    frame.render_widget(Clear, dialog_area);

    // Dialog border
    let border_style = if state.is_dangerous {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {} ", state.title))
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // Layout for content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Message
            Constraint::Min(1),    // Details
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Buttons
        ])
        .split(inner_area);

    // Message
    let message_style = if state.is_dangerous {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let message = Paragraph::new(state.message.clone())
        .style(message_style)
        .alignment(Alignment::Center);
    frame.render_widget(message, chunks[0]);

    // Details
    if !state.details.is_empty() {
        let details_text: Vec<Line> = state
            .details
            .iter()
            .take(chunks[1].height as usize)
            .map(|d| {
                let line_text = if d.starts_with(' ') {
                    d.clone()
                } else {
                    format!(" {}", d)
                };
                Line::from(line_text).style(Style::default().fg(Color::DarkGray))
            })
            .collect();
        let details = Paragraph::new(details_text);
        frame.render_widget(details, chunks[1]);
    }

    // Buttons
    let button_area = chunks[3];
    let button_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(button_area);

    // Cancel button
    let cancel_style = if !state.selected_confirm {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let cancel_text = format!("[ {} ]", state.cancel_label);
    let cancel = Paragraph::new(cancel_text)
        .style(cancel_style)
        .alignment(Alignment::Center);
    frame.render_widget(cancel, button_layout[0]);

    // Confirm button
    let confirm_style = if state.selected_confirm {
        if state.is_dangerous {
            Style::default().bg(Color::Red).fg(Color::White)
        } else {
            Style::default().bg(Color::Green).fg(Color::Black)
        }
    } else if state.is_dangerous {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    let confirm_text = format!("[ {} ]", state.confirm_label);
    let confirm = Paragraph::new(confirm_text)
        .style(confirm_style)
        .alignment(Alignment::Center);
    frame.render_widget(confirm, button_layout[1]);
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
    fn test_confirm_state_toggle() {
        let mut state = ConfirmState::new();
        assert!(!state.selected_confirm);

        state.toggle_selection();
        assert!(state.selected_confirm);

        state.toggle_selection();
        assert!(!state.selected_confirm);
    }

    #[test]
    fn test_delete_dialog() {
        let state = ConfirmState::delete("feature/test");
        assert!(state.is_dangerous);
        assert!(state.message.contains("feature/test"));
        assert_eq!(state.confirm_label, "Delete");
    }

    #[test]
    fn test_cleanup_dialog() {
        let branches = vec!["branch1".to_string(), "branch2".to_string()];
        let state = ConfirmState::cleanup(&branches);
        assert!(state.is_dangerous);
        assert_eq!(state.details.len(), 2);
    }
}
