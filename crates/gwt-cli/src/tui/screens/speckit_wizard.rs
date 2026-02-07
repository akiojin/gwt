//! Spec Kit Wizard Screen
//!
//! FR-019: Spec Kit wizard with step-based flow for generating
//! specification, plan, and task artifacts from a feature description.

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Spec Kit wizard step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecKitWizardStep {
    /// Step 1: Clarify requirements
    #[default]
    Clarify,
    /// Step 2: Generate specification
    Specify,
    /// Step 3: Generate plan
    Plan,
    /// Step 4: Generate tasks
    Tasks,
    /// Step 5: Done
    Done,
}

impl SpecKitWizardStep {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Clarify => "Clarify",
            Self::Specify => "Specify",
            Self::Plan => "Plan",
            Self::Tasks => "Tasks",
            Self::Done => "Done",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Self::Clarify => 0,
            Self::Specify => 1,
            Self::Plan => 2,
            Self::Tasks => 3,
            Self::Done => 4,
        }
    }

    pub fn total() -> usize {
        5
    }
}

/// Spec Kit wizard state
#[derive(Debug, Default)]
pub struct SpecKitWizardState {
    /// Whether wizard is visible
    pub visible: bool,
    /// Current step
    pub step: SpecKitWizardStep,
    /// Feature description input
    pub input: String,
    /// Cursor position for input
    pub input_cursor: usize,
    /// Generated artifact paths
    pub artifacts: Vec<String>,
    /// Progress message
    pub progress_message: Option<String>,
    /// Error message
    pub error: Option<String>,
    /// Whether currently processing
    pub is_processing: bool,
}

impl SpecKitWizardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the wizard
    pub fn open(&mut self) {
        self.reset();
        self.visible = true;
    }

    /// Close the wizard
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.step = SpecKitWizardStep::Clarify;
        self.input.clear();
        self.input_cursor = 0;
        self.artifacts.clear();
        self.progress_message = None;
        self.error = None;
        self.is_processing = false;
    }

    /// Advance to next step
    pub fn next_step(&mut self) {
        self.step = match self.step {
            SpecKitWizardStep::Clarify => SpecKitWizardStep::Specify,
            SpecKitWizardStep::Specify => SpecKitWizardStep::Plan,
            SpecKitWizardStep::Plan => SpecKitWizardStep::Tasks,
            SpecKitWizardStep::Tasks => SpecKitWizardStep::Done,
            SpecKitWizardStep::Done => SpecKitWizardStep::Done,
        };
        self.error = None;
    }

    /// Set processing state with message
    pub fn set_processing(&mut self, message: &str) {
        self.is_processing = true;
        self.progress_message = Some(message.to_string());
        self.error = None;
    }

    /// Clear processing state
    pub fn clear_processing(&mut self) {
        self.is_processing = false;
        self.progress_message = None;
    }

    /// Set error message
    pub fn set_error(&mut self, error: &str) {
        self.error = Some(error.to_string());
        self.is_processing = false;
        self.progress_message = None;
    }
}

/// Render the Spec Kit wizard overlay
pub fn render_speckit_wizard(frame: &mut Frame, area: Rect, state: &SpecKitWizardState) {
    if !state.visible {
        return;
    }

    // Calculate popup area (70% x 60% centered)
    let popup_width = (area.width as f32 * 0.7) as u16;
    let popup_height = (area.height as f32 * 0.6) as u16;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Outer block
    let step_label = format!(
        " Spec Kit Wizard [{}/{}] {} ",
        state.step.index() + 1,
        SpecKitWizardStep::total(),
        state.step.label()
    );
    let block = Block::default()
        .title(step_label)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Split inner into content + help bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    // Render content based on step
    match state.step {
        SpecKitWizardStep::Clarify => {
            render_clarify_step(frame, chunks[0], state);
        }
        SpecKitWizardStep::Specify | SpecKitWizardStep::Plan | SpecKitWizardStep::Tasks => {
            render_processing_step(frame, chunks[0], state);
        }
        SpecKitWizardStep::Done => {
            render_done_step(frame, chunks[0], state);
        }
    }

    // Help bar
    let help_text = if state.is_processing {
        "[Esc] Cancel"
    } else if state.step == SpecKitWizardStep::Done {
        "[Enter] Close | [Esc] Close"
    } else {
        "[Enter] Next | [Esc] Cancel"
    };
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help, chunks[1]);
}

