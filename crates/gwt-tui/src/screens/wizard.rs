//! Wizard overlay screen — branch-first agent launch wizard.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

/// Which step of the wizard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    QuickStart,
    #[default]
    BranchAction,
    AgentSelect,
    ModelSelect,
    ReasoningLevel,
    VersionSelect,
    ExecutionMode,
    ConvertAgentSelect,
    ConvertSessionSelect,
    BranchTypeSelect,
    BranchNameInput,
    AIBranchSuggest,
    IssueSelect,
    SkipPermissions,
    CodexFastMode,
}

impl WizardStep {
    /// Human-readable title for this step.
    pub fn title(self) -> &'static str {
        match self {
            Self::QuickStart => "Quick Start",
            Self::BranchAction => "Branch Action",
            Self::AgentSelect => "Select Coding Agent",
            Self::ModelSelect => "Select Model",
            Self::ReasoningLevel => "Reasoning Level",
            Self::VersionSelect => "Select Version",
            Self::ExecutionMode => "Execution Mode",
            Self::ConvertAgentSelect => "Convert From Agent",
            Self::ConvertSessionSelect => "Select Session",
            Self::BranchTypeSelect => "Branch Type",
            Self::BranchNameInput => "Branch Name",
            Self::AIBranchSuggest => "AI Branch Suggestion",
            Self::IssueSelect => "Link Issue",
            Self::SkipPermissions => "Skip Permissions",
            Self::CodexFastMode => "Codex Fast Mode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeModelFamily {
    Opus,
    Sonnet,
    Haiku,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningStepKind {
    None,
    Codex,
    ClaudeEffort { max_available: bool },
}

/// Determine the next step based on current step and wizard context.
///
/// Restores the old branch-first flow:
/// - Existing branch: BranchAction → AgentSelect → ...
/// - New branch/spec prefill: BranchType → Issue → AI → Branch Name → AgentSelect → ...
fn next_step(current: WizardStep, state: &WizardState) -> Option<WizardStep> {
    match current {
        WizardStep::QuickStart => match state.selected_quick_start_action() {
            QuickStartAction::ChooseDifferent => Some(WizardStep::BranchAction),
            QuickStartAction::ResumeWithPrevious | QuickStartAction::StartNewWithPrevious => {
                Some(WizardStep::SkipPermissions)
            }
        },
        WizardStep::BranchAction => {
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
            if state.agent_uses_reasoning_step() {
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
        WizardStep::ExecutionMode => {
            if state.mode == "convert" {
                Some(WizardStep::ConvertAgentSelect)
            } else {
                Some(WizardStep::SkipPermissions)
            }
        }
        WizardStep::ConvertAgentSelect => Some(WizardStep::ConvertSessionSelect),
        WizardStep::ConvertSessionSelect => Some(WizardStep::SkipPermissions),
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
        WizardStep::SkipPermissions => {
            if state.agent_is_codex() {
                Some(WizardStep::CodexFastMode)
            } else {
                None
            }
        }
        WizardStep::CodexFastMode => None,
    }
}

/// Determine the previous step based on current step and wizard context.
fn prev_step(current: WizardStep, state: &WizardState) -> Option<WizardStep> {
    match current {
        WizardStep::QuickStart => None,
        WizardStep::BranchAction => {
            if state.has_quick_start && !state.quick_start_entries.is_empty() {
                Some(WizardStep::QuickStart)
            } else {
                None
            }
        }
        WizardStep::AgentSelect => {
            if state.is_new_branch {
                Some(WizardStep::BranchNameInput)
            } else {
                Some(WizardStep::BranchAction)
            }
        }
        WizardStep::ModelSelect => Some(WizardStep::AgentSelect),
        WizardStep::ReasoningLevel => Some(WizardStep::ModelSelect),
        WizardStep::VersionSelect => {
            if state.agent_uses_reasoning_step() {
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
            } else if state.agent_uses_reasoning_step() {
                Some(WizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(WizardStep::ModelSelect)
            } else {
                Some(WizardStep::AgentSelect)
            }
        }
        WizardStep::ConvertAgentSelect => Some(WizardStep::ExecutionMode),
        WizardStep::ConvertSessionSelect => Some(WizardStep::ConvertAgentSelect),
        WizardStep::BranchTypeSelect => {
            if state.base_branch_name.is_some() {
                Some(WizardStep::BranchAction)
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
        WizardStep::SkipPermissions => {
            if state.mode == "convert" {
                Some(WizardStep::ConvertSessionSelect)
            } else {
                Some(WizardStep::ExecutionMode)
            }
        }
        WizardStep::CodexFastMode => Some(WizardStep::SkipPermissions),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickStartAction {
    ResumeWithPrevious,
    StartNewWithPrevious,
    ChooseDifferent,
}

/// Persisted launch metadata that can be replayed from Quick Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickStartEntry {
    pub agent_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub version: Option<String>,
    pub resume_session_id: Option<String>,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
}

/// SPEC context for prefilling the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecContext {
    pub spec_id: String,
    pub title: String,
    pub spec_body: String,
}

impl SpecContext {
    pub fn new(
        spec_id: impl Into<String>,
        title: impl Into<String>,
        spec_body: impl Into<String>,
    ) -> Self {
        let spec_id = spec_id.into();
        let title = title.into();
        let spec_body = spec_body.into();
        Self {
            spec_id,
            title,
            spec_body,
        }
    }

    pub fn branch_seed(&self) -> Option<String> {
        let branch_seed = derive_spec_branch_seed(&self.spec_id, &self.title);
        if branch_seed == "feature/" {
            None
        } else {
            Some(branch_seed)
        }
    }
}

/// A version option for the VersionSelect step.
pub use gwt_agent::version_cache::VersionOption;

/// State for the wizard overlay.
#[derive(Debug, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub detected_agents: Vec<AgentOption>,
    pub selected: usize,
    pub has_quick_start: bool,
    pub quick_start_entries: Vec<QuickStartEntry>,
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
    pub resume_session_id: Option<String>,
    pub branch_name: String,
    pub issue_id: String,
    pub skip_perms: bool,
    pub codex_fast_mode: bool,
    pub convert_source_agents: Vec<String>,
    pub convert_sessions: Vec<String>,
    /// AI branch suggestion state.
    pub ai_suggest: AISuggestState,
    /// Whether the wizard has been completed (caller should read config).
    pub completed: bool,
    /// Whether the wizard has been cancelled.
    pub cancelled: bool,
    /// Optional SPEC context for prefilling.
    pub spec_context: Option<SpecContext>,
    /// Worktree path for the selected branch (None = use repo root).
    pub worktree_path: Option<std::path::PathBuf>,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            step: WizardStep::default(),
            detected_agents: Vec::new(),
            selected: 0,
            has_quick_start: false,
            quick_start_entries: Vec::new(),
            is_new_branch: false,
            base_branch_name: None,
            gh_cli_available: true,
            ai_enabled: false,
            agent_id: String::new(),
            model: String::new(),
            reasoning: String::new(),
            version: String::new(),
            version_options: Vec::new(),
            mode: "normal".to_string(),
            resume_session_id: None,
            branch_name: String::new(),
            issue_id: String::new(),
            skip_perms: false,
            codex_fast_mode: false,
            convert_source_agents: Vec::new(),
            convert_sessions: Vec::new(),
            ai_suggest: AISuggestState::default(),
            completed: false,
            cancelled: false,
            spec_context: None,
            worktree_path: None,
        }
    }
}

impl WizardState {
    fn flow_start_step(&self) -> WizardStep {
        if self.is_new_branch {
            WizardStep::BranchTypeSelect
        } else if self.has_quick_start && !self.quick_start_entries.is_empty() {
            WizardStep::QuickStart
        } else {
            WizardStep::BranchAction
        }
    }

    fn quick_start_option_count(&self) -> usize {
        if self.quick_start_entries.is_empty() {
            0
        } else {
            self.quick_start_entries.len() * 2 + 1
        }
    }

    fn selected_quick_start_action(&self) -> QuickStartAction {
        let choose_different_index = self.quick_start_entries.len() * 2;
        if self.quick_start_entries.is_empty() || self.selected >= choose_different_index {
            QuickStartAction::ChooseDifferent
        } else if self.selected.is_multiple_of(2) {
            QuickStartAction::ResumeWithPrevious
        } else {
            QuickStartAction::StartNewWithPrevious
        }
    }

    fn selected_quick_start_entry(&self) -> Option<&QuickStartEntry> {
        if self.quick_start_entries.is_empty() {
            None
        } else {
            self.quick_start_entries.get(self.selected / 2)
        }
    }

    fn apply_quick_start_selection(&mut self) {
        let action = self.selected_quick_start_action();
        let Some(entry) = self.selected_quick_start_entry().cloned() else {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
            return;
        };

        self.agent_id = entry.agent_id.clone();
        if let Some(agent_index) = self
            .detected_agents
            .iter()
            .position(|agent| agent.id == entry.agent_id)
        {
            self.selected = agent_index;
            self.sync_selected_agent_options();
        }

        if let Some(model) = entry.model {
            self.model = model;
        }
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        } else if entry.agent_id == "claude" {
            self.reasoning = "auto".to_string();
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_perms = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode;
        if !self.agent_is_codex() {
            self.codex_fast_mode = false;
        }
        self.sync_reasoning_state();

        match action {
            QuickStartAction::ResumeWithPrevious => {
                if let Some(session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(session_id);
                } else {
                    self.mode = "continue".to_string();
                    self.resume_session_id = None;
                }
            }
            QuickStartAction::StartNewWithPrevious => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
            }
            QuickStartAction::ChooseDifferent => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
            }
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

        if !self.agent_is_codex() {
            self.codex_fast_mode = false;
        }
        self.sync_reasoning_state();
    }

    fn reasoning_step_kind(&self) -> ReasoningStepKind {
        if self.agent_is_codex() {
            return ReasoningStepKind::Codex;
        }

        if self.effective_agent_id() != "claude" {
            return ReasoningStepKind::None;
        }

        match claude_model_family(&self.model) {
            Some(ClaudeModelFamily::Opus) => ReasoningStepKind::ClaudeEffort {
                max_available: true,
            },
            Some(ClaudeModelFamily::Sonnet) => ReasoningStepKind::ClaudeEffort {
                max_available: false,
            },
            _ => ReasoningStepKind::None,
        }
    }

    pub fn agent_uses_reasoning_step(&self) -> bool {
        !matches!(self.reasoning_step_kind(), ReasoningStepKind::None)
    }

    fn current_reasoning_options(&self) -> &'static [ReasoningDisplayOption] {
        match self.reasoning_step_kind() {
            ReasoningStepKind::Codex => &CODEX_REASONING_DISPLAY_OPTIONS,
            ReasoningStepKind::ClaudeEffort {
                max_available: true,
            } => &CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS,
            ReasoningStepKind::ClaudeEffort {
                max_available: false,
            } => &CLAUDE_SONNET_EFFORT_DISPLAY_OPTIONS,
            ReasoningStepKind::None => &[],
        }
    }

    fn current_reasoning_value(&self) -> Option<&str> {
        let value = self.reasoning.as_str();
        if value.is_empty() {
            return None;
        }
        self.current_reasoning_options()
            .iter()
            .find(|option| option.stored_value == value)
            .map(|option| option.stored_value)
    }

    fn sync_reasoning_state(&mut self) {
        if self.current_reasoning_value().is_none() {
            self.reasoning.clear();
        }
    }

    fn reasoning_title(&self) -> String {
        let model = reasoning_title_model(self);
        match self.reasoning_step_kind() {
            ReasoningStepKind::Codex => format!("Select Reasoning Level for {model}"),
            ReasoningStepKind::ClaudeEffort { .. } => {
                format!("Select Effort Level for {model}")
            }
            ReasoningStepKind::None => WizardStep::ReasoningLevel.title().to_string(),
        }
    }

    /// Number of selectable options for the current step.
    pub fn option_count(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => self.quick_start_option_count(),
            WizardStep::BranchAction => 2, // existing branch / create new branch
            WizardStep::AgentSelect => self.detected_agents.len().max(1),
            WizardStep::ModelSelect => self.current_model_options().len(),
            WizardStep::ReasoningLevel => self.current_reasoning_options().len(),
            WizardStep::VersionSelect => self.version_options.len().max(1),
            WizardStep::ExecutionMode => 4, // normal, continue, resume, convert
            WizardStep::ConvertAgentSelect => self.convert_source_agents.len().max(1),
            WizardStep::ConvertSessionSelect => self.convert_sessions.len().max(1),
            WizardStep::BranchTypeSelect => 4, // feature, bugfix, hotfix, release
            WizardStep::BranchNameInput => 0,  // text input, no list
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
            WizardStep::CodexFastMode => 2,   // on / off
        }
    }

    /// Static option labels for the current step.
    pub fn current_static_options(&self) -> Vec<&'static str> {
        match self.step {
            WizardStep::BranchAction => vec!["Use selected branch", "Create new from selected"],
            WizardStep::ReasoningLevel => self
                .current_reasoning_options()
                .iter()
                .map(|option| option.label)
                .collect(),
            WizardStep::ExecutionMode => vec!["Normal", "Continue", "Resume", "Convert"],
            WizardStep::BranchTypeSelect => vec!["feature/", "bugfix/", "hotfix/", "release/"],
            WizardStep::SkipPermissions => vec!["Yes", "No"],
            WizardStep::CodexFastMode => vec!["On", "Off"],
            _ => vec![],
        }
    }

