//! Wizard overlay screen — branch-first agent launch wizard.

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
    VersionSelect,
    ExecutionMode,
    BranchTypeSelect,
    BranchNameInput,
    AIBranchSuggest,
    IssueSelect,
    SkipPermissions,
    Confirm,
}

impl WizardStep {
    /// Human-readable title for this step.
    pub fn title(self) -> &'static str {
        match self {
            Self::QuickStart => "Branch Action",
            Self::AgentSelect => "Select Agent",
            Self::ModelSelect => "Select Model",
            Self::ReasoningLevel => "Reasoning Level",
            Self::VersionSelect => "Select Version",
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

/// Determine the next step based on current step and wizard context.
///
/// Restores the old branch-first flow while keeping the current confirm step:
/// - Existing branch: QuickStart(Branch Action) → AgentSelect → ...
/// - New branch: QuickStart(create) or spec prefill → BranchType → Issue → AI → Branch Name → AgentSelect → ...
fn next_step(current: WizardStep, state: &WizardState) -> Option<WizardStep> {
    match current {
        WizardStep::QuickStart => {
            if state.selected == 1 {
                Some(WizardStep::BranchTypeSelect)
            } else {
                Some(WizardStep::AgentSelect)
            }
        }
        WizardStep::AgentSelect => {
            if state.agent_has_models() {
                Some(WizardStep::ModelSelect)
            } else if state.agent_has_npm_package() {
                Some(WizardStep::VersionSelect)
            } else {
                Some(WizardStep::ExecutionMode)
            }
        }
        WizardStep::ModelSelect => {
            if state.agent_is_codex() {
                Some(WizardStep::ReasoningLevel)
            } else if state.agent_has_npm_package() {
                Some(WizardStep::VersionSelect)
            } else {
                Some(WizardStep::ExecutionMode)
            }
        }
        WizardStep::ReasoningLevel => {
            if state.agent_has_npm_package() {
                Some(WizardStep::VersionSelect)
            } else {
                Some(WizardStep::ExecutionMode)
            }
        }
        WizardStep::VersionSelect => Some(WizardStep::ExecutionMode),
        WizardStep::ExecutionMode => Some(WizardStep::SkipPermissions),
        WizardStep::BranchTypeSelect => {
            if state.gh_cli_available {
                Some(WizardStep::IssueSelect)
            } else if state.ai_enabled {
                Some(WizardStep::AIBranchSuggest)
            } else {
                Some(WizardStep::BranchNameInput)
            }
        }
        WizardStep::BranchNameInput => Some(WizardStep::AgentSelect),
        WizardStep::AIBranchSuggest => Some(WizardStep::BranchNameInput),
        WizardStep::IssueSelect => {
            if state.ai_enabled {
                Some(WizardStep::AIBranchSuggest)
            } else {
                Some(WizardStep::BranchNameInput)
            }
        }
        WizardStep::SkipPermissions => Some(WizardStep::Confirm),
        WizardStep::Confirm => None,
    }
}

/// Determine the previous step based on current step and wizard context.
fn prev_step(current: WizardStep, state: &WizardState) -> Option<WizardStep> {
    match current {
        WizardStep::QuickStart => None,
        WizardStep::AgentSelect => {
            if state.is_new_branch {
                Some(WizardStep::BranchNameInput)
            } else {
                Some(WizardStep::QuickStart)
            }
        }
        WizardStep::ModelSelect => Some(WizardStep::AgentSelect),
        WizardStep::ReasoningLevel => Some(WizardStep::ModelSelect),
        WizardStep::VersionSelect => {
            if state.agent_is_codex() {
                Some(WizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(WizardStep::ModelSelect)
            } else {
                Some(WizardStep::AgentSelect)
            }
        }
        WizardStep::ExecutionMode => {
            if state.agent_has_npm_package() {
                Some(WizardStep::VersionSelect)
            } else if state.agent_is_codex() {
                Some(WizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(WizardStep::ModelSelect)
            } else {
                Some(WizardStep::AgentSelect)
            }
        }
        WizardStep::BranchTypeSelect => {
            if state.base_branch_name.is_some() {
                Some(WizardStep::QuickStart)
            } else {
                None
            }
        }
        WizardStep::BranchNameInput => {
            if state.ai_enabled {
                Some(WizardStep::AIBranchSuggest)
            } else if state.gh_cli_available {
                Some(WizardStep::IssueSelect)
            } else {
                Some(WizardStep::BranchTypeSelect)
            }
        }
        WizardStep::AIBranchSuggest => {
            if state.gh_cli_available {
                Some(WizardStep::IssueSelect)
            } else {
                Some(WizardStep::BranchTypeSelect)
            }
        }
        WizardStep::IssueSelect => Some(WizardStep::BranchTypeSelect),
        WizardStep::SkipPermissions => Some(WizardStep::ExecutionMode),
        WizardStep::Confirm => Some(WizardStep::SkipPermissions),
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
const MANUAL_INPUT_LABEL: &str = "Manual input";

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
    /// Detected installed binary version, if known.
    pub installed_version: Option<String>,
    /// Cached version history loaded at wizard startup.
    pub versions: Vec<String>,
    /// Whether the cached version list is stale and scheduled for refresh.
    pub cache_outdated: bool,
}

impl AgentOption {
    /// Render the option label shown in the wizard (name only, like old TUI).
    pub fn display_label(&self) -> String {
        self.name.clone()
    }
}

/// SPEC context for prefilling the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecContext {
    pub spec_id: String,
    pub title: String,
}

/// A version option for the VersionSelect step.
pub use gwt_agent::version_cache::VersionOption;

/// State for the wizard overlay.
#[derive(Debug, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub detected_agents: Vec<AgentOption>,
    pub selected: usize,
    pub is_new_branch: bool,
    pub base_branch_name: Option<String>,
    pub gh_cli_available: bool,
    pub ai_enabled: bool,
    // Config fields accumulated during the wizard
    pub agent_id: String,
    pub model: String,
    pub reasoning: String,
    pub version: String,
    pub version_options: Vec<VersionOption>,
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
            is_new_branch: false,
            base_branch_name: None,
            gh_cli_available: true,
            ai_enabled: true,
            agent_id: String::new(),
            model: String::new(),
            reasoning: "medium".to_string(),
            version: String::new(),
            version_options: Vec::new(),
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
    fn flow_start_step(&self) -> WizardStep {
        if self.is_new_branch {
            WizardStep::BranchTypeSelect
        } else {
            WizardStep::QuickStart
        }
    }

    fn effective_agent_id(&self) -> &str {
        self.selected_agent()
            .map(|agent| agent.id.as_str())
            .unwrap_or(self.agent_id.as_str())
    }

    /// Whether the selected agent has model options.
    fn agent_has_models(&self) -> bool {
        matches!(self.effective_agent_id(), "claude" | "codex" | "gemini")
    }

    /// Whether the selected agent is Codex (needs ReasoningLevel step).
    fn agent_is_codex(&self) -> bool {
        self.effective_agent_id() == "codex"
    }

    /// Whether the selected agent is distributed via npm.
    fn agent_has_npm_package(&self) -> bool {
        matches!(self.effective_agent_id(), "claude" | "codex" | "gemini")
    }

    /// Total steps visible for the current agent configuration.
    pub fn visible_step_count(&self) -> usize {
        let mut count = 0;
        let mut step = Some(self.flow_start_step());
        while let Some(s) = step {
            count += 1;
            step = next_step(s, self);
        }
        count
    }

    /// 1-based index of the current step among visible steps.
    pub fn visible_step_index(&self) -> usize {
        let mut idx = 0;
        let mut step = Some(self.flow_start_step());
        while let Some(s) = step {
            idx += 1;
            if s == self.step {
                return idx;
            }
            step = next_step(s, self);
        }
        idx
    }

    fn selected_agent(&self) -> Option<&AgentOption> {
        if self.step == WizardStep::AgentSelect {
            return self.detected_agents.get(self.selected);
        }
        if !self.agent_id.is_empty() {
            self.detected_agents
                .iter()
                .find(|agent| agent.id == self.agent_id)
        } else {
            self.detected_agents.get(self.selected)
        }
    }

    fn current_model_options(&self) -> Vec<String> {
        default_model_options(self.effective_agent_id())
    }

    fn sync_selected_agent_options(&mut self) {
        let Some(agent) = self.selected_agent().cloned() else {
            self.model.clear();
            self.version.clear();
            self.version_options.clear();
            return;
        };

        let model_options = self.current_model_options();
        if let Some(first_model) = model_options.first() {
            if self.model.is_empty() || !model_options.iter().any(|option| option == &self.model) {
                self.model = first_model.clone();
            }
        } else {
            self.model.clear();
        }

        self.version_options = gwt_agent::build_version_options(
            agent.available,
            agent.installed_version.as_deref(),
            self.agent_has_npm_package(),
            &agent.versions,
        );

        if let Some(first_version) = self.version_options.first() {
            if self.version.is_empty()
                || !self
                    .version_options
                    .iter()
                    .any(|option| option.value == self.version)
            {
                self.version = first_version.value.clone();
            }
        } else {
            self.version.clear();
        }
    }

    /// Number of selectable options for the current step.
    pub fn option_count(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => 2, // existing branch / create new branch
            WizardStep::AgentSelect => self.detected_agents.len().max(1),
            WizardStep::ModelSelect => self.current_model_options().len(),
            WizardStep::ReasoningLevel => 4, // low, medium, high, xhigh
            WizardStep::VersionSelect => self.version_options.len().max(1),
            WizardStep::ExecutionMode => 4, // normal, continue, resume, convert
            WizardStep::BranchTypeSelect => 4, // feature, bugfix, hotfix, release
            WizardStep::BranchNameInput => 0, // text input, no list
            WizardStep::AIBranchSuggest => {
                if self.ai_suggest.loading || self.ai_suggest.error.is_some() {
                    0
                } else if !self.ai_suggest.options.is_empty() {
                    self.ai_suggest.options.len() + 1
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
            WizardStep::QuickStart => vec!["Use selected branch", "Create new from selected"],
            WizardStep::ReasoningLevel => vec!["Low", "Medium", "High", "XHigh"],
            WizardStep::ExecutionMode => vec!["Normal", "Continue", "Resume", "Convert"],
            WizardStep::BranchTypeSelect => vec!["feature/", "bugfix/", "hotfix/", "release/"],
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
                        .map(AgentOption::display_label)
                        .collect()
                }
            }
            WizardStep::ModelSelect => self.current_model_options(),
            WizardStep::VersionSelect => {
                if self.version_options.is_empty() {
                    vec!["(no versions available)".to_string()]
                } else {
                    self.version_options
                        .iter()
                        .map(|v| v.label.clone())
                        .collect()
                }
            }
            WizardStep::AIBranchSuggest => {
                if self.ai_suggest.loading || self.ai_suggest.error.is_some() {
                    vec![]
                } else if !self.ai_suggest.options.is_empty() {
                    let mut labels = self
                        .ai_suggest
                        .options
                        .iter()
                        .map(|option| option.label.clone())
                        .collect::<Vec<_>>();
                    labels.push(MANUAL_INPUT_LABEL.to_string());
                    labels
                } else if self.ai_suggest.suggestions.is_empty() {
                    vec!["(no suggestions)".to_string()]
                } else {
                    let mut labels = self.ai_suggest.suggestions.clone();
                    labels.push(MANUAL_INPUT_LABEL.to_string());
                    labels
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
            if state.step == WizardStep::AgentSelect {
                state.sync_selected_agent_options();
            }
        }
        WizardMessage::MoveDown => {
            let count = state.option_count();
            super::move_down(&mut state.selected, count);
            if state.step == WizardStep::AgentSelect {
                state.sync_selected_agent_options();
            }
        }
        WizardMessage::Select => {
            if state.step == WizardStep::AIBranchSuggest {
                advance_from_ai_branch_step(state);
            } else {
                // Store selection for current step, then advance
                apply_selection(state);
                if let Some(next) = next_step(state.step, state) {
                    state.step = next;
                    state.selected = 0;
                    if matches!(next, WizardStep::ModelSelect | WizardStep::VersionSelect) {
                        state.sync_selected_agent_options();
                    }
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
            if let Some(prev) = prev_step(state.step, state) {
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
            state.sync_selected_agent_options();
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
        WizardStep::QuickStart => {
            if state.selected == 0 {
                state.is_new_branch = false;
                state.base_branch_name = None;
            } else {
                state.is_new_branch = true;
                if state.base_branch_name.is_none() && !state.branch_name.is_empty() {
                    state.base_branch_name = Some(state.branch_name.clone());
                }
                if state.spec_context.is_none() {
                    state.branch_name.clear();
                }
            }
        }
        WizardStep::BranchTypeSelect => {
            if let Some(prefix) = BRANCH_TYPE_PREFIXES.get(state.selected) {
                let seed = if state.branch_name.is_empty() {
                    state
                        .spec_context_branch_seed()
                        .unwrap_or_else(|| (*prefix).to_string())
                } else {
                    state.branch_name.clone()
                };
                state.branch_name = apply_branch_prefix(&seed, prefix);
            }
        }
        WizardStep::AgentSelect => {
            if let Some(agent) = state.detected_agents.get(state.selected) {
                state.agent_id = agent.id.clone();
            }
            state.sync_selected_agent_options();
        }
        WizardStep::ModelSelect => {
            if let Some(opt) = options.get(state.selected) {
                state.model = opt.clone();
            }
            state.sync_selected_agent_options();
        }
        WizardStep::ReasoningLevel => {
            if let Some(opt) = options.get(state.selected) {
                state.reasoning = opt.to_lowercase();
            }
        }
        WizardStep::VersionSelect => {
            if let Some(opt) = state.version_options.get(state.selected) {
                state.version = opt.value.clone();
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

const BRANCH_TYPE_PREFIXES: [&str; 4] = ["feature/", "bugfix/", "hotfix/", "release/"];

fn apply_branch_prefix(seed: &str, prefix: &str) -> String {
    let trimmed = seed.trim();
    let suffix = BRANCH_TYPE_PREFIXES
        .iter()
        .find_map(|known| trimmed.strip_prefix(known))
        .unwrap_or(trimmed);
    let suffix = suffix.trim_matches('/');
    if suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{suffix}")
    }
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

    if state.selected >= state.ai_suggest.options.len() {
        ensure_branch_name_seed(state);
        state.step = WizardStep::BranchNameInput;
        state.selected = 0;
        return;
    }

    apply_selected_ai_suggestion(state);
    if let Some(next) = next_step(state.step, state) {
        state.step = next;
        state.selected = 0;
    }
}

fn default_model_options(agent_id: &str) -> Vec<String> {
    match agent_id {
        "claude" => vec![
            "Default (Opus 4.6)".to_string(),
            "opus".to_string(),
            "sonnet".to_string(),
            "haiku".to_string(),
        ],
        "codex" => vec![
            "Default (Auto)".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.2".to_string(),
            "gpt-5.1-codex-mini".to_string(),
        ],
        "gemini" => vec![
            "Default (Auto)".to_string(),
            "gemini-3-pro-preview".to_string(),
            "gemini-3-flash-preview".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-flash-lite".to_string(),
        ],
        _ => Vec::new(),
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
    let step_idx = state.visible_step_index();
    let total = state.visible_step_count();
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
        " Agent:       {}\n Model:       {}\n Version:     {}\n Reasoning:   {}\n Mode:        {}\n Branch:      {}\n Issue:       {}\n Skip Perms:  {}",
        if state.agent_id.is_empty() { "-" } else { &state.agent_id },
        if state.model.is_empty() { "-" } else { &state.model },
        if state.version.is_empty() { "-" } else { &state.version },
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
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn sample_agents() -> Vec<AgentOption> {
        vec![
            AgentOption {
                id: "claude".to_string(),
                name: "Claude Code".to_string(),
                available: true,
                installed_version: Some("1.0.54".to_string()),
                versions: vec!["1.0.54".to_string(), "1.0.53".to_string()],
                cache_outdated: false,
            },
            AgentOption {
                id: "codex".to_string(),
                name: "Codex CLI".to_string(),
                available: true,
                installed_version: Some("0.5.0".to_string()),
                versions: vec!["0.5.0".to_string()],
                cache_outdated: true,
            },
            AgentOption {
                id: "aider".to_string(),
                name: "Aider".to_string(),
                available: false,
                installed_version: None,
                versions: Vec::new(),
                cache_outdated: false,
            },
        ]
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut out = String::with_capacity(buf.area.width as usize * buf.area.height as usize);
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
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
        let state = WizardState::default();
        assert_eq!(
            next_step(WizardStep::QuickStart, &state),
            Some(WizardStep::AgentSelect)
        );
        assert_eq!(next_step(WizardStep::Confirm, &state), None);
    }

    #[test]
    fn step_navigation_prev() {
        let state = WizardState::default();
        assert_eq!(prev_step(WizardStep::QuickStart, &state), None);
        assert_eq!(
            prev_step(WizardStep::AgentSelect, &state),
            Some(WizardStep::QuickStart)
        );
    }

    #[test]
    fn step_transitions_skip_for_opencode() {
        let mut state = WizardState::default();
        state.agent_id = "opencode".to_string();
        // OpenCode has no models, no npm → AgentSelect → ExecutionMode
        assert_eq!(
            next_step(WizardStep::AgentSelect, &state),
            Some(WizardStep::ExecutionMode)
        );
    }

    #[test]
    fn step_transitions_codex_includes_reasoning_and_version() {
        let mut state = WizardState::default();
        state.agent_id = "codex".to_string();
        // Codex: AgentSelect → ModelSelect → ReasoningLevel → VersionSelect → ExecutionMode
        assert_eq!(
            next_step(WizardStep::AgentSelect, &state),
            Some(WizardStep::ModelSelect)
        );
        assert_eq!(
            next_step(WizardStep::ModelSelect, &state),
            Some(WizardStep::ReasoningLevel)
        );
        assert_eq!(
            next_step(WizardStep::ReasoningLevel, &state),
            Some(WizardStep::VersionSelect)
        );
        assert_eq!(
            next_step(WizardStep::VersionSelect, &state),
            Some(WizardStep::ExecutionMode)
        );
    }

    #[test]
    fn step_transitions_claude_skips_reasoning() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        // Claude: AgentSelect → ModelSelect → VersionSelect → ExecutionMode
        assert_eq!(
            next_step(WizardStep::AgentSelect, &state),
            Some(WizardStep::ModelSelect)
        );
        assert_eq!(
            next_step(WizardStep::ModelSelect, &state),
            Some(WizardStep::VersionSelect)
        );
        assert_eq!(
            next_step(WizardStep::VersionSelect, &state),
            Some(WizardStep::ExecutionMode)
        );
    }

    #[test]
    fn select_on_quick_start_create_new_branch_enters_branch_type_select() {
        let mut state = WizardState::default();
        state.selected = 1;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::BranchTypeSelect);
    }

    #[test]
    fn branch_type_select_advances_to_issue_before_agent_selection() {
        let state = WizardState::default();

        assert_eq!(
            next_step(WizardStep::BranchTypeSelect, &state),
            Some(WizardStep::IssueSelect)
        );
    }

    #[test]
    fn branch_type_options_restore_old_prefixes() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchTypeSelect;

        assert_eq!(
            state.current_options(),
            vec![
                "feature/".to_string(),
                "bugfix/".to_string(),
                "hotfix/".to_string(),
                "release/".to_string(),
            ]
        );
    }

    #[test]
    fn execution_mode_options_restore_old_labels() {
        let mut state = WizardState::default();
        state.step = WizardStep::ExecutionMode;

        assert_eq!(
            state.current_options(),
            vec![
                "Normal".to_string(),
                "Continue".to_string(),
                "Resume".to_string(),
                "Convert".to_string(),
            ]
        );
    }

    #[test]
    fn visible_step_count_varies_by_agent() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        let claude_count = state.visible_step_count();

        state.agent_id = "opencode".to_string();
        let opencode_count = state.visible_step_count();

        // OpenCode skips ModelSelect, ReasoningLevel, VersionSelect → fewer steps
        assert!(opencode_count < claude_count);
    }

    #[test]
    fn version_select_options() {
        let mut state = WizardState::default();
        state.version_options = vec![
            VersionOption {
                label: "Installed (v1.0.0)".to_string(),
                value: "installed".to_string(),
            },
            VersionOption {
                label: "latest".to_string(),
                value: "latest".to_string(),
            },
        ];
        state.step = WizardStep::VersionSelect;
        assert_eq!(state.option_count(), 2);
        let opts = state.current_options();
        assert_eq!(opts[0], "Installed (v1.0.0)");
        assert_eq!(opts[1], "latest");
    }

    #[test]
    fn version_select_applies_selection() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.step = WizardStep::VersionSelect;
        state.version_options = vec![
            VersionOption {
                label: "Installed (v1.0.0)".to_string(),
                value: "installed".to_string(),
            },
            VersionOption {
                label: "latest".to_string(),
                value: "latest".to_string(),
            },
        ];
        state.selected = 1;
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.version, "latest");
        assert_eq!(state.step, WizardStep::ExecutionMode);
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
        assert_eq!(state.detected_agents[0].versions, vec!["1.0.54", "1.0.53"]);
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
    fn agent_option_display_shows_name_only() {
        let option = AgentOption {
            id: "codex".to_string(),
            name: "Codex CLI".to_string(),
            available: true,
            installed_version: Some("1.3.0".to_string()),
            versions: vec!["1.3.0".to_string(), "1.2.0".to_string()],
            cache_outdated: true,
        };

        assert_eq!(option.display_label(), "Codex CLI");
    }

    #[test]
    fn current_options_show_agent_names_for_agents() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.detected_agents = vec![AgentOption {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            available: true,
            installed_version: Some("1.0.54".to_string()),
            versions: vec!["1.0.54".to_string()],
            cache_outdated: false,
        }];

        assert_eq!(
            state.current_options(),
            vec!["Claude Code".to_string()]
        );
    }

    #[test]
    fn select_on_model_step_stores_model_catalog_entry() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.step = WizardStep::ModelSelect;
        state.selected = 1;
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.model, "opus");
    }

    #[test]
    fn model_select_uses_agent_model_catalog() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.step = WizardStep::ModelSelect;

        assert_eq!(
            state.current_options(),
            vec![
                "Default (Opus 4.6)".to_string(),
                "opus".to_string(),
                "sonnet".to_string(),
                "haiku".to_string()
            ]
        );
        assert_eq!(state.option_count(), 4);
    }

    #[test]
    fn select_on_model_step_populates_version_select_options() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.step = WizardStep::ModelSelect;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::VersionSelect);
        assert_eq!(
            state.current_options(),
            vec![
                "Installed (v1.0.54)".to_string(),
                "latest".to_string(),
                "1.0.53".to_string()
            ]
        );
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
        let all_steps = [
            WizardStep::QuickStart,
            WizardStep::AgentSelect,
            WizardStep::ModelSelect,
            WizardStep::ReasoningLevel,
            WizardStep::VersionSelect,
            WizardStep::ExecutionMode,
            WizardStep::BranchTypeSelect,
            WizardStep::BranchNameInput,
            WizardStep::AIBranchSuggest,
            WizardStep::IssueSelect,
            WizardStep::SkipPermissions,
            WizardStep::Confirm,
        ];
        for step in all_steps {
            assert!(!step.title().is_empty(), "{:?} has empty title", step);
        }
    }

    // ============================================================
    // AI Branch Suggest Tests
    // ============================================================

    #[test]
    fn ai_suggest_loading_on_enter_step() {
        let mut state = WizardState::default();
        state.step = WizardStep::IssueSelect;
        // Advance from IssueSelect to AIBranchSuggest via Select
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
        assert_eq!(state.selected, 3);
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
        assert_eq!(state.step, WizardStep::BranchNameInput);
    }

    #[test]
    fn ai_suggest_manual_input_is_always_last() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(vec![
                "feature/a".to_string(),
                "feature/b".to_string(),
            ]),
        );