/// Render the clarify (input) step
fn render_clarify_step(frame: &mut Frame, area: Rect, state: &SpecKitWizardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    // Label
    let label = Paragraph::new("Describe the feature you want to implement:")
        .style(Style::default().fg(Color::White));
    frame.render_widget(label, chunks[0]);

    // Input field
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input = Paragraph::new(state.input.as_str())
        .block(input_block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, chunks[1]);

    // Cursor
    frame.set_cursor_position(Position::new(
        chunks[1].x + 1 + state.input_cursor as u16,
        chunks[1].y + 1,
    ));

    // Error display
    if let Some(ref error) = state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[2]);
    }
}

/// Render a processing step (Specify/Plan/Tasks)
fn render_processing_step(frame: &mut Frame, area: Rect, state: &SpecKitWizardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Progress display
    let progress_msg = state.progress_message.as_deref().unwrap_or("Processing...");
    let progress = Paragraph::new(progress_msg)
        .style(Style::default().fg(Color::Yellow))
        .alignment(Alignment::Center);
    frame.render_widget(progress, chunks[0]);

    // Step progress bar
    let completed = state.step.index();
    let total = SpecKitWizardStep::total();
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(completed as f64 / total as f64)
        .label(format!("{}/{}", completed, total));
    frame.render_widget(gauge, chunks[1]);

    // Error display
    if let Some(ref error) = state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[1]);
    }
}

/// Render the done step
fn render_done_step(frame: &mut Frame, area: Rect, state: &SpecKitWizardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new("Spec Kit artifacts generated successfully!")
        .style(Style::default().fg(Color::Green))
        .alignment(Alignment::Center);
    frame.render_widget(header, chunks[0]);

    // Artifact list
    if !state.artifacts.is_empty() {
        let items: Vec<ListItem> = state
            .artifacts
            .iter()
            .map(|path| ListItem::new(format!("  * {}", path)))
            .collect();
        let list = List::new(items)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .title(" Generated Artifacts ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(list, chunks[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_state_new() {
        let state = SpecKitWizardState::new();
        assert!(!state.visible);
        assert_eq!(state.step, SpecKitWizardStep::Clarify);
        assert!(state.input.is_empty());
        assert!(state.artifacts.is_empty());
    }

    #[test]
    fn test_wizard_open_close() {
        let mut state = SpecKitWizardState::new();
        state.open();
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn test_wizard_reset() {
        let mut state = SpecKitWizardState::new();
        state.input = "test".to_string();
        state.step = SpecKitWizardStep::Plan;
        state.artifacts.push("file.md".to_string());
        state.reset();
        assert!(state.input.is_empty());
        assert_eq!(state.step, SpecKitWizardStep::Clarify);
        assert!(state.artifacts.is_empty());
    }

    #[test]
    fn test_wizard_next_step() {
        let mut state = SpecKitWizardState::new();
        assert_eq!(state.step, SpecKitWizardStep::Clarify);
        state.next_step();
        assert_eq!(state.step, SpecKitWizardStep::Specify);
        state.next_step();
        assert_eq!(state.step, SpecKitWizardStep::Plan);
        state.next_step();
        assert_eq!(state.step, SpecKitWizardStep::Tasks);
        state.next_step();
        assert_eq!(state.step, SpecKitWizardStep::Done);
        // Should stay at Done
        state.next_step();
        assert_eq!(state.step, SpecKitWizardStep::Done);
    }

    #[test]
    fn test_wizard_step_labels() {
        assert_eq!(SpecKitWizardStep::Clarify.label(), "Clarify");
        assert_eq!(SpecKitWizardStep::Specify.label(), "Specify");
        assert_eq!(SpecKitWizardStep::Plan.label(), "Plan");
        assert_eq!(SpecKitWizardStep::Tasks.label(), "Tasks");
        assert_eq!(SpecKitWizardStep::Done.label(), "Done");
    }

    #[test]
    fn test_wizard_processing_state() {
        let mut state = SpecKitWizardState::new();
        state.set_processing("Generating spec...");
        assert!(state.is_processing);
        assert_eq!(
            state.progress_message.as_deref(),
            Some("Generating spec...")
        );

        state.clear_processing();
        assert!(!state.is_processing);
        assert!(state.progress_message.is_none());
    }

    #[test]
    fn test_wizard_error_state() {
        let mut state = SpecKitWizardState::new();
        state.set_processing("Working...");
        state.set_error("Something failed");
        assert!(!state.is_processing);
        assert_eq!(state.error.as_deref(), Some("Something failed"));
    }
}
