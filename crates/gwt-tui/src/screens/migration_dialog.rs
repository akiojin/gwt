//! Migration Dialog Screen
//!
//! Displays forced migration dialog when .worktrees/ method is detected.
//! Guides user through bare repository migration with confirmation,
//! progress tracking, and error handling.

#![allow(dead_code)]

use gwt_core::migration::MigrationState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Phases of the migration dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPhase {
    /// Initial confirmation - user must choose to proceed or exit
    Confirm,
    /// Migration in progress (validating + executing)
    InProgress,
    /// Migration completed successfully
    Done,
    /// Migration failed with error
    Error,
    /// User chose to exit without migration
    Exited,
}

/// Migration dialog state
#[derive(Debug)]
pub struct MigrationDialogState {
    /// Current phase of the dialog
    pub phase: MigrationPhase,
    /// Source path being migrated
    pub source_path: String,
    /// Target path for bare repo
    pub target_path: String,
    /// Progress message
    pub progress: Option<String>,
    /// Core migration state (for detailed status)
    pub migration_state: MigrationState,
    /// Error message if migration failed
    pub error: Option<String>,
    /// Whether user accepted the migration
    pub accepted: bool,
    /// Currently selected button (true = proceed, false = exit)
    pub selected_proceed: bool,
}

impl Default for MigrationDialogState {
    fn default() -> Self {
        Self {
            phase: MigrationPhase::Confirm,
            source_path: String::new(),
            target_path: String::new(),
            progress: None,
            migration_state: MigrationState::Pending,
            error: None,
            accepted: false,
            selected_proceed: false, // Default to Exit for safety
        }
    }
}

impl MigrationDialogState {
    /// Create a new migration dialog with paths
    pub fn new(source_path: &str, target_path: &str) -> Self {
        Self {
            source_path: source_path.to_string(),
            target_path: target_path.to_string(),
            ..Default::default()
        }
    }

    /// Toggle button selection
    pub fn toggle_selection(&mut self) {
        if self.phase == MigrationPhase::Confirm {
            self.selected_proceed = !self.selected_proceed;
        }
    }

    /// Accept the migration (user chose to proceed)
    pub fn accept(&mut self) {
        self.accepted = true;
        self.phase = MigrationPhase::InProgress;
        self.progress = Some("Validating prerequisites...".to_string());
    }

    /// Reject the migration (user chose to exit)
    pub fn reject(&mut self) {
        self.phase = MigrationPhase::Exited;
    }

    /// Update migration state
    pub fn update_state(&mut self, state: MigrationState) {
        self.progress = Some(state.description());
        self.migration_state = state;
        match state {
            MigrationState::Completed => {
                self.phase = MigrationPhase::Done;
            }
            MigrationState::Failed | MigrationState::Cancelled => {
                self.phase = MigrationPhase::Error;
            }
            _ => {
                self.phase = MigrationPhase::InProgress;
            }
        }
    }

    /// Set error
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.phase = MigrationPhase::Error;
    }

    /// Check if dialog requires user action
    pub fn requires_action(&self) -> bool {
        matches!(
            self.phase,
            MigrationPhase::Confirm | MigrationPhase::Done | MigrationPhase::Error
        )
    }

    /// Check if migration is in progress
    pub fn is_in_progress(&self) -> bool {
        self.phase == MigrationPhase::InProgress
    }

    /// Check if user chose to exit
    pub fn is_exited(&self) -> bool {
        self.phase == MigrationPhase::Exited
    }

    /// Check if migration completed
    pub fn is_completed(&self) -> bool {
        self.phase == MigrationPhase::Done
    }
}

/// Render migration dialog
pub fn render_migration_dialog(state: &MigrationDialogState, buf: &mut Buffer, area: Rect) {
    match state.phase {
        MigrationPhase::Confirm => render_confirmation(state, buf, area),
        MigrationPhase::InProgress => render_progress(state, buf, area),
        MigrationPhase::Done => render_completed(state, buf, area),
        MigrationPhase::Error => render_failed(state, buf, area),
        MigrationPhase::Exited => {} // Nothing to render
    }
}

fn render_confirmation(state: &MigrationDialogState, buf: &mut Buffer, area: Rect) {
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let dialog_height = 14.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Migration Required ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    block.render(dialog_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Message
            Constraint::Length(3), // Details
            Constraint::Length(2), // Warning
            Constraint::Min(1),    // Spacer
            Constraint::Length(1), // Buttons
        ])
        .split(inner_area);

    let message = Paragraph::new(vec![
        Line::from("gwt has detected an outdated worktree structure."),
        Line::from("Migration to bare repository method is required."),
    ])
    .style(Style::default().fg(Color::White));
    message.render(chunks[0], buf);

    let details = Paragraph::new(vec![
        Line::from(Span::styled(
            "What will happen:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from("  - Backup current state"),
        Line::from("  - Convert to bare repository + worktrees"),
    ])
    .style(Style::default().fg(Color::DarkGray));
    details.render(chunks[1], buf);

    let warning = Paragraph::new(vec![Line::from(Span::styled(
        "! Choosing [Exit] will close gwt without migration.",
        Style::default().fg(Color::Yellow),
    ))]);
    warning.render(chunks[2], buf);

    // Buttons
    let exit_style = if !state.selected_proceed {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let proceed_style = if state.selected_proceed {
        Style::default().bg(Color::Green).fg(Color::Black)
    } else {
        Style::default().fg(Color::Green)
    };

    let button_line = Line::from(vec![
        Span::styled("[ Exit ]", exit_style),
        Span::raw("  "),
        Span::styled("[ Migrate ]", proceed_style),
    ]);
    let buttons = Paragraph::new(button_line).alignment(Alignment::Center);
    buttons.render(chunks[4], buf);
}

fn render_progress(state: &MigrationDialogState, buf: &mut Buffer, area: Rect) {
    let dialog_width = 50.min(area.width.saturating_sub(4));
    let dialog_height = 10.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Migration in Progress ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    block.render(dialog_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Status
            Constraint::Length(2), // Progress indicator
            Constraint::Min(1),    // Spacer
        ])
        .split(inner_area);

    let status_text = state.progress.as_deref().unwrap_or("Working...");
    let status = Paragraph::new(vec![
        Line::from(Span::styled(status_text, Style::default().fg(Color::White))),
        Line::from(""),
    ])
    .alignment(Alignment::Center);
    status.render(chunks[0], buf);

    let progress = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled("*", Style::default().fg(Color::Cyan)),
        Span::raw(" Working... Please wait."),
    ]))
    .alignment(Alignment::Center);
    progress.render(chunks[1], buf);
}

