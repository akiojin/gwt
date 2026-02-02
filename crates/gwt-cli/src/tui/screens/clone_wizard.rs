//! Clone Wizard Screen (SPEC-a70a1ece US3)
//!
//! Provides a wizard for cloning repositories as bare repositories.

#![allow(dead_code)] // Methods will be used when polling is implemented

use gwt_core::git::{clone_bare, CloneConfig};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

/// Clone wizard step (SPEC-a70a1ece T305)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneWizardStep {
    /// URL input step
    UrlInput,
    /// Clone type selection (bare recommended)
    TypeSelect,
    /// Cloning in progress
    Cloning,
    /// Clone completed successfully
    Complete,
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

/// Clone result from background thread
pub enum CloneResult {
    Success(PathBuf),
    Error(String),
}

/// Clone wizard state (SPEC-a70a1ece T304)
#[derive(Debug)]
pub struct CloneWizardState {
    /// Current wizard step
    pub step: CloneWizardStep,
    /// Repository URL input
    pub url: String,
    /// Selected clone type
    pub clone_type: CloneType,
    /// Clone type selection index
    pub type_index: usize,
    /// Target directory for clone
    pub target_dir: PathBuf,
    /// Clone result receiver
    pub clone_rx: Option<Receiver<CloneResult>>,
    /// Cloned repository path on success
    pub cloned_path: Option<PathBuf>,
    /// Error message on failure
    pub error_message: Option<String>,
    /// Progress message during clone
    pub progress_message: String,
}

impl Default for CloneWizardState {
    fn default() -> Self {
        Self::new()
    }
}

impl CloneWizardState {
    pub fn new() -> Self {
        Self {
            step: CloneWizardStep::UrlInput,
            url: String::new(),
            clone_type: CloneType::BareShallow,
            type_index: 1, // Default to shallow
            target_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            clone_rx: None,
            cloned_path: None,
            error_message: None,
            progress_message: String::new(),
        }
    }

    /// Reset the wizard to initial state
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Handle character input for URL
    pub fn handle_char(&mut self, c: char) {
        if self.step == CloneWizardStep::UrlInput {
            self.url.push(c);
        }
    }

    /// Handle backspace for URL
    pub fn handle_backspace(&mut self) {
        if self.step == CloneWizardStep::UrlInput {
            self.url.pop();
        }
    }

    /// Move to next step or confirm selection
    pub fn next(&mut self) {
        match self.step {
            CloneWizardStep::UrlInput => {
                if !self.url.is_empty() {
                    self.step = CloneWizardStep::TypeSelect;
                }
            }
            CloneWizardStep::TypeSelect => {
                self.start_clone();
            }
            _ => {}
        }
    }

    /// Move to previous step
    pub fn prev(&mut self) {
        match self.step {
            CloneWizardStep::TypeSelect => {
                self.step = CloneWizardStep::UrlInput;
            }
            CloneWizardStep::Failed => {
                self.step = CloneWizardStep::UrlInput;
                self.error_message = None;
            }
            _ => {}
        }
    }

    /// Move selection up in type select
    pub fn up(&mut self) {
        if self.step == CloneWizardStep::TypeSelect && self.type_index > 0 {
            self.type_index -= 1;
            self.clone_type = match self.type_index {
                0 => CloneType::Bare,
                _ => CloneType::BareShallow,
            };
        }
    }

    /// Move selection down in type select
    pub fn down(&mut self) {
        if self.step == CloneWizardStep::TypeSelect && self.type_index < 1 {
            self.type_index += 1;
            self.clone_type = match self.type_index {
                0 => CloneType::Bare,
                _ => CloneType::BareShallow,
            };
        }
    }

    /// Start the clone operation in background
    fn start_clone(&mut self) {
        self.step = CloneWizardStep::Cloning;
        self.progress_message = "Cloning repository...".to_string();

        let (tx, rx): (Sender<CloneResult>, Receiver<CloneResult>) = channel();
        self.clone_rx = Some(rx);

        let url = self.url.clone();
        let target_dir = self.target_dir.clone();
        let clone_type = self.clone_type;

        thread::spawn(move || {
            let config = match clone_type {
                CloneType::Bare => CloneConfig::bare(&url, &target_dir),
                CloneType::BareShallow => CloneConfig::bare_shallow(&url, &target_dir, 1),
            };

            match clone_bare(&config) {
                Ok(path) => {
                    let _ = tx.send(CloneResult::Success(path));
                }
                Err(e) => {
                    let _ = tx.send(CloneResult::Error(e.to_string()));
                }
            }
        });
    }

    /// Poll for clone completion
    pub fn poll_clone(&mut self) -> bool {
        if let Some(ref rx) = self.clone_rx {
            match rx.try_recv() {
                Ok(CloneResult::Success(path)) => {
                    self.step = CloneWizardStep::Complete;
                    self.cloned_path = Some(path);
                    self.clone_rx = None;
                    return true;
                }
                Ok(CloneResult::Error(msg)) => {
                    self.step = CloneWizardStep::Failed;
                    self.error_message = Some(msg);
                    self.clone_rx = None;
                    return true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.step = CloneWizardStep::Failed;
                    self.error_message = Some("Clone process terminated unexpectedly".to_string());
                    self.clone_rx = None;
                    return true;
                }
            }
        }
        false
    }

