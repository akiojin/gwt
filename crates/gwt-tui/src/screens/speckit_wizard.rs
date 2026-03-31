//! Spec Kit Wizard Screen
//!
//! Step-based wizard for generating specification, plan, and task artifacts
//! from a feature description. Steps: Clarify -> Specify -> Plan -> Tasks -> Done.

#![allow(dead_code)]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph};

/// Spec Kit wizard step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecKitStep {
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

impl SpecKitStep {
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
pub struct SpecKitState {
    /// Whether wizard is visible
    pub visible: bool,
    /// Current step
    pub step: SpecKitStep,
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
    /// SPEC ID being created/edited
    pub spec_id: Option<String>,
    /// SPEC title
    pub spec_title: Option<String>,
    /// List of existing specs for browsing
    pub specs: Vec<SpecEntry>,
    /// Selected index in spec list
    pub selected: usize,
}

/// Entry in the spec list
#[derive(Debug, Clone)]
pub struct SpecEntry {
    pub id: String,
    pub title: String,
    pub status: String,
}

impl SpecKitState {
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
        self.step = SpecKitStep::Clarify;
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
            SpecKitStep::Clarify => SpecKitStep::Specify,
            SpecKitStep::Specify => SpecKitStep::Plan,
            SpecKitStep::Plan => SpecKitStep::Tasks,
            SpecKitStep::Tasks => SpecKitStep::Done,
            SpecKitStep::Done => SpecKitStep::Done,
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

    /// Handle character input
    pub fn handle_char(&mut self, c: char) {
        if self.step == SpecKitStep::Clarify && !self.is_processing {
            self.input.insert(self.input_cursor, c);
            self.input_cursor += 1;
        }
    }

    /// Handle backspace
    pub fn handle_backspace(&mut self) {
        if self.step == SpecKitStep::Clarify && self.input_cursor > 0 {
            self.input_cursor -= 1;
            self.input.remove(self.input_cursor);
        }
    }
}

/// Render the Spec Kit wizard overlay
pub fn render_speckit_wizard(state: &SpecKitState, buf: &mut Buffer, area: Rect) {
    if !state.visible {
        return;
    }

    // Calculate popup area (70% x 60% centered)
    let popup_width = ((area.width as f32) * 0.7) as u16;
    let popup_height = ((area.height as f32) * 0.6) as u16;
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    Clear.render(popup_area, buf);

    let step_label = format!(
        " Spec Kit Wizard [{}/{}] {} ",
        state.step.index() + 1,
        SpecKitStep::total(),
        state.step.label()
    );
    let block = Block::default()
        .title(step_label)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup_area);
    block.render(popup_area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    match state.step {
        SpecKitStep::Clarify => render_clarify_step(state, buf, chunks[0]),
        SpecKitStep::Specify | SpecKitStep::Plan | SpecKitStep::Tasks => {
            render_processing_step(state, buf, chunks[0])
        }
        SpecKitStep::Done => render_done_step(state, buf, chunks[0]),
    }

    // Help bar
    let help_text = if state.is_processing {
        "[Esc] Cancel"
    } else if state.step == SpecKitStep::Done {
        "[Enter] Close | [Esc] Close"
    } else {
        "[Enter] Next | [Esc] Cancel"
    };
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    help.render(chunks[1], buf);
}

fn render_clarify_step(state: &SpecKitState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let label = Paragraph::new("Describe the feature you want to implement:")
        .style(Style::default().fg(Color::White));
    label.render(chunks[0], buf);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let input = Paragraph::new(state.input.as_str())
        .block(input_block)
        .style(Style::default().fg(Color::White));
    input.render(chunks[1], buf);

    if let Some(ref error) = state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        error_text.render(chunks[2], buf);
    }
}

