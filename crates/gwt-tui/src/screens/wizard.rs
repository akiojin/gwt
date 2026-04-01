//! 15-step Launch Wizard (gwt-cli wizard.rs migration)
//!
//! Provides a centered overlay popup wizard for agent launch configuration.
//! Steps: QuickStart, BranchAction, AgentSelect, ModelSelect, ReasoningLevel,
//! VersionSelect, CollaborationModes, ExecutionMode, ConvertAgentSelect,
//! ConvertSessionSelect, SkipPermissions, BranchTypeSelect, IssueSelect,
//! AIBranchSuggest, BranchNameInput.

use std::process::Command;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

// ---------------------------------------------------------------------------
// WizardStep — 15-step enum
// ---------------------------------------------------------------------------

/// All wizard steps in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    /// Quick Start: recall previous settings per agent
    QuickStart,
    /// Use selected branch or create new
    BranchAction,
    /// Pick agent (builtin + custom)
    #[default]
    AgentSelect,
    /// Pick model
    ModelSelect,
    /// Codex-only reasoning level
    ReasoningLevel,
    /// npm registry version lookup
    VersionSelect,
    /// Codex collaboration modes
    CollaborationModes,
    /// Normal/Resume/Continue/Convert
    ExecutionMode,
    /// Session conversion: source agent
    ConvertAgentSelect,
    /// Session conversion: pick session
    ConvertSessionSelect,
    /// Boolean skip permissions
    SkipPermissions,
    /// Local vs Remote branch type
    BranchTypeSelect,
    /// GitHub issue linking
    IssueSelect,
    /// AI branch name suggestion
    AIBranchSuggest,
    /// Manual branch name input
    BranchNameInput,
}

impl WizardStep {
    /// Human-readable label for each step.
    pub fn label(&self) -> &'static str {
        match self {
            Self::QuickStart => "Quick Start",
            Self::BranchAction => "Branch Action",
            Self::AgentSelect => "Select Agent",
            Self::ModelSelect => "Select Model",
            Self::ReasoningLevel => "Reasoning Level",
            Self::VersionSelect => "Select Version",
            Self::CollaborationModes => "Collaboration Modes",
            Self::ExecutionMode => "Execution Mode",
            Self::ConvertAgentSelect => "Convert: Select Agent",
            Self::ConvertSessionSelect => "Convert: Select Session",
            Self::SkipPermissions => "Skip Permissions",
            Self::BranchTypeSelect => "Branch Type",
            Self::IssueSelect => "Link Issue",
            Self::AIBranchSuggest => "AI Branch Suggest",
            Self::BranchNameInput => "Branch Name",
        }
    }

    /// 1-based step number for display.
    pub fn number(&self) -> u8 {
        match self {
            Self::QuickStart => 1,
            Self::BranchAction => 2,
            Self::AgentSelect => 3,
            Self::ModelSelect => 4,
            Self::ReasoningLevel => 5,
            Self::VersionSelect => 6,
            Self::CollaborationModes => 7,
            Self::ExecutionMode => 8,
            Self::ConvertAgentSelect => 9,
            Self::ConvertSessionSelect => 10,
            Self::SkipPermissions => 11,
            Self::BranchTypeSelect => 12,
            Self::IssueSelect => 13,
            Self::AIBranchSuggest => 14,
            Self::BranchNameInput => 15,
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Execution mode for agent launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardExecutionMode {
    #[default]
    Normal,
    Continue,
    Resume,
    Convert,
}

impl WizardExecutionMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Continue => "Continue",
            Self::Resume => "Resume",
            Self::Convert => "Convert",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Normal => "Start a new session",
            Self::Continue => "Continue from last session",
            Self::Resume => "Resume a specific session",
            Self::Convert => "Convert session from another agent",
        }
    }

    pub const ALL: [WizardExecutionMode; 4] =
        [Self::Normal, Self::Continue, Self::Resume, Self::Convert];
}

/// Branch type prefix for new branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchType {
    #[default]
    Feature,
    Bugfix,
    Hotfix,
    Release,
}

impl BranchType {
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Feature => "feature/",
            Self::Bugfix => "bugfix/",
            Self::Hotfix => "hotfix/",
            Self::Release => "release/",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Feature => "Feature",
            Self::Bugfix => "Bugfix",
            Self::Hotfix => "Hotfix",
            Self::Release => "Release",
        }
    }

    pub const ALL: [BranchType; 4] = [Self::Feature, Self::Bugfix, Self::Hotfix, Self::Release];
}

/// Quick Start entry for a tool.
#[derive(Debug, Clone)]
pub struct QuickStartEntry {
    pub tool_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub version: Option<String>,
    pub session_id: Option<String>,
    pub skip_permissions: Option<bool>,
    pub branch: String,
}

/// Unified agent entry for display.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub display_name: String,
    pub is_installed: bool,
    pub version: Option<String>,
    pub color: Color,
}

impl AgentEntry {
    /// Create a builtin agent entry.
    pub fn builtin(id: &str, name: &str, color: Color, installed: bool) -> Self {
        Self {
            id: id.to_string(),
            display_name: name.to_string(),
            is_installed: installed,
            version: None,
            color,
        }
    }

    /// Create with a detected version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
}

/// Reasoning level (Codex only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningLevel {
    Low,
    #[default]
    Medium,
    High,
    XHigh,
}

impl ReasoningLevel {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Low => "Faster, less thorough",
            Self::Medium => "Balanced",
            Self::High => "Slower, more thorough",
            Self::XHigh => "Extended high reasoning",
        }
    }

    pub const ALL: [ReasoningLevel; 4] = [Self::Low, Self::Medium, Self::High, Self::XHigh];
}

// ---------------------------------------------------------------------------
// ModelOption — rich model descriptor
// ---------------------------------------------------------------------------

/// Model option with description and reasoning level support.
#[derive(Debug, Clone)]
pub struct ModelOption {
    /// Model identifier (empty string for default).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Human-readable description.
    pub description: String,
    /// Whether this is the default model for the agent.
    pub is_default: bool,
    /// Supported inference/reasoning levels (Codex models).
    pub inference_levels: Vec<ReasoningLevel>,
    /// Default inference level for this model.
    pub default_inference: Option<ReasoningLevel>,
}

impl ModelOption {
    fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            description: description.to_string(),
            is_default: false,
            inference_levels: vec![],
            default_inference: None,
        }
    }

    fn default_option(label: &str, description: &str) -> Self {
        Self {
            id: String::new(),
            label: label.to_string(),
            description: description.to_string(),
            is_default: true,
            inference_levels: vec![],
            default_inference: None,
        }
    }

    fn with_base_levels(mut self) -> Self {
        self.inference_levels = vec![
            ReasoningLevel::High,
            ReasoningLevel::Medium,
            ReasoningLevel::Low,
        ];
        self.default_inference = Some(ReasoningLevel::High);
        self
    }

    fn with_max_levels(mut self) -> Self {
        self.inference_levels = vec![
            ReasoningLevel::XHigh,
            ReasoningLevel::High,
            ReasoningLevel::Medium,
            ReasoningLevel::Low,
        ];
        self.default_inference = Some(ReasoningLevel::Medium);
        self
    }

    fn with_default_inference(mut self, level: ReasoningLevel) -> Self {
        self.default_inference = Some(level);
        self
    }

    /// Display string for list rendering: "label  description".
    pub fn display(&self) -> String {
        format!("{:<28} {}", self.label, self.description)
    }
}

// ---------------------------------------------------------------------------
// CodingAgent — builtin agent enum for model/version detection
// ---------------------------------------------------------------------------

/// Known builtin coding agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodingAgent {
    ClaudeCode,
    CodexCli,
    GeminiCli,
    OpenCode,
}

impl CodingAgent {
    pub const ALL: [CodingAgent; 4] = [
        Self::ClaudeCode,
        Self::CodexCli,
        Self::GeminiCli,
        Self::OpenCode,
    ];

