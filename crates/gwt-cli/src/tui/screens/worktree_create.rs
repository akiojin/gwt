//! Worktree Create Wizard Screen

#![allow(dead_code)] // Screen components for future use

use ratatui::{prelude::*, widgets::*};

/// Worktree creation wizard steps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeCreateStep {
    BranchName,
    BaseBranch,
    Confirm,
}

/// Worktree creation state
#[derive(Debug)]
pub struct WorktreeCreateState {
    pub step: WorktreeCreateStep,
    pub branch_name: String,
    pub branch_name_cursor: usize,
    pub base_branches: Vec<String>,
    pub selected_base: usize,
    pub create_new_branch: bool,
    pub error_message: Option<String>,
}

impl Default for WorktreeCreateState {
    fn default() -> Self {
        Self {
            step: WorktreeCreateStep::BranchName,
            branch_name: String::new(),
            branch_name_cursor: 0,
            base_branches: vec!["main".to_string(), "develop".to_string()],
            selected_base: 0,
            create_new_branch: true,
            error_message: None,
        }
    }
}

impl WorktreeCreateState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_branches(mut self, branches: Vec<String>) -> Self {
        self.base_branches = branches;
        self
    }

    /// Move to next step
    pub fn next_step(&mut self) -> bool {
        match self.step {
            WorktreeCreateStep::BranchName => {
                if self.branch_name.is_empty() {
                    self.error_message = Some("Branch name is required".to_string());
                    return false;
                }
                self.error_message = None;
                self.step = WorktreeCreateStep::BaseBranch;
                true
            }
            WorktreeCreateStep::BaseBranch => {
                self.step = WorktreeCreateStep::Confirm;
                true
            }
            WorktreeCreateStep::Confirm => {
                // Ready to execute
                true
            }
        }
    }

    /// Move to previous step
    pub fn prev_step(&mut self) -> bool {
        match self.step {
            WorktreeCreateStep::BranchName => false,
            WorktreeCreateStep::BaseBranch => {
                self.step = WorktreeCreateStep::BranchName;
                true
            }
            WorktreeCreateStep::Confirm => {
                self.step = WorktreeCreateStep::BaseBranch;
                true
            }
        }
    }

    /// Insert character at cursor
    pub fn insert_char(&mut self, c: char) {
        if self.step == WorktreeCreateStep::BranchName {
            self.branch_name.insert(self.branch_name_cursor, c);
            self.branch_name_cursor += 1;
            self.error_message = None;
        }
    }

    /// Delete character before cursor
    pub fn delete_char(&mut self) {
        if self.step == WorktreeCreateStep::BranchName && self.branch_name_cursor > 0 {
            self.branch_name_cursor -= 1;
            self.branch_name.remove(self.branch_name_cursor);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.branch_name_cursor > 0 {
            self.branch_name_cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.branch_name_cursor < self.branch_name.len() {
            self.branch_name_cursor += 1;
        }
    }

    /// Select previous base branch
    pub fn select_prev_base(&mut self) {
        if self.step == WorktreeCreateStep::BaseBranch && self.selected_base > 0 {
            self.selected_base -= 1;
        }
    }

    /// Select next base branch
    pub fn select_next_base(&mut self) {
        if self.step == WorktreeCreateStep::BaseBranch
            && self.selected_base < self.base_branches.len() - 1
        {
            self.selected_base += 1;
        }
    }

    /// Get selected base branch
    pub fn selected_base_branch(&self) -> Option<&str> {
        self.base_branches
            .get(self.selected_base)
            .map(|s| s.as_str())
    }

    /// Is confirmation step?
    pub fn is_confirm_step(&self) -> bool {
        self.step == WorktreeCreateStep::Confirm
    }
}

/// Render worktree create wizard
pub fn render_worktree_create(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Progress
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Instructions
        ])
        .split(area);

    // Progress indicator
    render_progress(state, frame, chunks[0]);

    // Content based on step
    match state.step {
        WorktreeCreateStep::BranchName => {
            render_branch_name_step(state, frame, chunks[1]);
        }
        WorktreeCreateStep::BaseBranch => {
            render_base_branch_step(state, frame, chunks[1]);
        }
        WorktreeCreateStep::Confirm => {
            render_confirm_step(state, frame, chunks[1]);
        }
    }

    // Instructions
    render_instructions(state, frame, chunks[2]);
}

