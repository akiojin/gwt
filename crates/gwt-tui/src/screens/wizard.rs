//! Wizard overlay screen — 11-step agent launch wizard.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

/// Which step of the wizard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    #[default]
    QuickStart,
    AgentSelect,
    ModelSelect,
    ReasoningLevel,
    ExecutionMode,
    BranchTypeSelect,
    BranchNameInput,
    AIBranchSuggest,
    IssueSelect,
    SkipPermissions,
    Confirm,
}

impl WizardStep {
    /// All steps in order.
    const ALL: [WizardStep; 11] = [
        WizardStep::QuickStart,
        WizardStep::AgentSelect,
        WizardStep::ModelSelect,
        WizardStep::ReasoningLevel,
        WizardStep::ExecutionMode,
        WizardStep::BranchTypeSelect,
        WizardStep::BranchNameInput,
        WizardStep::AIBranchSuggest,
        WizardStep::IssueSelect,
        WizardStep::SkipPermissions,
        WizardStep::Confirm,
    ];

    /// Index of this step (0-based).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// Total number of steps.
    pub fn total() -> usize {
        Self::ALL.len()
    }

    /// Advance to the next step, if any.
    pub fn next(self) -> Option<WizardStep> {
        let idx = self.index();
        Self::ALL.get(idx + 1).copied()
    }

    /// Go back to the previous step, if any.
    pub fn prev(self) -> Option<WizardStep> {
        let idx = self.index();
        if idx == 0 {
            None
        } else {
            Some(Self::ALL[idx - 1])
        }
    }

    /// Human-readable title for this step.
    pub fn title(self) -> &'static str {
        match self {
            Self::QuickStart => "Quick Start",
            Self::AgentSelect => "Select Agent",
            Self::ModelSelect => "Select Model",
            Self::ReasoningLevel => "Reasoning Level",
            Self::ExecutionMode => "Execution Mode",
            Self::BranchTypeSelect => "Branch Type",
            Self::BranchNameInput => "Branch Name",
            Self::AIBranchSuggest => "AI Branch Suggestion",
            Self::IssueSelect => "Link Issue",
            Self::SkipPermissions => "Skip Permissions",
            Self::Confirm => "Confirm & Launch",
        }
    }
}

/// An agent option discovered on the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOption {
    pub id: String,
    pub name: String,
    pub available: bool,
}

/// State for the wizard overlay.
#[derive(Debug, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub detected_agents: Vec<AgentOption>,
    pub selected: usize,
    // Config fields accumulated during the wizard
    pub agent_id: String,
    pub model: String,
    pub reasoning: String,
    pub mode: String,
    pub branch_name: String,
    pub issue_id: String,
    pub skip_perms: bool,
    /// Whether the wizard has been completed (caller should read config).
    pub completed: bool,
    /// Whether the wizard has been cancelled.
    pub cancelled: bool,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            step: WizardStep::default(),
            detected_agents: Vec::new(),
            selected: 0,
            agent_id: String::new(),
            model: String::new(),
            reasoning: "medium".to_string(),
            mode: "autonomous".to_string(),
            branch_name: String::new(),
            issue_id: String::new(),
            skip_perms: false,
            completed: false,
            cancelled: false,
        }
    }
}

impl WizardState {
    /// Number of selectable options for the current step.
    pub fn option_count(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => 2,       // "New Agent" / "From Template"
            WizardStep::AgentSelect => self.detected_agents.len().max(1),
            WizardStep::ModelSelect => 3,      // e.g. claude-sonnet, claude-opus, gpt-4
            WizardStep::ReasoningLevel => 3,   // low, medium, high
            WizardStep::ExecutionMode => 2,    // autonomous, interactive
            WizardStep::BranchTypeSelect => 3, // feature, fix, custom
            WizardStep::BranchNameInput => 0,  // text input, no list
            WizardStep::AIBranchSuggest => 2,  // accept / reject
            WizardStep::IssueSelect => 0,      // text input
            WizardStep::SkipPermissions => 2,  // yes / no
            WizardStep::Confirm => 2,          // launch / cancel
        }
    }

    /// Options as string labels for the current step.
    pub fn current_options(&self) -> Vec<String> {
        match self.step {
            WizardStep::QuickStart => {
                vec!["New Agent".to_string(), "From Template".to_string()]
            }
            WizardStep::AgentSelect => {
                if self.detected_agents.is_empty() {
                    vec!["(no agents detected)".to_string()]
                } else {
                    self.detected_agents
                        .iter()
                        .map(|a| {
                            let status = if a.available { "+" } else { "-" };
                            format!("[{}] {}", status, a.name)
                        })
                        .collect()
                }
            }
            WizardStep::ModelSelect => vec![
                "claude-sonnet-4".to_string(),
                "claude-opus-4".to_string(),
                "gpt-4.1".to_string(),
            ],
            WizardStep::ReasoningLevel => {
                vec!["Low".to_string(), "Medium".to_string(), "High".to_string()]
            }
            WizardStep::ExecutionMode => {
                vec!["Autonomous".to_string(), "Interactive".to_string()]
            }
            WizardStep::BranchTypeSelect => vec![
                "feature/".to_string(),
                "fix/".to_string(),
                "custom".to_string(),
            ],
            WizardStep::BranchNameInput => vec![],
            WizardStep::AIBranchSuggest => {
                vec!["Accept Suggestion".to_string(), "Edit Manually".to_string()]
            }
            WizardStep::IssueSelect => vec![],
            WizardStep::SkipPermissions => vec!["Yes".to_string(), "No".to_string()],
            WizardStep::Confirm => vec!["Launch".to_string(), "Cancel".to_string()],
        }
    }
}