    /// Shell command name for this agent.
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::CodexCli => "codex",
            Self::GeminiCli => "gemini",
            Self::OpenCode => "opencode",
        }
    }

    /// Agent ID used in wizard state.
    pub fn agent_id(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::CodexCli => "codex",
            Self::GeminiCli => "gemini",
            Self::OpenCode => "opencode",
        }
    }

    /// Get model options for this agent.
    pub fn models(&self) -> Vec<ModelOption> {
        match self {
            CodingAgent::ClaudeCode => vec![
                ModelOption::default_option(
                    "Default (recommended)",
                    "Opus 4.6 - Most capable for complex work",
                ),
                ModelOption::new("opus", "Opus 4.6", "Most capable for complex work"),
                ModelOption::new("sonnet", "Sonnet 4.5", "Best for everyday tasks"),
                ModelOption::new("haiku", "Haiku 4.5", "Fastest for quick answers"),
            ],
            CodingAgent::CodexCli => vec![
                ModelOption::default_option("Default (Auto)", "Use Codex default model")
                    .with_base_levels()
                    .with_default_inference(ReasoningLevel::High),
                ModelOption::new(
                    "gpt-5.3-codex",
                    "gpt-5.3-codex",
                    "Latest frontier agentic coding model.",
                )
                .with_max_levels()
                .with_default_inference(ReasoningLevel::High),
                ModelOption::new(
                    "gpt-5.2-codex",
                    "gpt-5.2-codex",
                    "Codex flagship with extra-high reasoning support.",
                )
                .with_max_levels()
                .with_default_inference(ReasoningLevel::High),
                ModelOption::new(
                    "gpt-5.1-codex-max",
                    "gpt-5.1-codex-max",
                    "Codex-optimized flagship for deep and fast reasoning.",
                )
                .with_max_levels(),
                ModelOption::new(
                    "gpt-5.2",
                    "gpt-5.2",
                    "Latest frontier model with improvements across knowledge, reasoning and coding",
                )
                .with_max_levels(),
                ModelOption::new(
                    "gpt-5.1-codex-mini",
                    "gpt-5.1-codex-mini",
                    "Optimized for codex. Cheaper, faster, but less capable.",
                )
                .with_base_levels(),
            ],
            CodingAgent::GeminiCli => vec![
                ModelOption::default_option("Default (Auto)", "Use Gemini default model"),
                ModelOption::new(
                    "gemini-3-pro-preview",
                    "Pro (gemini-3-pro-preview)",
                    "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable.",
                ),
                ModelOption::new(
                    "gemini-3-flash-preview",
                    "Flash (gemini-3-flash-preview)",
                    "Next-generation high-speed model",
                ),
                ModelOption::new(
                    "gemini-2.5-pro",
                    "Pro (gemini-2.5-pro)",
                    "Stable Pro model for deep reasoning and creativity",
                ),
                ModelOption::new(
                    "gemini-2.5-flash",
                    "Flash (gemini-2.5-flash)",
                    "Balance of speed and reasoning",
                ),
                ModelOption::new(
                    "gemini-2.5-flash-lite",
                    "Flash-Lite (gemini-2.5-flash-lite)",
                    "Fastest for simple tasks",
                ),
            ],
            CodingAgent::OpenCode => vec![
                ModelOption::default_option("Default (Auto)", "Use OpenCode default model"),
                ModelOption::new(
                    "__custom__",
                    "Custom (provider/model)",
                    "Enter a provider/model identifier",
                ),
            ],
        }
    }
}

/// Installed version information for an agent.
#[derive(Debug, Clone)]
pub struct InstalledVersionInfo {
    pub version: String,
    pub path: String,
}

/// Detect installed version of an agent using `--version` flag.
pub fn detect_installed_version(agent: CodingAgent) -> Option<InstalledVersionInfo> {
    let cmd_name = agent.command_name();

    // Try to get version using --version flag
    let output = Command::new(cmd_name).arg("--version").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let version_str = String::from_utf8_lossy(&output.stdout);
    // Parse version from output (format varies: "v1.0.3", "1.0.3", "claude 1.0.3", etc.)
    let version = version_str.lines().next().and_then(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        parts.iter().find_map(|p| {
            let v = p.trim_start_matches('v');
            if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                Some(v.to_string())
            } else {
                None
            }
        })
    })?;

    // Try to get path using 'which' command
    let path = Command::new("which")
        .arg(cmd_name)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| cmd_name.to_string());

    Some(InstalledVersionInfo { version, path })
}

/// GitHub issue item for IssueSelect step.
#[derive(Debug, Clone)]
pub struct IssueItem {
    pub number: u64,
    pub title: String,
}

/// Result of wizard confirm action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    /// Advance to the next step.
    Advance,
    /// Wizard completed, ready to launch.
    Complete,
    /// Focus an existing pane (agent already running).
    FocusPane(usize),
    /// Wizard cancelled.
    Cancel,
    /// No action (key consumed but no state change).
    Noop,
}

// ---------------------------------------------------------------------------
// WizardState
// ---------------------------------------------------------------------------

/// Full wizard state.
#[derive(Debug)]
pub struct WizardState {
    /// Current step.
    pub step: WizardStep,
    /// Step history for back navigation.
    pub step_history: Vec<WizardStep>,

    // Agent selection
    pub agents: Vec<AgentEntry>,
    pub selected_agent: usize,

    // Model
    pub model: String,
    pub model_options: Vec<ModelOption>,
    pub model_index: usize,

    // Version
    pub version: String,
    pub version_options: Vec<String>,
    pub version_index: usize,

    // Branch
    pub branch_name: String,
    pub is_new_branch: bool,
    pub branch_type: BranchType,
    pub new_branch_name: String,
    pub cursor: usize,

    // Execution mode
    pub execution_mode: WizardExecutionMode,
    pub execution_mode_index: usize,
    pub session_id: Option<String>,

    // Quick Start
    pub quick_start_entries: Vec<QuickStartEntry>,
    pub quick_start_index: usize,
    pub has_quick_start: bool,

    // Skip permissions
    pub skip_permissions: bool,

    // Reasoning (Codex)
    pub reasoning_level: ReasoningLevel,
    pub reasoning_level_index: usize,

    // Collaboration modes (Codex)
    pub collaboration_modes: bool,

    // Issue linking
    pub issues: Vec<IssueItem>,
    pub issue_index: usize,
    pub issue_search: String,

    // AI branch suggest
    pub ai_description: String,
    pub ai_suggestions: Vec<String>,
    pub ai_selected: usize,
    pub ai_enabled: bool,

    // Session convert
    pub convert_agents: Vec<String>,
    pub convert_agent_index: usize,
    pub convert_sessions: Vec<String>,
    pub convert_session_index: usize,

    // Branch action
    pub branch_action_index: usize,
    pub has_branch_action: bool,

    // Error / loading
    pub error: Option<String>,
    pub loading: bool,
}

impl Default for WizardState {
    fn default() -> Self {
        Self::new()
    }
}

impl WizardState {
    /// Create a new wizard with default state.
    pub fn new() -> Self {
        Self {
            step: WizardStep::AgentSelect,
            step_history: Vec::new(),
            agents: default_agents(),
            selected_agent: 0,
            model: String::new(),
            model_options: CodingAgent::ClaudeCode.models(),
            model_index: 0,
            version: "installed".to_string(),
            version_options: vec!["installed".to_string(), "latest".to_string()],
            version_index: 0,
            branch_name: String::new(),
            is_new_branch: false,
            branch_type: BranchType::default(),
            new_branch_name: String::new(),
            cursor: 0,
            execution_mode: WizardExecutionMode::default(),
            execution_mode_index: 0,
            session_id: None,
            quick_start_entries: Vec::new(),
            quick_start_index: 0,
            has_quick_start: false,
            skip_permissions: false,
            reasoning_level: ReasoningLevel::default(),
            reasoning_level_index: 1, // Medium
            collaboration_modes: false,
            issues: Vec::new(),
            issue_index: 0,
            issue_search: String::new(),
            ai_description: String::new(),
            ai_suggestions: Vec::new(),
            ai_selected: 0,
            ai_enabled: false,
            convert_agents: Vec::new(),
            convert_agent_index: 0,
            convert_sessions: Vec::new(),
            convert_session_index: 0,
            branch_action_index: 0,
            has_branch_action: false,
            error: None,
            loading: false,
        }
    }

    /// Open wizard for an existing branch.
    pub fn open_for_branch(branch_name: &str, history: Vec<QuickStartEntry>) -> Self {
        let mut state = Self::new();
        state.branch_name = branch_name.to_string();
        state.is_new_branch = false;
        state.has_branch_action = true;

        if history.is_empty() {
            state.step = WizardStep::BranchAction;
            state.has_quick_start = false;
        } else {
            state.step = WizardStep::QuickStart;
            state.has_quick_start = true;
            state.quick_start_entries = history;
        }
        state
    }

    /// Open wizard for a new branch.
    pub fn open_for_new_branch() -> Self {
        let mut state = Self::new();
        state.is_new_branch = true;
        state.step = WizardStep::BranchTypeSelect;
        state.has_quick_start = false;
        state.has_branch_action = false;
        state
    }

    // -----------------------------------------------------------------------
    // Agent helpers
    // -----------------------------------------------------------------------

    /// Get the current agent ID.
    pub fn current_agent_id(&self) -> &str {
        self.agents
            .get(self.selected_agent)
            .map(|a| a.id.as_str())
            .unwrap_or("claude")
    }

    /// Whether the current agent is Codex.
    pub fn is_codex(&self) -> bool {
        self.current_agent_id() == "codex"
    }

    // -----------------------------------------------------------------------
    // Step navigation
    // -----------------------------------------------------------------------

