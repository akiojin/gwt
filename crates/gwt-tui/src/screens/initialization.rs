//! Initialization screen — clone wizard and bare migration guidance.
//!
//! Shown when gwt-tui is launched in a directory without a git repository
//! (clone wizard) or in a bare repository (migration instructions).

use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::theme;

const GWT_LOGO: [&str; 3] = [
    "\u{256D}\u{2500} gwt \u{2500}\u{256E}",
    "\u{2502} \u{25C6} \u{25C7} \u{25C6} \u{2502}",
    "\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256F}",
];

/// Clone operation status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CloneStatus {
    /// Waiting for user to enter a URL.
    #[default]
    Idle,
    /// Clone is in progress.
    Cloning,
    /// Clone completed successfully.
    Success(PathBuf),
    /// Clone failed with an error message.
    Error(String),
}

/// State for the initialization screen.
#[derive(Debug, Clone)]
pub struct InitializationState {
    /// URL input buffer.
    pub url_input: String,
    /// Current clone status.
    pub clone_status: CloneStatus,
    /// Whether this is a bare repo migration screen (no clone wizard).
    pub bare_migration: bool,
}

impl InitializationState {
    /// Create a new initialization state.
    pub fn new(bare_migration: bool) -> Self {
        Self {
            url_input: String::new(),
            clone_status: CloneStatus::Idle,
            bare_migration,
        }
    }
}

impl Default for InitializationState {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Messages for the initialization screen.
#[derive(Debug, Clone)]
pub enum InitializationMessage {
    /// A character was typed into the URL input.
    InputChar(char),
    /// Backspace pressed — delete last character.
    Backspace,
    /// Enter pressed — start clone.
    StartClone,
    /// Clone completed successfully.
    CloneSuccess(PathBuf),
    /// Clone failed.
    CloneError(String),
    /// Esc pressed — exit gwt-tui.
    Exit,
}

/// Update the initialization state.
pub fn update(state: &mut InitializationState, msg: InitializationMessage) {
    match msg {
        InitializationMessage::InputChar(ch) => {
            if state.clone_status == CloneStatus::Idle
                || matches!(state.clone_status, CloneStatus::Error(_))
            {
                state.url_input.push(ch);
                // Clear error when user starts typing again
                if matches!(state.clone_status, CloneStatus::Error(_)) {
                    state.clone_status = CloneStatus::Idle;
                }
            }
        }
        InitializationMessage::Backspace => {
            if state.clone_status == CloneStatus::Idle
                || matches!(state.clone_status, CloneStatus::Error(_))
            {
                state.url_input.pop();
                if matches!(state.clone_status, CloneStatus::Error(_)) {
                    state.clone_status = CloneStatus::Idle;
                }
            }
        }
        InitializationMessage::StartClone => {
            if !state.url_input.is_empty() {
                state.clone_status = CloneStatus::Cloning;
            }
        }
        InitializationMessage::CloneSuccess(path) => {
            state.clone_status = CloneStatus::Success(path);
        }
        InitializationMessage::CloneError(err) => {
            state.clone_status = CloneStatus::Error(err);
        }
        InitializationMessage::Exit => {
            // Handled by app.rs (sets model.quit)
        }
    }
}

/// Render the initialization screen (fullscreen).
pub fn render(state: &InitializationState, frame: &mut Frame, area: Rect) {
    if state.bare_migration {
        render_bare_migration(frame, area);
    } else {
        render_clone_wizard(state, frame, area);
    }
}

/// Render the clone wizard UI.
fn render_clone_wizard(state: &InitializationState, frame: &mut Frame, area: Rect) {
    // Center a dialog box
    let dialog = centered_rect(60, 17, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" gwt — Clone Repository ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(theme::color::FOCUS))
        .border_type(theme::border::modal());
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Logo + spacing
            Constraint::Length(2), // Instructions
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Label
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Spacer
            Constraint::Min(1),    // Status / help
        ])
        .split(inner);

    // Logo
    let logo_lines: Vec<Line> = GWT_LOGO
        .iter()
        .map(|line| Line::from(Span::styled(*line, theme::style::header())))
        .collect();
    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    frame.render_widget(logo, chunks[0]);

    // Instructions
    let instructions = Paragraph::new("Enter a repository URL to clone into this directory.")
        .style(Style::default().fg(theme::color::TEXT_PRIMARY))
        .alignment(Alignment::Center);
    frame.render_widget(instructions, chunks[1]);

    // Label
    let label = Paragraph::new("Repository URL:").style(theme::style::active_item());
    frame.render_widget(label, chunks[3]);

    // Input field
    let input_style = match &state.clone_status {
        CloneStatus::Error(_) => Style::default().fg(theme::color::ERROR),
        _ => Style::default().fg(theme::color::TEXT_PRIMARY),
    };
    let cursor = if state.clone_status == CloneStatus::Idle
        || matches!(state.clone_status, CloneStatus::Error(_))
    {
        theme::icon::BLOCK_CURSOR
    } else {
        ""
    };
    let input_text = format!("{}{}", state.url_input, cursor);
    let input = Paragraph::new(input_text)
        .style(input_style)
        .block(Block::default().borders(Borders::ALL).border_type(theme::border::default()));
    frame.render_widget(input, chunks[4]);

