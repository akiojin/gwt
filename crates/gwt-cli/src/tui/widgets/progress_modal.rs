//! Progress modal widget for worktree preparation (FR-041 - FR-060)

use crate::{ProgressStep, ProgressStepKind, StepStatus};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::time::Instant;

/// State for the progress modal
#[derive(Debug, Clone)]
pub struct ProgressModalState {
    /// Whether the modal is visible
    pub visible: bool,
    /// Current progress steps
    pub steps: Vec<ProgressStep>,
    /// When the modal was first shown
    pub start_time: Instant,
    /// Whether cancellation has been requested
    pub cancellation_requested: bool,
    /// Whether all steps completed successfully
    pub completed: bool,
    /// Time when completion was detected (for 2-second summary display)
    pub completed_at: Option<Instant>,
    /// Whether waiting for key press after error
    pub waiting_for_key: bool,
}

impl Default for ProgressModalState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressModalState {
    /// Create a new progress modal state with all steps pending
    pub fn new() -> Self {
        let steps = ProgressStepKind::all()
            .into_iter()
            .map(ProgressStep::new)
            .collect();

        Self {
            visible: true,
            steps,
            start_time: Instant::now(),
            cancellation_requested: false,
            completed: false,
            completed_at: None,
            waiting_for_key: false,
        }
    }

    /// Get total elapsed time since modal was shown
    pub fn total_elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Check if all steps are completed or skipped
    pub fn all_done(&self) -> bool {
        self.steps.iter().all(|s| {
            matches!(
                s.status,
                StepStatus::Completed | StepStatus::Skipped | StepStatus::Failed
            )
        })
    }

    /// Check if any step has failed
    pub fn has_failed(&self) -> bool {
        self.steps.iter().any(|s| s.status == StepStatus::Failed)
    }

    /// Get the current running step (if any)
    pub fn current_step(&self) -> Option<&ProgressStep> {
        self.steps.iter().find(|s| s.status == StepStatus::Running)
    }

    /// Get a dynamic title based on current state
    pub fn title(&self) -> &'static str {
        if self.has_failed() {
            "Preparation Failed"
        } else if self.completed {
            "Preparation Complete"
        } else if let Some(step) = self.current_step() {
            step.kind.message()
        } else {
            "Preparing Worktree..."
        }
    }

    /// Update a step's status by kind
    pub fn update_step(&mut self, kind: ProgressStepKind, status: StepStatus) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.kind == kind) {
            match status {
                StepStatus::Running => step.start(),
                StepStatus::Completed => step.complete(),
                StepStatus::Skipped => step.skip(),
                _ => step.status = status,
            }
        }
    }

    /// Set error on a step
    pub fn set_step_error(&mut self, kind: ProgressStepKind, message: String) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.kind == kind) {
            step.fail(message);
        }
        self.waiting_for_key = true;
    }

    /// Mark as completed
    pub fn mark_completed(&mut self) {
        self.completed = true;
        self.completed_at = Some(Instant::now());
    }

    /// Check if summary display time (2 seconds) has elapsed
    pub fn summary_display_elapsed(&self) -> bool {
        self.completed_at
            .map(|t| t.elapsed().as_secs_f64() >= 2.0)
            .unwrap_or(false)
    }
}

/// Progress modal widget for rendering
pub struct ProgressModal<'a> {
    state: &'a ProgressModalState,
}

impl<'a> ProgressModal<'a> {
    pub fn new(state: &'a ProgressModalState) -> Self {
        Self { state }
    }

    /// Calculate modal dimensions (FR-045: width >= 80 chars)
    fn modal_area(&self, area: Rect) -> Rect {
        let modal_width = 80.min(area.width.saturating_sub(4));
        // Title(1) + border(2) + 6 steps + error line(1) + empty(1) + summary(1) = 12, +2 for padding
        let modal_height = 14.min(area.height.saturating_sub(4));

        let x = (area.width.saturating_sub(modal_width)) / 2;
        let y = (area.height.saturating_sub(modal_height)) / 2;

        Rect::new(area.x + x, area.y + y, modal_width, modal_height)
    }

