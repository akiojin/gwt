//! Progress modal overlay widget with 6-stage launch progress

#![allow(dead_code)]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Status of a single progress stage
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageStatus {
    Pending,
    InProgress,
    Done,
    Error(String),
}

/// A single stage in the launch progress
#[derive(Debug, Clone)]
pub struct ProgressStage {
    pub name: String,
    pub status: StageStatus,
    pub detail: Option<String>,
}

impl ProgressStage {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            status: StageStatus::Pending,
            detail: None,
        }
    }

    /// Unicode marker for rendering
    fn marker(&self) -> &'static str {
        match self.status {
            StageStatus::Pending => "\u{25CB}",    // ○
            StageStatus::InProgress => "\u{25CF}", // ●
            StageStatus::Done => "\u{2713}",       // ✓
            StageStatus::Error(_) => "\u{2717}",   // ✗
        }
    }

    /// Color for this stage's status
    fn color(&self) -> Color {
        match self.status {
            StageStatus::Pending => Color::DarkGray,
            StageStatus::InProgress => Color::Yellow,
            StageStatus::Done => Color::Green,
            StageStatus::Error(_) => Color::Red,
        }
    }
}

/// Full progress state for a multi-stage operation
#[derive(Debug, Clone)]
pub struct ProgressState {
    pub stages: Vec<ProgressStage>,
    pub current_stage: usize,
    pub cancellable: bool,
    pub cancelled: bool,
    pub title: String,
}

impl Default for ProgressState {
    fn default() -> Self {
        Self::launch_stages()
    }
}

impl ProgressState {
    /// Create the standard 6-stage launch progress
    pub fn launch_stages() -> Self {
        Self {
            stages: vec![
                ProgressStage::new("Fetch remote branches"),
                ProgressStage::new("Validate configuration"),
                ProgressStage::new("Creating worktree"),
                ProgressStage::new("Register skills"),
                ProgressStage::new("Resolve dependencies"),
                ProgressStage::new("Launch agent"),
            ],
            current_stage: 0,
            cancellable: true,
            cancelled: false,
            title: "Launching Agent".to_string(),
        }
    }

    /// Create a simple progress with custom title and detail
    pub fn simple(title: &str, detail: Option<&str>) -> Self {
        Self {
            stages: vec![ProgressStage {
                name: title.to_string(),
                status: StageStatus::InProgress,
                detail: detail.map(|s| s.to_string()),
            }],
            current_stage: 0,
            cancellable: false,
            cancelled: false,
            title: title.to_string(),
        }
    }

    /// Advance the current stage to Done and start the next
    pub fn advance(&mut self) {
        if self.current_stage < self.stages.len() {
            self.stages[self.current_stage].status = StageStatus::Done;
            self.current_stage += 1;
            if self.current_stage < self.stages.len() {
                self.stages[self.current_stage].status = StageStatus::InProgress;
            }
        }
    }

    /// Start the first stage
    pub fn start(&mut self) {
        if !self.stages.is_empty() {
            self.stages[0].status = StageStatus::InProgress;
        }
    }

    /// Set the current stage to error
    pub fn set_error(&mut self, message: String) {
        if self.current_stage < self.stages.len() {
            self.stages[self.current_stage].status = StageStatus::Error(message);
        }
    }

    /// Check if all stages are done
    pub fn all_done(&self) -> bool {
        self.stages.iter().all(|s| s.status == StageStatus::Done)
    }

    /// Check if any stage has error
    pub fn has_error(&self) -> bool {
        self.stages
            .iter()
            .any(|s| matches!(s.status, StageStatus::Error(_)))
    }
}

