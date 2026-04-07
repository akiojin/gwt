//! Branch Cleanup progress modal (FR-018g/h).
//!
//! Drives the progress bar plus per-branch outcome list while a cleanup run
//! is in flight. Input is fully blocked while `phase == Running`; only after
//! `phase == Done` does `Enter` / `Esc` dismiss the modal.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};

use crate::screens::branches::{CleanupResultRow, CleanupRunPhase, CleanupRunState};
use crate::theme;

/// Visibility wrapper around `Option<CleanupRunState>` so render() can stay
/// pure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupProgressState {
    pub visible: bool,
    pub run: Option<CleanupRunState>,
}

impl CleanupProgressState {
    pub fn show(&mut self, total: usize, delete_remote: bool) {
        self.run = Some(CleanupRunState::new(total, delete_remote));
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn set_current(&mut self, branch: impl Into<String>) {
        if let Some(run) = self.run.as_mut() {
            run.current = Some(branch.into());
        }
    }

    pub fn record(&mut self, row: CleanupResultRow) {
        if let Some(run) = self.run.as_mut() {
            run.record_result(row);
        }
    }

    pub fn finish(&mut self) {
        if let Some(run) = self.run.as_mut() {
            run.finish();
        }
    }

    pub fn phase(&self) -> CleanupRunPhase {
        self.run
            .as_ref()
            .map(|r| r.phase)
            .unwrap_or(CleanupRunPhase::Done)
    }

    pub fn is_running(&self) -> bool {
        self.phase() == CleanupRunPhase::Running
    }

    /// Returns true when the modal currently consumes the keyboard and the
    /// caller must not forward the key event anywhere else.
    pub fn captures_input(&self) -> bool {
        self.visible
    }
}

/// Messages emitted by the cleanup runner background job.
#[derive(Debug, Clone)]
pub enum CleanupProgressMessage {
    /// A new branch is being processed.
    Started { branch: String },
    /// A branch finished — record its outcome.
    Finished {
        branch: String,
        success: bool,
        message: Option<String>,
    },
    /// The whole run finished. Caller should next dispatch `Dismiss` once the
    /// user acknowledges.
    Completed,
    /// User dismissed the modal after the run finished.
    Dismiss,
}

/// Drive the modal forward with one message.
pub fn update(state: &mut CleanupProgressState, msg: CleanupProgressMessage) {
    match msg {
        CleanupProgressMessage::Started { branch } => state.set_current(branch),
        CleanupProgressMessage::Finished {
            branch,
            success,
            message,
        } => state.record(CleanupResultRow {
            branch,
            success,
            message,
        }),
        CleanupProgressMessage::Completed => state.finish(),
        // FR-018g: `Dismiss` must be ignored while the run is in progress so
        // a stray key or caller mistake cannot close the modal mid-run.
        CleanupProgressMessage::Dismiss => {
            if state.phase() == CleanupRunPhase::Done {
                state.hide();
            }
        }
    }
}

/// Render the modal centered inside `area`.
pub fn render(state: &CleanupProgressState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }
    let Some(run) = state.run.as_ref() else {
        return;
    };