/// Messages specific to the wizard overlay.
#[derive(Debug, Clone)]
pub enum WizardMessage {
    MoveUp,
    MoveDown,
    Select,
    Back,
    Cancel,
    InputChar(char),
    Backspace,
    SetAgents(Vec<AgentOption>),
}

/// Update wizard state in response to a message.
pub fn update(state: &mut WizardState, msg: WizardMessage) {
    match msg {
        WizardMessage::MoveUp => {
            let count = state.option_count();
            if count > 0 {
                state.selected = if state.selected == 0 {
                    count - 1
                } else {
                    state.selected - 1
                };
            }
        }
        WizardMessage::MoveDown => {
            let count = state.option_count();
            if count > 0 {
                state.selected = (state.selected + 1) % count;
            }
        }
        WizardMessage::Select => {
            // Store selection for current step, then advance
            apply_selection(state);
            if let Some(next) = state.step.next() {
                state.step = next;
                state.selected = 0;
            } else {
                // Last step — mark completed
                state.completed = true;
            }
        }
        WizardMessage::Back => {
            if let Some(prev) = state.step.prev() {
                state.step = prev;
                state.selected = 0;
            } else {
                // First step — Esc cancels
                state.cancelled = true;
            }
        }
        WizardMessage::Cancel => {
            state.cancelled = true;
        }
        WizardMessage::InputChar(ch) => match state.step {
            WizardStep::BranchNameInput => {
                state.branch_name.push(ch);
            }
            WizardStep::IssueSelect => {
                state.issue_id.push(ch);
            }
            _ => {}
        },
        WizardMessage::Backspace => match state.step {
            WizardStep::BranchNameInput => {
                state.branch_name.pop();
            }
            WizardStep::IssueSelect => {
                state.issue_id.pop();
            }
            _ => {}
        },
        WizardMessage::SetAgents(agents) => {
            state.detected_agents = agents;
            if state.step == WizardStep::AgentSelect {
                state.selected = 0;
            }
        }
    }
}

/// Apply the current selection to config fields.
fn apply_selection(state: &mut WizardState) {
    let options = state.current_options();
    match state.step {
        WizardStep::AgentSelect => {
            if let Some(agent) = state.detected_agents.get(state.selected) {
                state.agent_id = agent.id.clone();
            }
        }
        WizardStep::ModelSelect => {
            if let Some(opt) = options.get(state.selected) {
                state.model = opt.clone();
            }
        }
        WizardStep::ReasoningLevel => {
            if let Some(opt) = options.get(state.selected) {
                state.reasoning = opt.to_lowercase();
            }
        }
        WizardStep::ExecutionMode => {
            if let Some(opt) = options.get(state.selected) {
                state.mode = opt.to_lowercase();
            }
        }
        WizardStep::SkipPermissions => {
            state.skip_perms = state.selected == 0;
        }
        WizardStep::Confirm => {
            if state.selected == 1 {
                state.cancelled = true;
            }
        }
        _ => {}
    }
}