    /// Get color for step status (FR-050)
    fn status_color(status: StepStatus) -> Color {
        match status {
            StepStatus::Completed => Color::Green,
            StepStatus::Running => Color::Yellow,
            StepStatus::Pending => Color::DarkGray,
            StepStatus::Failed => Color::Red,
            StepStatus::Skipped => Color::DarkGray,
        }
    }

    /// Build lines for a single step (FR-052a: error on separate indented line)
    fn step_lines(step: &ProgressStep, inner_width: u16) -> Vec<Line<'static>> {
        let color = Self::status_color(step.status);
        let marker = step.marker();
        let message = step.kind.message();

        let mut spans = vec![
            Span::styled(format!("  {} ", marker), Style::default().fg(color)),
            Span::styled(message.to_string(), Style::default().fg(color)),
        ];

        // Show elapsed time if >= 3 seconds (FR-049)
        if step.status == StepStatus::Running && step.should_show_elapsed() {
            if let Some(secs) = step.elapsed_secs() {
                spans.push(Span::styled(
                    format!(" {:.1}s", secs),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::DIM),
                ));
            }
        }

        let mut lines = vec![Line::from(spans)];

        // Show error message on separate indented line if failed (FR-052a)
        if step.status == StepStatus::Failed {
            if let Some(ref msg) = step.error_message {
                let indent = "      ";
                let max_msg_len = (inner_width as usize).saturating_sub(indent.len());
                let truncated = if msg.len() > max_msg_len {
                    let cut = max_msg_len.saturating_sub(3);
                    format!("{}{}...", indent, &msg[..cut])
                } else {
                    format!("{}{}", indent, msg)
                };
                lines.push(Line::from(Span::styled(
                    truncated,
                    Style::default().fg(Color::Red),
                )));
            }
        }

        lines
    }
}