    let row_count = run.total.max(1) as u16;
    let height = (row_count + 6).min(area.height).max(8);
    let width = 60_u16.min(area.width);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let title = if run.phase == CleanupRunPhase::Running {
        " Cleaning Up "
    } else {
        " Cleanup Complete "
    };
    let border_color = if run.phase == CleanupRunPhase::Done && run.failed() > 0 {
        theme::color::ERROR
    } else if run.phase == CleanupRunPhase::Done {
        theme::color::SUCCESS
    } else {
        theme::color::FOCUS
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme::border::modal())
        .title(title)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if inner.height < 4 || inner.width < 8 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // Status / counts
    let header_text = if run.phase == CleanupRunPhase::Running {
        format!("Processing {} / {}", run.processed, run.total)
    } else {
        format!("Cleaned {}, failed {}", run.succeeded(), run.failed())
    };
    let header = Paragraph::new(Line::from(Span::styled(
        header_text,
        Style::default()
            .fg(border_color)
            .add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(header, layout[0]);

    // Progress gauge
    let ratio = if run.total == 0 {
        1.0
    } else {
        (run.processed as f64 / run.total as f64).clamp(0.0, 1.0)
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(border_color).bg(theme::color::SURFACE))
        .ratio(ratio);
    frame.render_widget(gauge, layout[1]);

    // Per-branch list
    let mut lines: Vec<Line> = run
        .results
        .iter()
        .map(|row| {
            let (icon, color) = if row.success {
                ("\u{2713} ", theme::color::SUCCESS)
            } else {
                ("\u{2717} ", theme::color::ERROR)
            };
            let mut spans = vec![
                Span::styled(icon, Style::default().fg(color)),
                Span::styled(
                    row.branch.clone(),
                    Style::default().fg(theme::color::TEXT_PRIMARY),
                ),
            ];
            if let Some(message) = row.message.as_ref() {
                spans.push(Span::styled(
                    format!(": {message}"),
                    Style::default().fg(theme::color::TEXT_SECONDARY),
                ));
            }
            Line::from(spans)
        })
        .collect();

    if run.phase == CleanupRunPhase::Running {
        if let Some(current) = run.current.as_ref() {
            lines.push(Line::from(vec![
                Span::styled("\u{2192} ", Style::default().fg(theme::color::ACTIVE)),
                Span::styled(
                    format!("Removing {current}\u{2026}"),
                    Style::default().fg(theme::color::TEXT_PRIMARY),
                ),
            ]));
        }
    }

    let body = Paragraph::new(lines);
    frame.render_widget(body, layout[2]);

    let hint_text = if run.phase == CleanupRunPhase::Running {
        "  Cleanup running — input blocked"
    } else {
        "  [Enter] Dismiss   [Esc] Dismiss"
    };
    let hint = Paragraph::new(Line::from(Span::styled(
        hint_text,
        Style::default().fg(theme::color::TEXT_SECONDARY),
    )));
    frame.render_widget(hint, layout[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_text(state: &CleanupProgressState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(state, f, f.area())).unwrap();
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
    fn show_creates_running_run_state() {
        let mut state = CleanupProgressState::default();
        state.show(3, false);
        assert!(state.visible);
        assert!(state.is_running());
        assert_eq!(state.run.as_ref().unwrap().total, 3);
    }

    #[test]
    fn started_then_finished_records_progress() {
        let mut state = CleanupProgressState::default();
        state.show(2, false);

        update(
            &mut state,
            CleanupProgressMessage::Started {
                branch: "feature/foo".to_string(),
            },
        );
        assert_eq!(
            state.run.as_ref().unwrap().current.as_deref(),
            Some("feature/foo")
        );

        update(
            &mut state,
            CleanupProgressMessage::Finished {
                branch: "feature/foo".to_string(),
                success: true,
                message: None,
            },
        );
        let run = state.run.as_ref().unwrap();
        assert_eq!(run.processed, 1);
        assert_eq!(run.results.len(), 1);
        assert!(run.current.is_none());
    }

    #[test]
    fn completed_transitions_to_done_phase() {
        let mut state = CleanupProgressState::default();
        state.show(1, false);
        update(
            &mut state,
            CleanupProgressMessage::Started {
                branch: "feature/x".to_string(),
            },
        );
        update(
            &mut state,
            CleanupProgressMessage::Finished {
                branch: "feature/x".to_string(),
                success: true,
                message: None,
            },
        );
        update(&mut state, CleanupProgressMessage::Completed);
        assert_eq!(state.phase(), CleanupRunPhase::Done);
        assert!(!state.is_running());
        // Modal stays visible until Dismiss.
        assert!(state.visible);
    }

    #[test]
    fn dismiss_hides_modal_only_when_done() {
        let mut state = CleanupProgressState::default();
        state.show(1, false);
        // Dismiss while Running must be ignored (FR-018g).
        update(&mut state, CleanupProgressMessage::Dismiss);
        assert!(state.visible, "dismiss must be ignored while running");
        assert!(state.is_running());
        update(&mut state, CleanupProgressMessage::Completed);
        update(&mut state, CleanupProgressMessage::Dismiss);
        assert!(!state.visible);
    }

    #[test]
    fn captures_input_while_visible() {
        let mut state = CleanupProgressState::default();
        assert!(!state.captures_input());
        state.show(1, false);
        assert!(state.captures_input());
    }

    #[test]
    fn render_running_shows_progress_and_blocked_hint() {
        let mut state = CleanupProgressState::default();
        state.show(3, false);
        update(
            &mut state,
            CleanupProgressMessage::Started {
                branch: "feature/foo".to_string(),
            },
        );
        let text = render_text(&state);
        assert!(text.contains("Cleaning Up"), "{text}");
        assert!(text.contains("Processing 0 / 3"), "{text}");
        assert!(text.contains("Removing feature/foo"), "{text}");
        assert!(text.contains("input blocked"), "{text}");
    }

    #[test]
    fn render_done_shows_summary_and_dismiss_hint() {
        let mut state = CleanupProgressState::default();
        state.show(2, false);
        update(
            &mut state,
            CleanupProgressMessage::Finished {
                branch: "feature/foo".to_string(),
                success: true,
                message: None,
            },
        );
        update(
            &mut state,
            CleanupProgressMessage::Finished {
                branch: "feature/bar".to_string(),
                success: false,
                message: Some("worktree busy".to_string()),
            },
        );
        update(&mut state, CleanupProgressMessage::Completed);

        let text = render_text(&state);
        assert!(text.contains("Cleanup Complete"), "{text}");
        assert!(text.contains("Cleaned 1, failed 1"), "{text}");
        assert!(text.contains("feature/foo"), "{text}");
        assert!(text.contains("feature/bar"), "{text}");
        assert!(text.contains("worktree busy"), "{text}");
        assert!(text.contains("[Enter] Dismiss"), "{text}");
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = CleanupProgressState::default();
        let text = render_text(&state);
        assert!(!text.contains("Cleaning Up"));
        assert!(!text.contains("Cleanup Complete"));
    }
}
