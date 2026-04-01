//! Clone Wizard Screen
//!
//! Provides a step-based wizard for cloning repositories.
//! Steps: URL input -> cloning -> done/failed.

#![allow(dead_code)]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::path::PathBuf;

/// Clone wizard step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneStep {
    /// URL input step
    UrlInput,
    /// Cloning in progress
    Cloning,
    /// Clone completed
    Done,
    /// Clone failed
    Failed,
}

/// Clone wizard state
#[derive(Debug)]
pub struct CloneWizardState {
    /// Current wizard step
    pub step: CloneStep,
    /// Repository URL input
    pub url_input: String,
    /// Target directory
    pub target_dir: String,
    /// Clone progress message
    pub clone_progress: Option<String>,
    /// Error message on failure
    pub error: Option<String>,
    /// Cloned repository path on success
    pub cloned_path: Option<PathBuf>,
}

impl Default for CloneWizardState {
    fn default() -> Self {
        Self::new()
    }
}

impl CloneWizardState {
    pub fn new() -> Self {
        Self {
            step: CloneStep::UrlInput,
            url_input: String::new(),
            target_dir: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            clone_progress: None,
            error: None,
            cloned_path: None,
        }
    }

    /// Reset the wizard to initial state
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Handle character input for URL
    pub fn handle_char(&mut self, c: char) {
        if self.step == CloneStep::UrlInput {
            self.url_input.push(c);
        }
    }

    /// Handle backspace for URL
    pub fn handle_backspace(&mut self) {
        if self.step == CloneStep::UrlInput {
            self.url_input.pop();
        }
    }

    /// Move to next step or confirm selection
    pub fn next(&mut self) {
        if self.step == CloneStep::UrlInput && !self.url_input.is_empty() {
            self.step = CloneStep::Cloning;
            self.clone_progress = Some("Cloning repository...".to_string());
        }
    }

    /// Move to previous step
    pub fn prev(&mut self) {
        if self.step == CloneStep::Failed {
            self.step = CloneStep::UrlInput;
            self.error = None;
        }
    }

    /// Check if clone is in progress
    pub fn is_cloning(&self) -> bool {
        self.step == CloneStep::Cloning
    }

    /// Check if wizard is complete
    pub fn is_complete(&self) -> bool {
        self.step == CloneStep::Done
    }
}

/// Render the clone wizard as an overlay
pub fn render_clone_wizard(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let block = Block::default()
        .title(" Clone Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    block.render(area, buf);

    match state.step {
        CloneStep::UrlInput => render_url_input(state, buf, inner),
        CloneStep::Cloning => render_cloning(state, buf, inner),
        CloneStep::Done => render_complete(state, buf, inner),
        CloneStep::Failed => render_failed(state, buf, inner),
    }
}

/// Render the clone wizard as a fullscreen initialization screen
pub fn render_fullscreen(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    // Clear background
    let clear = ratatui::widgets::Clear;
    clear.render(area, buf);

    // Center the content
    let v_padding = area.height.saturating_sub(14) / 2;
    let h_padding = area.width.saturating_sub(60) / 2;
    let content_area = Rect::new(
        area.x + h_padding,
        area.y + v_padding,
        area.width.saturating_sub(h_padding * 2).min(60),
        14.min(area.height),
    );

    match state.step {
        CloneStep::UrlInput => render_fullscreen_url_input(state, buf, content_area),
        CloneStep::Cloning => render_cloning(state, buf, content_area),
        CloneStep::Done => render_complete(state, buf, content_area),
        CloneStep::Failed => render_failed(state, buf, content_area),
    }
}

fn render_fullscreen_url_input(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new("Welcome to gwt")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);
    title.render(chunks[0], buf);

    let subtitle = Paragraph::new("Enter a repository URL to get started")
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);
    subtitle.render(chunks[2], buf);

    let url_block = Block::default()
        .title(" URL ")
        .borders(Borders::ALL)
        .border_style(if state.url_input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Green)
        });

    let url_text = if state.url_input.is_empty() {
        "https://github.com/user/repo.git"
    } else {
        &state.url_input
    };

    let url_paragraph = Paragraph::new(format!("{}|", url_text))
        .style(if state.url_input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        })
        .block(url_block);
    url_paragraph.render(chunks[4], buf);

    let help = Paragraph::new("[Enter] Clone  [Esc] Quit")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    help.render(chunks[6], buf);
}