impl Widget for ProgressModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible {
            return;
        }

        // FR-044: Semi-transparent overlay (dark background)
        // Fill entire area with dark background
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_bg(Color::Black);
                    cell.set_fg(Color::DarkGray);
                }
            }
        }

        let modal_area = self.modal_area(area);

        // Clear the modal area
        Clear.render(modal_area, buf);

        // Modal block with dynamic title (FR-046)
        let title = self.state.title();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.state.has_failed() {
                Color::Red
            } else if self.state.completed {
                Color::Green
            } else {
                Color::Cyan
            }));

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        // Build content lines
        let mut lines: Vec<Line> = Vec::new();

        // Step lines (FR-052a: error messages on separate lines)
        for step in &self.state.steps {
            lines.extend(Self::step_lines(step, inner.width));
        }

        // Add empty line before summary/error
        lines.push(Line::from(""));

        // Summary or error message (FR-051, FR-052)
        if self.state.has_failed() {
            lines.push(Line::from(vec![Span::styled(
                "  Press any key to continue...",
                Style::default().fg(Color::Yellow),
            )]));
        } else if self.state.completed {
            let total = self.state.total_elapsed_secs();
            let completed_count = self
                .state
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count();
            lines.push(Line::from(vec![Span::styled(
                format!("  Completed in {:.1}s ({} steps)", total, completed_count),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )]));
        }

        // Render content
        let content = Paragraph::new(lines);
        content.render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_modal_state_new() {
        let state = ProgressModalState::new();
        assert!(state.visible);
        assert_eq!(state.steps.len(), 6);
        assert!(!state.completed);
        assert!(!state.cancellation_requested);
    }

    #[test]
    fn test_progress_modal_state_all_done_false_initially() {
        let state = ProgressModalState::new();
        assert!(!state.all_done());
    }

    #[test]
    fn test_progress_modal_state_all_done_true_when_completed() {
        let mut state = ProgressModalState::new();
        for step in &mut state.steps {
            step.complete();
        }
        assert!(state.all_done());
    }

    #[test]
    fn test_progress_modal_state_has_failed() {
        let mut state = ProgressModalState::new();
        assert!(!state.has_failed());

        state.set_step_error(ProgressStepKind::FetchRemote, "Network error".to_string());
        assert!(state.has_failed());
    }

    #[test]
    fn test_progress_modal_state_update_step() {
        let mut state = ProgressModalState::new();
        state.update_step(ProgressStepKind::FetchRemote, StepStatus::Running);

        let step = state
            .steps
            .iter()
            .find(|s| s.kind == ProgressStepKind::FetchRemote)
            .unwrap();
        assert_eq!(step.status, StepStatus::Running);
        assert!(step.started_at.is_some());
    }

    #[test]
    fn test_progress_modal_state_title_dynamic() {
        let mut state = ProgressModalState::new();
        assert_eq!(state.title(), "Preparing Worktree...");

        state.update_step(ProgressStepKind::FetchRemote, StepStatus::Running);
        assert_eq!(state.title(), "Fetching remote...");

        state.mark_completed();
        assert_eq!(state.title(), "Preparation Complete");

        let mut state2 = ProgressModalState::new();
        state2.set_step_error(ProgressStepKind::FetchRemote, "error".to_string());
        assert_eq!(state2.title(), "Preparation Failed");
    }

    #[test]
    fn test_step_lines_no_error_returns_single_line() {
        let step = ProgressStep::new(ProgressStepKind::CreateWorktree);
        let lines = ProgressModal::step_lines(&step, 78);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_step_lines_short_error_on_separate_line() {
        let mut step = ProgressStep::new(ProgressStepKind::CreateWorktree);
        step.fail("short error".to_string());
        let lines = ProgressModal::step_lines(&step, 78);
        // FR-052a: error on separate indented line
        assert_eq!(lines.len(), 2);
        let error_line = lines[1].to_string();
        assert!(error_line.contains("short error"));
        // Check indentation (6 spaces)
        assert!(error_line.starts_with("      "));
    }

    #[test]
    fn test_step_lines_long_error_truncated_with_ellipsis() {
        let mut step = ProgressStep::new(ProgressStepKind::CreateWorktree);
        let long_msg =
            "[E1013] Git operation failed: worktree add: fatal: 'feature/long-branch-name' is already checked out at '/very/long/path/to/worktrees/feature/long-branch-name'";
        step.fail(long_msg.to_string());
        let lines = ProgressModal::step_lines(&step, 78);
        assert_eq!(lines.len(), 2);
        let error_line = lines[1].to_string();
        // Should be truncated with "..."
        assert!(error_line.ends_with("..."));
        // Should not exceed inner_width
        assert!(error_line.len() <= 78);
    }

    #[test]
    fn test_step_lines_error_fits_exactly_no_truncation() {
        let mut step = ProgressStep::new(ProgressStepKind::CreateWorktree);
        // 6 spaces indent, so 72 chars available for the message in a 78-wide area
        let msg = "x".repeat(72);
        step.fail(msg.clone());
        let lines = ProgressModal::step_lines(&step, 78);
        assert_eq!(lines.len(), 2);
        let error_line = lines[1].to_string();
        // Should NOT be truncated (exactly fits)
        assert!(!error_line.ends_with("..."));
        assert!(error_line.contains(&msg));
    }

    #[test]
    fn test_status_color() {
        assert_eq!(
            ProgressModal::status_color(StepStatus::Completed),
            Color::Green
        );
        assert_eq!(
            ProgressModal::status_color(StepStatus::Running),
            Color::Yellow
        );
        assert_eq!(
            ProgressModal::status_color(StepStatus::Pending),
            Color::DarkGray
        );
        assert_eq!(ProgressModal::status_color(StepStatus::Failed), Color::Red);
        assert_eq!(
            ProgressModal::status_color(StepStatus::Skipped),
            Color::DarkGray
        );
    }
}
