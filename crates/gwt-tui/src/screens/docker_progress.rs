//! Docker progress overlay screen.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Gauge, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::theme;

/// Docker build/launch stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockerStage {
    #[default]
    DetectingFiles,
    BuildingImage,
    StartingContainer,
    WaitingForServices,
    Ready,
    Failed,
}

impl DockerStage {
    /// All stages in order (excluding Failed).
    const PROGRESS: [DockerStage; 5] = [
        DockerStage::DetectingFiles,
        DockerStage::BuildingImage,
        DockerStage::StartingContainer,
        DockerStage::WaitingForServices,
        DockerStage::Ready,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::DetectingFiles => "Detecting files",
            Self::BuildingImage => "Building image",
            Self::StartingContainer => "Starting container",
            Self::WaitingForServices => "Waiting for services",
            Self::Ready => "Ready",
            Self::Failed => "Failed",
        }
    }

    /// Progress index (0-based) within the normal flow.
    pub fn index(self) -> usize {
        Self::PROGRESS.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// Progress ratio (0.0 .. 1.0).
    pub fn ratio(self) -> f64 {
        if self == Self::Failed {
            return 0.0;
        }
        let idx = self.index();
        let total = Self::PROGRESS.len().saturating_sub(1).max(1);
        idx as f64 / total as f64
    }
}

/// State for the Docker progress overlay.
#[derive(Debug, Clone, Default)]
pub struct DockerProgressState {
    pub stage: DockerStage,
    pub message: String,
    pub error: Option<String>,
    pub visible: bool,
}

impl DockerProgressState {
    /// Make the overlay visible.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Hide the overlay.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Update the descriptive message shown above the stage list.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    /// Advance to the next stage in the normal flow.
    pub fn advance(&mut self) {
        let idx = self.stage.index();
        if let Some(&next) = DockerStage::PROGRESS.get(idx + 1) {
            self.stage = next;
            self.error = None;
        }
    }

    /// Transition to a failed state and surface the error.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.stage = DockerStage::Failed;
        self.error = Some(error.into());
        self.visible = true;
    }

    /// Restore the initial hidden state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Messages for the Docker progress overlay.
#[derive(Debug, Clone)]
pub enum DockerProgressMessage {
    /// Set the current stage and descriptive message from an external event source.
    SetStage { stage: DockerStage, message: String },
    /// Hide the overlay without resetting progress state.
    Hide,
    /// Advance to the next stage.
    Advance,
    /// Set an error and move to Failed stage.
    SetError(String),
    /// Reset to initial state.
    Reset,
}

/// Update Docker progress state.
pub fn update(state: &mut DockerProgressState, msg: DockerProgressMessage) {
    match msg {
        DockerProgressMessage::SetStage { stage, message } => {
            state.stage = stage;
            state.message = message;
            state.error = None;
            state.show();
        }
        DockerProgressMessage::Hide => state.hide(),
        DockerProgressMessage::Advance => state.advance(),
        DockerProgressMessage::SetError(err) => state.fail(err),
        DockerProgressMessage::Reset => state.reset(),
    }
}

