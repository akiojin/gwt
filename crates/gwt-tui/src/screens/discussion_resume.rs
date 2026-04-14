//! Discussion resume proposal overlay.

use gwt_agent::PendingDiscussionResume;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscussionResumeChoice {
    Resume,
    Park,
    Dismiss,
}

impl DiscussionResumeChoice {
    const ALL: [DiscussionResumeChoice; 3] = [
        DiscussionResumeChoice::Resume,
        DiscussionResumeChoice::Park,
        DiscussionResumeChoice::Dismiss,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume discussion",
            Self::Park => "Park proposal",
            Self::Dismiss => "Dismiss for now",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscussionResumeState {
    pub session_id: String,
    pub pending: PendingDiscussionResume,
    pub selected: usize,
    pub visible: bool,
}

impl DiscussionResumeState {
    pub fn with_pending(session_id: impl Into<String>, pending: PendingDiscussionResume) -> Self {
        Self {
            session_id: session_id.into(),
            pending,
            selected: 0,
            visible: true,
        }
    }

    pub fn selected_choice(&self) -> DiscussionResumeChoice {
        DiscussionResumeChoice::ALL
            .get(self.selected)
            .copied()
            .unwrap_or(DiscussionResumeChoice::Resume)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscussionResumeMessage {
    MoveUp,
    MoveDown,
    Select,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscussionResumeOutcome {
    Pending,
    Selected(DiscussionResumeChoice),
}

pub fn update(
    state: &mut DiscussionResumeState,
    msg: DiscussionResumeMessage,
) -> DiscussionResumeOutcome {
    match msg {
        DiscussionResumeMessage::MoveUp => {
            crate::screens::move_up(&mut state.selected, DiscussionResumeChoice::ALL.len());
            DiscussionResumeOutcome::Pending
        }
        DiscussionResumeMessage::MoveDown => {
            crate::screens::move_down(&mut state.selected, DiscussionResumeChoice::ALL.len());
            DiscussionResumeOutcome::Pending
        }
        DiscussionResumeMessage::Select => {
            state.visible = false;
            DiscussionResumeOutcome::Selected(state.selected_choice())
        }
        DiscussionResumeMessage::Cancel => {
            state.visible = false;
            DiscussionResumeOutcome::Selected(DiscussionResumeChoice::Dismiss)
        }
    }
}

pub fn render(state: &DiscussionResumeState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let inner = super::render_modal_frame(
        frame,
        area,
        " Resume Discussion ",
        theme::color::ACTIVE,
        72,
        12,
    );

    let layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let summary = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                state.pending.proposal_label.clone(),
                Style::default()
                    .fg(theme::color::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - "),
            Span::styled(
                state.pending.proposal_title.clone(),
                Style::default().fg(theme::color::TEXT_SECONDARY),
            ),
        ]),
        Line::from("Unfinished discussion detected for this agent session."),
    ]);
    frame.render_widget(summary, layout[0]);

    let next_question = state
        .pending
        .next_question
        .as_deref()
        .filter(|question| !question.trim().is_empty())
        .unwrap_or("No next question recorded.");
    let question = Paragraph::new(vec![
        Line::from(Span::styled("Next question", theme::style::header())),
        Line::from(next_question),
    ]);
    frame.render_widget(question, layout[1]);

    let items: Vec<ListItem> = DiscussionResumeChoice::ALL
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let style = if index == state.selected {
                Style::default()
                    .fg(theme::color::TEXT_PRIMARY)
                    .bg(theme::color::AGENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::TEXT_PRIMARY)
            };
            let prefix = if index == state.selected {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{}", choice.label()),
                style,
            )))
        })
        .collect();
    frame.render_widget(List::new(items), layout[2]);

    let footer = Paragraph::new("↑↓ select  Enter apply  Esc dismiss");
    frame.render_widget(footer, layout[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_pending() -> PendingDiscussionResume {
        PendingDiscussionResume {
            proposal_label: "Proposal A".to_string(),
            proposal_title: "Hook-driven resume".to_string(),
            next_question: Some("Should SessionStart surface the proposal?".to_string()),
        }
    }

    #[test]
    fn cancel_maps_to_dismiss_choice() {
        let mut state = DiscussionResumeState::with_pending("session-1", sample_pending());
        let outcome = update(&mut state, DiscussionResumeMessage::Cancel);
        assert_eq!(
            outcome,
            DiscussionResumeOutcome::Selected(DiscussionResumeChoice::Dismiss)
        );
        assert!(!state.visible);
    }

    #[test]
    fn move_down_then_select_returns_park_choice() {
        let mut state = DiscussionResumeState::with_pending("session-1", sample_pending());
        assert_eq!(
            update(&mut state, DiscussionResumeMessage::MoveDown),
            DiscussionResumeOutcome::Pending
        );
        let outcome = update(&mut state, DiscussionResumeMessage::Select);
        assert_eq!(
            outcome,
            DiscussionResumeOutcome::Selected(DiscussionResumeChoice::Park)
        );
    }

    #[test]
    fn render_visible_includes_resume_copy() {
        let state = DiscussionResumeState::with_pending("session-1", sample_pending());
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(&state, frame, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains("Resume Discussion"));
        assert!(text.contains("Resume discussion"));
        assert!(text.contains("Park proposal"));
        assert!(text.contains("Dismiss for now"));
    }
}
