//! Migration Dialog Screen (SPEC-a70a1ece T705-T710)
//!
//! Displays forced migration dialog when .worktrees/ method is detected.

#![allow(dead_code)] // Migration dialog components for full migration feature

use gwt_core::migration::{MigrationConfig, MigrationState, ValidationResult};
use ratatui::{prelude::*, widgets::*};

/// Migration dialog state
#[derive(Debug)]
pub struct MigrationDialogState {
    /// Current phase of the dialog
    pub phase: MigrationDialogPhase,
    /// Migration configuration
    pub config: Option<MigrationConfig>,
    /// Validation result (after validation)
    pub validation: Option<ValidationResult>,
    /// Current migration state (during execution)
    pub migration_state: MigrationState,
    /// Error message if migration failed
    pub error: Option<String>,
    /// Whether user accepted the migration
    pub accepted: bool,
    /// Currently selected button (true = proceed, false = exit)
    pub selected_proceed: bool,
    // Mouse click support
    /// Cached popup area
    pub popup_area: Option<Rect>,
    /// Cached exit button area
    pub exit_button_area: Option<Rect>,
    /// Cached proceed button area
    pub proceed_button_area: Option<Rect>,
}

/// Phases of the migration dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDialogPhase {
    /// Initial confirmation - user must choose to proceed or exit
    Confirmation,
    /// Validating prerequisites
    Validating,
    /// Migration in progress
    InProgress,
    /// Migration completed successfully
    Completed,
    /// Migration failed with error
    Failed,
    /// User chose to exit without migration
    Exited,
}

impl Default for MigrationDialogState {
    fn default() -> Self {
        Self {
            phase: MigrationDialogPhase::Confirmation,
            config: None,
            validation: None,
            migration_state: MigrationState::Pending,
            error: None,
            accepted: false,
            selected_proceed: false, // Default to Exit for safety
            popup_area: None,
            exit_button_area: None,
            proceed_button_area: None,
        }
    }
}

impl MigrationDialogState {
    /// Create a new migration dialog
    pub fn new(config: MigrationConfig) -> Self {
        Self {
            config: Some(config),
            ..Default::default()
        }
    }

    /// Toggle button selection
    pub fn toggle_selection(&mut self) {
        if self.phase == MigrationDialogPhase::Confirmation {
            self.selected_proceed = !self.selected_proceed;
        }
    }

    /// Select proceed
    pub fn select_proceed(&mut self) {
        self.selected_proceed = true;
    }

    /// Select exit
    pub fn select_exit(&mut self) {
        self.selected_proceed = false;
    }

    /// Accept the migration (user chose to proceed)
    pub fn accept(&mut self) {
        self.accepted = true;
        self.phase = MigrationDialogPhase::Validating;
    }

    /// Reject the migration (user chose to exit)
    pub fn reject(&mut self) {
        self.phase = MigrationDialogPhase::Exited;
    }

    /// Update migration state
    pub fn update_state(&mut self, state: MigrationState) {
        self.migration_state = state;
        match state {
            MigrationState::Validating => {
                self.phase = MigrationDialogPhase::Validating;
            }
            MigrationState::BackingUp
            | MigrationState::CreatingBareRepo
            | MigrationState::MigratingWorktrees { .. }
            | MigrationState::CleaningUp
            | MigrationState::RollingBack => {
                self.phase = MigrationDialogPhase::InProgress;
            }
            MigrationState::Completed => {
                self.phase = MigrationDialogPhase::Completed;
            }
            MigrationState::Failed | MigrationState::Cancelled => {
                self.phase = MigrationDialogPhase::Failed;
            }
            MigrationState::Pending => {}
        }
    }

    /// Set validation result
    pub fn set_validation(&mut self, result: ValidationResult) {
        if !result.passed {
            self.phase = MigrationDialogPhase::Failed;
            self.error = Some(
                result
                    .errors
                    .first()
                    .map(|e: &gwt_core::migration::MigrationError| e.to_string())
                    .unwrap_or_else(|| "Validation failed".to_string()),
            );
        } else {
            self.phase = MigrationDialogPhase::InProgress;
        }
        self.validation = Some(result);
    }

