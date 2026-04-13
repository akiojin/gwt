//! Branch Cleanup confirmation modal (FR-018e).
//!
//! Listed branches come from the Branches list selection. The modal lets the
//! user toggle the run-wide `delete_remote` setting with `r`, confirm with
//! `Enter`, and cancel with `Esc`. All other input is ignored so the modal
//! owns the keyboard while it is visible.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use gwt_git::MergeTarget;

use super::branches::CleanupSelectionRisk;
use crate::theme;

/// One row in the confirm modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupConfirmRow {
    pub branch: String,
    pub target: Option<MergeTarget>,
    pub execution_branch: String,
    pub upstream: Option<String>,
    pub risks: Vec<CleanupSelectionRisk>,
}

/// State of the Branch Cleanup confirm modal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupConfirmState {
    pub visible: bool,
    pub rows: Vec<CleanupConfirmRow>,
    pub delete_remote: bool,
}

impl CleanupConfirmState {
    /// Open the modal with the given selection.
    pub fn show(&mut self, rows: Vec<CleanupConfirmRow>, delete_remote: bool) {
        self.rows = rows;
        self.delete_remote = delete_remote;
        self.visible = true;
    }

    /// Hide the modal without dropping its state.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Number of branches that will be deleted on confirm.
    pub fn count(&self) -> usize {
        self.rows.len()
    }

    fn has_risky_rows(&self) -> bool {
        self.rows.iter().any(|row| !row.risks.is_empty())
    }
}

/// Messages for the confirm modal.
#[derive(Debug, Clone)]
pub enum CleanupConfirmMessage {
    /// `r` pressed — caller should flip the run-wide remote-delete toggle.
    ToggleRemote,
    /// `Enter` pressed — caller should start the cleanup run.
    Confirm,
    /// `Esc` pressed — caller should drop the modal.
    Cancel,
}

/// Outcome returned by [`update`] so the caller can react.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupConfirmOutcome {
    /// Modal is still open.
    Pending,
    /// User confirmed; caller should start the cleanup run.
    Confirmed,
    /// User cancelled.
    Cancelled,
}

/// Drive the modal forward by one message.
pub fn update(
    state: &mut CleanupConfirmState,
    msg: CleanupConfirmMessage,
) -> CleanupConfirmOutcome {
    match msg {
        CleanupConfirmMessage::ToggleRemote => {
            state.delete_remote = !state.delete_remote;
            CleanupConfirmOutcome::Pending
        }
        CleanupConfirmMessage::Confirm => {
            state.hide();
            CleanupConfirmOutcome::Confirmed
        }
        CleanupConfirmMessage::Cancel => {
            state.hide();
            CleanupConfirmOutcome::Cancelled
        }
    }
}

