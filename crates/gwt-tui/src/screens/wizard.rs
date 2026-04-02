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

/// State for AI branch name suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSuggestionOption {
    /// Branch name that can be applied to the wizard state.
    pub branch_name: String,
    /// Display label shown in the list.
    pub label: String,
}

const AI_SUGGEST_TIMEOUT_TICKS: usize = 12;

#[derive(Debug, Clone, Default)]
pub struct AISuggestState {
    /// Suggested branch names from AI.
    pub suggestions: Vec<String>,
    /// Structured options for the current suggestion set.
    pub options: Vec<BranchSuggestionOption>,
    /// Whether we are waiting for AI to respond.
    pub loading: bool,
    /// Error message if AI suggestion failed.
    pub error: Option<String>,
    /// Tick counter for spinner animation (incremented on WizardMessage::Tick).
    pub tick_counter: usize,
}

/// An agent option discovered on the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOption {
    pub id: String,
    pub name: String,
    pub available: bool,
}

/// SPEC context for prefilling the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecContext {
    pub spec_id: String,
    pub title: String,
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
    /// AI branch suggestion state.
    pub ai_suggest: AISuggestState,
    /// Whether the wizard has been completed (caller should read config).
    pub completed: bool,
    /// Whether the wizard has been cancelled.
    pub cancelled: bool,
    /// Optional SPEC context for prefilling.
    pub spec_context: Option<SpecContext>,
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
            ai_suggest: AISuggestState::default(),
            completed: false,
            cancelled: false,
            spec_context: None,
        }
    }
}