    /// Set error
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.phase = MigrationDialogPhase::Failed;
    }

    /// Check if dialog requires user action
    pub fn requires_action(&self) -> bool {
        matches!(
            self.phase,
            MigrationDialogPhase::Confirmation
                | MigrationDialogPhase::Completed
                | MigrationDialogPhase::Failed
        )
    }

    /// Check if migration is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self.phase,
            MigrationDialogPhase::Validating | MigrationDialogPhase::InProgress
        )
    }

    /// Check if user chose to exit
    pub fn is_exited(&self) -> bool {
        self.phase == MigrationDialogPhase::Exited
    }

    /// Check if migration completed
    pub fn is_completed(&self) -> bool {
        self.phase == MigrationDialogPhase::Completed
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

    /// Check if point is on exit button
    pub fn is_exit_button_at(&self, x: u16, y: u16) -> bool {
        self.exit_button_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    }

    /// Check if point is on proceed button
    pub fn is_proceed_button_at(&self, x: u16, y: u16) -> bool {
        self.proceed_button_area.is_some_and(|area| {
            x >= area.x
                && x < area.x.saturating_add(area.width)
                && y >= area.y
                && y < area.y.saturating_add(area.height)
        })
    }
}

/// Render migration dialog
pub fn render_migration_dialog(state: &mut MigrationDialogState, frame: &mut Frame, area: Rect) {
    match state.phase {
        MigrationDialogPhase::Confirmation => render_confirmation(state, frame, area),
        MigrationDialogPhase::Validating | MigrationDialogPhase::InProgress => {
            render_progress(state, frame, area)
        }
        MigrationDialogPhase::Completed => render_completed(state, frame, area),
        MigrationDialogPhase::Failed => render_failed(state, frame, area),
        MigrationDialogPhase::Exited => {} // Nothing to render, dialog closed
    }
}