    /// Determine the next step based on current state.
    pub fn next_step(&self) -> WizardStep {
        match self.step {
            WizardStep::QuickStart => {
                // If a quick start entry is selected (not "Choose different"),
                // we skip ahead to SkipPermissions (caller applies settings)
                if self.quick_start_index < self.quick_start_entries.len() * 2 {
                    WizardStep::SkipPermissions
                } else {
                    WizardStep::BranchAction
                }
            }
            WizardStep::BranchAction => {
                if self.branch_action_index == 0 {
                    // Use selected branch
                    WizardStep::AgentSelect
                } else {
                    // Create new branch
                    WizardStep::BranchTypeSelect
                }
            }
            WizardStep::BranchTypeSelect => WizardStep::IssueSelect,
            WizardStep::IssueSelect => {
                if self.ai_enabled {
                    WizardStep::AIBranchSuggest
                } else {
                    WizardStep::BranchNameInput
                }
            }
            WizardStep::AIBranchSuggest => WizardStep::BranchNameInput,
            WizardStep::BranchNameInput => WizardStep::AgentSelect,
            WizardStep::AgentSelect => {
                if self.model_options.is_empty() {
                    WizardStep::VersionSelect
                } else {
                    WizardStep::ModelSelect
                }
            }
            WizardStep::ModelSelect => {
                if self.is_codex() {
                    WizardStep::ReasoningLevel
                } else {
                    WizardStep::VersionSelect
                }
            }
            WizardStep::ReasoningLevel => WizardStep::VersionSelect,
            WizardStep::VersionSelect => WizardStep::ExecutionMode,
            WizardStep::CollaborationModes => WizardStep::ExecutionMode,
            WizardStep::ExecutionMode => {
                if self.execution_mode == WizardExecutionMode::Convert {
                    WizardStep::ConvertAgentSelect
                } else {
                    WizardStep::SkipPermissions
                }
            }
            WizardStep::ConvertAgentSelect => WizardStep::ConvertSessionSelect,
            WizardStep::ConvertSessionSelect => WizardStep::SkipPermissions,
            WizardStep::SkipPermissions => WizardStep::SkipPermissions, // terminal
        }
    }

    /// Determine the previous step based on current state.
    pub fn prev_step(&self) -> Option<WizardStep> {
        match self.step {
            WizardStep::QuickStart => None, // closes wizard
            WizardStep::BranchAction => {
                if self.has_quick_start {
                    Some(WizardStep::QuickStart)
                } else {
                    None
                }
            }
            WizardStep::BranchTypeSelect => {
                if self.has_branch_action {
                    Some(WizardStep::BranchAction)
                } else {
                    None
                }
            }
            WizardStep::IssueSelect => Some(WizardStep::BranchTypeSelect),
            WizardStep::AIBranchSuggest => Some(WizardStep::IssueSelect),
            WizardStep::BranchNameInput => {
                if self.ai_enabled {
                    Some(WizardStep::AIBranchSuggest)
                } else {
                    Some(WizardStep::IssueSelect)
                }
            }
            WizardStep::AgentSelect => {
                if self.is_new_branch {
                    Some(WizardStep::BranchNameInput)
                } else if self.has_branch_action {
                    Some(WizardStep::BranchAction)
                } else if self.has_quick_start {
                    Some(WizardStep::QuickStart)
                } else {
                    None
                }
            }
            WizardStep::ModelSelect => Some(WizardStep::AgentSelect),
            WizardStep::ReasoningLevel => Some(WizardStep::ModelSelect),
            WizardStep::VersionSelect => {
                if self.is_codex() {
                    Some(WizardStep::ReasoningLevel)
                } else if !self.model_options.is_empty() {
                    Some(WizardStep::ModelSelect)
                } else {
                    Some(WizardStep::AgentSelect)
                }
            }
            WizardStep::CollaborationModes => Some(WizardStep::VersionSelect),
            WizardStep::ExecutionMode => Some(WizardStep::VersionSelect),
            WizardStep::ConvertAgentSelect => Some(WizardStep::ExecutionMode),
            WizardStep::ConvertSessionSelect => Some(WizardStep::ConvertAgentSelect),
            WizardStep::SkipPermissions => {
                if self.execution_mode == WizardExecutionMode::Convert {
                    Some(WizardStep::ConvertSessionSelect)
                } else {
                    Some(WizardStep::ExecutionMode)
                }
            }
        }
    }

    /// Whether the wizard is at the final step (ready to launch).
    pub fn is_complete(&self) -> bool {
        self.step == WizardStep::SkipPermissions
    }

    /// Advance to the next step, pushing current step to history.
    pub fn advance(&mut self) {
        let next = self.next_step();
        if next != self.step {
            self.step_history.push(self.step);
            self.step = next;
            self.update_model_options_for_agent();
        }
    }

    /// Go back to the previous step. Returns false if wizard should close.
    pub fn go_back(&mut self) -> bool {
        if let Some(prev) = self.prev_step() {
            self.step = prev;
            // Pop history to match
            if let Some(last) = self.step_history.last() {
                if *last == prev {
                    self.step_history.pop();
                }
            }
            true
        } else {
            false
        }
    }

    /// Select next item in the current step's list.
    pub fn select_next(&mut self) {
        match self.step {
            WizardStep::QuickStart => {
                let max = self.quick_start_option_count().saturating_sub(1);
                if self.quick_start_index < max {
                    self.quick_start_index += 1;
                }
            }
            WizardStep::BranchAction => {
                if self.branch_action_index < 1 {
                    self.branch_action_index += 1;
                }
            }
            WizardStep::AgentSelect => {
                if self.selected_agent < self.agents.len().saturating_sub(1) {
                    self.selected_agent += 1;
                }
            }
            WizardStep::ModelSelect => {
                if self.model_index < self.model_options.len().saturating_sub(1) {
                    self.model_index += 1;
                    self.sync_model_from_index();
                }
            }
            WizardStep::ReasoningLevel => {
                let levels = self.available_reasoning_levels();
                if self.reasoning_level_index < levels.len().saturating_sub(1) {
                    self.reasoning_level_index += 1;
                    self.reasoning_level = levels[self.reasoning_level_index];
                }
            }
            WizardStep::VersionSelect => {
                if self.version_index < self.version_options.len().saturating_sub(1) {
                    self.version_index += 1;
                    if let Some(v) = self.version_options.get(self.version_index) {
                        self.version = v.clone();
                    }
                }
            }
            WizardStep::ExecutionMode => {
                let max = WizardExecutionMode::ALL.len().saturating_sub(1);
                if self.execution_mode_index < max {
                    self.execution_mode_index += 1;
                    self.execution_mode = WizardExecutionMode::ALL[self.execution_mode_index];
                }
            }
            WizardStep::SkipPermissions => {
                self.skip_permissions = !self.skip_permissions;
            }
            WizardStep::BranchTypeSelect => {
                let idx = BranchType::ALL
                    .iter()
                    .position(|t| *t == self.branch_type)
                    .unwrap_or(0);
                if idx < BranchType::ALL.len() - 1 {
                    self.branch_type = BranchType::ALL[idx + 1];
                }
            }
            WizardStep::IssueSelect => {
                if self.issue_index < self.issues.len() {
                    self.issue_index += 1;
                }
            }
            WizardStep::AIBranchSuggest => {
                if self.ai_selected < self.ai_suggestions.len().saturating_sub(1) {
                    self.ai_selected += 1;
                }
            }
            WizardStep::ConvertAgentSelect => {
                if self.convert_agent_index < self.convert_agents.len().saturating_sub(1) {
                    self.convert_agent_index += 1;
                }
            }
            WizardStep::ConvertSessionSelect => {
                if self.convert_session_index < self.convert_sessions.len().saturating_sub(1) {
                    self.convert_session_index += 1;
                }
            }
            WizardStep::CollaborationModes => {
                self.collaboration_modes = !self.collaboration_modes;
            }
            WizardStep::BranchNameInput => {
                // Text input — no list selection
            }
        }
    }

    /// Select previous item in the current step's list.
    pub fn select_prev(&mut self) {
        match self.step {
            WizardStep::QuickStart => {
                self.quick_start_index = self.quick_start_index.saturating_sub(1);
            }
            WizardStep::BranchAction => {
                self.branch_action_index = self.branch_action_index.saturating_sub(1);
            }
            WizardStep::AgentSelect => {
                self.selected_agent = self.selected_agent.saturating_sub(1);
            }
            WizardStep::ModelSelect => {
                self.model_index = self.model_index.saturating_sub(1);
                self.sync_model_from_index();
            }
            WizardStep::ReasoningLevel => {
                self.reasoning_level_index = self.reasoning_level_index.saturating_sub(1);
                let levels = self.available_reasoning_levels();
                if self.reasoning_level_index < levels.len() {
                    self.reasoning_level = levels[self.reasoning_level_index];
                }
            }
            WizardStep::VersionSelect => {
                self.version_index = self.version_index.saturating_sub(1);
                if let Some(v) = self.version_options.get(self.version_index) {
                    self.version = v.clone();
                }
            }
            WizardStep::ExecutionMode => {
                self.execution_mode_index = self.execution_mode_index.saturating_sub(1);
                self.execution_mode = WizardExecutionMode::ALL[self.execution_mode_index];
            }
            WizardStep::SkipPermissions => {
                self.skip_permissions = !self.skip_permissions;
            }
            WizardStep::BranchTypeSelect => {
                let idx = BranchType::ALL
                    .iter()
                    .position(|t| *t == self.branch_type)
                    .unwrap_or(0);
                if idx > 0 {
                    self.branch_type = BranchType::ALL[idx - 1];
                }
            }
            WizardStep::IssueSelect => {
                self.issue_index = self.issue_index.saturating_sub(1);
            }
            WizardStep::AIBranchSuggest => {
                self.ai_selected = self.ai_selected.saturating_sub(1);
            }
            WizardStep::ConvertAgentSelect => {
                self.convert_agent_index = self.convert_agent_index.saturating_sub(1);
            }
            WizardStep::ConvertSessionSelect => {
                self.convert_session_index = self.convert_session_index.saturating_sub(1);
            }
            WizardStep::CollaborationModes => {
                self.collaboration_modes = !self.collaboration_modes;
            }
            WizardStep::BranchNameInput => {
                // Text input — no list selection
            }
        }
    }