    // Status / help
    let status: Paragraph = match &state.clone_status {
        CloneStatus::Idle => {
            let help = Line::from(vec![
                Span::styled("Enter", theme::style::success_text()),
                Span::raw(" Clone  "),
                Span::styled("Esc", theme::style::error_text()),
                Span::raw(" Exit"),
            ]);
            Paragraph::new(help).alignment(Alignment::Center)
        }
        CloneStatus::Cloning => Paragraph::new("Cloning repository...")
            .style(Style::default().fg(theme::color::ACTIVE))
            .alignment(Alignment::Center),
        CloneStatus::Success(_) => Paragraph::new("Clone successful! Loading workspace...")
            .style(Style::default().fg(theme::color::SUCCESS))
            .alignment(Alignment::Center),
        CloneStatus::Error(err) => {
            let lines = vec![
                Line::from(Span::styled(
                    format!("Error: {err}"),
                    Style::default().fg(theme::color::ERROR),
                )),
                Line::from(vec![
                    Span::raw("Edit URL and press "),
                    Span::styled("Enter", theme::style::success_text()),
                    Span::raw(" to retry"),
                ]),
            ];
            Paragraph::new(lines).alignment(Alignment::Center)
        }
    };
    frame.render_widget(status, chunks[6]);
}

/// Render the bare repository migration instructions.
fn render_bare_migration(frame: &mut Frame, area: Rect) {
    let dialog = centered_rect(65, 19, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" gwt — Bare Repository Detected ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(theme::color::ERROR))
        .border_type(theme::border::modal());
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    let lines = vec![
        Line::from(Span::styled(GWT_LOGO[0], theme::style::header())),
        Line::from(Span::styled(GWT_LOGO[1], theme::style::header())),
        Line::from(Span::styled(GWT_LOGO[2], theme::style::header())),
        Line::from(""),
        Line::from(Span::styled(
            "This directory contains a bare Git repository.",
            theme::style::active_item(),
        )),
        Line::from(""),
        Line::from("gwt requires a normal (non-bare) clone to work properly."),
        Line::from("To migrate, re-clone your repository:"),
        Line::from(""),
        Line::from(Span::styled(
            "  1. Move to a new directory",
            Style::default().fg(theme::color::TEXT_PRIMARY),
        )),
        Line::from(Span::styled(
            "  2. git clone --depth=1 <url> .",
            theme::style::header(),
        )),
        Line::from(Span::styled(
            "  3. Launch gwt-tui in the new clone",
            Style::default().fg(theme::color::TEXT_PRIMARY),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Press "),
            Span::styled("Esc", theme::style::error_text()),
            Span::raw(" to exit"),
        ]),
    ];

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

/// Reuse the shared centered_rect utility.
use super::centered_rect;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_state_defaults() {
        let state = InitializationState::default();
        assert!(state.url_input.is_empty());
        assert_eq!(state.clone_status, CloneStatus::Idle);
        assert!(!state.bare_migration);
    }

    #[test]
    fn initialization_state_bare_migration() {
        let state = InitializationState::new(true);
        assert!(state.bare_migration);
    }

    #[test]
    fn update_input_char_appends() {
        let mut state = InitializationState::default();
        update(&mut state, InitializationMessage::InputChar('h'));
        update(&mut state, InitializationMessage::InputChar('i'));
        assert_eq!(state.url_input, "hi");
    }

    #[test]
    fn update_backspace_removes_last() {
        let mut state = InitializationState::default();
        update(&mut state, InitializationMessage::InputChar('a'));
        update(&mut state, InitializationMessage::InputChar('b'));
        update(&mut state, InitializationMessage::Backspace);
        assert_eq!(state.url_input, "a");
    }

    #[test]
    fn update_backspace_on_empty_is_noop() {
        let mut state = InitializationState::default();
        update(&mut state, InitializationMessage::Backspace);
        assert!(state.url_input.is_empty());
    }

    #[test]
    fn update_start_clone_sets_cloning() {
        let mut state = InitializationState {
            url_input: "https://example.com/repo.git".to_string(),
            ..Default::default()
        };
        update(&mut state, InitializationMessage::StartClone);
        assert_eq!(state.clone_status, CloneStatus::Cloning);
    }

    #[test]
    fn update_start_clone_empty_url_stays_idle() {
        let mut state = InitializationState::default();
        update(&mut state, InitializationMessage::StartClone);
        assert_eq!(state.clone_status, CloneStatus::Idle);
    }

    #[test]
    fn update_clone_success() {
        let mut state = InitializationState {
            clone_status: CloneStatus::Cloning,
            ..Default::default()
        };
        update(
            &mut state,
            InitializationMessage::CloneSuccess(PathBuf::from("/tmp/repo")),
        );
        assert_eq!(
            state.clone_status,
            CloneStatus::Success(PathBuf::from("/tmp/repo"))
        );
    }

    #[test]
    fn update_clone_error() {
        let mut state = InitializationState {
            clone_status: CloneStatus::Cloning,
            ..Default::default()
        };
        update(
            &mut state,
            InitializationMessage::CloneError("network error".to_string()),
        );
        assert!(matches!(state.clone_status, CloneStatus::Error(_)));
    }

    #[test]
    fn update_input_clears_error_state() {
        let mut state = InitializationState {
            clone_status: CloneStatus::Error("fail".to_string()),
            ..Default::default()
        };
        update(&mut state, InitializationMessage::InputChar('x'));
        assert_eq!(state.clone_status, CloneStatus::Idle);
        assert_eq!(state.url_input, "x");
    }

    #[test]
    fn update_ignores_input_while_cloning() {
        let mut state = InitializationState {
            url_input: "url".to_string(),
            clone_status: CloneStatus::Cloning,
            ..Default::default()
        };
        update(&mut state, InitializationMessage::InputChar('x'));
        assert_eq!(state.url_input, "url"); // unchanged
    }
}
