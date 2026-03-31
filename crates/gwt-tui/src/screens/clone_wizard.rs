//! Clone Wizard Screen
//!
//! Provides a step-based wizard for cloning repositories as bare repositories.
//! Steps: URL input -> clone type selection -> cloning -> done/failed.

#![allow(dead_code)]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use std::path::PathBuf;

/// Clone wizard step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneStep {
    /// URL input step
    UrlInput,
    /// Clone type selection
    TypeSelect,
    /// Cloning in progress
    Cloning,
    /// Clone completed
    Done,
    /// Clone failed
    Failed,
}

/// Clone type selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneType {
    /// Bare clone (recommended for gwt workflow)
    Bare,
    /// Bare with shallow clone (--depth=1)
    BareShallow,
}

impl CloneType {
    fn label(&self) -> &'static str {
        match self {
            CloneType::Bare => "Bare clone (recommended)",
            CloneType::BareShallow => "Bare shallow clone (--depth=1, faster)",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            CloneType::Bare => "Full history, better for long-term projects",
            CloneType::BareShallow => "Faster clone, good for quick setup",
        }
    }
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
    /// Selected clone type
    pub clone_type: CloneType,
    /// Clone type selection index
    pub type_index: usize,
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
            clone_type: CloneType::BareShallow,
            type_index: 1,
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
        match self.step {
            CloneStep::UrlInput => {
                if !self.url_input.is_empty() {
                    self.step = CloneStep::TypeSelect;
                }
            }
            CloneStep::TypeSelect => {
                self.step = CloneStep::Cloning;
                self.clone_progress = Some("Cloning repository...".to_string());
            }
            _ => {}
        }
    }

    /// Move to previous step
    pub fn prev(&mut self) {
        match self.step {
            CloneStep::TypeSelect => {
                self.step = CloneStep::UrlInput;
            }
            CloneStep::Failed => {
                self.step = CloneStep::UrlInput;
                self.error = None;
            }
            _ => {}
        }
    }

    /// Move selection up in type select
    pub fn up(&mut self) {
        if self.step == CloneStep::TypeSelect && self.type_index > 0 {
            self.type_index -= 1;
            self.clone_type = CloneType::Bare;
        }
    }

    /// Move selection down in type select
    pub fn down(&mut self) {
        if self.step == CloneStep::TypeSelect && self.type_index < 1 {
            self.type_index += 1;
            self.clone_type = CloneType::BareShallow;
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

/// Render the clone wizard
pub fn render_clone_wizard(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let block = Block::default()
        .title(" Clone Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    block.render(area, buf);

    match state.step {
        CloneStep::UrlInput => render_url_input(state, buf, inner),
        CloneStep::TypeSelect => render_type_select(state, buf, inner),
        CloneStep::Cloning => render_cloning(state, buf, inner),
        CloneStep::Done => render_complete(state, buf, inner),
        CloneStep::Failed => render_failed(state, buf, inner),
    }
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

    let title = Paragraph::new("Enter repository URL to clone as bare repository")
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

    let help = Paragraph::new("[Enter] Continue  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    help.render(chunks[4], buf);
}

fn render_type_select(state: &CloneWizardState, buf: &mut Buffer, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new("Select clone type").style(Style::default().fg(Color::White));
    title.render(chunks[0], buf);

    let items: Vec<ListItem> = [CloneType::Bare, CloneType::BareShallow]
        .iter()
        .enumerate()
        .map(|(i, ct)| {
            let prefix = if i == state.type_index { "> " } else { "  " };
            let style = if i == state.type_index {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::raw(prefix),
                Span::styled(ct.label(), style),
            ]))
        })
        .collect();

    let list = List::new(items);
    ratatui::prelude::Widget::render(list, chunks[2], buf);

    let desc =
        Paragraph::new(state.clone_type.description()).style(Style::default().fg(Color::DarkGray));
    desc.render(chunks[3], buf);

    let help = Paragraph::new("[Up/Down] Select  [Enter] Clone  [Backspace] Back  [Esc] Cancel")
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

    let help = Paragraph::new("[Enter] Continue to create worktree")
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
        assert_eq!(state.step, CloneStep::TypeSelect);
    }

    #[test]
    fn test_clone_wizard_empty_url_stays() {
        let mut state = CloneWizardState::new();
        state.next(); // Empty URL should not advance
        assert_eq!(state.step, CloneStep::UrlInput);
    }

    #[test]
    fn test_clone_wizard_prev_step() {
        let mut state = CloneWizardState::new();
        state.step = CloneStep::TypeSelect;
        state.prev();
        assert_eq!(state.step, CloneStep::UrlInput);
    }

    #[test]
    fn test_clone_wizard_type_select() {
        let mut state = CloneWizardState::new();
        state.step = CloneStep::TypeSelect;
        state.type_index = 1;
        state.up();
        assert_eq!(state.type_index, 0);
        assert_eq!(state.clone_type, CloneType::Bare);
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
}