    /// Handle Enter key press. Returns the resulting action.
    pub fn confirm(&mut self) -> WizardAction {
        if self.is_complete() {
            return WizardAction::Complete;
        }
        self.advance();
        WizardAction::Advance
    }

    /// Handle Escape key press. Returns the resulting action.
    pub fn cancel(&mut self) -> WizardAction {
        if self.go_back() {
            WizardAction::Noop
        } else {
            WizardAction::Cancel
        }
    }

    /// Handle a character input (for text fields).
    pub fn input_char(&mut self, ch: char) {
        match self.step {
            WizardStep::BranchNameInput => {
                self.new_branch_name.insert(self.cursor, ch);
                self.cursor += ch.len_utf8();
            }
            WizardStep::AIBranchSuggest => {
                self.ai_description.push(ch);
            }
            WizardStep::IssueSelect => {
                self.issue_search.push(ch);
            }
            _ => {}
        }
    }

    /// Handle backspace input.
    pub fn input_backspace(&mut self) {
        match self.step {
            WizardStep::BranchNameInput => {
                if self.cursor > 0 {
                    let prev = prev_char_boundary(&self.new_branch_name, self.cursor);
                    self.new_branch_name.drain(prev..self.cursor);
                    self.cursor = prev;
                }
            }
            WizardStep::AIBranchSuggest => {
                self.ai_description.pop();
            }
            WizardStep::IssueSelect => {
                self.issue_search.pop();
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Quick Start helpers
    // -----------------------------------------------------------------------

    /// Total number of Quick Start options.
    /// Each tool has 2 options (Resume, Start New) + 1 "Choose different".
    pub fn quick_start_option_count(&self) -> usize {
        if self.quick_start_entries.is_empty() {
            0
        } else {
            self.quick_start_entries.len() * 2 + 1
        }
    }

    // -----------------------------------------------------------------------
    // Build launch config
    // -----------------------------------------------------------------------

    /// Build a launch configuration from the current wizard state.
    pub fn build_launch_config(&self) -> Result<WizardLaunchConfig, String> {
        let agent_id = self.current_agent_id().to_string();
        // Use the selected ModelOption's id; empty id means default (None)
        let model = self.model_options.get(self.model_index).and_then(|opt| {
            if opt.is_default || opt.id.is_empty() {
                None
            } else {
                Some(opt.id.clone())
            }
        });
        let version = if self.version == "installed" {
            None
        } else {
            Some(self.version.clone())
        };
        let branch = if self.is_new_branch {
            format!("{}{}", self.branch_type.prefix(), self.new_branch_name)
        } else {
            self.branch_name.clone()
        };
        if branch.is_empty() && !self.is_new_branch {
            return Err("No branch selected".to_string());
        }

        Ok(WizardLaunchConfig {
            agent_id,
            model,
            version,
            branch_name: branch,
            is_new_branch: self.is_new_branch,
            execution_mode: self.execution_mode,
            session_id: self.session_id.clone(),
            skip_permissions: self.skip_permissions,
            reasoning_level: if self.is_codex() {
                Some(self.reasoning_level)
            } else {
                None
            },
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Update model_options when agent changes.
    fn update_model_options_for_agent(&mut self) {
        if self.step == WizardStep::ModelSelect || self.step == WizardStep::AgentSelect {
            self.model_options = models_for_agent_id(self.current_agent_id());
            self.model_index = 0;
            self.sync_model_from_index();
        }
    }

    /// Sync the `model` string from the currently selected model option.
    fn sync_model_from_index(&mut self) {
        if let Some(opt) = self.model_options.get(self.model_index) {
            self.model = if opt.is_default {
                String::new()
            } else {
                opt.id.clone()
            };
        } else {
            self.model.clear();
        }
    }

    /// Get available reasoning levels for the currently selected model.
    /// Falls back to `ReasoningLevel::ALL` when the selected model has no levels.
    pub fn available_reasoning_levels(&self) -> Vec<ReasoningLevel> {
        if let Some(opt) = self.model_options.get(self.model_index) {
            if !opt.inference_levels.is_empty() {
                return opt.inference_levels.clone();
            }
        }
        ReasoningLevel::ALL.to_vec()
    }

    /// Get the selected `ModelOption`, if any.
    pub fn selected_model_option(&self) -> Option<&ModelOption> {
        self.model_options.get(self.model_index)
    }
}

/// Launch configuration produced by the wizard.
#[derive(Debug, Clone)]
pub struct WizardLaunchConfig {
    pub agent_id: String,
    pub model: Option<String>,
    pub version: Option<String>,
    pub branch_name: String,
    pub is_new_branch: bool,
    pub execution_mode: WizardExecutionMode,
    pub session_id: Option<String>,
    pub skip_permissions: bool,
    pub reasoning_level: Option<ReasoningLevel>,
}

// ---------------------------------------------------------------------------
// Default data
// ---------------------------------------------------------------------------

/// Build default agent entries with version detection.
fn default_agents() -> Vec<AgentEntry> {
    let agents = [
        (
            CodingAgent::ClaudeCode,
            "claude",
            "Claude Code",
            Color::Yellow,
        ),
        (CodingAgent::CodexCli, "codex", "Codex CLI", Color::Cyan),
        (
            CodingAgent::GeminiCli,
            "gemini",
            "Gemini CLI",
            Color::Magenta,
        ),
        (CodingAgent::OpenCode, "opencode", "OpenCode", Color::Green),
    ];

    agents
        .iter()
        .map(|(agent, id, name, color)| {
            let info = detect_installed_version(*agent);
            let mut entry = AgentEntry::builtin(id, name, *color, info.is_some());
            if let Some(ref i) = info {
                entry = entry.with_version(&i.version);
            }
            entry
        })
        .collect()
}

/// Get model options for a given agent ID.
fn models_for_agent_id(agent_id: &str) -> Vec<ModelOption> {
    match agent_id {
        "claude" => CodingAgent::ClaudeCode.models(),
        "codex" => CodingAgent::CodexCli.models(),
        "gemini" => CodingAgent::GeminiCli.models(),
        "opencode" => CodingAgent::OpenCode.models(),
        _ => vec![ModelOption::default_option("Default", "Use default model")],
    }
}

// ---------------------------------------------------------------------------
// npm registry version fetch
// ---------------------------------------------------------------------------

/// Version option for VersionSelect step.
#[derive(Debug, Clone)]
pub struct VersionOption {
    pub label: String,
    pub value: String,
    pub description: String,
    pub is_prerelease: bool,
}

impl VersionOption {
    /// "installed" version option.
    pub fn installed(version: Option<&str>) -> Self {
        let desc = match version {
            Some(v) => format!("Currently installed (v{})", v),
            None => "Currently installed".to_string(),
        };
        Self {
            label: "installed".to_string(),
            value: "installed".to_string(),
            description: desc,
            is_prerelease: false,
        }
    }

    /// "latest" version option.
    pub fn latest() -> Self {
        Self {
            label: "latest".to_string(),
            value: "latest".to_string(),
            description: "Use latest release".to_string(),
            is_prerelease: false,
        }
    }

    /// Version from npm registry.
    pub fn from_npm(version: &str, published_at: Option<&str>, is_prerelease: bool) -> Self {
        let desc = match published_at {
            Some(ts) => {
                // Shorten ISO timestamp to date
                let date = ts.split('T').next().unwrap_or(ts);
                format!("Published {}", date)
            }
            None => String::new(),
        };
        let label = if is_prerelease {
            format!("{} (pre-release)", version)
        } else {
            version.to_string()
        };
        Self {
            label,
            value: version.to_string(),
            description: desc,
            is_prerelease,
        }
    }

    /// Display string for rendering.
    pub fn display(&self) -> String {
        if self.description.is_empty() {
            self.label.clone()
        } else {
            format!("{:<24} {}", self.label, self.description)
        }
    }
}

/// Check if a version string looks like a pre-release.
fn is_prerelease(version: &str) -> bool {
    version.contains('-')
}

#[derive(serde::Deserialize)]
struct NpmRegistryResponse {
    versions: Option<std::collections::HashMap<String, serde_json::Value>>,
    time: Option<std::collections::HashMap<String, String>>,
}

/// Fetch package versions from the npm registry.
/// Returns up to 10 recent versions sorted by publish date (newest first).
pub fn fetch_package_versions(package_name: &str) -> Vec<VersionOption> {
    const TIMEOUT_SECS: u64 = 3;
    const LIMIT: usize = 10;

    let url = format!("https://registry.npmjs.org/{}", package_name);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let data: NpmRegistryResponse = match response.json() {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    let versions = match data.versions {
        Some(v) => v,
        None => return vec![],
    };

    let time = match data.time {
        Some(t) => t,
        None => return vec![],
    };

    // Collect versions with publish times
    let mut versions_with_time: Vec<(String, String)> = versions
        .keys()
        .filter_map(|v| time.get(v).map(|t| (v.clone(), t.clone())))
        .collect();

    // Sort by publish date (newest first)
    versions_with_time.sort_by(|a, b| b.1.cmp(&a.1));

    // Take top N
    versions_with_time
        .into_iter()
        .take(LIMIT)
        .map(|(version, published_at)| {
            VersionOption::from_npm(&version, Some(&published_at), is_prerelease(&version))
        })
        .collect()
}

fn prev_char_boundary(s: &str, cursor: usize) -> usize {
    let cursor = cursor.min(s.len());
    if cursor == 0 {
        return 0;
    }
    s[..cursor]
        .char_indices()
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the wizard as a centered overlay popup.
pub fn render(buf: &mut Buffer, area: Rect, state: &WizardState) {
    // Calculate popup area: 70% width, 60% height, centered
    let popup_w = (area.width * 70 / 100)
        .max(40)
        .min(area.width.saturating_sub(4));
    let popup_h = (area.height * 60 / 100)
        .max(12)
        .min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup_area = Rect::new(x, y, popup_w, popup_h);

    Clear.render(popup_area, buf);

    let title = format!(
        " Launch Agent \u{2500}\u{2500} Step {}/15 ",
        state.step.number()
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::new().bg(Color::Black));

    let inner = block.inner(popup_area);
    block.render(popup_area, buf);

    if inner.height < 3 || inner.width < 20 {
        return;
    }

    // Render step content
    let content_area = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    render_step_content(buf, content_area, state);

    // Render footer hint
    let footer_y = popup_area.y + popup_area.height - 1;
    if footer_y > popup_area.y {
        let hint = step_hint(state);
        let hint_area = Rect::new(
            popup_area.x + 1,
            footer_y,
            popup_area.width.saturating_sub(2),
            1,
        );
        Paragraph::new(Line::from(vec![Span::styled(
            hint,
            Style::new().fg(Color::DarkGray),
        )]))
        .render(hint_area, buf);
    }
}

/// Render the content for the current wizard step.
fn render_step_content(buf: &mut Buffer, area: Rect, state: &WizardState) {
    match state.step {
        WizardStep::QuickStart => render_quick_start(buf, area, state),
        WizardStep::BranchAction => render_branch_action(buf, area, state),
        WizardStep::AgentSelect => render_agent_select(buf, area, state),
        WizardStep::ModelSelect => render_model_select(buf, area, state),
        WizardStep::ReasoningLevel => render_reasoning_level(buf, area, state),
        WizardStep::VersionSelect => render_list_select(
            buf,
            area,
            "Select Version:",
            &state.version_options,
            state.version_index,
        ),
        WizardStep::CollaborationModes => {
            render_toggle(buf, area, "Collaboration Modes:", state.collaboration_modes)
        }
        WizardStep::ExecutionMode => render_execution_mode(buf, area, state),
        WizardStep::ConvertAgentSelect => render_list_select(
            buf,
            area,
            "Convert From Agent:",
            &state.convert_agents,
            state.convert_agent_index,
        ),
        WizardStep::ConvertSessionSelect => render_list_select(
            buf,
            area,
            "Select Session:",
            &state.convert_sessions,
            state.convert_session_index,
        ),
        WizardStep::SkipPermissions => {
            render_toggle(buf, area, "Skip Permissions:", state.skip_permissions)
        }
        WizardStep::BranchTypeSelect => render_branch_type_select(buf, area, state),
        WizardStep::IssueSelect => render_issue_select(buf, area, state),
        WizardStep::AIBranchSuggest => render_ai_branch_suggest(buf, area, state),
        WizardStep::BranchNameInput => render_branch_name_input(buf, area, state),
    }
}

fn render_quick_start(buf: &mut Buffer, area: Rect, state: &WizardState) {
    if area.height < 2 {
        return;
    }

    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Quick Start:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    let mut idx = 0;
    for entry in &state.quick_start_entries {
        let model_str = entry.model.as_deref().unwrap_or("default");
        // Resume option
        let marker = if idx == state.quick_start_index {
            "> "
        } else {
            "  "
        };
        let style = if idx == state.quick_start_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}Resume {} ({})", entry.tool_label, model_str),
            style,
        )));
        idx += 1;

        // Start new option
        let marker = if idx == state.quick_start_index {
            "> "
        } else {
            "  "
        };
        let style = if idx == state.quick_start_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{marker}Start new with {} ({})",
                entry.tool_label, model_str
            ),
            style,
        )));
        idx += 1;
    }