    /// Options as string labels for the current step.
    pub fn current_options(&self) -> Vec<String> {
        match self.step {
            WizardStep::QuickStart => {
                let mut options = Vec::with_capacity(self.quick_start_option_count());
                let single_entry = self.quick_start_entries.len() == 1;
                for (entry_index, entry) in self.quick_start_entries.iter().enumerate() {
                    let resume_index = entry_index * 2;
                    let show_resume_hint = single_entry || self.selected == resume_index;
                    options.push(quick_start_action_label(
                        entry,
                        "Resume",
                        show_resume_hint,
                        !single_entry,
                    ));
                    options.push(quick_start_action_label(entry, "Start new", false, false));
                }
                if !self.quick_start_entries.is_empty() {
                    options.push("Choose different".to_string());
                }
                options
            }
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
            WizardStep::ConvertAgentSelect => {
                if self.convert_source_agents.is_empty() {
                    vec!["(no source agents available)".to_string()]
                } else {
                    self.convert_source_agents.clone()
                }
            }
            WizardStep::ConvertSessionSelect => {
                if self.convert_sessions.is_empty() {
                    vec!["(no sessions available)".to_string()]
                } else {
                    self.convert_sessions.clone()
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
        self.spec_context
            .as_ref()
            .and_then(SpecContext::branch_seed)
    }
}

fn derive_spec_branch_seed(spec_id: &str, title: &str) -> String {
    let mut suffix = slugify_branch_component(spec_id);
    if !title.trim().is_empty() {
        let title = slugify_branch_component(title);
        if !title.is_empty() {
            suffix.push('-');
            suffix.push_str(&title);
        }
    }
    format!("feature/{suffix}")
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
                    state.selected = step_default_selection(next, state);
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
                state.selected = step_default_selection(prev, state);
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

fn step_default_selection(step: WizardStep, state: &WizardState) -> usize {
    match step {
        WizardStep::ReasoningLevel => reasoning_default_selection(state),
        WizardStep::SkipPermissions => usize::from(!state.skip_perms),
        WizardStep::CodexFastMode => usize::from(!state.codex_fast_mode),
        _ => 0,
    }
}

/// Apply the current selection to config fields.
fn apply_selection(state: &mut WizardState) {
    let options = state.current_options();
    match state.step {
        WizardStep::QuickStart => {
            if !matches!(
                state.selected_quick_start_action(),
                QuickStartAction::ChooseDifferent
            ) {
                state.apply_quick_start_selection();
            }
        }
        WizardStep::BranchAction => {
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
            if let Some(opt) = state.current_reasoning_options().get(state.selected) {
                state.reasoning = opt.stored_value.to_string();
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
        WizardStep::ConvertAgentSelect => {}
        WizardStep::ConvertSessionSelect => {}
        WizardStep::AIBranchSuggest => {
            apply_selected_ai_suggestion(state);
        }
        WizardStep::SkipPermissions => {
            state.skip_perms = state.selected == 0;
        }
        WizardStep::CodexFastMode => {
            state.codex_fast_mode = state.selected == 0;
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
            "gpt-5.4".to_string(),
            "gpt-5.4-mini".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.3-codex-spark".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5.2".to_string(),
            "gpt-5.1-codex-max".to_string(),
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

#[derive(Clone, Copy)]
struct ModelDisplayOption {
    label: &'static str,
    description: &'static str,
}

const CLAUDE_MODEL_DISPLAY_OPTIONS: [ModelDisplayOption; 4] = [
    ModelDisplayOption {
        label: "Default (recommended)",
        description: "Opus 4.6 - Most capable for complex work",
    },
    ModelDisplayOption {
        label: "Opus 4.6",
        description: "Most capable for complex work",
    },
    ModelDisplayOption {
        label: "Sonnet 4.6",
        description: "Best for everyday tasks",
    },
    ModelDisplayOption {
        label: "Haiku 4.5",
        description: "Fastest for quick answers",
    },
];

const CODEX_MODEL_DISPLAY_OPTIONS: [ModelDisplayOption; 9] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Codex default model",
    },
    ModelDisplayOption {
        label: "gpt-5.4",
        description: "Latest frontier agentic coding model.",
    },
    ModelDisplayOption {
        label: "gpt-5.4-mini",
        description: "Smaller frontier agentic coding model.",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex",
        description: "Frontier Codex-optimized agentic coding model.",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex-spark",
        description: "Ultra-fast coding model.",
    },
    ModelDisplayOption {
        label: "gpt-5.2-codex",
        description: "Frontier agentic coding model.",
    },
    ModelDisplayOption {
        label: "gpt-5.2",
        description: "Optimized for professional work and long-running agents.",
    },
    ModelDisplayOption {
        label: "gpt-5.1-codex-max",
        description: "Codex-optimized model for deep and fast reasoning.",
    },
    ModelDisplayOption {
        label: "gpt-5.1-codex-mini",
        description: "Optimized for codex. Cheaper, faster, but less capable.",
    },
];

const GEMINI_MODEL_DISPLAY_OPTIONS: [ModelDisplayOption; 6] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Gemini default model",
    },
    ModelDisplayOption {
        label: "Pro (gemini-3-pro-preview)",
        description: "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable.",
    },
    ModelDisplayOption {
        label: "Flash (gemini-3-flash-preview)",
        description: "Next-generation high-speed model",
    },
    ModelDisplayOption {
        label: "Pro (gemini-2.5-pro)",
        description: "Stable Pro model for deep reasoning and creativity",
    },
    ModelDisplayOption {
        label: "Flash (gemini-2.5-flash)",
        description: "Balance of speed and reasoning",
    },
    ModelDisplayOption {
        label: "Flash-Lite (gemini-2.5-flash-lite)",
        description: "Fastest for simple tasks",
    },
];

#[derive(Clone, Copy)]
struct ReasoningDisplayOption {
    label: &'static str,
    stored_value: &'static str,
    description: &'static str,
    is_default: bool,
}

const CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS: [ReasoningDisplayOption; 5] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Let the model decide how deeply to think",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast, cheap responses for simple renames, greps, and quick questions",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balanced reasoning for everyday agentic coding and tool-heavy work",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Deeper reasoning for complex problems",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Deepest reasoning with no token-spending constraint",
        is_default: false,
    },
];

const CLAUDE_SONNET_EFFORT_DISPLAY_OPTIONS: [ReasoningDisplayOption; 4] = [
    CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS[0],
    CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS[1],
    CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS[2],
    CLAUDE_OPUS_EFFORT_DISPLAY_OPTIONS[3],
];

const CODEX_REASONING_DISPLAY_OPTIONS: [ReasoningDisplayOption; 4] = [
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses with lighter reasoning",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balances speed and reasoning depth for everyday tasks",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Greater reasoning depth for complex problems",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Extra high",
        stored_value: "xhigh",
        description: "Extra high reasoning depth for complex problems",
        is_default: false,
    },
];

const EXECUTION_MODE_DISPLAY_OPTIONS: [(&str, &str); 4] = [
    ("Normal", "Start a new session"),
    ("Continue", "Continue from last session"),
    ("Resume", "Resume a specific session"),
    ("Convert", "Convert session from another agent"),
];

const SKIP_PERMISSION_DISPLAY_OPTIONS: [(&str, &str); 2] = [
    ("Yes", "Skip permission prompts"),
    ("No", "Show permission prompts"),
];

const CODEX_FAST_MODE_DISPLAY_OPTIONS: [(&str, &str); 2] = [
    ("On", "Use service_tier=fast"),
    ("Off", "Use default service tier"),
];

fn model_display_options(agent_id: &str) -> &'static [ModelDisplayOption] {
    match agent_id {
        "claude" => &CLAUDE_MODEL_DISPLAY_OPTIONS,
        "codex" => &CODEX_MODEL_DISPLAY_OPTIONS,
        "gemini" => &GEMINI_MODEL_DISPLAY_OPTIONS,
        _ => &[],
    }
}

fn claude_model_family(model: &str) -> Option<ClaudeModelFamily> {
    match model {
        "Default (Opus 4.6)" | "opus" => Some(ClaudeModelFamily::Opus),
        "sonnet" => Some(ClaudeModelFamily::Sonnet),
        "haiku" => Some(ClaudeModelFamily::Haiku),
        _ => None,
    }
}

fn reasoning_title_model(state: &WizardState) -> &str {
    match claude_model_family(&state.model) {
        Some(ClaudeModelFamily::Opus) => "opus",
        Some(ClaudeModelFamily::Sonnet) => "sonnet",
        Some(ClaudeModelFamily::Haiku) => "haiku",
        None if state.model.is_empty() => "model",
        None => state.model.as_str(),
    }
}

fn reasoning_label(option: &ReasoningDisplayOption, is_current: bool) -> String {
    let mut label = option.label.to_string();
    if option.is_default {
        label.push_str(" (default)");
    }
    if is_current {
        label.push_str(" (current)");
    }
    label
}

fn reasoning_default_selection(state: &WizardState) -> usize {
    if let Some(value) = state.current_reasoning_value() {
        return state
            .current_reasoning_options()
            .iter()
            .position(|option| option.stored_value == value)
            .unwrap_or(0);
    }

    state
        .current_reasoning_options()
        .iter()
        .position(|option| match state.reasoning_step_kind() {
            ReasoningStepKind::Codex => option.stored_value == "medium",
            ReasoningStepKind::ClaudeEffort { .. } => option.stored_value == "low",
            ReasoningStepKind::None => false,
        })
        .unwrap_or(0)
}

fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return "...".chars().take(max_width).collect();
    }

    let mut truncated = String::with_capacity(max_width);
    for ch in text.chars().take(max_width - 3) {
        truncated.push(ch);
    }
    truncated.push_str("...");
    truncated
}