fn render_url_input(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new("Enter repository URL to clone")
        .style(Style::default().fg(Color::White));
    title.render(chunks[0], buf);

    let url_block = Block::default()
        .title(" URL ")
        .borders(Borders::ALL)
        .border_style(if state.url_input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Green)
        });

    let url_text = if state.url_input.is_empty() {
        "https://github.com/user/repo.git"
    } else {
        &state.url_input
    };

    let url_paragraph = Paragraph::new(format!("{}|", url_text))
        .style(if state.url_input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        })
        .block(url_block);
    url_paragraph.render(chunks[2], buf);

    let help = Paragraph::new("[Enter] Clone  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    help.render(chunks[4], buf);
}

fn render_cloning(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);

    let progress_msg = state.clone_progress.as_deref().unwrap_or("Cloning...");
    let progress = Paragraph::new(progress_msg).style(Style::default().fg(Color::Yellow));
    progress.render(chunks[0], buf);

    let url_info = Paragraph::new(format!("URL: {}", state.url_input))
        .style(Style::default().fg(Color::DarkGray));
    url_info.render(chunks[1], buf);
}

fn render_complete(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let success = Paragraph::new("Repository cloned successfully!").style(
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    success.render(chunks[0], buf);

    if let Some(ref path) = state.cloned_path {
        let path_info = Paragraph::new(format!("Location: {}", path.display()))
            .style(Style::default().fg(Color::White));
        path_info.render(chunks[1], buf);
    }

    let help = Paragraph::new("[Enter] Continue")
        .style(Style::default().fg(Color::DarkGray));
    help.render(chunks[3], buf);
}

fn render_failed(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new("Clone failed")
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
    title.render(chunks[0], buf);

    if let Some(ref msg) = state.error {
        let error = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true });
        error.render(chunks[1], buf);
    }

    let help = Paragraph::new("[Backspace] Try again  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    help.render(chunks[3], buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_wizard_initial_state() {
        let state = CloneWizardState::new();
        assert_eq!(state.step, CloneStep::UrlInput);
        assert!(state.url_input.is_empty());
    }

    #[test]
    fn test_clone_wizard_url_input() {
        let mut state = CloneWizardState::new();
        state.handle_char('h');
        state.handle_char('t');
        state.handle_char('t');
        state.handle_char('p');
        assert_eq!(state.url_input, "http");
    }

    #[test]
    fn test_clone_wizard_backspace() {
        let mut state = CloneWizardState::new();
        state.handle_char('a');
        state.handle_char('b');
        state.handle_backspace();
        assert_eq!(state.url_input, "a");
    }

    #[test]
    fn test_clone_wizard_next_step() {
        let mut state = CloneWizardState::new();
        state.url_input = "https://github.com/user/repo".to_string();
        state.next();
        assert_eq!(state.step, CloneStep::Cloning);
    }

    #[test]
    fn test_clone_wizard_empty_url_stays() {
        let mut state = CloneWizardState::new();
        state.next(); // Empty URL should not advance
        assert_eq!(state.step, CloneStep::UrlInput);
    }

    #[test]
    fn test_clone_wizard_is_cloning() {
        let mut state = CloneWizardState::new();
        assert!(!state.is_cloning());
        state.step = CloneStep::Cloning;
        assert!(state.is_cloning());
    }

    #[test]
    fn test_clone_wizard_is_complete() {
        let mut state = CloneWizardState::new();
        assert!(!state.is_complete());
        state.step = CloneStep::Done;
        assert!(state.is_complete());
    }

    #[test]
    fn test_render_clone_wizard_no_panic() {
        let state = CloneWizardState::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_clone_wizard(&state, &mut buf, area);
    }

    #[test]
    fn test_render_fullscreen_no_panic() {
        let state = CloneWizardState::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_fullscreen(&state, &mut buf, area);
    }
}
