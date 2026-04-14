//! Confirmation dialog overlay.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::theme;

/// Which button is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmChoice {
    Yes,
    #[default]
    No,
}

/// State for the confirmation dialog.
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub message: String,
    pub selected: ConfirmChoice,
    pub visible: bool,
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self {
            message: String::new(),
            selected: ConfirmChoice::No,
            visible: false,
        }
    }
}

impl ConfirmState {
    /// Create a new visible confirm dialog with the given message.
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            selected: ConfirmChoice::No,
            visible: true,
        }
    }

    /// Whether the user accepted (Yes).
    pub fn accepted(&self) -> bool {
        self.selected == ConfirmChoice::Yes
    }
}

/// Messages specific to the confirmation dialog.
#[derive(Debug, Clone)]
pub enum ConfirmMessage {
    Toggle,
    Accept,
    Cancel,
}

/// Update confirm state in response to a message.
pub fn update(state: &mut ConfirmState, msg: ConfirmMessage) {
    match msg {
        ConfirmMessage::Toggle => {
            state.selected = match state.selected {
                ConfirmChoice::Yes => ConfirmChoice::No,
                ConfirmChoice::No => ConfirmChoice::Yes,
            };
        }
        ConfirmMessage::Accept => {
            state.visible = false;
        }
        ConfirmMessage::Cancel => {
            state.selected = ConfirmChoice::No;
            state.visible = false;
        }
    }
}

/// Render the confirmation dialog as a centered overlay.
pub fn render(state: &ConfirmState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let inner = super::render_modal_frame(frame, area, "Confirm", theme::color::ACTIVE, 40, 7);

    let yes_style = if state.selected == ConfirmChoice::Yes {
        Style::default()
            .fg(theme::color::TEXT_PRIMARY)
            .bg(theme::color::SUCCESS)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::color::SUCCESS)
    };

    let no_style = if state.selected == ConfirmChoice::No {
        Style::default()
            .fg(theme::color::TEXT_PRIMARY)
            .bg(theme::color::ERROR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::color::ERROR)
    };

    let text = vec![
        Line::from(Span::raw(&state.message)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [ Yes ]  ", yes_style),
            Span::raw("  "),
            Span::styled("  [ No ]  ", no_style),
        ]),
    ];

    let paragraph = Paragraph::new(text).style(Style::default().fg(theme::color::TEXT_PRIMARY));
    frame.render_widget(paragraph, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn default_state() {
        let state = ConfirmState::default();
        assert!(state.message.is_empty());
        assert_eq!(state.selected, ConfirmChoice::No);
        assert!(!state.visible);
    }

    #[test]
    fn with_message_creates_visible() {
        let state = ConfirmState::with_message("Delete this?");
        assert_eq!(state.message, "Delete this?");
        assert!(state.visible);
        assert_eq!(state.selected, ConfirmChoice::No);
    }

    #[test]
    fn toggle_switches_choice() {
        let mut state = ConfirmState::with_message("test");
        assert_eq!(state.selected, ConfirmChoice::No);

        update(&mut state, ConfirmMessage::Toggle);
        assert_eq!(state.selected, ConfirmChoice::Yes);

        update(&mut state, ConfirmMessage::Toggle);
        assert_eq!(state.selected, ConfirmChoice::No);
    }

    #[test]
    fn accept_hides_dialog() {
        let mut state = ConfirmState::with_message("test");
        state.selected = ConfirmChoice::Yes;

        update(&mut state, ConfirmMessage::Accept);
        assert!(!state.visible);
        assert!(state.accepted());
    }

    #[test]
    fn cancel_hides_and_resets_to_no() {
        let mut state = ConfirmState::with_message("test");
        state.selected = ConfirmChoice::Yes;

        update(&mut state, ConfirmMessage::Cancel);
        assert!(!state.visible);
        assert_eq!(state.selected, ConfirmChoice::No);
        assert!(!state.accepted());
    }

    #[test]
    fn render_visible_does_not_panic() {
        let state = ConfirmState::with_message("Are you sure?");
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Check that "Confirm" title is rendered somewhere
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(full_text.contains("Confirm"));
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = ConfirmState::default(); // visible = false
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        // Should not contain "Confirm" title since invisible
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(!full_text.contains("Confirm"));
    }
}