fn format_label_description_line(
    marker: &str,
    label: &str,
    description: &str,
    available_width: usize,
    label_width_cap: usize,
) -> String {
    if description.is_empty() {
        return truncate_with_ellipsis(&format!("{marker}{label}"), available_width);
    }

    let separator = " - ";
    let label_width = label.chars().count().min(label_width_cap);
    let max_desc_width =
        available_width.saturating_sub(marker.chars().count() + label_width + separator.len());

    let rendered_desc = if max_desc_width == 0 {
        String::new()
    } else if description.chars().count() > max_desc_width {
        truncate_with_ellipsis(description, max_desc_width)
    } else {
        description.to_string()
    };

    if rendered_desc.is_empty() {
        truncate_with_ellipsis(&format!("{marker}{label}"), available_width)
    } else {
        truncate_with_ellipsis(
            &format!("{marker}{label}{separator}{rendered_desc}"),
            available_width,
        )
    }
}

fn format_fixed_width_line(
    marker: &str,
    label: &str,
    description: &str,
    label_width: usize,
    available_width: usize,
) -> String {
    truncate_with_ellipsis(
        &format!("{marker}{label:<label_width$} {description}"),
        available_width,
    )
}

fn render_list_content(frame: &mut Frame, area: Rect, items: Vec<ListItem>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(List::new(items), area);
}

fn wizard_row_style(is_selected: bool) -> Style {
    wizard_row_style_with_fg(is_selected, theme::color::TEXT_PRIMARY)
}

fn wizard_row_style_with_fg(is_selected: bool, fg: Color) -> Style {
    if is_selected {
        Style::default().bg(theme::color::FOCUS).fg(Color::Black)
    } else {
        Style::default().fg(fg)
    }
}

fn version_option_description(option: &VersionOption) -> &'static str {
    match option.value.as_str() {
        "installed" => "Use installed version",
        "latest" => "Always use the latest version",
        _ => "Use cached version",
    }
}

fn render_model_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let available_width = area.width as usize;
    let display_options = model_display_options(state.effective_agent_id());
    let fallback_options = state.current_model_options();
    let items = if display_options.is_empty() {
        fallback_options
            .iter()
            .enumerate()
            .map(|(idx, label)| {
                let marker = if idx == state.selected { "> " } else { "  " };
                ListItem::new(truncate_with_ellipsis(
                    &format!("{marker}{label}"),
                    available_width,
                ))
                .style(wizard_row_style(idx == state.selected))
            })
            .collect()
    } else {
        display_options
            .iter()
            .enumerate()
            .map(|(idx, option)| {
                let marker = if idx == state.selected { "> " } else { "  " };
                let text = format_label_description_line(
                    marker,
                    option.label,
                    option.description,
                    available_width,
                    25,
                );
                ListItem::new(text).style(wizard_row_style(idx == state.selected))
            })
            .collect()
    };
    render_list_content(frame, area, items);
}

fn render_reasoning_level_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let available_width = area.width as usize;
    let label_width = state
        .current_reasoning_options()
        .iter()
        .enumerate()
        .map(|(idx, option)| {
            reasoning_label(option, idx == state.selected)
                .chars()
                .count()
        })
        .max()
        .unwrap_or(10);
    let items = state
        .current_reasoning_options()
        .iter()
        .enumerate()
        .map(|(idx, option)| {
            let marker = if idx == state.selected { "> " } else { "  " };
            let text = format_fixed_width_line(
                marker,
                &reasoning_label(option, idx == state.selected),
                option.description,
                label_width,
                available_width,
            );
            ListItem::new(text).style(wizard_row_style(idx == state.selected))
        })
        .collect();
    render_list_content(frame, area, items);
}

fn render_execution_mode_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let available_width = area.width as usize;
    let items = EXECUTION_MODE_DISPLAY_OPTIONS
        .iter()
        .enumerate()
        .map(|(idx, (label, description))| {
            let marker = if idx == state.selected { "> " } else { "  " };
            let text = format_fixed_width_line(marker, label, description, 12, available_width);
            ListItem::new(text).style(wizard_row_style(idx == state.selected))
        })
        .collect();
    render_list_content(frame, area, items);
}

fn render_skip_permissions_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let available_width = area.width as usize;
    let items = SKIP_PERMISSION_DISPLAY_OPTIONS
        .iter()
        .enumerate()
        .map(|(idx, (label, description))| {
            let marker = if idx == state.selected { "> " } else { "  " };
            let text = format_fixed_width_line(marker, label, description, 6, available_width);
            ListItem::new(text).style(wizard_row_style(idx == state.selected))
        })
        .collect();
    render_list_content(frame, area, items);
}

fn render_codex_fast_mode_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let available_width = area.width as usize;
    let items = CODEX_FAST_MODE_DISPLAY_OPTIONS
        .iter()
        .enumerate()
        .map(|(idx, (label, description))| {
            let marker = if idx == state.selected { "> " } else { "  " };
            let text = format_fixed_width_line(marker, label, description, 6, available_width);
            ListItem::new(text).style(wizard_row_style(idx == state.selected))
        })
        .collect();
    render_list_content(frame, area, items);
}