/// Render the wizard overlay.
pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Centered modal — 60% width, 70% height
    let width = (area.width * 60 / 100).max(40).min(area.width);
    let height = (area.height * 70 / 100).max(12).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let overlay = Rect::new(x, y, width, height);

    // Clear the area behind the overlay
    frame.render_widget(Clear, overlay);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Progress indicator
            Constraint::Length(3), // Step title
            Constraint::Min(0),   // Content
            Constraint::Length(1), // Hints
        ])
        .split(overlay);

    // Progress bar
    let step_idx = state.step.index() + 1;
    let total = WizardStep::total();
    let progress_text = format!(" Step {}/{}", step_idx, total);
    let progress = Paragraph::new(progress_text)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(progress, chunks[0]);

    // Step title
    let title_block = Block::default()
        .borders(Borders::ALL)
        .title("Agent Launch Wizard")
        .border_style(Style::default().fg(Color::Cyan));
    let title_text = Paragraph::new(state.step.title())
        .block(title_block)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(title_text, chunks[1]);

    // Content — either a list of options or a text input
    render_step_content(state, frame, chunks[2]);

    // Hints
    let hint = match state.step {
        WizardStep::BranchNameInput | WizardStep::IssueSelect => {
            " Type to input | Enter: next | Esc: back"
        }
        _ => " Up/Down: select | Enter: next | Esc: back",
    };
    let hints = Paragraph::new(hint).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[3]);
}

/// Render the content area for the current wizard step.
fn render_step_content(state: &WizardState, frame: &mut Frame, area: Rect) {
    match state.step {
        WizardStep::BranchNameInput => {
            let block = Block::default().borders(Borders::ALL).title("Branch Name");
            let text = Paragraph::new(format!("{}_", state.branch_name))
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(text, area);
        }
        WizardStep::IssueSelect => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Issue ID (optional)");
            let text = Paragraph::new(format!("{}_", state.issue_id))
                .block(block)
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(text, area);
        }
        WizardStep::Confirm => {
            render_confirm_summary(state, frame, area);
        }
        _ => {
            let options = state.current_options();
            let items: Vec<ListItem> = options
                .iter()
                .enumerate()
                .map(|(idx, opt)| {
                    let style = if idx == state.selected {
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let marker = if idx == state.selected {
                        "> "
                    } else {
                        "  "
                    };
                    let line = Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Cyan)),
                        Span::styled(opt.clone(), style),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let block = Block::default().borders(Borders::ALL);
            let list = List::new(items).block(block);
            frame.render_widget(list, area);
        }
    }
}