/// Render confirmation phase
fn render_confirmation(state: &mut MigrationDialogState, frame: &mut Frame, area: Rect) {
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let dialog_height = 14.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    state.popup_area = Some(dialog_area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Migration Required ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

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

    // Message
    let message = Paragraph::new(vec![
        Line::from("gwt has detected an outdated worktree structure."),
        Line::from("Migration to bare repository method is required."),
    ])
    .style(Style::default().fg(Color::White));
    frame.render_widget(message, chunks[0]);

    // Details
    let details = Paragraph::new(vec![
        Line::from(Span::styled(
            "What will happen:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from("  - Backup current state"),
        Line::from("  - Convert to bare repository + worktrees"),
    ])
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(details, chunks[1]);

    // Warning
    let warning = Paragraph::new(vec![Line::from(Span::styled(
        "! Choosing [Exit] will close gwt without migration.",
        Style::default().fg(Color::Yellow),
    ))]);
    frame.render_widget(warning, chunks[2]);

    // Buttons
    render_buttons(state, frame, chunks[4], "Exit", "Migrate");
}

/// Render progress phase
fn render_progress(state: &mut MigrationDialogState, frame: &mut Frame, area: Rect) {
    let dialog_width = 50.min(area.width.saturating_sub(4));
    let dialog_height = 10.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    state.popup_area = Some(dialog_area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Migration in Progress ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Status
            Constraint::Length(2), // Progress indicator
            Constraint::Min(1),    // Spacer
        ])
        .split(inner_area);

    // Status message
    let status_text = state.migration_state.description();
    let status = Paragraph::new(vec![
        Line::from(Span::styled(status_text, Style::default().fg(Color::White))),
        Line::from(""),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(status, chunks[0]);

    // Simple progress indicator (spinner-like)
    let spinner_chars = ["|", "/", "-", "\\"];
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 200) as usize % 4)
        .unwrap_or(0);
    let progress = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(spinner_chars[tick], Style::default().fg(Color::Cyan)),
        Span::raw(" Working... Please wait."),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(progress, chunks[1]);
}

/// Render completed phase
fn render_completed(state: &mut MigrationDialogState, frame: &mut Frame, area: Rect) {
    let dialog_width = 50.min(area.width.saturating_sub(4));
    let dialog_height = 8.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    state.popup_area = Some(dialog_area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(" Migration Complete ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Message
            Constraint::Min(1),    // Spacer
            Constraint::Length(1), // Button
        ])
        .split(inner_area);

    // Success message
    let message = Paragraph::new(vec![
        Line::from(Span::styled(
            "Migration completed successfully!",
            Style::default().fg(Color::Green),
        )),
        Line::from("Press Enter to continue."),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(message, chunks[0]);

    // Continue button
    let button = Paragraph::new(Line::from(Span::styled(
        "[ Continue ]",
        Style::default().bg(Color::Green).fg(Color::Black),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(button, chunks[2]);
}

/// Render failed phase
fn render_failed(state: &mut MigrationDialogState, frame: &mut Frame, area: Rect) {
    let dialog_width = 60.min(area.width.saturating_sub(4));
    let dialog_height = 12.min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    state.popup_area = Some(dialog_area);
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Migration Failed ")
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner_area = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

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

    // Error title
    let title = Paragraph::new(Line::from(Span::styled(
        "An error occurred during migration:",
        Style::default().fg(Color::Red),
    )));
    frame.render_widget(title, chunks[0]);

    // Error message
    let error_msg = state.error.as_deref().unwrap_or("Unknown error occurred");
    let error = Paragraph::new(error_msg)
        .style(Style::default().fg(Color::Yellow))
        .wrap(Wrap { trim: true });
    frame.render_widget(error, chunks[1]);

    // Exit button
    let button = Paragraph::new(Line::from(Span::styled(
        "[ Exit ]",
        Style::default().bg(Color::Red).fg(Color::White),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(button, chunks[3]);
}

/// Render buttons for confirmation phase
fn render_buttons(
    state: &mut MigrationDialogState,
    frame: &mut Frame,
    area: Rect,
    exit_label: &str,
    proceed_label: &str,
) {
    let exit_text = format!("[ {} ]", exit_label);
    let proceed_text = format!("[ {} ]", proceed_label);
    let exit_width = exit_text.chars().count() as u16;
    let proceed_width = proceed_text.chars().count() as u16;

    // Exit button style
    let exit_style = if !state.selected_proceed {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Proceed button style
    let proceed_style = if state.selected_proceed {
        Style::default().bg(Color::Green).fg(Color::Black)
    } else {
        Style::default().fg(Color::Green)
    };

    let button_line = Line::from(vec![
        Span::styled(exit_text, exit_style),
        Span::raw("  "),
        Span::styled(proceed_text, proceed_style),
    ]);
    let buttons = Paragraph::new(button_line).alignment(Alignment::Center);
    frame.render_widget(buttons, area);

    // Calculate button areas for mouse click detection
    let gap_width: u16 = 2;
    let total_button_width = exit_width + gap_width + proceed_width;
    let buttons_start_x = area.x + (area.width.saturating_sub(total_button_width)) / 2;

    state.exit_button_area = Some(Rect::new(buttons_start_x, area.y, exit_width, 1));
    state.proceed_button_area = Some(Rect::new(
        buttons_start_x + exit_width + gap_width,
        area.y,
        proceed_width,
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
    fn test_migration_dialog_default() {
        let state = MigrationDialogState::default();
        assert_eq!(state.phase, MigrationDialogPhase::Confirmation);
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
        assert_eq!(state.phase, MigrationDialogPhase::Validating);
    }

    #[test]
    fn test_reject_changes_phase() {
        let mut state = MigrationDialogState::default();
        state.reject();
        assert_eq!(state.phase, MigrationDialogPhase::Exited);
    }

    #[test]
    fn test_update_state_completed() {
        let mut state = MigrationDialogState::default();
        state.update_state(MigrationState::Completed);
        assert_eq!(state.phase, MigrationDialogPhase::Completed);
        assert!(state.is_completed());
    }

    #[test]
    fn test_update_state_failed() {
        let mut state = MigrationDialogState::default();
        state.update_state(MigrationState::Failed);
        assert_eq!(state.phase, MigrationDialogPhase::Failed);
    }

    #[test]
    fn test_set_error() {
        let mut state = MigrationDialogState::default();
        state.set_error("Test error".to_string());
        assert_eq!(state.phase, MigrationDialogPhase::Failed);
        assert_eq!(state.error, Some("Test error".to_string()));
    }

    #[test]
    fn test_requires_action() {
        let mut state = MigrationDialogState::default();
        assert!(state.requires_action()); // Confirmation requires action

        state.phase = MigrationDialogPhase::InProgress;
        assert!(!state.requires_action()); // In progress doesn't

        state.phase = MigrationDialogPhase::Completed;
        assert!(state.requires_action()); // Completed requires action
    }

    #[test]
    fn test_is_in_progress() {
        let mut state = MigrationDialogState::default();
        assert!(!state.is_in_progress());

        state.phase = MigrationDialogPhase::Validating;
        assert!(state.is_in_progress());

        state.phase = MigrationDialogPhase::InProgress;
        assert!(state.is_in_progress());
    }
}
