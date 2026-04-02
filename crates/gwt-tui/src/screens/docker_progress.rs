//! Docker progress overlay screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};

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
        Self::PROGRESS
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0)
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

/// Messages for the Docker progress overlay.
#[derive(Debug, Clone)]
pub enum DockerProgressMessage {
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
        DockerProgressMessage::Advance => {
            let idx = state.stage.index();
            if let Some(&next) = DockerStage::PROGRESS.get(idx + 1) {
                state.stage = next;
                state.error = None;
            }
        }
        DockerProgressMessage::SetError(err) => {
            state.stage = DockerStage::Failed;
            state.error = Some(err);
        }
        DockerProgressMessage::Reset => {
            state.stage = DockerStage::DetectingFiles;
            state.message = String::new();
            state.error = None;
        }
    }
}

/// Render the Docker progress overlay.
pub fn render(state: &DockerProgressState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 50_u16.min(area.width);
    let height = 12_u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let border_color = if state.stage == DockerStage::Failed {
        Color::Red
    } else if state.stage == DockerStage::Ready {
        Color::Green
    } else {
        Color::Cyan
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Docker")
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if inner.height < 3 || inner.width < 4 {
        return;
    }

    // Stage list with spinner/check marks
    let mut lines: Vec<Line> = Vec::new();
    for &stage in &DockerStage::PROGRESS {
        let (icon, style) = if stage == state.stage && stage != DockerStage::Ready {
            (
                "\u{25B6} ", // right-pointing triangle (spinner stand-in)
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if stage.index() < state.stage.index()
            || state.stage == DockerStage::Ready
        {
            ("\u{2714} ", Style::default().fg(Color::Green)) // check mark
        } else {
            ("\u{25CB} ", Style::default().fg(Color::DarkGray)) // circle
        };
        lines.push(Line::from(Span::styled(
            format!("{icon}{}", stage.label()),
            style,
        )));
    }

    // Error line
    if let Some(ref err) = state.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Progress gauge at bottom
    let gauge_area = Rect::new(inner.x, inner.y + inner.height.saturating_sub(1), inner.width, 1);
    let text_area = Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(2));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, text_area);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(border_color).bg(Color::DarkGray))
        .ratio(state.stage.ratio());
    frame.render_widget(gauge, gauge_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(full_text.contains("Docker"));
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = DockerProgressState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(!full_text.contains("Docker"));
    }
}