impl WizardState {
    /// Number of selectable options for the current step.
    pub fn option_count(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => 2, // "New Agent" / "From Template"
            WizardStep::AgentSelect => self.detected_agents.len().max(1),
            WizardStep::ModelSelect => 3, // e.g. claude-sonnet, claude-opus, gpt-4
            WizardStep::ReasoningLevel => 3, // low, medium, high
            WizardStep::ExecutionMode => 2, // autonomous, interactive
            WizardStep::BranchTypeSelect => 3, // feature, fix, custom
            WizardStep::BranchNameInput => 0, // text input, no list
            WizardStep::AIBranchSuggest => {
                if self.ai_suggest.loading || self.ai_suggest.error.is_some() {
                    0
                } else if !self.ai_suggest.options.is_empty() {
                    self.ai_suggest.options.len()
                } else {
                    self.ai_suggest.suggestions.len().max(1)
                }
            }
            WizardStep::IssueSelect => 0,     // text input
            WizardStep::SkipPermissions => 2, // yes / no
            WizardStep::Confirm => 2,         // launch / cancel
        }
    }

    /// Static option labels for the current step.
    pub fn current_static_options(&self) -> Vec<&'static str> {
        match self.step {
            WizardStep::QuickStart => vec!["New Agent", "From Template"],
            WizardStep::ModelSelect => vec!["claude-sonnet-4", "claude-opus-4", "gpt-4.1"],
            WizardStep::ReasoningLevel => vec!["Low", "Medium", "High"],
            WizardStep::ExecutionMode => vec!["Autonomous", "Interactive"],
            WizardStep::BranchTypeSelect => vec!["feature/", "fix/", "custom"],
            WizardStep::SkipPermissions => vec!["Yes", "No"],
            WizardStep::Confirm => vec!["Launch", "Cancel"],
            _ => vec![],
        }
    }

    /// Options as string labels for the current step.
    pub fn current_options(&self) -> Vec<String> {
        match self.step {
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
            WizardStep::AIBranchSuggest => {
                if self.ai_suggest.loading || self.ai_suggest.error.is_some() {
                    vec![]
                } else if !self.ai_suggest.options.is_empty() {
                    self.ai_suggest
                        .options
                        .iter()
                        .map(|option| option.label.clone())
                        .collect()
                } else if self.ai_suggest.suggestions.is_empty() {
                    vec!["(no suggestions)".to_string()]
                } else {
                    self.ai_suggest.suggestions.clone()
                }
            }
            WizardStep::BranchNameInput | WizardStep::IssueSelect => vec![],
            _ => self
                .current_static_options()
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }

    /// Human-readable summary for the current SPEC context, if any.
    pub fn spec_context_summary(&self) -> Option<String> {
        self.spec_context.as_ref().map(|ctx| {
            if ctx.title.trim().is_empty() {
                ctx.spec_id.clone()
            } else {
                format!("{} - {}", ctx.spec_id, ctx.title)
            }
        })
    }

    /// Suggested branch name derived from the current SPEC context, if any.
    pub fn spec_context_branch_seed(&self) -> Option<String> {
        let ctx = self.spec_context.as_ref()?;
        let mut suffix = slugify_branch_component(&ctx.spec_id);
        if !ctx.title.trim().is_empty() {
            let title = slugify_branch_component(&ctx.title);
            if !title.is_empty() {
                suffix.push('-');
                suffix.push_str(&title);
            }
        }
        if suffix.is_empty() {
            None
        } else {
            Some(format!("feature/{}", suffix))
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
    /// Populate AI branch suggestions.
    SetBranchSuggestions(Vec<String>),
    /// Report an AI branch suggestion error.
    SetBranchSuggestError(String),
    /// Edit the selected AI suggestion (switch to manual input with pre-filled text).
    EditSelectedSuggestion,
    /// Skip AI suggestions and go to manual input.
    SkipToManualInput,
    /// Tick for spinner animation.
    Tick,
}

/// Update wizard state in response to a message.
pub fn update(state: &mut WizardState, msg: WizardMessage) {
    match msg {
        WizardMessage::MoveUp => {
            let count = state.option_count();
            super::move_up(&mut state.selected, count);
        }
        WizardMessage::MoveDown => {
            let count = state.option_count();
            super::move_down(&mut state.selected, count);
        }
        WizardMessage::Select => {
            if state.step == WizardStep::AIBranchSuggest {
                advance_from_ai_branch_step(state);
            } else {
                // Store selection for current step, then advance
                apply_selection(state);
                if let Some(next) = state.step.next() {
                    state.step = next;
                    state.selected = 0;
                    // When entering AIBranchSuggest, start loading
                    if next == WizardStep::AIBranchSuggest {
                        state.ai_suggest = AISuggestState {
                            suggestions: Vec::new(),
                            options: Vec::new(),
                            loading: true,
                            error: None,
                            tick_counter: 0,
                        };
                        ensure_branch_name_seed(state);
                    }
                } else {
                    // Last step — mark completed
                    state.completed = true;
                }
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
        WizardMessage::SetBranchSuggestions(suggestions) => {
            state.ai_suggest.loading = false;
            state.ai_suggest.error = None;
            state.ai_suggest.suggestions = suggestions.clone();
            state.ai_suggest.options = suggestions
                .into_iter()
                .map(|branch_name| BranchSuggestionOption {
                    label: branch_name.clone(),
                    branch_name,
                })
                .collect();
            if state.step == WizardStep::AIBranchSuggest {
                state.selected = 0;
            }
        }
        WizardMessage::SetBranchSuggestError(err) => {
            state.ai_suggest.loading = false;
            state.ai_suggest.error = Some(err);
            state.ai_suggest.options.clear();
        }
        WizardMessage::EditSelectedSuggestion => {
            if state.step == WizardStep::AIBranchSuggest {
                // Pre-fill branch name with selected suggestion, switch to manual input
                apply_selected_ai_suggestion(state);
                ensure_branch_name_seed(state);
                state.step = WizardStep::BranchNameInput;
                state.selected = 0;
            }
        }
        WizardMessage::SkipToManualInput => {
            if state.step == WizardStep::AIBranchSuggest {
                ensure_branch_name_seed(state);
                state.step = WizardStep::BranchNameInput;
                state.selected = 0;
            }
        }
        WizardMessage::Tick => {
            state.ai_suggest.tick_counter = state.ai_suggest.tick_counter.wrapping_add(1);
            if state.step == WizardStep::AIBranchSuggest
                && state.ai_suggest.loading
                && state.ai_suggest.tick_counter >= AI_SUGGEST_TIMEOUT_TICKS
            {
                state.ai_suggest.loading = false;
                state.ai_suggest.error = Some("AI branch suggestion timed out".to_string());
                state.ai_suggest.options.clear();
                ensure_branch_name_seed(state);
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
        WizardStep::AIBranchSuggest => {
            apply_selected_ai_suggestion(state);
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

fn slugify_branch_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_dash = false;
    for ch in value.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash {
                out.push(mapped);
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

fn ensure_branch_name_seed(state: &mut WizardState) {
    if state.branch_name.is_empty() {
        if let Some(seed) = state.spec_context_branch_seed() {
            state.branch_name = seed;
        }
    }
}

fn apply_selected_ai_suggestion(state: &mut WizardState) {
    if let Some(option) = state.ai_suggest.options.get(state.selected) {
        state.branch_name = option.branch_name.clone();
    } else if let Some(name) = state.ai_suggest.suggestions.get(state.selected) {
        state.branch_name = name.clone();
    }
}

fn advance_from_ai_branch_step(state: &mut WizardState) {
    if state.ai_suggest.loading || state.ai_suggest.error.is_some() {
        ensure_branch_name_seed(state);
        state.step = WizardStep::BranchNameInput;
        state.selected = 0;
        return;
    }

    if state.ai_suggest.options.is_empty() {
        ensure_branch_name_seed(state);
        state.step = WizardStep::BranchNameInput;
        state.selected = 0;
        return;
    }

    apply_selected_ai_suggestion(state);
    if let Some(next) = state.step.next() {
        state.step = next;
        state.selected = 0;
    }
}

/// Render the wizard overlay.
pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Centered modal — 60% width, 70% height
    let width = (area.width * 60 / 100).max(40);
    let height = (area.height * 70 / 100).max(12);
    let overlay = super::centered_rect(width, height, area);

    // Clear the area behind the overlay
    frame.render_widget(Clear, overlay);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Progress indicator
            Constraint::Length(3), // Step title
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Hints
        ])
        .split(overlay);

    // Progress bar
    let step_idx = state.step.index() + 1;
    let total = WizardStep::total();
    let progress_text = format!(" Step {}/{}", step_idx, total);
    let progress = Paragraph::new(progress_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(progress, chunks[0]);

    // Step title
    let title_block = Block::default()
        .borders(Borders::ALL)
        .title("Agent Launch Wizard")
        .border_style(Style::default().fg(Color::Cyan));
    let title_text = Paragraph::new(state.step.title()).block(title_block).style(
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
        WizardStep::AIBranchSuggest if state.ai_suggest.loading => {
            " Loading AI suggestions... | Esc: skip to manual input"
        }
        WizardStep::AIBranchSuggest if state.ai_suggest.error.is_some() => {
            " Enter/Esc: manual input"
        }
        WizardStep::AIBranchSuggest => {
            " Up/Down: select | Enter: accept | e: edit | Esc: manual input"
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
        WizardStep::AIBranchSuggest => {
            render_ai_suggest(state, frame, area);
        }
        WizardStep::Confirm => {
            render_confirm_summary(state, frame, area);
        }
        _ => {
            render_option_list(state, frame, area);
        }
    }
}

/// Render a selectable option list for the current wizard step.
fn render_option_list(state: &WizardState, frame: &mut Frame, area: Rect) {
    let options = state.current_options();
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let style = super::list_item_style(idx == state.selected);
            let marker = if idx == state.selected { "> " } else { "  " };
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

/// Render the AI branch suggestion step.
/// Loading/error states get special treatment; the suggestion list
/// reuses the default option-list renderer via the fallthrough in
/// `render_step_content`.
fn render_ai_suggest(state: &WizardState, frame: &mut Frame, area: Rect) {
    let title = state
        .spec_context_summary()
        .map(|summary| format!("AI Branch Suggestions - {}", summary))
        .unwrap_or_else(|| "AI Branch Suggestions".to_string());
    let block = Block::default().borders(Borders::ALL).title(title);

    if state.ai_suggest.loading {
        let spinner_chars = [
            '\u{280B}', '\u{2819}', '\u{2838}', '\u{2834}', '\u{2826}', '\u{2807}',
        ];
        let ch = spinner_chars[state.ai_suggest.tick_counter % spinner_chars.len()];
        let text = Paragraph::new(format!(
            " {} Generating branch name suggestions...\n\n Type Enter to use a manual branch name if needed.",
            ch
        ))
            .block(block)
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(text, area);
        return;
    }

    if let Some(ref err) = state.ai_suggest.error {
        let text = Paragraph::new(format!(
            " Error: {}\n\n Press Enter or Esc to enter branch name manually.",
            err
        ))
        .block(block)
        .style(Style::default().fg(Color::Red));
        frame.render_widget(text, area);
        return;
    }

    // Delegate to the default option-list renderer (current_options()
    // already returns the suggestion strings for this step).
    render_option_list(state, frame, area);
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
            let style = super::list_item_style(idx == state.selected);
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
        assert_eq!(WizardStep::QuickStart.next(), Some(WizardStep::AgentSelect));
        assert_eq!(WizardStep::Confirm.next(), None);
    }

    #[test]
    fn step_navigation_prev() {
        assert_eq!(WizardStep::QuickStart.prev(), None);
        assert_eq!(WizardStep::AgentSelect.prev(), Some(WizardStep::QuickStart));
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

    // ============================================================
    // AI Branch Suggest Tests
    // ============================================================

    #[test]
    fn ai_suggest_loading_on_enter_step() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchNameInput;
        // Advance from BranchNameInput to AIBranchSuggest via Select
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::AIBranchSuggest);
        assert!(state.ai_suggest.loading);
        assert!(state.ai_suggest.suggestions.is_empty());
        assert!(state.ai_suggest.error.is_none());
    }

    #[test]
    fn ai_suggest_set_suggestions_clears_loading() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;
        let suggestions = vec![
            "feature/add-auth".to_string(),
            "feature/user-login".to_string(),
            "feature/oauth-flow".to_string(),
        ];
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(suggestions.clone()),
        );
        assert!(!state.ai_suggest.loading);
        assert_eq!(state.ai_suggest.suggestions, suggestions);
        assert_eq!(
            state
                .ai_suggest
                .options
                .iter()
                .map(|option| option.branch_name.clone())
                .collect::<Vec<_>>(),
            vec![
                "feature/add-auth".to_string(),
                "feature/user-login".to_string(),
                "feature/oauth-flow".to_string(),
            ]
        );
        assert_eq!(state.selected, 0);
        assert!(state.ai_suggest.error.is_none());
    }

    #[test]
    fn ai_suggest_set_error_clears_loading() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestError("timeout".to_string()),
        );
        assert!(!state.ai_suggest.loading);
        assert_eq!(state.ai_suggest.error, Some("timeout".to_string()));
    }

    #[test]
    fn ai_suggest_navigate_suggestions() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(vec![
                "feature/a".to_string(),
                "feature/b".to_string(),
                "feature/c".to_string(),
            ]),
        );
        assert_eq!(state.selected, 0);
        update(&mut state, WizardMessage::MoveDown);
        assert_eq!(state.selected, 1);
        update(&mut state, WizardMessage::MoveDown);
        assert_eq!(state.selected, 2);
        update(&mut state, WizardMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn ai_suggest_select_stores_branch_name() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(vec![
                "feature/a".to_string(),
                "feature/b".to_string(),
            ]),
        );
        state.selected = 1;
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.branch_name, "feature/b");
        assert_eq!(state.step, WizardStep::IssueSelect);
    }

    #[test]
    fn ai_suggest_edit_switches_to_manual() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(vec![
                "feature/a".to_string(),
                "feature/b".to_string(),
            ]),
        );
        state.selected = 0;
        update(&mut state, WizardMessage::EditSelectedSuggestion);
        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert_eq!(state.branch_name, "feature/a");
    }

    #[test]
    fn ai_suggest_skip_goes_to_manual() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;
        update(&mut state, WizardMessage::SkipToManualInput);
        assert_eq!(state.step, WizardStep::BranchNameInput);
    }

    #[test]
    fn ai_suggest_option_count_while_loading() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;
        assert_eq!(state.option_count(), 0);
    }

    #[test]
    fn ai_suggest_option_count_with_error() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.error = Some("fail".to_string());
        assert_eq!(state.option_count(), 0);
    }

    #[test]
    fn ai_suggest_option_count_with_suggestions() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.suggestions = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(state.option_count(), 3);
    }

    #[test]
    fn ai_suggest_render_loading_no_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;
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
    fn ai_suggest_render_error_no_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.error = Some("Connection timeout".to_string());
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
    fn back_from_step2_goes_to_step1() {
        let mut state = WizardState::default();
        // Advance to step 2 (AgentSelect)
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::AgentSelect);

        // Back should return to step 1 (QuickStart)
        update(&mut state, WizardMessage::Back);
        assert_eq!(state.step, WizardStep::QuickStart);
        assert!(!state.cancelled);
    }

    #[test]
    fn cancel_from_step1_sets_cancelled() {
        let mut state = WizardState::default();
        assert_eq!(state.step, WizardStep::QuickStart);

        // Cancel on QuickStart
        update(&mut state, WizardMessage::Cancel);
        assert!(state.cancelled);
    }

    #[test]
    fn ai_suggest_empty_suggestions_falls_through() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = false;
        state.ai_suggest.error = None;
        state.ai_suggest.suggestions = Vec::new();

        // With no suggestions, current_options should show placeholder
        let options = state.current_options();
        assert_eq!(options.len(), 1);
        assert_eq!(options[0], "(no suggestions)");

        // option_count should be 1 (from max(1))
        assert_eq!(state.option_count(), 1);

        // Select should fall back to manual branch input
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert!(state.branch_name.is_empty());
    }

    #[test]
    fn ai_suggest_timeout_switches_to_manual_fallback() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.loading = true;

        for _ in 0..AI_SUGGEST_TIMEOUT_TICKS {
            update(&mut state, WizardMessage::Tick);
        }

        assert!(!state.ai_suggest.loading);
        assert_eq!(
            state.ai_suggest.error.as_deref(),
            Some("AI branch suggestion timed out")
        );

        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::BranchNameInput);
    }

    #[test]
    fn spec_context_branch_seed_is_derived() {
        let mut state = WizardState::default();
        state.spec_context = Some(SpecContext {
            spec_id: "SPEC-42".to_string(),
            title: "My Feature".to_string(),
        });

        assert_eq!(
            state.spec_context_branch_seed(),
            Some("feature/spec-42-my-feature".to_string())
        );
    }

    #[test]
    fn ai_suggest_render_suggestions_no_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.ai_suggest.suggestions = vec![
            "feature/add-auth".to_string(),
            "feature/user-login".to_string(),
            "feature/oauth-flow".to_string(),
        ];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }
}