fn render_progress(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let steps = ["1. Branch Name", "2. Base Branch", "3. Confirm"];
    let current_step = match state.step {
        WorktreeCreateStep::BranchName => 0,
        WorktreeCreateStep::BaseBranch => 1,
        WorktreeCreateStep::Confirm => 2,
    };

    let spans: Vec<Span> = steps
        .iter()
        .enumerate()
        .flat_map(|(i, step)| {
            let style = if i == current_step {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if i < current_step {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            vec![
                Span::styled(step.to_string(), style),
                Span::raw(if i < steps.len() - 1 { " > " } else { "" }),
            ]
        })
        .collect();

    let progress = Paragraph::new(Line::from(spans))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Create Worktree "),
        );
    frame.render_widget(progress, area);
}

fn render_branch_name_step(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Input
            Constraint::Length(2), // Error
            Constraint::Min(0),    // Spacer
        ])
        .split(area);

    // Input field
    let input_style = if state.error_message.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };

    let display_text = if state.branch_name.is_empty() {
        "feature/my-new-feature"
    } else {
        &state.branch_name
    };

    let text_style = if state.branch_name.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let input = Paragraph::new(display_text).style(text_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(input_style)
            .title(" Branch Name "),
    );
    frame.render_widget(input, chunks[0]);

    // Cursor
    frame.set_cursor_position(Position::new(
        chunks[0].x + state.branch_name_cursor as u16 + 1,
        chunks[0].y + 1,
    ));

    // Error message
    if let Some(ref error) = state.error_message {
        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[1]);
    }
}

fn render_base_branch_step(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let items: Vec<ListItem> = state
        .base_branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let style = if i == state.selected_base {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(format!("  {}", branch)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Base Branch "),
    );
    frame.render_widget(list, area);
}

fn render_confirm_step(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let base = state.selected_base_branch().unwrap_or("main");
    let branch_line = format!("    Branch: {}", state.branch_name);
    let base_line = format!("    Base: {}", base);
    let new_branch_line = format!(
        "    Create new branch: {}",
        if state.create_new_branch { "Yes" } else { "No" }
    );

    let text = [
        "",
        "  Summary:",
        "",
        &branch_line,
        &base_line,
        &new_branch_line,
        "",
        "  Press Enter to create, or Esc to go back.",
    ];

    let paragraph = Paragraph::new(text.join("\n"))
        .block(Block::default().borders(Borders::ALL).title(" Confirm "));
    frame.render_widget(paragraph, area);
}

fn render_instructions(state: &WorktreeCreateState, frame: &mut Frame, area: Rect) {
    let instructions = match state.step {
        WorktreeCreateStep::BranchName => "[Enter] Next | [Esc] Cancel",
        WorktreeCreateStep::BaseBranch => "[Up/Down] Select | [Enter] Next | [Esc] Back",
        WorktreeCreateStep::Confirm => "[Enter] Create | [Esc] Back",
    };

    let paragraph =
        Paragraph::new(format!(" {} ", instructions)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_navigation() {
        let mut state = WorktreeCreateState::new();
        assert_eq!(state.step, WorktreeCreateStep::BranchName);

        // Can't proceed without branch name
        assert!(!state.next_step());
        assert!(state.error_message.is_some());

        // Set branch name
        state.branch_name = "feature/test".to_string();
        assert!(state.next_step());
        assert_eq!(state.step, WorktreeCreateStep::BaseBranch);

        // Go back
        assert!(state.prev_step());
        assert_eq!(state.step, WorktreeCreateStep::BranchName);

        // Forward to confirm
        state.next_step();
        state.next_step();
        assert_eq!(state.step, WorktreeCreateStep::Confirm);
    }

    #[test]
    fn test_text_input() {
        let mut state = WorktreeCreateState::new();

        state.insert_char('t');
        state.insert_char('e');
        state.insert_char('s');
        state.insert_char('t');

        assert_eq!(state.branch_name, "test");
        assert_eq!(state.branch_name_cursor, 4);

        state.delete_char();
        assert_eq!(state.branch_name, "tes");
        assert_eq!(state.branch_name_cursor, 3);
    }
}
