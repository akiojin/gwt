//! Confirmation Dialog Screen
//!
//! Displays a centered confirmation dialog with configurable title, message,
//! button labels, and dangerous-action styling.

#![allow(dead_code)]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Action to perform when confirmation is accepted
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    QuitWithAgents,
    CloseSession(String),
    DeleteBranch(String),
    DeleteWorktree(String),
    TerminateAgent(String),
    ForceKillAgent(String),
    Custom(String),
}

/// Confirmation dialog state
#[derive(Debug)]
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
    /// Action to perform on confirm
    pub on_confirm: ConfirmAction,
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self::new()
    }
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
            on_confirm: ConfirmAction::Custom("confirm".to_string()),
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
            on_confirm: ConfirmAction::DeleteBranch(item_name.to_string()),
        }
    }

    /// Create exit confirmation dialog when agents are running
    pub fn exit_with_running_agents(agent_count: usize) -> Self {
        Self {
            title: "Running Agents".to_string(),
            message: format!(
                "{} agent(s) are still running.\nExit will terminate all agents.",
                agent_count
            ),
            details: vec![
                "Press Enter to exit and terminate agents.".to_string(),
                "Press Esc to cancel and keep working.".to_string(),
            ],
            confirm_label: "Exit".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
            on_confirm: ConfirmAction::QuitWithAgents,
        }
    }

    /// Create termination confirmation dialog for a single agent
    pub fn terminate_agent(branch_name: &str, agent_name: &str) -> Self {
        Self {
            title: "Terminate Agent".to_string(),
            message: format!(
                "Terminate {} agent on branch '{}'?",
                agent_name, branch_name
            ),
            details: vec!["The agent will be sent SIGTERM to allow graceful shutdown.".to_string()],
            confirm_label: "Terminate".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
            on_confirm: ConfirmAction::TerminateAgent(branch_name.to_string()),
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
pub fn render_confirm(state: &ConfirmState, buf: &mut Buffer, area: Rect) {
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let detail_lines = state.details.len() as u16;
    let dialog_height = (8 + detail_lines).min(area.height.saturating_sub(4));

    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

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
    block.render(dialog_area, buf);

    let content_area = Rect::new(
        inner_area.x + 2,
        inner_area.y,
        inner_area.width.saturating_sub(4),
        inner_area.height,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                   // Message
            Constraint::Length(detail_lines.max(1)), // Details
            Constraint::Length(1),                   // Spacer
            Constraint::Length(1),                   // Buttons
        ])
        .split(content_area);

    // Message
    let message_style = if state.is_dangerous {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let message = Paragraph::new(state.message.as_str())
        .style(message_style)
        .wrap(Wrap { trim: false });
    message.render(chunks[0], buf);

    // Details
    if !state.details.is_empty() {
        let details_text: Vec<Line> = state
            .details
            .iter()
            .map(|line| Line::from(line.as_str()).style(Style::default().fg(Color::DarkGray)))
            .collect();
        let details = Paragraph::new(details_text).wrap(Wrap { trim: false });
        details.render(chunks[1], buf);
    }

    // Buttons
    let cancel_style = if !state.selected_confirm {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let cancel_text = format!("[ {} ]", state.cancel_label);

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

    let button_line = Line::from(vec![
        Span::styled(cancel_text, cancel_style),
        Span::raw("  "),
        Span::styled(confirm_text, confirm_style),
    ]);
    let buttons = Paragraph::new(button_line).alignment(Alignment::Center);
    buttons.render(chunks[3], buf);
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
    fn test_exit_with_running_agents() {
        let state = ConfirmState::exit_with_running_agents(3);
        assert!(state.is_dangerous);
        assert!(state.message.contains("3 agent"));
        assert_eq!(state.confirm_label, "Exit");
    }

    #[test]
    fn test_terminate_agent() {
        let state = ConfirmState::terminate_agent("feature/test", "claude");
        assert!(state.is_dangerous);
        assert!(state.message.contains("claude"));
        assert!(state.message.contains("feature/test"));
        assert_eq!(state.confirm_label, "Terminate");
    }

    #[test]
    fn test_select_confirm_cancel() {
        let mut state = ConfirmState::new();
        state.select_confirm();
        assert!(state.is_confirmed());
        state.select_cancel();
        assert!(!state.is_confirmed());
    }

    #[test]
    fn test_render_confirm_no_panic() {
        let state = ConfirmState::delete("test-branch");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_confirm(&state, &mut buf, area);
    }
}