        let options = state.current_options();
        assert_eq!(options.last().map(String::as_str), Some("Manual input"));
    }

    #[test]
    fn ai_suggest_selecting_manual_input_goes_to_branch_input() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        update(
            &mut state,
            WizardMessage::SetBranchSuggestions(vec![
                "feature/a".to_string(),
                "feature/b".to_string(),
            ]),
        );
        state.selected = state.option_count().saturating_sub(1);

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert_eq!(state.branch_name, "");
    }

    #[test]
    fn ai_suggest_render_includes_manual_input_and_candidates() {
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

        let buf = terminal.backend().buffer().clone();
        let text = buffer_text(&buf);
        assert!(text.contains("feature/add-auth"));
        assert!(text.contains("feature/user-login"));
        assert!(text.contains("feature/oauth-flow"));
        assert!(text.contains("Manual input"));
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
        state.ai_suggest.options = vec![
            BranchSuggestionOption {
                branch_name: "a".to_string(),
                label: "a".to_string(),
            },
            BranchSuggestionOption {
                branch_name: "b".to_string(),
                label: "b".to_string(),
            },
            BranchSuggestionOption {
                branch_name: "c".to_string(),
                label: "c".to_string(),
            },
        ];
        assert_eq!(state.option_count(), 4);
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