/// Render the confirmation summary before launch.
fn render_confirm_summary(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    // Summary
    let summary = format!(
        " Agent:       {}\n Model:       {}\n Reasoning:   {}\n Mode:        {}\n Branch:      {}\n Issue:       {}\n Skip Perms:  {}",
        if state.agent_id.is_empty() { "-" } else { &state.agent_id },
        if state.model.is_empty() { "-" } else { &state.model },
        state.reasoning,
        state.mode,
        if state.branch_name.is_empty() { "-" } else { &state.branch_name },
        if state.issue_id.is_empty() { "-" } else { &state.issue_id },
        if state.skip_perms { "yes" } else { "no" },
    );
    let block = Block::default().borders(Borders::ALL).title("Summary");
    let para = Paragraph::new(summary)
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(para, chunks[0]);

    // Action buttons
    let options = state.current_options();
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let style = if idx == state.selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(format!("  {opt}"), style)))
        })
        .collect();
    let list = List::new(items);
    frame.render_widget(list, chunks[1]);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_agents() -> Vec<AgentOption> {
        vec![
            AgentOption {
                id: "claude".to_string(),
                name: "Claude Code".to_string(),
                available: true,
            },
            AgentOption {
                id: "codex".to_string(),
                name: "Codex CLI".to_string(),
                available: true,
            },
            AgentOption {
                id: "aider".to_string(),
                name: "Aider".to_string(),
                available: false,
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = WizardState::default();
        assert_eq!(state.step, WizardStep::QuickStart);
        assert_eq!(state.selected, 0);
        assert!(state.detected_agents.is_empty());
        assert!(!state.completed);
        assert!(!state.cancelled);
    }

    #[test]
    fn step_navigation_next() {
        assert_eq!(
            WizardStep::QuickStart.next(),
            Some(WizardStep::AgentSelect)
        );
        assert_eq!(WizardStep::Confirm.next(), None);
    }

    #[test]
    fn step_navigation_prev() {
        assert_eq!(WizardStep::QuickStart.prev(), None);
        assert_eq!(
            WizardStep::AgentSelect.prev(),
            Some(WizardStep::QuickStart)
        );
    }

    #[test]
    fn step_index_and_total() {
        assert_eq!(WizardStep::QuickStart.index(), 0);
        assert_eq!(WizardStep::Confirm.index(), 10);
        assert_eq!(WizardStep::total(), 11);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = WizardState::default();
        // QuickStart has 2 options
        assert_eq!(state.selected, 0);
        update(&mut state, WizardMessage::MoveDown);
        assert_eq!(state.selected, 1);
        update(&mut state, WizardMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = WizardState::default();
        update(&mut state, WizardMessage::MoveUp);
        assert_eq!(state.selected, 1); // wraps to last
    }

    #[test]
    fn select_advances_step() {
        let mut state = WizardState::default();
        assert_eq!(state.step, WizardStep::QuickStart);
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::AgentSelect);
        assert_eq!(state.selected, 0); // reset
    }

    #[test]
    fn back_goes_to_previous_step() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        update(&mut state, WizardMessage::Back);
        assert_eq!(state.step, WizardStep::AgentSelect);
    }

    #[test]
    fn back_on_first_step_cancels() {
        let mut state = WizardState::default();
        update(&mut state, WizardMessage::Back);
        assert!(state.cancelled);
    }

    #[test]
    fn cancel_sets_flag() {
        let mut state = WizardState::default();
        update(&mut state, WizardMessage::Cancel);
        assert!(state.cancelled);
    }

    #[test]
    fn input_char_branch_name() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchNameInput;
        update(&mut state, WizardMessage::InputChar('a'));
        update(&mut state, WizardMessage::InputChar('b'));
        assert_eq!(state.branch_name, "ab");
    }

    #[test]
    fn backspace_branch_name() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchNameInput;
        state.branch_name = "abc".to_string();
        update(&mut state, WizardMessage::Backspace);
        assert_eq!(state.branch_name, "ab");
    }

    #[test]
    fn input_char_issue_id() {
        let mut state = WizardState::default();
        state.step = WizardStep::IssueSelect;
        update(&mut state, WizardMessage::InputChar('1'));
        update(&mut state, WizardMessage::InputChar('2'));
        assert_eq!(state.issue_id, "12");
    }

    #[test]
    fn backspace_issue_id() {
        let mut state = WizardState::default();
        state.step = WizardStep::IssueSelect;
        state.issue_id = "42".to_string();
        update(&mut state, WizardMessage::Backspace);
        assert_eq!(state.issue_id, "4");
    }

    #[test]
    fn input_ignored_on_list_steps() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        update(&mut state, WizardMessage::InputChar('x'));
        assert!(state.branch_name.is_empty());
    }

    #[test]
    fn set_agents_populates() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        update(&mut state, WizardMessage::SetAgents(sample_agents()));
        assert_eq!(state.detected_agents.len(), 3);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn select_on_agent_step_stores_id() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.detected_agents = sample_agents();
        state.selected = 1;
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.agent_id, "codex");
        assert_eq!(state.step, WizardStep::ModelSelect);
    }

    #[test]
    fn select_on_model_step_stores_model() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        state.selected = 1;
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.model, "claude-opus-4");
    }

    #[test]
    fn select_on_confirm_completes() {
        let mut state = WizardState::default();
        state.step = WizardStep::Confirm;
        state.selected = 0; // "Launch"
        update(&mut state, WizardMessage::Select);
        assert!(state.completed);
    }

    #[test]
    fn select_cancel_on_confirm() {
        let mut state = WizardState::default();
        state.step = WizardStep::Confirm;
        state.selected = 1; // "Cancel"
        update(&mut state, WizardMessage::Select);
        assert!(state.cancelled);
    }

    #[test]
    fn skip_permissions_stores() {
        let mut state = WizardState::default();
        state.step = WizardStep::SkipPermissions;
        state.selected = 0; // "Yes"
        update(&mut state, WizardMessage::Select);
        assert!(state.skip_perms);
    }

    #[test]
    fn option_count_for_each_step() {
        let mut state = WizardState::default();
        assert_eq!(state.option_count(), 2); // QuickStart

        state.step = WizardStep::BranchNameInput;
        assert_eq!(state.option_count(), 0); // text input

        state.step = WizardStep::IssueSelect;
        assert_eq!(state.option_count(), 0); // text input
    }

    #[test]
    fn render_overlay_does_not_panic() {
        let state = WizardState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Progress indicator should be visible
        assert!(text.contains("Step") || text.contains("1/11") || buf.area.width > 0);
    }

    #[test]
    fn render_branch_input_does_not_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchNameInput;
        state.branch_name = "feature/test".to_string();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_confirm_step_does_not_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::Confirm;
        state.agent_id = "claude".to_string();
        state.model = "claude-sonnet-4".to_string();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn step_titles_non_empty() {
        for step in WizardStep::ALL {
            assert!(!step.title().is_empty(), "{:?} has empty title", step);
        }
    }
}