fn render_version_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 || state.version_options.is_empty() {
        return;
    }

    let total = state.version_options.len();
    let max_rows = area.height as usize;
    let (start, list_rows) = if total > max_rows {
        let visible_rows = max_rows.saturating_sub(2).max(1);
        let mut start = state.selected.saturating_sub(visible_rows / 2);
        if start + visible_rows > total {
            start = total.saturating_sub(visible_rows);
        }
        (start, visible_rows)
    } else {
        (0, total)
    };
    let end = (start + list_rows).min(total);
    let has_more_above = start > 0;
    let has_more_below = end < total;
    let available_width = area.width as usize;

    let mut y = area.y;
    if has_more_above {
        frame.render_widget(
            Paragraph::new("  ^ more above ^").style(theme::style::muted_text()),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }

    let items = state.version_options[start..end]
        .iter()
        .enumerate()
        .map(|(offset, option)| {
            let idx = start + offset;
            let marker = if idx == state.selected { "> " } else { "  " };
            let text = format_label_description_line(
                marker,
                &option.label,
                version_option_description(option),
                available_width,
                20,
            );
            ListItem::new(text).style(wizard_row_style(idx == state.selected))
        })
        .collect::<Vec<_>>();

    let list_height = area
        .height
        .saturating_sub(has_more_above as u16 + has_more_below as u16);
    frame.render_widget(
        List::new(items),
        Rect::new(area.x, y, area.width, list_height),
    );

    if has_more_below {
        frame.render_widget(
            Paragraph::new("  v more below v").style(theme::style::muted_text()),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
}

fn quick_start_agent_color(agent_id: &str) -> Color {
    match agent_id {
        "claude" => theme::color::ACTIVE,
        "codex" => theme::color::FOCUS,
        "gemini" => theme::color::ACCENT,
        "opencode" => theme::color::SUCCESS,
        _ => theme::color::TEXT_PRIMARY,
    }
}

fn agent_row_color(agent_id: &str) -> Color {
    match agent_id {
        "claude" => theme::color::ACTIVE,
        "codex" => theme::color::FOCUS,
        "gemini" => theme::color::ACCENT,
        "opencode" => theme::color::SUCCESS,
        "gh" => theme::color::AGENT,
        _ => theme::color::TEXT_PRIMARY,
    }
}

fn quick_start_title_summary(entry: &QuickStartEntry) -> String {
    match entry.model.as_deref() {
        Some(model) => format!("{} ({})", entry.tool_label, model),
        None => entry.tool_label.clone(),
    }
}

fn quick_start_action_label(
    entry: &QuickStartEntry,
    action_label: &str,
    show_resume_hint: bool,
    include_agent_label: bool,
) -> String {
    let mut label = String::new();
    if include_agent_label {
        label.push_str(&entry.tool_label);
        label.push(' ');
    }
    label.push_str(action_label);
    if action_label == "Resume" && show_resume_hint {
        if let Some(session_id) = &entry.resume_session_id {
            label.push_str(&format!(" ({}...)", &session_id[..session_id.len().min(8)]));
        }
    }
    label
}

fn quick_start_start_new_marker(is_selected: bool, single_entry: bool) -> &'static str {
    if single_entry {
        if is_selected {
            "> "
        } else {
            "  "
        }
    } else if is_selected {
        ">   "
    } else {
        "    "
    }
}

/// Compute popup width based on actual content (title + options + hints).
///
/// Mirrors old-TUI `wizard_popup_width`: measure the widest content line,
/// add border + padding, then clamp to [min_width, max_width].
fn wizard_popup_width(state: &WizardState, max_width: u16) -> u16 {
    let min_width = 40u16.min(max_width);

    let mut max_line = 0usize;

    // Title + [ESC] badge
    let title_len = popup_title(state).len() + " [ESC] ".len() + 4;
    max_line = max_line.max(title_len);

    // Measure rendered content width per step type.
    // Fixed-width steps use `format_fixed_width_line` with a column width;
    // version and model steps use label-description pairs.
    match state.step {
        WizardStep::ExecutionMode => {
            for &(label, desc) in &EXECUTION_MODE_DISPLAY_OPTIONS {
                // marker(2) + max(label_width, label.len)(12) + space(1) + description
                max_line = max_line.max(2 + 12.max(label.len()) + 1 + desc.len());
            }
        }
        WizardStep::ReasoningLevel => {
            let label_width = state
                .current_reasoning_options()
                .iter()
                .enumerate()
                .map(|(idx, option)| reasoning_label(option, idx == state.selected).len())
                .max()
                .unwrap_or(10);
            for (idx, option) in state.current_reasoning_options().iter().enumerate() {
                let label = reasoning_label(option, idx == state.selected);
                max_line =
                    max_line.max(2 + label_width.max(label.len()) + 1 + option.description.len());
            }
        }
        WizardStep::SkipPermissions => {
            for &(label, desc) in &SKIP_PERMISSION_DISPLAY_OPTIONS {
                max_line = max_line.max(2 + 6.max(label.len()) + 1 + desc.len());
            }
        }
        WizardStep::ModelSelect => {
            for opt in model_display_options(&state.agent_id) {
                // marker(4) + label + " - " + description
                max_line = max_line.max(4 + opt.label.len() + 3 + opt.description.len());
            }
        }
        WizardStep::VersionSelect => {
            for opt in &state.version_options {
                // marker(2) + label + " - " + description (description from render)
                // Version render uses format_label_description_line which adds description
                max_line = max_line.max(2 + opt.label.len() + 30);
            }
        }
        _ => {
            for opt in state.current_options() {
                max_line = max_line.max(opt.len() + 4);
            }
        }
    }

    // Hint line width
    let hint_len = match state.step {
        WizardStep::SkipPermissions if state.agent_is_codex() => {
            " Up/Down: select | Enter: next | Esc: back".len()
        }
        WizardStep::SkipPermissions => "[Enter] Launch  [Esc] Back  [Up/Down] Navigate".len(),
        WizardStep::CodexFastMode => " Up/Down: select | Enter: launch | Esc: back".len(),
        _ => "[Enter] Select  [Esc] Back  [Up/Down] Navigate".len(),
    };
    max_line = max_line.max(hint_len);

    // borders(2) + H_PAD on each side (2*2=4)
    let desired = (max_line as u16).saturating_add(2 + 4);
    desired.max(min_width).min(max_width)
}

fn popup_title(state: &WizardState) -> String {
    if state.step == WizardStep::QuickStart && state.quick_start_entries.len() == 1 {
        format!(
            "{} — {}",
            state.step.title(),
            quick_start_title_summary(&state.quick_start_entries[0])
        )
    } else if state.step == WizardStep::ReasoningLevel {
        state.reasoning_title()
    } else {
        state.step.title().to_string()
    }
}

fn render_quick_start_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(
        Paragraph::new(truncate_with_ellipsis(
            &state.branch_name,
            area.width as usize,
        ))
        .style(theme::style::header()),
        Rect::new(area.x, area.y, area.width, 1),
    );

    if area.height <= 1 {
        return;
    }

    let list_area = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    let mut items = Vec::new();
    let single_entry = state.quick_start_entries.len() == 1;

    for (entry_index, entry) in state.quick_start_entries.iter().enumerate() {
        let resume_index = entry_index * 2;
        let show_resume_hint = single_entry || state.selected == resume_index;
        let resume_text = format!(
            "{}{}",
            if state.selected == resume_index {
                "> "
            } else {
                "  "
            },
            quick_start_action_label(entry, "Resume", show_resume_hint, !single_entry)
        );
        items.push(
            ListItem::new(truncate_with_ellipsis(
                &resume_text,
                list_area.width as usize,
            ))
            .style(if single_entry {
                wizard_row_style(state.selected == resume_index)
            } else {
                wizard_row_style_with_fg(
                    state.selected == resume_index,
                    quick_start_agent_color(&entry.agent_id),
                )
            }),
        );

        let start_new_index = resume_index + 1;
        let start_new_text = format!(
            "{}{}",
            quick_start_start_new_marker(state.selected == start_new_index, single_entry),
            quick_start_action_label(entry, "Start new", false, false)
        );
        items.push(
            ListItem::new(truncate_with_ellipsis(
                &start_new_text,
                list_area.width as usize,
            ))
            .style(wizard_row_style(state.selected == start_new_index)),
        );
    }

    let choose_index = state.quick_start_entries.len() * 2;
    let choose_marker = if state.selected >= choose_index {
        "> "
    } else {
        "  "
    };
    let choose_text = format!("{choose_marker}Choose different");
    items.push(
        ListItem::new(truncate_with_ellipsis(
            &choose_text,
            list_area.width as usize,
        ))
        .style(wizard_row_style(state.selected >= choose_index)),
    );

    frame.render_widget(List::new(items), list_area);
}

fn render_agent_select_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let start_y = if !state.is_new_branch {
        frame.render_widget(
            Paragraph::new(truncate_with_ellipsis(
                &state.branch_name,
                area.width as usize,
            ))
            .style(theme::style::header()),
            Rect::new(area.x, area.y, area.width, 1),
        );
        1
    } else {
        0
    };

    let items = if state.detected_agents.is_empty() {
        vec![ListItem::new(truncate_with_ellipsis(
            "(no agents detected)",
            area.width as usize,
        ))]
    } else {
        state
            .detected_agents
            .iter()
            .enumerate()
            .map(|(idx, agent)| {
                let marker = if idx == state.selected { "> " } else { "  " };
                let style =
                    wizard_row_style_with_fg(idx == state.selected, agent_row_color(&agent.id));
                let text = truncate_with_ellipsis(
                    &format!("{marker}{}", agent.display_label()),
                    area.width as usize,
                );
                ListItem::new(text).style(style)
            })
            .collect::<Vec<_>>()
    };

    let list_area = Rect::new(
        area.x,
        area.y + start_y,
        area.width,
        area.height.saturating_sub(start_y),
    );
    frame.render_widget(List::new(items), list_area);
}

/// Render the wizard overlay.
pub fn render(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Dark overlay background — dims the content behind the modal
    let overlay_bg = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(overlay_bg, area);

    // Content-aware width, 60% height (matches old-TUI sizing)
    let max_width = area.width.saturating_sub(2).max(1);
    let width = wizard_popup_width(state, max_width);
    let height = (area.height * 60 / 100).max(15);
    let popup = super::centered_rect(width, height, area);

    frame.render_widget(Clear, popup);

    // Single outer border wrapping the entire modal
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::color::FOCUS))
        .title_top(
            Line::from(format!(" {} ", popup_title(state))).style(
                Style::default()
                    .fg(theme::color::FOCUS)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .title_top(Line::from(" [ESC] ").right_aligned());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Horizontal padding + vertical margins inside the border
    const H_PAD: u16 = 2;
    let content_area = Rect::new(
        inner.x + H_PAD,
        inner.y + 1,
        inner.width.saturating_sub(H_PAD * 2),
        inner.height.saturating_sub(4), // top margin(1) + bottom gap(2) + footer(1)
    );

    // Content — either a list of options or a text input
    render_step_content(state, frame, content_area);

    // Footer — centered keybind hints at the bottom with margin
    let footer_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(2),
        inner.width,
        1,
    );
    let hint = match state.step {
        WizardStep::BranchNameInput | WizardStep::IssueSelect => "[Enter] Confirm  [Esc] Back",
        WizardStep::AIBranchSuggest if state.ai_suggest.loading => "[Esc] Cancel",
        WizardStep::AIBranchSuggest if state.ai_suggest.error.is_some() => {
            "[Enter] Manual input  [Esc] Retry"
        }
        WizardStep::AIBranchSuggest => "[Enter] Select  [Esc] Back  [Up/Down] Navigate",
        WizardStep::SkipPermissions if state.agent_is_codex() => {
            " Up/Down: select | Enter: next | Esc: back"
        }
        WizardStep::SkipPermissions => "[Enter] Launch  [Esc] Back  [Up/Down] Navigate",
        WizardStep::CodexFastMode => " Up/Down: select | Enter: launch | Esc: back",
        _ => "[Enter] Select  [Esc] Back  [Up/Down] Navigate",
    };
    let footer = Paragraph::new(hint)
        .style(theme::style::muted_text())
        .alignment(Alignment::Center);
    frame.render_widget(footer, footer_area);
}