fn render_completed(_state: &MigrationDialogState, buf: &mut Buffer, area: Rect) {
    let dialog_width = 50.min(area.width.saturating_sub(4));
    let dialog_height = 8.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(" Migration Complete ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    block.render(dialog_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Message
            Constraint::Min(1),    // Spacer
            Constraint::Length(1), // Button
        ])
        .split(inner_area);

    let message = Paragraph::new(vec![
        Line::from(Span::styled(
            "Migration completed successfully!",
            Style::default().fg(Color::Green),
        )),
        Line::from("Press Enter to continue."),
    ])
    .alignment(Alignment::Center);
    message.render(chunks[0], buf);

    let button = Paragraph::new(Line::from(Span::styled(
        "[ Continue ]",
        Style::default().bg(Color::Green).fg(Color::Black),
    )))
    .alignment(Alignment::Center);
    button.render(chunks[2], buf);
}

fn render_failed(state: &MigrationDialogState, buf: &mut Buffer, area: Rect) {
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let dialog_height = 12.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Migration Failed ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    block.render(dialog_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Error title
            Constraint::Length(3), // Error message
            Constraint::Min(1),    // Spacer
            Constraint::Length(1), // Button
        ])
        .split(inner_area);

    let title = Paragraph::new(Line::from(Span::styled(
        "An error occurred during migration:",
        Style::default().fg(Color::Red),
    )));
    title.render(chunks[0], buf);

    let error_msg = state.error.as_deref().unwrap_or("Unknown error occurred");
    let error = Paragraph::new(error_msg)
        .style(Style::default().fg(Color::Yellow))
        .wrap(ratatui::widgets::Wrap { trim: true });
    error.render(chunks[1], buf);

    let button = Paragraph::new(Line::from(Span::styled(
        "[ Exit ]",
        Style::default().bg(Color::Red).fg(Color::White),
    )))
    .alignment(Alignment::Center);
    button.render(chunks[3], buf);
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
    fn test_migration_dialog_default() {
        let state = MigrationDialogState::default();
        assert_eq!(state.phase, MigrationPhase::Confirm);
        assert!(!state.accepted);
        assert!(!state.selected_proceed);
    }

    #[test]
    fn test_toggle_selection() {
        let mut state = MigrationDialogState::default();
        assert!(!state.selected_proceed);
        state.toggle_selection();
        assert!(state.selected_proceed);
        state.toggle_selection();
        assert!(!state.selected_proceed);
    }

    #[test]
    fn test_accept_changes_phase() {
        let mut state = MigrationDialogState::default();
        state.accept();
        assert!(state.accepted);
        assert_eq!(state.phase, MigrationPhase::InProgress);
    }

    #[test]
    fn test_reject_changes_phase() {
        let mut state = MigrationDialogState::default();
        state.reject();
        assert_eq!(state.phase, MigrationPhase::Exited);
        assert!(state.is_exited());
    }

    #[test]
    fn test_update_state_completed() {
        let mut state = MigrationDialogState::default();
        state.update_state(MigrationState::Completed);
        assert_eq!(state.phase, MigrationPhase::Done);
        assert!(state.is_completed());
    }

    #[test]
    fn test_update_state_failed() {
        let mut state = MigrationDialogState::default();
        state.update_state(MigrationState::Failed);
        assert_eq!(state.phase, MigrationPhase::Error);
    }

    #[test]
    fn test_set_error() {
        let mut state = MigrationDialogState::default();
        state.set_error("Test error".to_string());
        assert_eq!(state.phase, MigrationPhase::Error);
        assert_eq!(state.error, Some("Test error".to_string()));
    }

    #[test]
    fn test_requires_action() {
        let mut state = MigrationDialogState::default();
        assert!(state.requires_action()); // Confirm requires action

        state.phase = MigrationPhase::InProgress;
        assert!(!state.requires_action()); // In progress doesn't

        state.phase = MigrationPhase::Done;
        assert!(state.requires_action()); // Done requires action
    }

    #[test]
    fn test_is_in_progress() {
        let mut state = MigrationDialogState::default();
        assert!(!state.is_in_progress());
        state.phase = MigrationPhase::InProgress;
        assert!(state.is_in_progress());
    }

    #[test]
    fn test_new_with_paths() {
        let state = MigrationDialogState::new("/src/repo", "/target/repo.git");
        assert_eq!(state.source_path, "/src/repo");
        assert_eq!(state.target_path, "/target/repo.git");
    }

    #[test]
    fn test_render_migration_dialog_no_panic() {
        let state = MigrationDialogState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_migration_dialog(&state, &mut buf, area);
    }
}