    // "Choose different" option
    let marker = if idx == state.quick_start_index {
        "> "
    } else {
        "  "
    };
    let style = if idx == state.quick_start_index {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(
        format!("{marker}Choose different settings"),
        style,
    )));

    Paragraph::new(lines).render(area, buf);
}

fn render_branch_action(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let options = ["Use selected branch", "Create new from selected"];
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Branch Action:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, opt) in options.iter().enumerate() {
        let marker = if i == state.branch_action_index {
            "> "
        } else {
            "  "
        };
        let style = if i == state.branch_action_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{opt}"), style)));
    }

    if !state.branch_name.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Branch: {}", state.branch_name),
            Style::new().fg(Color::DarkGray),
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_agent_select(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Select Agent:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, agent) in state.agents.iter().enumerate() {
        let marker = if i == state.selected_agent {
            "> "
        } else {
            "  "
        };
        let status = if agent.is_installed {
            if let Some(ref v) = agent.version {
                format!("(installed v{})", v)
            } else {
                "(installed)".to_string()
            }
        } else {
            "(not installed)".to_string()
        };
        let style = if i == state.selected_agent {
            Style::new().fg(agent.color).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(agent.color)
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{:<16} {}", agent.display_name, status),
            style,
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_list_select(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    options: &[String],
    selected: usize,
) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, opt) in options.iter().enumerate() {
        let marker = if i == selected { "> " } else { "  " };
        let style = if i == selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{opt}"), style)));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_model_select(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Select Model:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, opt) in state.model_options.iter().enumerate() {
        let marker = if i == state.model_index { "> " } else { "  " };
        let style = if i == state.model_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}", opt.display()),
            style,
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_reasoning_level(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let levels = state.available_reasoning_levels();
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Reasoning Level:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, level) in levels.iter().enumerate() {
        let marker = if i == state.reasoning_level_index {
            "> "
        } else {
            "  "
        };
        let style = if i == state.reasoning_level_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{:<8} {}", level.label(), level.description()),
            style,
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_execution_mode(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Execution Mode:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, mode) in WizardExecutionMode::ALL.iter().enumerate() {
        let marker = if i == state.execution_mode_index {
            "> "
        } else {
            "  "
        };
        let style = if i == state.execution_mode_index {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{:<10} {}", mode.label(), mode.description()),
            style,
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_toggle(buf: &mut Buffer, area: Rect, title: &str, value: bool) {
    let check = if value { "x" } else { " " };
    let lines = vec![
        Line::from(Span::styled(
            title,
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  [{check}] Enabled"),
            Style::new().add_modifier(Modifier::REVERSED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Enter to toggle, then Enter again to confirm",
            Style::new().fg(Color::DarkGray),
        )),
    ];
    Paragraph::new(lines).render(area, buf);
}

fn render_branch_type_select(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Branch Type:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for bt in &BranchType::ALL {
        let selected = *bt == state.branch_type;
        let marker = if selected { "> " } else { "  " };
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{:<10} {}", bt.label(), bt.prefix()),
            style,
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_issue_select(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Link Issue (optional):",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Skip option
    let skip_selected = state.issue_index == 0;
    let marker = if skip_selected { "> " } else { "  " };
    let style = if skip_selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    lines.push(Line::from(Span::styled(
        format!("{marker}Skip (no issue)"),
        style,
    )));

    for (i, issue) in state.issues.iter().enumerate() {
        let selected = i + 1 == state.issue_index;
        let marker = if selected { "> " } else { "  " };
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let title_truncated: String = issue.title.chars().take(40).collect();
        lines.push(Line::from(Span::styled(
            format!("{marker}#{} {}", issue.number, title_truncated),
            style,
        )));
    }

    if !state.issue_search.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Search: {}", state.issue_search),
            Style::new().fg(Color::Yellow),
        )));
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_ai_branch_suggest(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "AI Branch Suggest:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if state.ai_suggestions.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Describe your change: {}_", state.ai_description),
            Style::new().fg(Color::White),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press Enter to get AI suggestions",
            Style::new().fg(Color::DarkGray),
        )));
    } else {
        for (i, suggestion) in state.ai_suggestions.iter().enumerate() {
            let selected = i == state.ai_selected;
            let marker = if selected { "> " } else { "  " };
            let style = if selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{marker}{suggestion}"),
                style,
            )));
        }
    }

    Paragraph::new(lines).render(area, buf);
}

