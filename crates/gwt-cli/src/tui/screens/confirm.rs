//! Confirmation Dialog Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Truncate a path for display, keeping the end portion
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}

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
    // Mouse click support
    /// Cached popup area
    pub popup_area: Option<Rect>,
    /// Cached cancel button area
    pub cancel_button_area: Option<Rect>,
    /// Cached confirm button area
    pub confirm_button_area: Option<Rect>,
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        }
    }

    /// Create exit confirmation dialog when agents are running (tmux mode)
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
            ..Default::default()
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
            ..Default::default()
        }
    }

    /// Create force kill confirmation dialog
    pub fn force_kill_agent(branch_name: &str, agent_name: &str) -> Self {
        Self {
            title: "Force Kill Agent".to_string(),
            message: format!(
                "Force kill {} agent on branch '{}'?",
                agent_name, branch_name
            ),
            details: vec![
                "SIGKILL will be sent. Unsaved work may be lost.".to_string(),
                "Use this only if the agent is not responding.".to_string(),
            ],
            confirm_label: "Force Kill".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: true,
            ..Default::default()
        }
    }

    /// Create duplicate agent launch warning dialog
    pub fn duplicate_agent_warning(branch_name: &str, agent_name: &str) -> Self {
        Self {
            title: "Duplicate Agent".to_string(),
            message: format!(
                "{} agent is already running on branch '{}'.",
                agent_name, branch_name
            ),
            details: vec!["Launch another instance anyway?".to_string()],
            confirm_label: "Launch".to_string(),
            cancel_label: "Cancel".to_string(),
            selected_confirm: false,
            is_dangerous: false,
            ..Default::default()
        }
    }

    /// Create Claude Code hook setup proposal dialog (SPEC-861d8cdf T-104)
    pub fn hook_setup() -> Self {
        Self {
            title: "Setup Agent Status Tracking".to_string(),
            message: "Enable Claude Code hook for agent status tracking?".to_string(),
            details: vec![
                "This will add gwt hooks to Claude Code settings.".to_string(),
                "You can run 'gwt hook status' to check at any time.".to_string(),
            ],
            confirm_label: "Setup".to_string(),
            cancel_label: "Skip".to_string(),
            selected_confirm: true, // Default to setup for better UX
            is_dangerous: false,
            ..Default::default()
        }
    }

    /// Create hook setup warning dialog for temporary execution (FR-102i)
    ///
    /// When gwt is running from bunx/npx cache, hooks may not work correctly
    /// because the executable path will change after cache cleanup.
    pub fn hook_setup_with_warning(exe_path: &str) -> Self {
        Self {
            title: "Warning: Temporary Execution".to_string(),
            message: "Hook setup may not work correctly.".to_string(),
            details: vec![
                format!("Running from: {}", truncate_path(exe_path, 50)),
                "Hooks will break after cache cleanup.".to_string(),
                "Install globally: npm install -g @akiojin/gwt".to_string(),
            ],
            confirm_label: "Setup Anyway".to_string(),
            cancel_label: "Skip".to_string(),
            selected_confirm: false, // Default to Skip for safety
            is_dangerous: true,
            ..Default::default()
        }
    }

    /// Create Claude Code plugin setup confirmation dialog (SPEC-f8dab6e2 T-109)
    ///
    /// Proposes to register gwt-plugins marketplace and enable worktree-protection-hooks plugin.
    pub fn plugin_setup() -> Self {
        Self {
            title: "Setup Worktree Protection Plugin".to_string(),
            message: "Enable worktree-protection-hooks plugin for Claude Code?".to_string(),
            details: vec![
                "This will register gwt-plugins marketplace.".to_string(),
                "Plugin protects against dangerous operations.".to_string(),
            ],
            confirm_label: "Setup".to_string(),
            cancel_label: "Skip".to_string(),
            selected_confirm: true, // Default to Setup for better UX
            is_dangerous: false,
            ..Default::default()
        }
    }

    // Mouse click support methods

    /// Check if point is within popup area
    pub fn is_point_in_popup(&self, x: u16, y: u16) -> bool {
        self.popup_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    }

    /// Check if point is on cancel button
    pub fn is_cancel_button_at(&self, x: u16, y: u16) -> bool {
        self.cancel_button_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    }

    /// Check if point is on confirm button
    pub fn is_confirm_button_at(&self, x: u16, y: u16) -> bool {
        self.confirm_button_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
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
pub fn render_confirm(state: &mut ConfirmState, frame: &mut Frame, area: Rect) {
    const H_PADDING: usize = 2;
    let message_lines: Vec<String> = state
        .message
        .lines()
        .map(|line| format!(" {}", line))
        .collect();
    let details_lines: Vec<String> = state
        .details
        .iter()
        .map(|detail| {
            if detail.starts_with(' ') {
                detail.clone()
            } else {
                format!("  {}", detail)
            }
        })
        .collect();
    let button_text = format!("[ {} ]  [ {} ]", state.cancel_label, state.confirm_label);

    let max_line_len = |lines: &[String]| -> usize {
        lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
    };

    let max_content_len = max_line_len(&message_lines)
        .max(max_line_len(&details_lines))
        .max(button_text.chars().count())
        .max(state.title.chars().count() + 2);

    // Calculate dialog size
    let max_width = area.width.saturating_sub(4).max(20) as usize;
    let desired_width = max_content_len + 2 + (H_PADDING * 2);
    let dialog_width = (desired_width.max(40)).min(max_width) as u16;
    let inner_width = dialog_width
        .saturating_sub(2 + (H_PADDING * 2) as u16)
        .max(1) as usize;

    let wrapped_line_count = |lines: &[String], width: usize| -> usize {
        if width == 0 {
            return lines.len();
        }
        lines
            .iter()
            .map(|line| {
                let len = line.chars().count();
                if len == 0 {
                    1
                } else {
                    len.div_ceil(width)
                }
            })
            .sum()
    };

    let mut message_height = wrapped_line_count(&message_lines, inner_width);
    let mut details_height = wrapped_line_count(&details_lines, inner_width);
    let spacer_height = 1usize;
    let buttons_height = 1usize;

    let content_height = message_height + details_height + spacer_height + buttons_height;
    let max_height = area.height.saturating_sub(4).max(5) as usize;
    let dialog_height = (content_height + 2).min(max_height) as u16;

    let available_inner = dialog_height.saturating_sub(2) as usize;
    let reserved = spacer_height + buttons_height;
    if message_height + reserved > available_inner {
        details_height = 0;
    } else {
        let max_details = available_inner.saturating_sub(message_height + reserved);
        if details_height > max_details {
            details_height = max_details;
        }
    }

    if message_height > available_inner.saturating_sub(reserved) {
        message_height = available_inner.saturating_sub(reserved);
    }

    // Center the dialog
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // Store popup area for mouse click detection
    state.popup_area = Some(dialog_area);

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
    let content_area = Rect::new(
        inner_area.x + H_PADDING as u16,
        inner_area.y,
        inner_area.width.saturating_sub((H_PADDING * 2) as u16),
        inner_area.height,
    );
    frame.render_widget(block, dialog_area);

    // Layout for content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(message_height as u16), // Message
            Constraint::Length(details_height as u16), // Details
            Constraint::Length(1),                     // Spacer
            Constraint::Length(1),                     // Buttons
        ])
        .split(content_area);

    // Message
    let message_style = if state.is_dangerous {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let message = Paragraph::new(
        message_lines
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect::<Vec<Line>>(),
    )
    .style(message_style)
    .wrap(Wrap { trim: false });
    frame.render_widget(message, chunks[0]);

    // Details
    if !details_lines.is_empty() && details_height > 0 {
        let details_text: Vec<Line> = details_lines
            .iter()
            .take(details_height)
            .map(|line| Line::from(line.as_str()).style(Style::default().fg(Color::DarkGray)))
            .collect();
        let details = Paragraph::new(details_text).wrap(Wrap { trim: false });
        frame.render_widget(details, chunks[1]);
    }

    // Buttons
    // Cancel button
    let cancel_style = if !state.selected_confirm {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let cancel_text = format!("[ {} ]", state.cancel_label);
    let cancel_width = cancel_text.chars().count() as u16;

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
    let confirm_width = confirm_text.chars().count() as u16;

    let button_line = Line::from(vec![
        Span::styled(cancel_text, cancel_style),
        Span::raw("  "),
        Span::styled(confirm_text, confirm_style),
    ]);
    let buttons = Paragraph::new(button_line).alignment(Alignment::Center);
    frame.render_widget(buttons, chunks[3]);

    // Calculate and store button areas for mouse click detection
    // Button line: "[ Cancel ]  [ Confirm ]" centered in chunks[3]
    let gap_width: u16 = 2; // Two spaces between buttons
    let total_button_width = cancel_width + gap_width + confirm_width;
    let buttons_start_x = chunks[3].x + (chunks[3].width.saturating_sub(total_button_width)) / 2;

    state.cancel_button_area = Some(Rect::new(buttons_start_x, chunks[3].y, cancel_width, 1));
    state.confirm_button_area = Some(Rect::new(
        buttons_start_x + cancel_width + gap_width,
        chunks[3].y,
        confirm_width,
        1,
    ));
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
    fn test_force_kill_agent() {
        let state = ConfirmState::force_kill_agent("feature/test", "codex");
        assert!(state.is_dangerous);
        assert!(state.message.contains("codex"));
        assert_eq!(state.confirm_label, "Force Kill");
    }

    #[test]
    fn test_duplicate_agent_warning() {
        let state = ConfirmState::duplicate_agent_warning("main", "claude");
        assert!(!state.is_dangerous); // Not dangerous, just a warning
        assert!(state.message.contains("claude"));
        assert!(state.message.contains("main"));
    }

    #[test]
    fn test_hook_setup_with_warning() {
        let exe_path = "/home/user/.bun/install/cache/@akiojin/gwt@1.0.0/gwt";
        let state = ConfirmState::hook_setup_with_warning(exe_path);

        assert!(state.is_dangerous);
        assert!(!state.selected_confirm); // Default to Skip for safety
        assert_eq!(state.confirm_label, "Setup Anyway");
        assert_eq!(state.cancel_label, "Skip");
        assert!(state.title.contains("Warning"));
        assert!(state.details.iter().any(|d| d.contains("npm install -g")));
    }

    #[test]
    fn test_hook_setup_normal() {
        let state = ConfirmState::hook_setup();

        assert!(!state.is_dangerous);
        assert!(state.selected_confirm); // Default to Setup for better UX
        assert_eq!(state.confirm_label, "Setup");
    }

    #[test]
    fn test_truncate_path_short() {
        let short_path = "/usr/local/bin/gwt";
        assert_eq!(truncate_path(short_path, 50), short_path);
    }

    #[test]
    fn test_truncate_path_long() {
        let long_path = "/home/user/.bun/install/cache/@akiojin/gwt@1.0.0/gwt";
        let truncated = truncate_path(long_path, 30);
        assert!(truncated.starts_with("..."));
        assert!(truncated.len() <= 30);
    }

    #[test]
    fn test_plugin_setup() {
        let state = ConfirmState::plugin_setup();

        assert!(!state.is_dangerous);
        assert!(state.selected_confirm); // Default to Setup for better UX
        assert_eq!(state.confirm_label, "Setup");
        assert_eq!(state.cancel_label, "Skip");
        assert!(state.title.contains("Worktree Protection"));
        assert!(state.message.contains("worktree-protection-hooks"));
        assert!(state.details.iter().any(|d| d.contains("gwt-plugins")));
    }
}