/// Render the content area for the current wizard step.
fn render_step_content(state: &WizardState, frame: &mut Frame, area: Rect) {
    match state.step {
        WizardStep::QuickStart => render_quick_start_step(state, frame, area),
        WizardStep::AgentSelect => render_agent_select_step(state, frame, area),
        WizardStep::BranchNameInput => {
            render_input_step(state, frame, area, "Branch Name:", &state.branch_name);
        }
        WizardStep::IssueSelect => {
            render_input_step(state, frame, area, "Issue ID (optional):", &state.issue_id);
        }
        WizardStep::AIBranchSuggest => {
            render_ai_suggest(state, frame, area);
        }
        WizardStep::ModelSelect => render_model_step(state, frame, area),
        WizardStep::ReasoningLevel => render_reasoning_level_step(state, frame, area),
        WizardStep::VersionSelect => render_version_step(state, frame, area),
        WizardStep::ExecutionMode => render_execution_mode_step(state, frame, area),
        WizardStep::SkipPermissions => render_skip_permissions_step(state, frame, area),
        WizardStep::CodexFastMode => render_codex_fast_mode_step(state, frame, area),
        _ => {
            render_option_list(state, frame, area);
        }
    }
}

fn render_input_step(
    _state: &WizardState,
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(
        Paragraph::new(label).style(theme::style::header()),
        Rect::new(area.x, area.y, area.width, 1),
    );

    if area.height <= 1 {
        return;
    }

    frame.render_widget(
        Paragraph::new(format!("{value}_")).style(Style::default().fg(theme::color::ACTIVE)),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
}

/// Render a selectable option list for the current wizard step.
fn render_option_list(state: &WizardState, frame: &mut Frame, area: Rect) {
    let options = state.current_options();
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let style = wizard_row_style(idx == state.selected);
            let marker = if idx == state.selected { "> " } else { "  " };
            let marker_style = if idx == state.selected {
                style
            } else {
                Style::default().fg(theme::color::FOCUS)
            };
            let line = Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(opt.clone(), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    render_list_content(frame, area, items);
}

/// Render the AI branch suggestion step.
/// Loading/error states get special treatment; the suggestion list
/// reuses the default option-list renderer via the fallthrough in
/// `render_step_content`.
fn render_ai_suggest(state: &WizardState, frame: &mut Frame, area: Rect) {
    let start_y = if let Some(summary) = state.spec_context_summary() {
        frame.render_widget(
            Paragraph::new(truncate_with_ellipsis(
                &format!("Context: {}", summary),
                area.width as usize,
            ))
            .style(theme::style::header()),
            Rect::new(area.x, area.y, area.width, 1),
        );
        2
    } else {
        0
    };

    let body_area = Rect::new(
        area.x,
        area.y + start_y,
        area.width,
        area.height.saturating_sub(start_y),
    );

    if state.ai_suggest.loading {
        let spinner_chars = [
            '\u{280B}', '\u{2819}', '\u{2838}', '\u{2834}', '\u{2826}', '\u{2807}',
        ];
        let ch = spinner_chars[state.ai_suggest.tick_counter % spinner_chars.len()];
        let text = Paragraph::new(format!(" {} Generating branch name suggestions...", ch))
            .style(Style::default().fg(theme::color::ACTIVE));
        frame.render_widget(text, body_area);
        return;
    }

    if let Some(ref err) = state.ai_suggest.error {
        let text = Paragraph::new(format!(" Error: {}", err))
            .style(Style::default().fg(theme::color::ERROR));
        frame.render_widget(text, body_area);
        return;
    }

    let list_area = Rect::new(
        area.x,
        area.y + start_y,
        area.width,
        area.height.saturating_sub(start_y),
    );

    if list_area.width == 0 || list_area.height == 0 {
        return;
    }

    let options = state.current_options();
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let style = super::list_item_style(idx == state.selected);
            let marker = if idx == state.selected { "> " } else { "  " };
            let line = Line::from(vec![
                Span::styled(marker, Style::default().fg(theme::color::FOCUS)),
                Span::styled(opt.clone(), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    render_list_content(frame, list_area, items);
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

    fn sample_quick_start_entries() -> Vec<QuickStartEntry> {
        vec![
            QuickStartEntry {
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.3-codex".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("latest".to_string()),
                resume_session_id: Some("sess-12345678".to_string()),
                skip_permissions: true,
                codex_fast_mode: true,
            },
            QuickStartEntry {
                agent_id: "claude".to_string(),
                tool_label: "Claude Code".to_string(),
                model: Some("sonnet".to_string()),
                reasoning: None,
                version: Some("1.0.54".to_string()),
                resume_session_id: None,
                skip_permissions: false,
                codex_fast_mode: false,
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

    fn render_buffer(state: &WizardState, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(state, f, area);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn render_text(state: &WizardState, width: u16, height: u16) -> String {
        let buf = render_buffer(state, width, height);
        buffer_text(&buf)
    }

    fn find_text_position(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
        for y in 0..buf.area.height {
            let line = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            if let Some(start) = line.find(needle) {
                return Some((start as u16, y));
            }
        }
        None
    }

    #[test]
    fn default_state() {
        let state = WizardState::default();
        assert_eq!(state.step, WizardStep::BranchAction);
        assert_eq!(state.selected, 0);
        assert!(state.detected_agents.is_empty());
        assert!(!state.completed);
        assert!(!state.cancelled);
    }

    #[test]
    fn step_navigation_next() {
        let state = WizardState::default();
        assert_eq!(state.flow_start_step(), WizardStep::BranchAction);
        assert_eq!(
            next_step(WizardStep::BranchAction, &state),
            Some(WizardStep::AgentSelect)
        );
        assert_eq!(next_step(WizardStep::SkipPermissions, &state), None);
    }

    #[test]
    fn step_navigation_prev() {
        let state = WizardState::default();
        assert_eq!(prev_step(WizardStep::BranchAction, &state), None);
        assert_eq!(
            prev_step(WizardStep::AgentSelect, &state),
            Some(WizardStep::BranchAction)
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
        //        → SkipPermissions → CodexFastMode
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
        assert_eq!(
            next_step(WizardStep::ExecutionMode, &state),
            Some(WizardStep::SkipPermissions)
        );
        assert_eq!(
            next_step(WizardStep::SkipPermissions, &state),
            Some(WizardStep::CodexFastMode)
        );
        assert_eq!(next_step(WizardStep::CodexFastMode, &state), None);
    }

    #[test]
    fn step_transitions_claude_opus_includes_reasoning() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.model = "opus".to_string();
        // Claude Opus: AgentSelect → ModelSelect → ReasoningLevel → VersionSelect → ExecutionMode
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
    fn step_transitions_claude_haiku_skips_reasoning() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.model = "haiku".to_string();

        assert_eq!(
            next_step(WizardStep::ModelSelect, &state),
            Some(WizardStep::VersionSelect)
        );
    }

    #[test]
    fn select_on_quick_start_create_new_branch_enters_branch_type_select() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchAction;
        state.selected = 1;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::BranchTypeSelect);
    }

    #[test]
    fn execution_mode_convert_routes_through_conversion_steps() {
        let mut state = WizardState::default();
        state.mode = "convert".to_string();

        assert_eq!(
            next_step(WizardStep::ExecutionMode, &state),
            Some(WizardStep::ConvertAgentSelect)
        );
        assert_eq!(
            next_step(WizardStep::ConvertAgentSelect, &state),
            Some(WizardStep::ConvertSessionSelect)
        );
        assert_eq!(
            next_step(WizardStep::ConvertSessionSelect, &state),
            Some(WizardStep::SkipPermissions)
        );
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
    fn issue_select_advances_to_branch_name_when_ai_suggest_is_disabled() {
        let mut state = WizardState::default();
        state.ai_enabled = false;

        assert_eq!(
            next_step(WizardStep::IssueSelect, &state),
            Some(WizardStep::BranchNameInput)
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
        // BranchAction has 2 options
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
        assert_eq!(state.step, WizardStep::BranchAction);
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
        state.step = WizardStep::BranchAction;
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

        assert_eq!(state.current_options(), vec!["Claude Code".to_string()]);
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
        state.selected = 3;

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
    fn selecting_claude_haiku_clears_staged_effort() {
        let mut state = WizardState::default();
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.step = WizardStep::ModelSelect;
        state.reasoning = "high".to_string();
        state.selected = 3;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.model, "haiku");
        assert!(state.reasoning.is_empty());
        assert_eq!(state.step, WizardStep::VersionSelect);
    }

    #[test]
    fn select_on_skip_permissions_completes_without_confirm() {
        let mut state = WizardState::default();
        state.step = WizardStep::SkipPermissions;
        state.selected = 0; // "Yes"
        update(&mut state, WizardMessage::Select);
        assert!(state.completed);
        assert_eq!(state.step, WizardStep::SkipPermissions);
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
    fn skip_permissions_for_codex_advances_to_fast_mode_step() {
        let mut state = WizardState::default();
        state.agent_id = "codex".to_string();
        state.codex_fast_mode = false;
        state.step = WizardStep::SkipPermissions;
        state.selected = 1; // "No"

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::CodexFastMode);
        assert_eq!(state.selected, 1);
        assert!(!state.completed);
        assert!(!state.skip_perms);
    }

    #[test]
    fn codex_fast_mode_step_stores_selection_and_completes() {
        let mut state = WizardState::default();
        state.agent_id = "codex".to_string();
        state.step = WizardStep::CodexFastMode;
        state.selected = 0; // "On"

        update(&mut state, WizardMessage::Select);

        assert!(state.completed);
        assert!(state.codex_fast_mode);
    }

    #[test]
    fn codex_fast_mode_step_reentry_preserves_saved_selection() {
        let mut state = WizardState::default();
        state.agent_id = "codex".to_string();
        state.step = WizardStep::SkipPermissions;
        state.selected = 0; // "Yes"
        state.codex_fast_mode = false; // previously saved "Off"

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::CodexFastMode);
        assert_eq!(state.selected, 1); // "Off"
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
    fn quick_start_option_count_tracks_entry_pairs_and_choose_different() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.quick_start_entries = sample_quick_start_entries();

        assert_eq!(state.option_count(), 5);
    }

    #[test]
    fn select_on_quick_start_resume_prefills_previous_settings_and_jumps_to_skip_permissions() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.quick_start_entries = sample_quick_start_entries();
        state.detected_agents = sample_agents();
        state.branch_name = "feature/test".to_string();

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::SkipPermissions);
        assert_eq!(state.agent_id, "codex");
        assert_eq!(state.model, "gpt-5.3-codex");
        assert_eq!(state.reasoning, "high");
        assert_eq!(state.version, "latest");
        assert_eq!(state.mode, "resume");
        assert_eq!(state.resume_session_id.as_deref(), Some("sess-12345678"));
        assert!(state.skip_perms);
        assert!(state.codex_fast_mode);
    }

    #[test]
    fn select_on_quick_start_without_reasoning_treats_legacy_claude_entry_as_auto() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.quick_start_entries = sample_quick_start_entries();
        state.detected_agents = sample_agents();
        state.selected = 2;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.agent_id, "claude");
        assert_eq!(state.model, "sonnet");
        assert_eq!(state.reasoning, "auto");
    }

    #[test]
    fn select_on_quick_start_without_resume_id_falls_back_to_continue() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.quick_start_entries = sample_quick_start_entries();
        state.detected_agents = sample_agents();
        state.selected = 2;

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::SkipPermissions);
        assert_eq!(state.agent_id, "claude");
        assert_eq!(state.mode, "continue");
        assert!(state.resume_session_id.is_none());
        assert!(!state.skip_perms);
        assert!(!state.codex_fast_mode);
    }

    #[test]
    fn select_on_quick_start_claude_restores_skip_permissions() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.detected_agents = sample_agents();
        state.quick_start_entries = vec![QuickStartEntry {
            agent_id: "claude".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("sonnet".to_string()),
            reasoning: None,
            version: Some("latest".to_string()),
            resume_session_id: Some("sess-claude".to_string()),
            skip_permissions: true,
            codex_fast_mode: false,
        }];

        update(&mut state, WizardMessage::Select);

        assert_eq!(state.step, WizardStep::SkipPermissions);
        assert_eq!(state.agent_id, "claude");
        assert_eq!(state.mode, "resume");
        assert_eq!(state.resume_session_id.as_deref(), Some("sess-claude"));
        assert!(state.skip_perms);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn back_from_branch_action_returns_to_quick_start_when_history_exists() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchAction;
        state.has_quick_start = true;
        state.quick_start_entries = sample_quick_start_entries();

        update(&mut state, WizardMessage::Back);

        assert_eq!(state.step, WizardStep::QuickStart);
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
        assert!(!text.contains("Step 1/"));
        assert!(buf.area.width > 0);
    }

    #[test]
    fn render_popup_chrome_omits_separate_progress_row() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 90, 24);

        assert!(!text.contains("Step 1/"));
        assert!(text.contains("Quick Start"));
        assert!(text.contains("feature/test"));
        assert!(!text.contains("Branch: feature/test"));
    }

    #[test]
    fn render_popup_chrome_shows_step_title_and_esc_hint() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.detected_agents = sample_agents();

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Select Coding Agent"));
        assert!(text.contains("[ESC]"));
        assert!(!text.contains("Agent Launch Wizard"));
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
    fn render_branch_input_uses_old_tui_two_row_layout() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchNameInput;
        state.branch_name = "feature/test".to_string();

        let buf = render_buffer(&state, 90, 24);
        let text = buffer_text(&buf);
        let (_, prompt_y) = find_text_position(&buf, "Branch Name:").expect("prompt line");
        let (_, value_y) = find_text_position(&buf, "feature/test_").expect("value line");

        assert!(text.contains("Branch Name:"));
        assert!(text.contains("feature/test_"));
        assert!(
            value_y > prompt_y,
            "input value should render on a row below the prompt"
        );
        assert!(
            text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1,
            "branch input should reuse the popup chrome instead of adding a second boxed title inside the content area"
        );
    }

    #[test]
    fn render_issue_input_uses_old_tui_two_row_layout() {
        let mut state = WizardState::default();
        state.step = WizardStep::IssueSelect;
        state.issue_id = "1234".to_string();

        let buf = render_buffer(&state, 90, 24);
        let text = buffer_text(&buf);
        let (_, prompt_y) = find_text_position(&buf, "Issue ID (optional):").expect("prompt line");
        let (_, value_y) = find_text_position(&buf, "1234_").expect("value line");

        assert!(text.contains("Issue ID (optional):"));
        assert!(text.contains("1234_"));
        assert!(
            value_y > prompt_y,
            "input value should render on a row below the prompt"
        );
        assert!(
            text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1,
            "issue input should use the same inline prompt style instead of adding another boxed title"
        );
    }

    #[test]
    fn render_skip_permissions_step_does_not_panic() {
        let mut state = WizardState::default();
        state.step = WizardStep::SkipPermissions;
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
    fn render_quick_start_shows_old_tui_grouped_history_layout() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("Quick Start"));
        assert!(text.contains("feature/test"));
        assert!(!text.contains("Branch: feature/test"));
        assert!(text.contains("> Codex Resume (sess-123...)"));
        assert!(text.contains("  Start new"));
        assert!(text.contains("  Claude Code Resume"));
        assert!(!text.contains("  Claude Code Start new"));
        assert!(text.contains("Choose different"));
    }

    #[test]
    fn render_quick_start_single_entry_moves_agent_summary_into_title() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![sample_quick_start_entries().into_iter().next().unwrap()];

        let text = render_text(&state, 100, 24);

        assert!(text.contains("Quick Start — Codex (gpt-5.3-codex)"));
        assert!(!text.contains("Reasoning: high"));
        assert_eq!(
            text.match_indices("Codex (gpt-5.3-codex)").count(),
            1,
            "single-entry quick start should show the agent summary only in the popup title"
        );
    }

    #[test]
    fn render_quick_start_single_entry_omits_default_placeholder_from_title() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![QuickStartEntry {
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: None,
            reasoning: Some("high".to_string()),
            version: Some("latest".to_string()),
            resume_session_id: Some("sess-12345678".to_string()),
            skip_permissions: true,
            codex_fast_mode: true,
        }];

        let text = render_text(&state, 100, 24);

        assert!(text.contains("Quick Start — Codex"));
        assert!(!text.contains("Quick Start — Codex (default)"));
    }

    #[test]
    fn render_quick_start_single_entry_starts_actions_directly_below_branch_context() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![sample_quick_start_entries().into_iter().next().unwrap()];

        let buf = render_buffer(&state, 100, 24);
        let (_, branch_y) = find_text_position(&buf, "feature/test").unwrap();
        let (_, resume_y) = find_text_position(&buf, "Resume").unwrap();

        assert_eq!(
            resume_y,
            branch_y + 1,
            "single-entry quick start should place the first action directly below the branch context"
        );
    }

    #[test]
    fn render_quick_start_multi_entry_keeps_generic_title_and_agent_labeled_rows() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("Quick Start"));
        assert!(!text.contains("Quick Start —"));
        assert!(text.contains("Codex Resume"));
        assert!(text.contains("Claude Code Resume"));
    }

    #[test]
    fn render_quick_start_multi_entry_inlines_agent_labels_into_action_rows() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("> Codex Resume (sess-123...)"));
        assert!(text.contains("  Start new"));
        assert!(text.contains("  Claude Code Resume"));
        assert!(!text.contains("  Claude Code Start new"));
    }

    #[test]
    fn render_quick_start_multi_entry_shows_resume_hint_only_for_selected_row() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![
            QuickStartEntry {
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.3-codex".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("latest".to_string()),
                resume_session_id: Some("sess-12345678".to_string()),
                skip_permissions: true,
                codex_fast_mode: true,
            },
            QuickStartEntry {
                agent_id: "claude".to_string(),
                tool_label: "Claude Code".to_string(),
                model: Some("sonnet".to_string()),
                reasoning: None,
                version: Some("1.0.54".to_string()),
                resume_session_id: Some("sess-abcdefgh".to_string()),
                skip_permissions: false,
                codex_fast_mode: false,
            },
        ];
        state.selected = 0;

        let text = render_text(&state, 100, 24);

        assert!(text.contains("> Codex Resume (sess-123...)"));
        assert!(!text.contains("  Claude Code Resume (sess-abc...)"));
        assert!(text.contains("  Claude Code Resume"));
    }

    #[test]
    fn render_quick_start_multi_entry_uses_compact_resume_and_start_new_labels() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("> Codex Resume (sess-123...)"));
        assert!(text.contains("  Start new"));
        assert!(text.contains("  Claude Code Resume"));
        assert!(!text.contains("  Claude Code Start new"));
        assert!(!text.contains("Start new session"));
        assert!(!text.contains("Resume session (sess-123...)"));
    }

    #[test]
    fn render_quick_start_multi_entry_uses_neutral_color_for_plain_start_new_rows() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let buf = render_buffer(&state, 100, 24);
        let (start_new_x, start_new_y) = find_text_position(&buf, "Start new").unwrap();

        assert_eq!(buf[(start_new_x, start_new_y)].fg, Color::White);
    }

    #[test]
    fn render_quick_start_multi_entry_indents_start_new_under_resume_row() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let buf = render_buffer(&state, 100, 24);
        let (resume_x, _) = find_text_position(&buf, "Codex Resume").unwrap();
        let (start_new_x, _) = find_text_position(&buf, "Start new").unwrap();

        assert_eq!(start_new_x, resume_x + 2);
    }

    #[test]
    fn render_quick_start_places_first_group_directly_below_branch_context() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let buf = render_buffer(&state, 100, 24);
        let (_, branch_y) = find_text_position(&buf, "feature/test").unwrap();
        let (_, header_y) = find_text_position(&buf, "Codex Resume").unwrap();

        assert_eq!(
            header_y,
            branch_y + 1,
            "the first quick-start group should start immediately below the branch context"
        );
    }

    #[test]
    fn render_quick_start_uses_compact_branch_context_without_prefix() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("feature/test"));
        assert!(!text.contains("Branch: feature/test"));
    }

    #[test]
    fn render_quick_start_places_next_group_directly_below_previous_entry_block() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let options = state.current_options();

        assert_eq!(options[0], "Codex Resume (sess-123...)");
        assert_eq!(options[1], "Start new");

        let text = render_text(&state, 100, 24);

        assert!(text.contains("Codex Resume"));
        assert!(text.contains("Start new"));
    }

    #[test]
    fn quick_start_current_options_use_compact_labels_for_multi_entry_history() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();
        state.selected = 0;

        let options = state.current_options();

        assert_eq!(options[0], "Codex Resume (sess-123...)");
        assert_eq!(options[1], "Start new");
        assert_eq!(options[2], "Claude Code Resume");
        assert_eq!(options[3], "Start new");
        assert_eq!(options[4], "Choose different");
    }

    #[test]
    fn quick_start_current_options_use_compact_labels_for_single_entry_history() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![sample_quick_start_entries().into_iter().next().unwrap()];

        let options = state.current_options();

        assert_eq!(options[0], "Resume (sess-123...)");
        assert_eq!(options[1], "Start new");
        assert_eq!(options[2], "Choose different");
    }

    #[test]
    fn render_quick_start_single_entry_uses_compact_action_labels() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![sample_quick_start_entries().into_iter().next().unwrap()];

        let text = render_text(&state, 100, 24);

        assert!(text.contains("> Resume (sess-123...)"));
        assert!(text.contains("  Start new"));
        assert!(!text.contains("Resume session"));
        assert!(!text.contains("Start new session"));
    }

    #[test]
    fn render_quick_start_old_tui_action_labels_remain_legible_on_narrow_width() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();

        let buf = render_buffer(&state, 100, 24);
        let (_, start_new_y) = find_text_position(&buf, "Start new").unwrap();
        let (_, next_header_y) = find_text_position(&buf, "Claude Code").unwrap();

        assert_eq!(
            next_header_y,
            start_new_y + 1,
            "the next quick-start group should start immediately below the previous group's actions"
        );

        let text = render_text(&state, 40, 24);
        assert!(text.contains("Resume"));
        assert!(text.contains("Start new"));
    }

    #[test]
    fn render_quick_start_choose_different_follows_last_group_without_separator() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = vec![sample_quick_start_entries().into_iter().next().unwrap()];

        let buf = render_buffer(&state, 100, 24);
        let (_, start_new_y) = find_text_position(&buf, "Start new").unwrap();
        let (_, choose_y) = find_text_position(&buf, "Choose different").unwrap();

        assert_eq!(
            choose_y,
            start_new_y + 1,
            "the final action should follow the last quick-start action without an extra separator row"
        );
    }

    #[test]
    fn render_quick_start_choose_different_uses_label_only_on_wide_width() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();
        state.selected = state.quick_start_entries.len() * 2;

        let text = render_text(&state, 100, 24);

        assert!(text.contains("> Choose different"));
        assert!(!text.contains("Open full setup"));
    }

    #[test]
    fn render_quick_start_choose_different_omits_description_on_narrow_width() {
        let mut state = WizardState::default();
        state.step = WizardStep::QuickStart;
        state.has_quick_start = true;
        state.branch_name = "feature/test".to_string();
        state.quick_start_entries = sample_quick_start_entries();
        state.selected = state.quick_start_entries.len() * 2;

        let text = render_text(&state, 40, 24);

        assert!(text.contains("> Choose different"));
        assert!(!text.contains("Open full setup"));
    }

    #[test]
    fn render_agent_select_for_existing_branch_shows_branch_and_name_only_rows() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.is_new_branch = false;
        state.branch_name = "feature/test".to_string();
        state.detected_agents = sample_agents();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("feature/test"));
        assert!(!text.contains("Branch: feature/test"));
        assert!(text.contains("> Claude Code"));
        assert!(text.contains("  Codex CLI"));
        assert!(text.contains("  Aider"));
        assert!(!text.contains("Installed"));
    }

    #[test]
    fn render_agent_select_without_agents_shows_empty_state_message() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.is_new_branch = false;
        state.branch_name = "feature/test".to_string();

        let text = render_text(&state, 100, 24);

        assert!(text.contains("feature/test"));
        assert!(!text.contains("Branch: feature/test"));
        assert!(text.contains("(no agents detected)"));
    }

    #[test]
    fn render_agent_select_uses_old_tui_selection_and_agent_colors() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.is_new_branch = false;
        state.branch_name = "feature/test".to_string();
        state.detected_agents = sample_agents();
        state.selected = 1;

        let buf = render_buffer(&state, 100, 24);
        let (codex_x, codex_y) = find_text_position(&buf, "Codex CLI").expect("codex row");
        let (claude_x, claude_y) = find_text_position(&buf, "Claude Code").expect("claude row");
        let selected_style = buf[(codex_x, codex_y)].style();
        let unselected_style = buf[(claude_x, claude_y)].style();

        assert_eq!(selected_style.bg, Some(Color::Cyan));
        assert_eq!(selected_style.fg, Some(Color::Black));
        assert_eq!(unselected_style.fg, Some(Color::Yellow));
    }

    #[test]
    fn render_agent_select_places_first_agent_directly_below_compact_branch_context() {
        let mut state = WizardState::default();
        state.step = WizardStep::AgentSelect;
        state.is_new_branch = false;
        state.branch_name = "feature/test".to_string();
        state.detected_agents = sample_agents();

        let buf = render_buffer(&state, 100, 24);
        let (_, branch_y) = find_text_position(&buf, "feature/test").unwrap();
        let (_, agent_y) = find_text_position(&buf, "Claude Code").unwrap();

        assert_eq!(
            agent_y,
            branch_y + 1,
            "existing-branch agent select should place the first agent row directly below the compact branch context"
        );
    }

    #[test]
    fn render_branch_action_reuses_popup_chrome_without_inner_box() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchAction;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Use selected branch"));
        assert!(text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1);
    }

    #[test]
    fn render_branch_action_uses_old_tui_cyan_selected_highlight() {
        let mut state = WizardState::default();
        state.step = WizardStep::BranchAction;
        state.selected = 1;

        let buf = render_buffer(&state, 90, 24);
        let (x, y) = find_text_position(&buf, "Create new from selected").expect("selected row");
        let style = buf[(x, y)].style();

        assert_eq!(style.bg, Some(Color::Cyan));
        assert_eq!(style.fg, Some(Color::Black));
    }

    #[test]
    fn render_model_step_shows_old_tui_label_and_description_layout() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.selected = 1;

        let text = render_text(&state, 160, 24);

        assert!(text.contains("Select Model"));
        assert!(text.contains("Default (recommended) - Opus 4.6 - Most capable for complex work"));
        assert!(text.contains("> Opus 4.6 - Most capable for complex work"));
        assert!(text.contains("  Sonnet 4.6 - Best for everyday tasks"));
        assert!(text.contains("  Haiku 4.5 - Fastest for quick answers"));
        assert!(text.contains("[Enter] Select  [Esc] Back  [Up/Down] Navigate"));
    }

    #[test]
    fn codex_model_options_match_latest_cli_snapshot() {
        assert_eq!(
            default_model_options("codex"),
            vec![
                "Default (Auto)".to_string(),
                "gpt-5.4".to_string(),
                "gpt-5.4-mini".to_string(),
                "gpt-5.3-codex".to_string(),
                "gpt-5.3-codex-spark".to_string(),
                "gpt-5.2-codex".to_string(),
                "gpt-5.2".to_string(),
                "gpt-5.1-codex-max".to_string(),
                "gpt-5.1-codex-mini".to_string(),
            ]
        );

        let display_options = model_display_options("codex");
        let labels = display_options
            .iter()
            .map(|option| option.label)
            .collect::<Vec<_>>();
        let descriptions = display_options
            .iter()
            .map(|option| option.description)
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                "Default (Auto)",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.3-codex-spark",
                "gpt-5.2-codex",
                "gpt-5.2",
                "gpt-5.1-codex-max",
                "gpt-5.1-codex-mini",
            ]
        );
        assert_eq!(
            descriptions,
            vec![
                "Use Codex default model",
                "Latest frontier agentic coding model.",
                "Smaller frontier agentic coding model.",
                "Frontier Codex-optimized agentic coding model.",
                "Ultra-fast coding model.",
                "Frontier agentic coding model.",
                "Optimized for professional work and long-running agents.",
                "Codex-optimized model for deep and fast reasoning.",
                "Optimized for codex. Cheaper, faster, but less capable.",
            ]
        );
    }

    #[test]
    fn render_model_step_for_codex_shows_latest_cli_snapshot() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        state.agent_id = "codex".to_string();
        state.detected_agents = sample_agents();
        state.selected = 1;

        let text = render_text(&state, 180, 24);

        assert!(text.contains("Select Model"));
        assert!(text.contains("Default (Auto) - Use Codex default model"));
        assert!(text.contains("> gpt-5.4 - Latest frontier agentic coding model."));
        assert!(text.contains("  gpt-5.4-mini - Smaller frontier agentic coding model."));
        assert!(text.contains("  gpt-5.3-codex-spark - Ultra-fast coding model."));
        assert!(text.contains(
            "  gpt-5.1-codex-mini - Optimized for codex. Cheaper, faster, but less capable."
        ));
    }

    #[test]
    fn render_model_step_reuses_popup_chrome_without_inner_box() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();

        let text = render_text(&state, 160, 24);

        assert!(text.contains("Default (recommended) - Opus 4.6"));
        assert!(text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1);
    }

    #[test]
    fn render_model_step_uses_old_tui_cyan_selected_highlight() {
        let mut state = WizardState::default();
        state.step = WizardStep::ModelSelect;
        state.agent_id = "claude".to_string();
        state.detected_agents = sample_agents();
        state.selected = 1;

        let buf = render_buffer(&state, 120, 24);
        let (x, y) = find_text_position(&buf, "> Opus 4.6 - Most capable for complex work")
            .expect("selected model row");
        let style = buf[(x, y)].style();

        assert_eq!(style.bg, Some(Color::Cyan));
        assert_eq!(style.fg, Some(Color::Black));
    }

    #[test]
    fn render_codex_reasoning_step_shows_model_title_and_refreshed_labels() {
        let mut state = WizardState::default();
        state.step = WizardStep::ReasoningLevel;
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.4".to_string();
        state.selected = 2;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Select Reasoning Level for gpt-5.4"));
        assert!(text.contains("Low"));
        assert!(text.contains("Fast responses with lighter reasoning"));
        assert!(text.contains("Medium (default)"));
        assert!(text.contains("> High"));
        assert!(text.contains("Extra high"));
    }

    #[test]
    fn render_claude_effort_step_uses_effort_title_and_opus_rows() {
        let mut state = WizardState::default();
        state.step = WizardStep::ReasoningLevel;
        state.agent_id = "claude".to_string();
        state.model = "opus".to_string();
        state.selected = 1;

        let text = render_text(&state, 120, 24);

        assert!(text.contains("Select Effort Level for opus"));
        assert!(text.contains("Auto"));
        assert!(text.contains("> Low"));
        assert!(text.contains("Medium (default)"));
        assert!(text.contains("High"));
        assert!(text.contains("Max"));
    }

    #[test]
    fn render_claude_effort_step_hides_max_for_sonnet() {
        let mut state = WizardState::default();
        state.step = WizardStep::ReasoningLevel;
        state.agent_id = "claude".to_string();
        state.model = "sonnet".to_string();
        state.selected = 1;

        let text = render_text(&state, 120, 24);

        assert!(text.contains("Select Effort Level for sonnet"));
        assert!(!text.contains("Max"));
    }

    #[test]
    fn render_execution_mode_shows_old_tui_descriptions() {
        let mut state = WizardState::default();
        state.step = WizardStep::ExecutionMode;
        state.selected = 2;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Execution Mode"));
        assert!(text.contains("  Normal       Start a new session"));
        assert!(text.contains("  Continue     Continue from last session"));
        assert!(text.contains("> Resume       Resume a specific session"));
        assert!(text.contains("  Convert      Convert session from another agent"));
    }

    #[test]
    fn render_skip_permissions_step_shows_old_tui_descriptions() {
        let mut state = WizardState::default();
        state.step = WizardStep::SkipPermissions;
        state.selected = 1;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Skip Permissions"));
        assert!(text.contains("  Yes    Skip permission prompts"));
        assert!(text.contains("> No     Show permission prompts"));
        assert!(text.contains("[Enter] Launch  [Esc] Back  [Up/Down] Navigate"));
    }

    #[test]
    fn render_skip_permissions_for_codex_shows_next_hint() {
        let mut state = WizardState::default();
        state.step = WizardStep::SkipPermissions;
        state.agent_id = "codex".to_string();
        state.selected = 1;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Skip Permissions"));
        assert!(text.contains("Up/Down: select | Enter: next | Esc: back"));
    }

    #[test]
    fn render_codex_fast_mode_step_shows_old_tui_descriptions() {
        let mut state = WizardState::default();
        state.step = WizardStep::CodexFastMode;
        state.agent_id = "codex".to_string();
        state.selected = 1;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Codex Fast Mode"));
        assert!(text.contains("  On     Use service_tier=fast"));
        assert!(text.contains("> Off    Use default service tier"));
        assert!(text.contains("Up/Down: select | Enter: launch | Esc: back"));
    }

    #[test]
    fn render_version_step_shows_descriptions_and_overflow_indicators() {
        let mut state = WizardState::default();
        state.step = WizardStep::VersionSelect;
        state.selected = 4;
        state.version_options = vec![
            VersionOption {
                label: "Installed (v1.0.0)".to_string(),
                value: "installed".to_string(),
            },
            VersionOption {
                label: "latest".to_string(),
                value: "latest".to_string(),
            },
            VersionOption {
                label: "1.0.1".to_string(),
                value: "1.0.1".to_string(),
            },
            VersionOption {
                label: "1.0.2".to_string(),
                value: "1.0.2".to_string(),
            },
            VersionOption {
                label: "1.0.3".to_string(),
                value: "1.0.3".to_string(),
            },
            VersionOption {
                label: "1.0.4".to_string(),
                value: "1.0.4".to_string(),
            },
            VersionOption {
                label: "1.0.5".to_string(),
                value: "1.0.5".to_string(),
            },
            VersionOption {
                label: "1.0.6".to_string(),
                value: "1.0.6".to_string(),
            },
            VersionOption {
                label: "1.0.7".to_string(),
                value: "1.0.7".to_string(),
            },
            VersionOption {
                label: "1.0.8".to_string(),
                value: "1.0.8".to_string(),
            },
        ];

        let text = render_text(&state, 70, 14);

        assert!(text.contains("Select Version"));
        assert!(text.contains("^ more above ^"));
        assert!(text.contains("v more below v"));
        assert!(text.contains("latest - Always use the latest version"));
        assert!(text.contains("> 1.0.3 - Use cached version"));
    }

    #[test]
    fn render_version_step_reuses_popup_chrome_without_inner_box() {
        let mut state = WizardState::default();
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

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Installed (v1.0.0) - Use installed version"));
        assert!(text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1);
    }

    #[test]
    fn step_titles_non_empty() {
        let all_steps = [
            WizardStep::QuickStart,
            WizardStep::BranchAction,
            WizardStep::AgentSelect,
            WizardStep::ModelSelect,
            WizardStep::ReasoningLevel,
            WizardStep::VersionSelect,
            WizardStep::ExecutionMode,
            WizardStep::ConvertAgentSelect,
            WizardStep::ConvertSessionSelect,
            WizardStep::BranchTypeSelect,
            WizardStep::BranchNameInput,
            WizardStep::AIBranchSuggest,
            WizardStep::IssueSelect,
            WizardStep::SkipPermissions,
            WizardStep::CodexFastMode,
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
        state.ai_enabled = true;
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
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));
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
        assert!(text.contains("Context: SPEC-42"));
        assert!(text.contains("feature/add-auth"));
        assert!(text.contains("feature/user-login"));
        assert!(text.contains("feature/oauth-flow"));
        assert!(text.contains("Manual input"));
        assert!(
            text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1,
            "AI suggestions should reuse the popup chrome instead of adding inner boxes"
        );
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
    fn ai_suggest_loading_reuses_popup_chrome_without_inner_box() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));
        state.ai_suggest.loading = true;

        let buf = render_buffer(&state, 90, 24);
        let text = buffer_text(&buf);
        let (context_x, context_y) =
            find_text_position(&buf, "Context: SPEC-42").expect("context line");
        let (_, loading_y) =
            find_text_position(&buf, "Generating branch name suggestions").expect("loading copy");

        assert!(text.contains("Generating branch name suggestions"));
        assert_eq!(buf[(context_x, context_y)].style().fg, Some(Color::Cyan));
        assert!(
            loading_y > context_y,
            "loading text should render below the context line"
        );
        assert!(text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1);
    }

    #[test]
    fn ai_suggest_loading_uses_compact_body_copy_without_manual_guidance() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));
        state.ai_suggest.loading = true;

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Generating branch name suggestions"));
        assert!(!text.contains("Type Enter to use a manual branch name if needed."));
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
    fn ai_suggest_error_keeps_context_line_above_error_copy() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));
        state.ai_suggest.error = Some("Connection timeout".to_string());

        let buf = render_buffer(&state, 90, 24);
        let text = buffer_text(&buf);
        let (context_x, context_y) =
            find_text_position(&buf, "Context: SPEC-42").expect("context line");
        let (_, error_y) =
            find_text_position(&buf, "Error: Connection timeout").expect("error copy");

        assert_eq!(buf[(context_x, context_y)].style().fg, Some(Color::Cyan));
        assert!(
            error_y > context_y,
            "error copy should render below the context line"
        );
        assert!(text.chars().filter(|c| "╭╔┌┏".contains(*c)).count() == 1);
    }

    #[test]
    fn ai_suggest_error_uses_compact_body_copy_without_manual_guidance() {
        let mut state = WizardState::default();
        state.step = WizardStep::AIBranchSuggest;
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));
        state.ai_suggest.error = Some("Connection timeout".to_string());

        let text = render_text(&state, 90, 24);

        assert!(text.contains("Error: Connection timeout"));
        assert!(!text.contains("Press Enter or Esc to enter branch name manually."));
    }

    #[test]
    fn back_from_step2_goes_to_step1() {
        let mut state = WizardState::default();
        // Advance to step 2 (AgentSelect)
        update(&mut state, WizardMessage::Select);
        assert_eq!(state.step, WizardStep::AgentSelect);

        // Back should return to step 1 (BranchAction)
        update(&mut state, WizardMessage::Back);
        assert_eq!(state.step, WizardStep::BranchAction);
        assert!(!state.cancelled);
    }

    #[test]
    fn cancel_from_step1_sets_cancelled() {
        let mut state = WizardState::default();
        assert_eq!(state.step, WizardStep::BranchAction);

        // Cancel on BranchAction
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
        state.spec_context = Some(SpecContext::new("SPEC-42", "My Feature", ""));

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