/// Render the 6-stage progress modal overlay.
pub fn render(buf: &mut Buffer, area: Rect, state: &ProgressState) {
    let modal_width = 50.min(area.width.saturating_sub(4));
    let stage_count = state.stages.len() as u16;
    // title border(1) + stages + empty line + cancel/status + border(1)
    let modal_height = (stage_count + 6).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    // Clear background
    Clear.render(modal_area, buf);

    let border_color = if state.has_error() {
        Color::Red
    } else if state.all_done() {
        Color::Green
    } else {
        Color::Cyan
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(format!(" {} ", state.title))
        .title_style(Style::default().add_modifier(Modifier::BOLD));

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from("")); // empty line at top

    for stage in &state.stages {
        let color = stage.color();
        let marker = stage.marker();
        let mut spans = vec![
            Span::styled(format!("  {} ", marker), Style::default().fg(color)),
            Span::styled(&stage.name, Style::default().fg(color)),
        ];

        // Show detail if in progress
        if stage.status == StageStatus::InProgress {
            if let Some(ref detail) = stage.detail {
                spans.push(Span::styled(
                    format!(" ({})", detail),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled("...", Style::default().fg(Color::DarkGray)));
            }
        }

        lines.push(Line::from(spans));

        // Show error on separate line
        if let StageStatus::Error(ref msg) = stage.status {
            lines.push(Line::from(Span::styled(
                format!("      {}", msg),
                Style::default().fg(Color::Red),
            )));
        }
    }

    lines.push(Line::from("")); // empty line

    // Cancel button or status
    if state.has_error() {
        lines.push(
            Line::from(Span::styled(
                "Press any key to continue...",
                Style::default().fg(Color::Yellow),
            ))
            .alignment(Alignment::Center),
        );
    } else if state.all_done() {
        lines.push(
            Line::from(Span::styled(
                "Complete!",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        );
    } else if state.cancellable {
        lines.push(
            Line::from(Span::styled(
                "[Cancel]",
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(Alignment::Center),
        );
    }

    let content = Paragraph::new(lines);
    ratatui::widgets::Widget::render(content, inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_state_launch_stages() {
        let state = ProgressState::launch_stages();
        assert_eq!(state.stages.len(), 6);
        assert!(!state.all_done());
        assert!(!state.has_error());
        assert!(state.cancellable);
    }

    #[test]
    fn test_progress_state_advance() {
        let mut state = ProgressState::launch_stages();
        state.start();
        assert_eq!(state.stages[0].status, StageStatus::InProgress);

        state.advance();
        assert_eq!(state.stages[0].status, StageStatus::Done);
        assert_eq!(state.stages[1].status, StageStatus::InProgress);
        assert_eq!(state.current_stage, 1);
    }

    #[test]
    fn test_progress_state_all_done() {
        let mut state = ProgressState::launch_stages();
        state.start();
        for _ in 0..6 {
            state.advance();
        }
        assert!(state.all_done());
    }

    #[test]
    fn test_progress_state_error() {
        let mut state = ProgressState::launch_stages();
        state.start();
        state.set_error("Network timeout".to_string());
        assert!(state.has_error());
        assert!(matches!(
            state.stages[0].status,
            StageStatus::Error(ref msg) if msg == "Network timeout"
        ));
    }

    #[test]
    fn test_progress_stage_markers() {
        let pending = ProgressStage::new("test");
        assert_eq!(pending.marker(), "\u{25CB}"); // ○

        let mut in_progress = ProgressStage::new("test");
        in_progress.status = StageStatus::InProgress;
        assert_eq!(in_progress.marker(), "\u{25CF}"); // ●

        let mut done = ProgressStage::new("test");
        done.status = StageStatus::Done;
        assert_eq!(done.marker(), "\u{2713}"); // ✓

        let mut error = ProgressStage::new("test");
        error.status = StageStatus::Error("err".to_string());
        assert_eq!(error.marker(), "\u{2717}"); // ✗
    }

    #[test]
    fn test_simple_progress() {
        let state = ProgressState::simple("Loading...", Some("step 1"));
        assert_eq!(state.stages.len(), 1);
        assert_eq!(state.title, "Loading...");
        assert!(!state.cancellable);
    }

    #[test]
    fn test_render_progress_no_panic() {
        let mut state = ProgressState::launch_stages();
        state.start();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
    }

    #[test]
    fn test_render_progress_with_error_no_panic() {
        let mut state = ProgressState::launch_stages();
        state.start();
        state.set_error("Failed to fetch".to_string());
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
    }

    #[test]
    fn test_render_progress_all_done_no_panic() {
        let mut state = ProgressState::launch_stages();
        state.start();
        for _ in 0..6 {
            state.advance();
        }
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, &state);
    }
}