fn render_processing_step(state: &SpecKitState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let progress_msg = state.progress_message.as_deref().unwrap_or("Processing...");
    let progress = Paragraph::new(progress_msg)
        .style(Style::default().fg(Color::Yellow))
        .alignment(Alignment::Center);
    progress.render(chunks[0], buf);

    let completed = state.step.index();
    let total = SpecKitStep::total();
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(completed as f64 / total as f64)
        .label(format!("{}/{}", completed, total));
    gauge.render(chunks[1], buf);

    if let Some(ref error) = state.error {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        error_text.render(chunks[1], buf);
    }
}

fn render_done_step(state: &SpecKitState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new("Spec Kit artifacts generated successfully!")
        .style(Style::default().fg(Color::Green))
        .alignment(Alignment::Center);
    header.render(chunks[0], buf);

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
        ratatui::prelude::Widget::render(list, chunks[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_state_new() {
        let state = SpecKitState::new();
        assert!(!state.visible);
        assert_eq!(state.step, SpecKitStep::Clarify);
        assert!(state.input.is_empty());
        assert!(state.artifacts.is_empty());
    }

    #[test]
    fn test_wizard_open_close() {
        let mut state = SpecKitState::new();
        state.open();
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn test_wizard_reset() {
        let mut state = SpecKitState::new();
        state.input = "test".to_string();
        state.step = SpecKitStep::Plan;
        state.artifacts.push("file.md".to_string());
        state.reset();
        assert!(state.input.is_empty());
        assert_eq!(state.step, SpecKitStep::Clarify);
        assert!(state.artifacts.is_empty());
    }

    #[test]
    fn test_wizard_next_step() {
        let mut state = SpecKitState::new();
        assert_eq!(state.step, SpecKitStep::Clarify);
        state.next_step();
        assert_eq!(state.step, SpecKitStep::Specify);
        state.next_step();
        assert_eq!(state.step, SpecKitStep::Plan);
        state.next_step();
        assert_eq!(state.step, SpecKitStep::Tasks);
        state.next_step();
        assert_eq!(state.step, SpecKitStep::Done);
        state.next_step();
        assert_eq!(state.step, SpecKitStep::Done);
    }

    #[test]
    fn test_wizard_step_labels() {
        assert_eq!(SpecKitStep::Clarify.label(), "Clarify");
        assert_eq!(SpecKitStep::Specify.label(), "Specify");
        assert_eq!(SpecKitStep::Plan.label(), "Plan");
        assert_eq!(SpecKitStep::Tasks.label(), "Tasks");
        assert_eq!(SpecKitStep::Done.label(), "Done");
    }

    #[test]
    fn test_wizard_processing_state() {
        let mut state = SpecKitState::new();
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
        let mut state = SpecKitState::new();
        state.set_processing("Working...");
        state.set_error("Something failed");
        assert!(!state.is_processing);
        assert_eq!(state.error.as_deref(), Some("Something failed"));
    }

    #[test]
    fn test_wizard_handle_char() {
        let mut state = SpecKitState::new();
        state.handle_char('a');
        state.handle_char('b');
        assert_eq!(state.input, "ab");
        assert_eq!(state.input_cursor, 2);
    }

    #[test]
    fn test_wizard_handle_backspace() {
        let mut state = SpecKitState::new();
        state.handle_char('a');
        state.handle_char('b');
        state.handle_backspace();
        assert_eq!(state.input, "a");
        assert_eq!(state.input_cursor, 1);
    }

    #[test]
    fn test_step_index() {
        assert_eq!(SpecKitStep::Clarify.index(), 0);
        assert_eq!(SpecKitStep::Done.index(), 4);
        assert_eq!(SpecKitStep::total(), 5);
    }

    #[test]
    fn test_render_speckit_wizard_invisible_no_panic() {
        let state = SpecKitState::new(); // visible = false
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_speckit_wizard(&state, &mut buf, area);
    }

    #[test]
    fn test_render_speckit_wizard_visible_no_panic() {
        let mut state = SpecKitState::new();
        state.open();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_speckit_wizard(&state, &mut buf, area);
    }
}