fn render_branch_name_input(buf: &mut Buffer, area: Rect, state: &WizardState) {
    let prefix = state.branch_type.prefix();
    let display_name = if state.new_branch_name.is_empty() {
        "<branch-name>"
    } else {
        &state.new_branch_name
    };

    let lines = vec![
        Line::from(Span::styled(
            "Branch Name:",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {prefix}"), Style::new().fg(Color::DarkGray)),
            Span::styled(
                display_name,
                Style::new()
                    .fg(Color::White)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Type branch name, then press Enter",
            Style::new().fg(Color::DarkGray),
        )),
    ];

    Paragraph::new(lines).render(area, buf);
}

fn step_hint(state: &WizardState) -> &'static str {
    match state.step {
        WizardStep::BranchNameInput | WizardStep::AIBranchSuggest => {
            "[Enter] Confirm  [Esc] Back  [Type] Input"
        }
        WizardStep::SkipPermissions | WizardStep::CollaborationModes => {
            "[Enter] Toggle/Confirm  [Esc] Back"
        }
        _ => "[Up/Down] Navigate  [Enter] Select  [Esc] Back/Cancel",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- WizardStep --

    #[test]
    fn step_labels_are_nonempty() {
        for step in [
            WizardStep::QuickStart,
            WizardStep::BranchAction,
            WizardStep::AgentSelect,
            WizardStep::ModelSelect,
            WizardStep::ReasoningLevel,
            WizardStep::VersionSelect,
            WizardStep::CollaborationModes,
            WizardStep::ExecutionMode,
            WizardStep::ConvertAgentSelect,
            WizardStep::ConvertSessionSelect,
            WizardStep::SkipPermissions,
            WizardStep::BranchTypeSelect,
            WizardStep::IssueSelect,
            WizardStep::AIBranchSuggest,
            WizardStep::BranchNameInput,
        ] {
            assert!(!step.label().is_empty());
            assert!(step.number() >= 1 && step.number() <= 15);
        }
    }

    #[test]
    fn default_step_is_agent_select() {
        assert_eq!(WizardStep::default(), WizardStep::AgentSelect);
    }

    // -- Navigation: next_step --

    #[test]
    fn next_step_agent_select_to_model_select() {
        let state = WizardState::new();
        assert_eq!(state.next_step(), WizardStep::ModelSelect);
    }

    #[test]
    fn next_step_model_select_to_version_for_non_codex() {
        let mut state = WizardState::new();
        state.step = WizardStep::ModelSelect;
        state.selected_agent = 0; // claude
        assert_eq!(state.next_step(), WizardStep::VersionSelect);
    }

    #[test]
    fn next_step_model_select_to_reasoning_for_codex() {
        let mut state = WizardState::new();
        state.step = WizardStep::ModelSelect;
        state.selected_agent = 1; // codex
        assert!(state.is_codex());
        assert_eq!(state.next_step(), WizardStep::ReasoningLevel);
    }

    #[test]
    fn next_step_reasoning_to_version() {
        let mut state = WizardState::new();
        state.step = WizardStep::ReasoningLevel;
        assert_eq!(state.next_step(), WizardStep::VersionSelect);
    }

    #[test]
    fn next_step_version_to_execution_mode() {
        let mut state = WizardState::new();
        state.step = WizardStep::VersionSelect;
        assert_eq!(state.next_step(), WizardStep::ExecutionMode);
    }

    #[test]
    fn next_step_execution_mode_normal_to_skip_permissions() {
        let mut state = WizardState::new();
        state.step = WizardStep::ExecutionMode;
        state.execution_mode = WizardExecutionMode::Normal;
        assert_eq!(state.next_step(), WizardStep::SkipPermissions);
    }

    #[test]
    fn next_step_execution_mode_convert_to_convert_agent() {
        let mut state = WizardState::new();
        state.step = WizardStep::ExecutionMode;
        state.execution_mode = WizardExecutionMode::Convert;
        assert_eq!(state.next_step(), WizardStep::ConvertAgentSelect);
    }

    #[test]
    fn next_step_convert_agent_to_convert_session() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertAgentSelect;
        assert_eq!(state.next_step(), WizardStep::ConvertSessionSelect);
    }

    #[test]
    fn next_step_convert_session_to_skip_permissions() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertSessionSelect;
        assert_eq!(state.next_step(), WizardStep::SkipPermissions);
    }

    #[test]
    fn next_step_skip_permissions_is_terminal() {
        let mut state = WizardState::new();
        state.step = WizardStep::SkipPermissions;
        assert_eq!(state.next_step(), WizardStep::SkipPermissions);
        assert!(state.is_complete());
    }

    // -- Navigation: prev_step --

    #[test]
    fn prev_step_agent_select_no_history_is_none() {
        let state = WizardState::new();
        assert!(state.prev_step().is_none());
    }

    #[test]
    fn prev_step_model_select_to_agent() {
        let mut state = WizardState::new();
        state.step = WizardStep::ModelSelect;
        assert_eq!(state.prev_step(), Some(WizardStep::AgentSelect));
    }

    #[test]
    fn prev_step_reasoning_to_model() {
        let mut state = WizardState::new();
        state.step = WizardStep::ReasoningLevel;
        assert_eq!(state.prev_step(), Some(WizardStep::ModelSelect));
    }

    #[test]
    fn prev_step_version_to_reasoning_for_codex() {
        let mut state = WizardState::new();
        state.step = WizardStep::VersionSelect;
        state.selected_agent = 1; // codex
        assert_eq!(state.prev_step(), Some(WizardStep::ReasoningLevel));
    }

    #[test]
    fn prev_step_version_to_model_for_non_codex() {
        let mut state = WizardState::new();
        state.step = WizardStep::VersionSelect;
        state.selected_agent = 0; // claude
        assert_eq!(state.prev_step(), Some(WizardStep::ModelSelect));
    }

    #[test]
    fn prev_step_execution_mode_to_version() {
        let mut state = WizardState::new();
        state.step = WizardStep::ExecutionMode;
        assert_eq!(state.prev_step(), Some(WizardStep::VersionSelect));
    }

    #[test]
    fn prev_step_skip_permissions_from_convert() {
        let mut state = WizardState::new();
        state.step = WizardStep::SkipPermissions;
        state.execution_mode = WizardExecutionMode::Convert;
        assert_eq!(state.prev_step(), Some(WizardStep::ConvertSessionSelect));
    }

    #[test]
    fn prev_step_skip_permissions_from_normal() {
        let mut state = WizardState::new();
        state.step = WizardStep::SkipPermissions;
        state.execution_mode = WizardExecutionMode::Normal;
        assert_eq!(state.prev_step(), Some(WizardStep::ExecutionMode));
    }

    // -- Navigation: new branch flow --

    #[test]
    fn new_branch_flow_navigation() {
        let mut state = WizardState::open_for_new_branch();
        assert_eq!(state.step, WizardStep::BranchTypeSelect);

        let next = state.next_step();
        assert_eq!(next, WizardStep::IssueSelect);

        state.step = WizardStep::IssueSelect;
        state.ai_enabled = false;
        assert_eq!(state.next_step(), WizardStep::BranchNameInput);

        state.step = WizardStep::BranchNameInput;
        assert_eq!(state.next_step(), WizardStep::AgentSelect);
    }

    #[test]
    fn new_branch_flow_with_ai() {
        let mut state = WizardState::open_for_new_branch();
        state.step = WizardStep::IssueSelect;
        state.ai_enabled = true;
        assert_eq!(state.next_step(), WizardStep::AIBranchSuggest);

        state.step = WizardStep::AIBranchSuggest;
        assert_eq!(state.next_step(), WizardStep::BranchNameInput);
    }

    // -- Navigation: branch action flow --

    #[test]
    fn branch_action_use_selected_goes_to_agent() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchAction;
        state.branch_action_index = 0; // Use selected
        assert_eq!(state.next_step(), WizardStep::AgentSelect);
    }

    #[test]
    fn branch_action_create_new_goes_to_branch_type() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchAction;
        state.branch_action_index = 1; // Create new
        assert_eq!(state.next_step(), WizardStep::BranchTypeSelect);
    }

    // -- Quick Start --

    #[test]
    fn quick_start_option_count() {
        let mut state = WizardState::new();
        assert_eq!(state.quick_start_option_count(), 0);

        state.quick_start_entries.push(QuickStartEntry {
            tool_id: "claude".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("opus".to_string()),
            version: None,
            session_id: None,
            skip_permissions: None,
            branch: "main".to_string(),
        });
        // 1 entry * 2 options + 1 "Choose different" = 3
        assert_eq!(state.quick_start_option_count(), 3);
    }

    #[test]
    fn quick_start_skip_to_skip_permissions() {
        let mut state = WizardState::new();
        state.step = WizardStep::QuickStart;
        state.quick_start_entries.push(QuickStartEntry {
            tool_id: "claude".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("opus".to_string()),
            version: None,
            session_id: None,
            skip_permissions: None,
            branch: "main".to_string(),
        });
        state.quick_start_index = 0; // Resume with first entry
        assert_eq!(state.next_step(), WizardStep::SkipPermissions);
    }

    #[test]
    fn quick_start_choose_different_goes_to_branch_action() {
        let mut state = WizardState::new();
        state.step = WizardStep::QuickStart;
        state.quick_start_entries.push(QuickStartEntry {
            tool_id: "claude".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("opus".to_string()),
            version: None,
            session_id: None,
            skip_permissions: None,
            branch: "main".to_string(),
        });
        state.quick_start_index = 2; // "Choose different"
        assert_eq!(state.next_step(), WizardStep::BranchAction);
    }

    // -- Codex-specific step skipping --

    #[test]
    fn codex_includes_reasoning_level() {
        let mut state = WizardState::new();
        state.selected_agent = 1; // codex
        state.step = WizardStep::ModelSelect;
        assert_eq!(state.next_step(), WizardStep::ReasoningLevel);
    }

    #[test]
    fn non_codex_skips_reasoning_level() {
        let mut state = WizardState::new();
        state.selected_agent = 0; // claude
        state.step = WizardStep::ModelSelect;
        assert_eq!(state.next_step(), WizardStep::VersionSelect);
    }

    // -- Selection cycling --

    #[test]
    fn select_next_agent_clamps() {
        let mut state = WizardState::new();
        state.selected_agent = state.agents.len() - 1;
        state.select_next();
        assert_eq!(state.selected_agent, state.agents.len() - 1);
    }

    #[test]
    fn select_prev_agent_clamps_at_zero() {
        let mut state = WizardState::new();
        state.selected_agent = 0;
        state.select_prev();
        assert_eq!(state.selected_agent, 0);
    }

    #[test]
    fn select_next_model_updates_model_string() {
        let mut state = WizardState::new();
        state.step = WizardStep::ModelSelect;
        state.model_index = 0;
        state.sync_model_from_index();
        state.select_next();
        assert_eq!(state.model_index, 1);
        // Second option for Claude Code is "opus"
        assert_eq!(state.model, state.model_options[1].id);
    }

    #[test]
    fn select_next_execution_mode() {
        let mut state = WizardState::new();
        state.step = WizardStep::ExecutionMode;
        state.execution_mode_index = 0;
        state.select_next();
        assert_eq!(state.execution_mode, WizardExecutionMode::Continue);
        state.select_next();
        assert_eq!(state.execution_mode, WizardExecutionMode::Resume);
        state.select_next();
        assert_eq!(state.execution_mode, WizardExecutionMode::Convert);
        state.select_next();
        // Clamped at last
        assert_eq!(state.execution_mode, WizardExecutionMode::Convert);
    }

    #[test]
    fn select_skip_permissions_toggles() {
        let mut state = WizardState::new();
        state.step = WizardStep::SkipPermissions;
        assert!(!state.skip_permissions);
        state.select_next();
        assert!(state.skip_permissions);
        state.select_next();
        assert!(!state.skip_permissions);
    }

    #[test]
    fn select_branch_type_cycles() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchTypeSelect;
        assert_eq!(state.branch_type, BranchType::Feature);
        state.select_next();
        assert_eq!(state.branch_type, BranchType::Bugfix);
        state.select_next();
        assert_eq!(state.branch_type, BranchType::Hotfix);
        state.select_next();
        assert_eq!(state.branch_type, BranchType::Release);
        state.select_next();
        // Clamped at last
        assert_eq!(state.branch_type, BranchType::Release);
    }

    #[test]
    fn select_prev_branch_type() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchTypeSelect;
        state.branch_type = BranchType::Hotfix;
        state.select_prev();
        assert_eq!(state.branch_type, BranchType::Bugfix);
        state.select_prev();
        assert_eq!(state.branch_type, BranchType::Feature);
        state.select_prev();
        assert_eq!(state.branch_type, BranchType::Feature); // clamped
    }

    #[test]
    fn select_reasoning_level() {
        let mut state = WizardState::new();
        state.step = WizardStep::ReasoningLevel;
        // Use ALL levels (default when model has none)
        state.reasoning_level_index = 0;
        state.reasoning_level = ReasoningLevel::Low;
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::Medium);
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::High);
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::XHigh);
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::XHigh); // clamped
    }

    #[test]
    fn select_reasoning_level_codex_model_base_levels() {
        let mut state = WizardState::new();
        state.step = WizardStep::ReasoningLevel;
        // Use Codex models (which have base levels: High, Medium, Low)
        state.model_options = CodingAgent::CodexCli.models();
        state.model_index = 0; // Default has base levels
        state.reasoning_level_index = 0;
        let levels = state.available_reasoning_levels();
        assert_eq!(levels.len(), 3); // High, Medium, Low
        assert_eq!(levels[0], ReasoningLevel::High);
        state.reasoning_level = levels[0];
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::Medium);
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::Low);
        state.select_next();
        assert_eq!(state.reasoning_level, ReasoningLevel::Low); // clamped
    }

    // -- Advance / Go back --

    #[test]
    fn advance_pushes_history() {
        let mut state = WizardState::new();
        assert!(state.step_history.is_empty());
        state.advance();
        assert_eq!(state.step, WizardStep::ModelSelect);
        assert_eq!(state.step_history.len(), 1);
        assert_eq!(state.step_history[0], WizardStep::AgentSelect);
    }

    #[test]
    fn go_back_returns_to_previous() {
        let mut state = WizardState::new();
        state.advance(); // AgentSelect -> ModelSelect
        assert_eq!(state.step, WizardStep::ModelSelect);
        let result = state.go_back();
        assert!(result);
        assert_eq!(state.step, WizardStep::AgentSelect);
    }

    #[test]
    fn go_back_at_start_returns_false() {
        let state = WizardState::new();
        let mut s = state;
        let result = s.go_back();
        assert!(!result);
    }

    // -- Confirm / Cancel --

    #[test]
    fn confirm_at_skip_permissions_returns_complete() {
        let mut state = WizardState::new();
        state.step = WizardStep::SkipPermissions;
        let action = state.confirm();
        assert_eq!(action, WizardAction::Complete);
    }

    #[test]
    fn confirm_at_agent_select_advances() {
        let mut state = WizardState::new();
        let action = state.confirm();
        assert_eq!(action, WizardAction::Advance);
        assert_eq!(state.step, WizardStep::ModelSelect);
    }

    #[test]
    fn cancel_at_start_returns_cancel() {
        let mut state = WizardState::new();
        let action = state.cancel();
        assert_eq!(action, WizardAction::Cancel);
    }

    #[test]
    fn cancel_from_model_goes_back() {
        let mut state = WizardState::new();
        state.advance(); // -> ModelSelect
        let action = state.cancel();
        assert_eq!(action, WizardAction::Noop);
        assert_eq!(state.step, WizardStep::AgentSelect);
    }

    // -- Text input --

    #[test]
    fn input_char_in_branch_name() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchNameInput;
        state.input_char('a');
        state.input_char('b');
        state.input_char('c');
        assert_eq!(state.new_branch_name, "abc");
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn input_backspace_in_branch_name() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchNameInput;
        state.input_char('a');
        state.input_char('b');
        state.input_backspace();
        assert_eq!(state.new_branch_name, "a");
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn input_backspace_empty_branch_name() {
        let mut state = WizardState::new();
        state.step = WizardStep::BranchNameInput;
        state.input_backspace(); // should not panic
        assert_eq!(state.new_branch_name, "");
        assert_eq!(state.cursor, 0);
    }

    // -- Agent entry --

    #[test]
    fn agent_entry_with_version() {
        let entry =
            AgentEntry::builtin("claude", "Claude Code", Color::Yellow, true).with_version("1.8.0");
        assert_eq!(entry.version, Some("1.8.0".to_string()));
        assert!(entry.is_installed);
    }

    // -- Build launch config --

    #[test]
    fn build_launch_config_basic() {
        let state = WizardState::new();
        let config = state.build_launch_config();
        // No branch selected for non-new-branch, should error
        assert!(config.is_err());
    }

    #[test]
    fn build_launch_config_with_branch() {
        let mut state = WizardState::new();
        state.branch_name = "feature/test".to_string();
        let config = state.build_launch_config().unwrap();
        assert_eq!(config.agent_id, "claude");
        assert_eq!(config.branch_name, "feature/test");
        assert!(!config.is_new_branch);
        assert!(!config.skip_permissions);
    }

    #[test]
    fn build_launch_config_new_branch() {
        let mut state = WizardState::open_for_new_branch();
        state.branch_type = BranchType::Feature;
        state.new_branch_name = "add-login".to_string();
        let config = state.build_launch_config().unwrap();
        assert_eq!(config.branch_name, "feature/add-login");
        assert!(config.is_new_branch);
    }

    #[test]
    fn build_launch_config_codex_includes_reasoning() {
        let mut state = WizardState::new();
        state.selected_agent = 1; // codex
        state.branch_name = "main".to_string();
        state.reasoning_level = ReasoningLevel::High;
        let config = state.build_launch_config().unwrap();
        assert_eq!(config.agent_id, "codex");
        assert_eq!(config.reasoning_level, Some(ReasoningLevel::High));
    }

    #[test]
    fn build_launch_config_non_codex_no_reasoning() {
        let mut state = WizardState::new();
        state.selected_agent = 0; // claude
        state.branch_name = "main".to_string();
        let config = state.build_launch_config().unwrap();
        assert_eq!(config.reasoning_level, None);
    }

    // -- open_for_branch --

    #[test]
    fn open_for_branch_with_history() {
        let history = vec![QuickStartEntry {
            tool_id: "claude".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("opus".to_string()),
            version: None,
            session_id: None,
            skip_permissions: None,
            branch: "main".to_string(),
        }];
        let state = WizardState::open_for_branch("main", history);
        assert_eq!(state.step, WizardStep::QuickStart);
        assert!(state.has_quick_start);
        assert_eq!(state.branch_name, "main");
    }

    #[test]
    fn open_for_branch_without_history() {
        let state = WizardState::open_for_branch("develop", vec![]);
        assert_eq!(state.step, WizardStep::BranchAction);
        assert!(!state.has_quick_start);
    }

    // -- Render smoke tests --

    #[test]
    fn render_wizard_does_not_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        let state = WizardState::new();
        render(&mut buf, Rect::new(0, 0, 100, 30), &state);
        let all: String = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all.contains("Launch Agent"));
        assert!(all.contains("Select Agent"));
    }

    #[test]
    fn render_wizard_each_step() {
        let steps = [
            WizardStep::QuickStart,
            WizardStep::BranchAction,
            WizardStep::AgentSelect,
            WizardStep::ModelSelect,
            WizardStep::ReasoningLevel,
            WizardStep::VersionSelect,
            WizardStep::CollaborationModes,
            WizardStep::ExecutionMode,
            WizardStep::ConvertAgentSelect,
            WizardStep::ConvertSessionSelect,
            WizardStep::SkipPermissions,
            WizardStep::BranchTypeSelect,
            WizardStep::IssueSelect,
            WizardStep::AIBranchSuggest,
            WizardStep::BranchNameInput,
        ];

        for step in steps {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
            let mut state = WizardState::new();
            state.step = step;
            // Add some data for steps that need it
            if step == WizardStep::QuickStart {
                state.quick_start_entries.push(QuickStartEntry {
                    tool_id: "claude".to_string(),
                    tool_label: "Claude".to_string(),
                    model: None,
                    version: None,
                    session_id: None,
                    skip_permissions: None,
                    branch: "main".to_string(),
                });
            }
            render(&mut buf, Rect::new(0, 0, 80, 24), &state);
        }
    }

    #[test]
    fn render_wizard_small_terminal() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        let state = WizardState::new();
        render(&mut buf, Rect::new(0, 0, 30, 10), &state);
        // Should not panic even with very small terminal
    }

    // -- Full workflow tests --

    #[test]
    fn full_non_codex_workflow() {
        let mut state = WizardState::open_for_branch("main", vec![]);
        assert_eq!(state.step, WizardStep::BranchAction);

        // BranchAction: use selected branch
        state.branch_action_index = 0;
        state.advance();
        assert_eq!(state.step, WizardStep::AgentSelect);

        // AgentSelect: keep claude (default)
        state.advance();
        assert_eq!(state.step, WizardStep::ModelSelect);

        // ModelSelect: keep default
        state.advance();
        assert_eq!(state.step, WizardStep::VersionSelect);

        // VersionSelect
        state.advance();
        assert_eq!(state.step, WizardStep::ExecutionMode);

        // ExecutionMode: normal
        state.advance();
        assert_eq!(state.step, WizardStep::SkipPermissions);

        assert!(state.is_complete());
        let action = state.confirm();
        assert_eq!(action, WizardAction::Complete);
    }

    #[test]
    fn full_codex_workflow_with_reasoning() {
        let mut state = WizardState::open_for_branch("main", vec![]);
        state.branch_action_index = 0;
        state.advance(); // -> AgentSelect

        // Select codex
        state.selected_agent = 1;
        state.advance(); // -> ModelSelect

        // ModelSelect
        state.advance(); // -> ReasoningLevel (Codex-specific)
        assert_eq!(state.step, WizardStep::ReasoningLevel);

        state.advance(); // -> VersionSelect
        assert_eq!(state.step, WizardStep::VersionSelect);

        state.advance(); // -> ExecutionMode
        state.advance(); // -> SkipPermissions
        assert!(state.is_complete());
    }

    #[test]
    fn full_convert_workflow() {
        let mut state = WizardState::open_for_branch("main", vec![]);
        state.branch_action_index = 0;
        state.advance(); // -> AgentSelect
        state.advance(); // -> ModelSelect
        state.advance(); // -> VersionSelect
        state.advance(); // -> ExecutionMode

        // Select Convert mode
        state.execution_mode = WizardExecutionMode::Convert;
        state.execution_mode_index = 3;
        state.advance(); // -> ConvertAgentSelect
        assert_eq!(state.step, WizardStep::ConvertAgentSelect);

        state.advance(); // -> ConvertSessionSelect
        assert_eq!(state.step, WizardStep::ConvertSessionSelect);

        state.advance(); // -> SkipPermissions
        assert_eq!(state.step, WizardStep::SkipPermissions);
        assert!(state.is_complete());
    }

    // -- ModelOption --

    #[test]
    fn model_option_display_includes_description() {
        let opt = ModelOption::new("opus", "Opus 4.6", "Most capable for complex work");
        let display = opt.display();
        assert!(display.contains("Opus 4.6"));
        assert!(display.contains("Most capable"));
    }

    #[test]
    fn model_option_default_has_empty_id() {
        let opt = ModelOption::default_option("Default", "Use default");
        assert!(opt.is_default);
        assert!(opt.id.is_empty());
    }

    #[test]
    fn model_option_with_base_levels() {
        let opt = ModelOption::new("test", "Test", "desc").with_base_levels();
        assert_eq!(opt.inference_levels.len(), 3);
        assert_eq!(opt.default_inference, Some(ReasoningLevel::High));
    }

    #[test]
    fn model_option_with_max_levels() {
        let opt = ModelOption::new("test", "Test", "desc").with_max_levels();
        assert_eq!(opt.inference_levels.len(), 4);
        assert!(opt.inference_levels.contains(&ReasoningLevel::XHigh));
        assert_eq!(opt.default_inference, Some(ReasoningLevel::Medium));
    }

    // -- CodingAgent --

    #[test]
    fn coding_agent_models_nonempty() {
        for agent in CodingAgent::ALL {
            let models = agent.models();
            assert!(!models.is_empty(), "{:?} has no models", agent);
            assert!(
                models[0].is_default,
                "{:?} first model should be default",
                agent
            );
        }
    }

    #[test]
    fn coding_agent_claude_models() {
        let models = CodingAgent::ClaudeCode.models();
        assert!(models.len() >= 4);
        assert_eq!(models[1].id, "opus");
        assert_eq!(models[2].id, "sonnet");
        assert_eq!(models[3].id, "haiku");
    }

    #[test]
    fn coding_agent_codex_models_have_inference_levels() {
        let models = CodingAgent::CodexCli.models();
        for model in &models {
            assert!(
                !model.inference_levels.is_empty(),
                "Codex model {:?} should have inference levels",
                model.id
            );
        }
    }

    #[test]
    fn coding_agent_command_names() {
        assert_eq!(CodingAgent::ClaudeCode.command_name(), "claude");
        assert_eq!(CodingAgent::CodexCli.command_name(), "codex");
        assert_eq!(CodingAgent::GeminiCli.command_name(), "gemini");
        assert_eq!(CodingAgent::OpenCode.command_name(), "opencode");
    }

    // -- VersionOption --

    #[test]
    fn version_option_installed_display() {
        let opt = VersionOption::installed(Some("1.2.3"));
        assert!(opt.display().contains("1.2.3"));
    }

    #[test]
    fn version_option_latest_display() {
        let opt = VersionOption::latest();
        assert!(opt.display().contains("latest"));
    }

    #[test]
    fn version_option_npm_prerelease() {
        let opt = VersionOption::from_npm("1.0.0-beta.1", Some("2025-01-01T00:00:00Z"), true);
        assert!(opt.is_prerelease);
        assert!(opt.label.contains("pre-release"));
    }

    #[test]
    fn is_prerelease_detection() {
        assert!(is_prerelease("1.0.0-beta.1"));
        assert!(is_prerelease("2.0.0-rc.1"));
        assert!(!is_prerelease("1.0.0"));
    }

    // -- available_reasoning_levels --

    #[test]
    fn available_reasoning_levels_defaults_to_all() {
        let state = WizardState::new();
        // Claude models have no inference levels, so fallback to ALL
        let levels = state.available_reasoning_levels();
        assert_eq!(levels.len(), 4);
    }

    #[test]
    fn available_reasoning_levels_codex_model() {
        let mut state = WizardState::new();
        state.model_options = CodingAgent::CodexCli.models();
        state.model_index = 1; // gpt-5.3-codex has max levels
        let levels = state.available_reasoning_levels();
        assert_eq!(levels.len(), 4); // XHigh, High, Medium, Low
    }

    // -- build_launch_config model --

    #[test]
    fn build_launch_config_default_model_is_none() {
        let mut state = WizardState::new();
        state.branch_name = "main".to_string();
        state.model_index = 0; // Default
        let config = state.build_launch_config().unwrap();
        assert!(config.model.is_none());
    }

    #[test]
    fn build_launch_config_specific_model() {
        let mut state = WizardState::new();
        state.branch_name = "main".to_string();
        state.model_index = 1; // opus
        let config = state.build_launch_config().unwrap();
        assert_eq!(config.model, Some("opus".to_string()));
    }
}