    /// Check if clone is in progress
    pub fn is_cloning(&self) -> bool {
        self.step == CloneWizardStep::Cloning
    }

    /// Check if wizard is complete (success or user wants to exit)
    pub fn is_complete(&self) -> bool {
        self.step == CloneWizardStep::Complete
    }
}

/// Render the clone wizard (SPEC-a70a1ece T306-T308)
pub fn render_clone_wizard(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Clone Repository ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.step {
        CloneWizardStep::UrlInput => render_url_input(state, frame, inner),
        CloneWizardStep::TypeSelect => render_type_select(state, frame, inner),
        CloneWizardStep::Cloning => render_cloning(state, frame, inner),
        CloneWizardStep::Complete => render_complete(state, frame, inner),
        CloneWizardStep::Failed => render_failed(state, frame, inner),
    }
}

fn render_url_input(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
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

    // Title
    let title = Paragraph::new("Enter repository URL to clone as bare repository")
        .style(Style::default().fg(Color::White));
    frame.render_widget(title, chunks[0]);

    // URL input field
    let url_block = Block::default()
        .title(" URL ")
        .borders(Borders::ALL)
        .border_style(if state.url.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Green)
        });

    let url_text = if state.url.is_empty() {
        "https://github.com/user/repo.git"
    } else {
        &state.url
    };

    let url_paragraph = Paragraph::new(format!("{}|", url_text))
        .style(if state.url.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        })
        .block(url_block);

    frame.render_widget(url_paragraph, chunks[2]);

    // Help text
    let help = Paragraph::new("[Enter] Continue  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[4]);
}

fn render_type_select(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
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

    // Title
    let title = Paragraph::new("Select clone type").style(Style::default().fg(Color::White));
    frame.render_widget(title, chunks[0]);

    // Clone type options
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
    frame.render_widget(list, chunks[2]);

    // Description
    let desc =
        Paragraph::new(state.clone_type.description()).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(desc, chunks[3]);

    // Help text
    let help = Paragraph::new("[Up/Down] Select  [Enter] Clone  [Backspace] Back  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[4]);
}

fn render_cloning(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);

    // Progress message
    let progress =
        Paragraph::new(&*state.progress_message).style(Style::default().fg(Color::Yellow));
    frame.render_widget(progress, chunks[0]);

    // URL being cloned
    let url_info =
        Paragraph::new(format!("URL: {}", state.url)).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(url_info, chunks[1]);
}

fn render_complete(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    // Success message
    let success = Paragraph::new("Repository cloned successfully!").style(
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(success, chunks[0]);

    // Cloned path
    if let Some(ref path) = state.cloned_path {
        let path_info = Paragraph::new(format!("Location: {}", path.display()))
            .style(Style::default().fg(Color::White));
        frame.render_widget(path_info, chunks[1]);
    }

    // Help text
    let help = Paragraph::new("[Enter] Continue to create worktree")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);
}

fn render_failed(state: &CloneWizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    // Error title
    let title = Paragraph::new("Clone failed")
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
    frame.render_widget(title, chunks[0]);

    // Error message
    if let Some(ref msg) = state.error_message {
        let error = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Red))
            .wrap(ratatui::widgets::Wrap { trim: true });
        frame.render_widget(error, chunks[1]);
    }

    // Help text
    let help = Paragraph::new("[Backspace] Try again  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_wizard_initial_state() {
        let state = CloneWizardState::new();
        assert_eq!(state.step, CloneWizardStep::UrlInput);
        assert!(state.url.is_empty());
    }

    #[test]
    fn test_clone_wizard_url_input() {
        let mut state = CloneWizardState::new();
        state.handle_char('h');
        state.handle_char('t');
        state.handle_char('t');
        state.handle_char('p');
        assert_eq!(state.url, "http");
    }

    #[test]
    fn test_clone_wizard_backspace() {
        let mut state = CloneWizardState::new();
        state.handle_char('a');
        state.handle_char('b');
        state.handle_backspace();
        assert_eq!(state.url, "a");
    }

    #[test]
    fn test_clone_wizard_next_step() {
        let mut state = CloneWizardState::new();
        state.url = "https://github.com/user/repo".to_string();
        state.next();
        assert_eq!(state.step, CloneWizardStep::TypeSelect);
    }

    #[test]
    fn test_clone_wizard_prev_step() {
        let mut state = CloneWizardState::new();
        state.step = CloneWizardStep::TypeSelect;
        state.prev();
        assert_eq!(state.step, CloneWizardStep::UrlInput);
    }

    #[test]
    fn test_clone_wizard_type_select() {
        let mut state = CloneWizardState::new();
        state.step = CloneWizardStep::TypeSelect;
        state.type_index = 1;
        state.up();
        assert_eq!(state.type_index, 0);
        assert_eq!(state.clone_type, CloneType::Bare);
    }
}