/// Render the Docker progress overlay.
pub fn render(state: &DockerProgressState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 50_u16.min(area.width);
    let inner_width = width.saturating_sub(2).max(1) as usize;

    // Calculate dynamic height: status(1) + error_or_message(wrapped) + empty(1)
    //   + stages(5) + gauge(1) + borders(2)
    let variable_text = if let Some(ref err) = state.error {
        format!("Error: {err}")
    } else if !state.message.is_empty() {
        format!("Message: {}", state.message)
    } else {
        String::new()
    };
    let variable_lines: u16 = if variable_text.is_empty() {
        0
    } else {
        let w = UnicodeWidthStr::width(variable_text.as_str());
        if w == 0 {
            1
        } else {
            w.div_ceil(inner_width) as u16
        }
    };
    // status(1) + variable + empty(1) + stages(5) + gauge(1) + borders(2)
    let height = (1 + variable_lines + 1 + 5 + 1 + 2)
        .max(10)
        .min(area.height);

    let border_color = if state.stage == DockerStage::Failed {
        theme::color::ERROR
    } else if state.stage == DockerStage::Ready {
        theme::color::SUCCESS
    } else {
        theme::color::FOCUS
    };

    let inner = super::render_modal_frame(frame, area, "Docker", border_color, width, height);

    if inner.height < 3 || inner.width < 4 {
        return;
    }

    // Stage list with spinner/check marks
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Status: {}", state.stage.label()),
        Style::default()
            .fg(border_color)
            .add_modifier(Modifier::BOLD),
    )));
    if let Some(ref err) = state.error {
        lines.push(Line::from(Span::styled(
            format!("Error: {err}"),
            Style::default().fg(theme::color::ERROR),
        )));
    } else if !state.message.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Message: {}", state.message),
            Style::default().fg(theme::color::TEXT_PRIMARY),
        )));
    }
    lines.push(Line::from(""));

    for &stage in &DockerStage::PROGRESS {
        let (icon, style) = if stage == state.stage && stage != DockerStage::Ready {
            (
                concat!("\u{25B6}", " "), // theme::icon::ARROW_RIGHT + space
                Style::default()
                    .fg(theme::color::ACTIVE)
                    .add_modifier(Modifier::BOLD),
            )
        } else if stage.index() < state.stage.index() || state.stage == DockerStage::Ready {
            (
                concat!("\u{2714}", " "), // theme::icon::CHECKMARK + space
                Style::default().fg(theme::color::SUCCESS),
            )
        } else {
            (
                concat!("\u{25CB}", " "), // theme::icon::CIRCLE_EMPTY + space
                Style::default().fg(theme::color::SURFACE),
            )
        };
        lines.push(Line::from(Span::styled(
            format!("{icon}{}", stage.label()),
            style,
        )));
    }

    // Progress gauge at bottom
    let gauge_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    let text_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, text_area);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(border_color).bg(theme::color::SURFACE))
        .ratio(state.stage.ratio());
    frame.render_widget(gauge, gauge_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_text(state: &DockerProgressState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn default_state() {
        let state = DockerProgressState::default();
        assert_eq!(state.stage, DockerStage::DetectingFiles);
        assert!(state.message.is_empty());
        assert!(state.error.is_none());
        assert!(!state.visible);
    }

    #[test]
    fn advance_progresses_through_stages() {
        let mut state = DockerProgressState::default();
        assert_eq!(state.stage, DockerStage::DetectingFiles);

        update(&mut state, DockerProgressMessage::Advance);
        assert_eq!(state.stage, DockerStage::BuildingImage);

        update(&mut state, DockerProgressMessage::Advance);
        assert_eq!(state.stage, DockerStage::StartingContainer);

        update(&mut state, DockerProgressMessage::Advance);
        assert_eq!(state.stage, DockerStage::WaitingForServices);

        update(&mut state, DockerProgressMessage::Advance);
        assert_eq!(state.stage, DockerStage::Ready);
    }

    #[test]
    fn advance_at_ready_stays_ready() {
        let mut state = DockerProgressState::default();
        for _ in 0..10 {
            update(&mut state, DockerProgressMessage::Advance);
        }
        assert_eq!(state.stage, DockerStage::Ready);
    }

    #[test]
    fn show_and_hide_toggle_overlay_visibility() {
        let mut state = DockerProgressState::default();

        state.show();
        assert!(state.visible);

        state.hide();
        assert!(!state.visible);
    }

    #[test]
    fn control_surface_advances_and_records_messages() {
        let mut state = DockerProgressState::default();

        state.show();
        state.set_message("Detecting compose files");
        state.advance();

        assert!(state.visible);
        assert_eq!(state.stage, DockerStage::BuildingImage);
        assert_eq!(state.message, "Detecting compose files");

        state.fail("docker daemon unavailable");
        assert_eq!(state.stage, DockerStage::Failed);
        assert_eq!(state.error.as_deref(), Some("docker daemon unavailable"));
        assert!(state.visible);
    }

    #[test]
    fn reset_restores_initial_hidden_state() {
        let mut state = DockerProgressState::default();

        state.show();
        state.set_message("Launching");
        state.advance();
        state.fail("boom");
        state.reset();

        assert_eq!(state.stage, DockerStage::DetectingFiles);
        assert!(state.message.is_empty());
        assert!(state.error.is_none());
        assert!(!state.visible);
    }

    #[test]
    fn set_error_moves_to_failed() {
        let mut state = DockerProgressState::default();
        update(&mut state, DockerProgressMessage::Advance);

        update(
            &mut state,
            DockerProgressMessage::SetError("build failed".into()),
        );
        assert_eq!(state.stage, DockerStage::Failed);
        assert_eq!(state.error.as_deref(), Some("build failed"));
    }

    #[test]
    fn set_stage_updates_message_and_makes_overlay_visible() {
        let mut state = DockerProgressState::default();

        update(
            &mut state,
            DockerProgressMessage::SetStage {
                stage: DockerStage::StartingContainer,
                message: "Starting api".into(),
            },
        );

        assert!(state.visible);
        assert_eq!(state.stage, DockerStage::StartingContainer);
        assert_eq!(state.message, "Starting api");
        assert!(state.error.is_none());
    }

    #[test]
    fn hide_hides_overlay_without_resetting_progress() {
        let mut state = DockerProgressState {
            visible: true,
            stage: DockerStage::WaitingForServices,
            message: "Waiting".into(),
            error: None,
        };

        update(&mut state, DockerProgressMessage::Hide);

        assert!(!state.visible);
        assert_eq!(state.stage, DockerStage::WaitingForServices);
        assert_eq!(state.message, "Waiting");
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut state = DockerProgressState::default();
        update(&mut state, DockerProgressMessage::Advance);
        update(&mut state, DockerProgressMessage::Advance);

        update(&mut state, DockerProgressMessage::Reset);
        assert_eq!(state.stage, DockerStage::DetectingFiles);
        assert!(state.error.is_none());
        assert!(state.message.is_empty());
    }

    #[test]
    fn stage_labels_are_non_empty() {
        for &stage in &DockerStage::PROGRESS {
            assert!(!stage.label().is_empty());
        }
        assert!(!DockerStage::Failed.label().is_empty());
    }

    #[test]
    fn render_shows_explicit_status_label_for_each_stage() {
        for &stage in &DockerStage::PROGRESS {
            let state = DockerProgressState {
                visible: true,
                stage,
                ..DockerProgressState::default()
            };
            let text = render_text(&state);
            assert!(
                text.contains(&format!("Status: {}", stage.label())),
                "missing status line for {:?}\n{}",
                stage,
                text
            );
        }
    }

    #[test]
    fn stage_ratio_increases() {
        let ratios: Vec<f64> = DockerStage::PROGRESS.iter().map(|s| s.ratio()).collect();
        for i in 1..ratios.len() {
            assert!(ratios[i] >= ratios[i - 1]);
        }
        assert_eq!(DockerStage::Failed.ratio(), 0.0);
    }

    #[test]
    fn render_visible_does_not_panic() {
        let state = DockerProgressState {
            visible: true,
            ..DockerProgressState::default()
        };
        let full_text = render_text(&state);
        assert!(full_text.contains("Docker"));
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = DockerProgressState::default();
        let full_text = render_text(&state);
        assert!(!full_text.contains("Docker"));
    }

    #[test]
    fn render_long_error_wraps_without_truncation() {
        let long_err = "E".repeat(120);
        let state = DockerProgressState {
            visible: true,
            stage: DockerStage::Failed,
            error: Some(long_err.clone()),
            ..DockerProgressState::default()
        };
        let text = render_text(&state);
        let e_count = text.chars().filter(|ch| *ch == 'E').count();
        assert!(
            e_count >= 120,
            "Expected at least 120 'E' chars in buffer, found {e_count}"
        );
    }

    #[test]
    fn render_failed_state_is_explicit_about_error() {
        let state = DockerProgressState {
            visible: true,
            stage: DockerStage::Failed,
            error: Some("Docker daemon not running".into()),
            ..DockerProgressState::default()
        };

        let text = render_text(&state);
        assert!(text.contains("Status: Failed"), "{}", text);
        assert!(
            text.contains("Error: Docker daemon not running"),
            "{}",
            text
        );
        assert!(text.contains("Failed"));
    }
}