/// Render the modal centered inside `area`.
pub fn render(state: &CleanupConfirmState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 60_u16.min(area.width);
    let body_height = (state.rows.len() as u16)
        .saturating_add(if state.has_risky_rows() { 9 } else { 8 })
        .min(area.height);
    let height = body_height.max(9);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme::border::modal())
        .title(" Confirm Cleanup ")
        .border_style(Style::default().fg(theme::color::WARNING));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if inner.height < 4 || inner.width < 8 {
        return;
    }

    let layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(if state.has_risky_rows() { 1 } else { 0 }),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("Delete {} ", state.rows.len()),
            Style::default()
                .fg(theme::color::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if state.rows.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            Style::default().fg(theme::color::TEXT_PRIMARY),
        ),
        Span::styled(
            " and their worktrees? This cannot be undone.",
            Style::default().fg(theme::color::TEXT_SECONDARY),
        ),
    ]));
    frame.render_widget(header, layout[0]);

    if state.has_risky_rows() {
        let warning = Paragraph::new(Line::from(vec![Span::styled(
            "  Warning: includes unmerged / remote-tracking branches.",
            Style::default().fg(theme::color::WARNING),
        )]));
        frame.render_widget(warning, layout[1]);
    }

    let body_lines: Vec<Line> = state
        .rows
        .iter()
        .map(|row| {
            let status = row
                .target
                .map(|target| target.label().to_string())
                .unwrap_or_else(|| "not merged".to_string());
            let mut risk_labels = row
                .risks
                .iter()
                .map(|risk| risk.label())
                .collect::<Vec<_>>();
            risk_labels.sort_unstable();

            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    row.branch.clone(),
                    Style::default().fg(theme::color::TEXT_PRIMARY),
                ),
                Span::styled("    ", Style::default()),
                Span::styled(status, Style::default().fg(theme::color::TEXT_SECONDARY)),
            ];
            if !risk_labels.is_empty() {
                spans.push(Span::styled("    ", Style::default()));
                spans.push(Span::styled(
                    risk_labels.join(", "),
                    Style::default().fg(theme::color::WARNING),
                ));
            }
            Line::from(spans)
        })
        .collect();
    let body = Paragraph::new(body_lines);
    frame.render_widget(body, layout[2]);

    let toggle = Paragraph::new(Line::from(vec![
        Span::styled(
            if state.delete_remote {
                "  [x] "
            } else {
                "  [ ] "
            },
            Style::default().fg(theme::color::ACTIVE),
        ),
        Span::styled(
            "Also delete remote (r)",
            Style::default().fg(theme::color::TEXT_SECONDARY),
        ),
    ]));
    frame.render_widget(toggle, layout[3]);

    let hint = Paragraph::new(Line::from(vec![Span::styled(
        "  [r] Toggle remote   [Enter] Confirm   [Esc] Cancel",
        Style::default().fg(theme::color::TEXT_SECONDARY),
    )]));
    frame.render_widget(hint, layout[4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rows() -> Vec<CleanupConfirmRow> {
        vec![
            CleanupConfirmRow {
                branch: "feature/foo".to_string(),
                target: Some(MergeTarget::Main),
                execution_branch: "feature/foo".to_string(),
                upstream: Some("origin/feature/foo".to_string()),
                risks: Vec::new(),
            },
            CleanupConfirmRow {
                branch: "feature/bar".to_string(),
                target: Some(MergeTarget::Develop),
                execution_branch: "feature/bar".to_string(),
                upstream: Some("origin/feature/bar".to_string()),
                risks: Vec::new(),
            },
            CleanupConfirmRow {
                branch: "feature/abandoned".to_string(),
                target: Some(MergeTarget::Gone),
                execution_branch: "feature/abandoned".to_string(),
                upstream: Some("origin/feature/abandoned".to_string()),
                risks: Vec::new(),
            },
        ]
    }

    fn render_text(state: &CleanupConfirmState) -> String {
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
    fn show_makes_modal_visible_with_rows() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);
        assert!(state.visible);
        assert_eq!(state.count(), 3);
    }

    #[test]
    fn confirm_returns_confirmed_outcome() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);
        let outcome = update(&mut state, CleanupConfirmMessage::Confirm);
        assert_eq!(outcome, CleanupConfirmOutcome::Confirmed);
        assert!(!state.visible);
    }

    #[test]
    fn cancel_hides_modal() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);
        let outcome = update(&mut state, CleanupConfirmMessage::Cancel);
        assert_eq!(outcome, CleanupConfirmOutcome::Cancelled);
        assert!(!state.visible);
    }

    #[test]
    fn toggle_remote_flips_delete_remote_and_stays_pending() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);

        let outcome = update(&mut state, CleanupConfirmMessage::ToggleRemote);

        assert_eq!(outcome, CleanupConfirmOutcome::Pending);
        assert!(state.visible);
        assert!(state.delete_remote);
    }

    #[test]
    fn render_lists_each_branch_with_its_target_label() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);
        let text = render_text(&state);
        assert!(text.contains("Confirm Cleanup"), "{text}");
        assert!(text.contains("Delete 3"), "{text}");
        assert!(text.contains("feature/foo"), "{text}");
        assert!(text.contains("merged → main"), "{text}");
        assert!(text.contains("feature/bar"), "{text}");
        assert!(text.contains("merged → develop"), "{text}");
        assert!(text.contains("feature/abandoned"), "{text}");
        assert!(text.contains("gone"), "{text}");
        assert!(text.contains("[Enter] Confirm"), "{text}");
        assert!(text.contains("[Esc] Cancel"), "{text}");
    }

    #[test]
    fn render_advertises_delete_remote_toggle() {
        let mut state = CleanupConfirmState::default();
        state.show(rows(), false);
        let text = render_text(&state);
        assert!(text.contains("Also delete remote"), "{text}");
    }

    #[test]
    fn render_shows_warning_summary_for_risky_rows() {
        let mut state = CleanupConfirmState::default();
        state.show(
            vec![
                CleanupConfirmRow {
                    branch: "feature/unmerged".to_string(),
                    target: None,
                    execution_branch: "feature/unmerged".to_string(),
                    upstream: None,
                    risks: vec![CleanupSelectionRisk::Unmerged],
                },
                CleanupConfirmRow {
                    branch: "origin/feature/foo".to_string(),
                    target: Some(MergeTarget::Main),
                    execution_branch: "feature/foo".to_string(),
                    upstream: Some("origin/feature/foo".to_string()),
                    risks: vec![CleanupSelectionRisk::RemoteTracking],
                },
            ],
            false,
        );
        let text = render_text(&state);
        assert!(text.contains("unmerged / remote-tracking"), "{text}");
        assert!(text.contains("feature/unmerged"), "{text}");
        assert!(text.contains("not merged"), "{text}");
        assert!(text.contains("origin/feature/foo"), "{text}");
        assert!(text.contains("remote-tracking"), "{text}");
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = CleanupConfirmState::default();
        let text = render_text(&state);
        assert!(!text.contains("Confirm Cleanup"));
    }
}
