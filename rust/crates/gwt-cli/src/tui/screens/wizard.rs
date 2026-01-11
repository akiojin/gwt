//! Wizard Popup Screen - TypeScript version compatible
//!
//! FR-044: Wizard popup overlay on branch selection
//! FR-045: Semi-transparent overlay background
//! FR-046: Centered popup with z-index
//! FR-047: Steps within same popup

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Wizard step types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    #[default]
    AgentSelect,
    ModelSelect,
    ReasoningLevel,    // Codex only
    VersionSelect,
    ExecutionMode,
    SkipPermissions,
    // New branch flow
    BranchTypeSelect,
    BranchNameInput,
}

/// Coding agent types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodingAgent {
    #[default]
    ClaudeCode,
    CodexCli,
    GeminiCli,
    OpenCode,
}

impl CodingAgent {
    pub fn label(&self) -> &'static str {
        match self {
            CodingAgent::ClaudeCode => "Claude Code",
            CodingAgent::CodexCli => "Codex CLI",
            CodingAgent::GeminiCli => "Gemini CLI",
            CodingAgent::OpenCode => "OpenCode",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            CodingAgent::ClaudeCode => "claude-code",
            CodingAgent::CodexCli => "codex-cli",
            CodingAgent::GeminiCli => "gemini-cli",
            CodingAgent::OpenCode => "opencode",
        }
    }

    /// Agent-specific colors per SPEC-3b0ed29b FR-025
    /// Claude Code=yellow, Codex=cyan, Gemini=magenta, OpenCode=green
    pub fn color(&self) -> Color {
        match self {
            CodingAgent::ClaudeCode => Color::Yellow,             // Yellow (#f6e05e)
            CodingAgent::CodexCli => Color::Cyan,                 // Cyan (#4fd1c5)
            CodingAgent::GeminiCli => Color::Magenta,             // Magenta (#d53f8c)
            CodingAgent::OpenCode => Color::Green,                // Green (#48bb78)
        }
    }

    pub fn all() -> &'static [CodingAgent] {
        &[
            CodingAgent::ClaudeCode,
            CodingAgent::CodexCli,
            CodingAgent::GeminiCli,
            CodingAgent::OpenCode,
        ]
    }
}

/// Reasoning level (Codex only) - defined before ModelOption which uses it
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
            ReasoningLevel::Low => "low",
            ReasoningLevel::Medium => "medium",
            ReasoningLevel::High => "high",
            ReasoningLevel::XHigh => "xhigh",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ReasoningLevel::Low => "Faster, less thorough",
            ReasoningLevel::Medium => "Balanced",
            ReasoningLevel::High => "Slower, more thorough",
            ReasoningLevel::XHigh => "Extended high reasoning",
        }
    }

    pub fn all() -> &'static [ReasoningLevel] {
        &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High, ReasoningLevel::XHigh]
    }
}

/// Model options for each agent (matches modelOptions.ts)
#[derive(Debug, Clone)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub is_default: bool,
    /// Supported inference levels for this model (Codex only)
    pub inference_levels: Vec<ReasoningLevel>,
    /// Default inference level for this model
    pub default_inference: Option<ReasoningLevel>,
}

impl ModelOption {
    fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            description: Some(description.to_string()),
            is_default: false,
            inference_levels: vec![],
            default_inference: None,
        }
    }

    fn default_option(label: &str, description: &str) -> Self {
        Self {
            id: String::new(),
            label: label.to_string(),
            description: Some(description.to_string()),
            is_default: true,
            inference_levels: vec![],
            default_inference: None,
        }
    }

    fn with_base_levels(mut self) -> Self {
        self.inference_levels = vec![ReasoningLevel::High, ReasoningLevel::Medium, ReasoningLevel::Low];
        self.default_inference = Some(ReasoningLevel::High);
        self
    }

    fn with_max_levels(mut self) -> Self {
        self.inference_levels = vec![ReasoningLevel::XHigh, ReasoningLevel::High, ReasoningLevel::Medium, ReasoningLevel::Low];
        self.default_inference = Some(ReasoningLevel::Medium);
        self
    }

    fn with_default_inference(mut self, level: ReasoningLevel) -> Self {
        self.default_inference = Some(level);
        self
    }
}

impl CodingAgent {
    /// Get model options matching modelOptions.ts
    pub fn models(&self) -> Vec<ModelOption> {
        match self {
            CodingAgent::ClaudeCode => vec![
                ModelOption::default_option("Default (Auto)", "Use Claude Code default behavior"),
                ModelOption::new("opus", "Opus 4.5", "Official Opus alias for Claude Code (non-custom, matches /model option)."),
                ModelOption::new("sonnet", "Sonnet 4.5", "Official Sonnet alias for Claude Code."),
                ModelOption::new("haiku", "Haiku 4.5", "Official Haiku alias for Claude Code (fastest model, non-custom)."),
            ],
            CodingAgent::CodexCli => vec![
                ModelOption::default_option("Default (Auto)", "Use Codex default model")
                    .with_base_levels()
                    .with_default_inference(ReasoningLevel::High),
                ModelOption::new("gpt-5.2-codex", "gpt-5.2-codex", "Latest frontier agentic coding model")
                    .with_max_levels()
                    .with_default_inference(ReasoningLevel::High),
                ModelOption::new("gpt-5.1-codex-max", "gpt-5.1-codex-max", "Codex-optimized flagship for deep and fast reasoning.")
                    .with_max_levels(),
                ModelOption::new("gpt-5.1-codex-mini", "gpt-5.1-codex-mini", "Optimized for codex. Cheaper, faster, but less capable.")
                    .with_base_levels(),
                ModelOption::new("gpt-5.2", "gpt-5.2", "Latest frontier model with improvements across knowledge, reasoning and coding")
                    .with_max_levels(),
            ],
            CodingAgent::GeminiCli => vec![
                ModelOption::default_option("Default (Auto)", "Use Gemini CLI default model"),
                ModelOption::new("gemini-3-pro-preview", "Pro (gemini-3-pro-preview)", "Default Pro. Falls back to gemini-2.5-pro when preview is unavailable."),
                ModelOption::new("gemini-3-flash-preview", "Flash (gemini-3-flash-preview)", "Next-generation high-speed model"),
                ModelOption::new("gemini-2.5-pro", "Pro (gemini-2.5-pro)", "Stable Pro model for deep reasoning and creativity"),
                ModelOption::new("gemini-2.5-flash", "Flash (gemini-2.5-flash)", "Balance of speed and reasoning"),
                ModelOption::new("gemini-2.5-flash-lite", "Flash-Lite (gemini-2.5-flash-lite)", "Fastest for simple tasks"),
            ],
            CodingAgent::OpenCode => vec![
                ModelOption::default_option("Default (Auto)", "Use OpenCode default model"),
                ModelOption::new("__custom__", "Custom (provider/model)", "Enter a provider/model identifier"),
            ],
        }
    }
}

/// Execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    #[default]
    Normal,
    Continue,
    Resume,
}

impl ExecutionMode {
    pub fn label(&self) -> &'static str {
        match self {
            ExecutionMode::Normal => "Normal",
            ExecutionMode::Continue => "Continue",
            ExecutionMode::Resume => "Resume",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ExecutionMode::Normal => "Start a new session",
            ExecutionMode::Continue => "Continue from last session",
            ExecutionMode::Resume => "Resume a specific session",
        }
    }

    pub fn all() -> &'static [ExecutionMode] {
        &[ExecutionMode::Normal, ExecutionMode::Continue, ExecutionMode::Resume]
    }
}

/// Branch type for new branches
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
            BranchType::Feature => "feature/",
            BranchType::Bugfix => "bugfix/",
            BranchType::Hotfix => "hotfix/",
            BranchType::Release => "release/",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            BranchType::Feature => "New feature branch",
            BranchType::Bugfix => "Bug fix branch",
            BranchType::Hotfix => "Hotfix branch",
            BranchType::Release => "Release branch",
        }
    }

    pub fn all() -> &'static [BranchType] {
        &[BranchType::Feature, BranchType::Bugfix, BranchType::Hotfix, BranchType::Release]
    }
}

/// Wizard state
#[derive(Debug, Default)]
pub struct WizardState {
    /// Whether wizard is visible
    pub visible: bool,
    /// Is this a new branch flow
    pub is_new_branch: bool,
    /// Current wizard step
    pub step: WizardStep,
    /// Selected branch name (for existing branch)
    pub branch_name: String,
    /// Selected branch type (for new branch)
    pub branch_type: BranchType,
    /// New branch name input
    pub new_branch_name: String,
    /// Cursor position for branch name input
    pub cursor: usize,
    /// Selected coding agent
    pub agent: CodingAgent,
    /// Selected agent index
    pub agent_index: usize,
    /// Selected model
    pub model: String,
    /// Selected model index
    pub model_index: usize,
    /// Selected reasoning level (Codex only)
    pub reasoning_level: ReasoningLevel,
    /// Selected reasoning level index
    pub reasoning_level_index: usize,
    /// Selected version
    pub version: String,
    /// Version options
    pub versions: Vec<String>,
    /// Selected version index
    pub version_index: usize,
    /// Selected execution mode
    pub execution_mode: ExecutionMode,
    /// Selected execution mode index
    pub execution_mode_index: usize,
    /// Skip permissions
    pub skip_permissions: bool,
    /// Scroll offset for popup content
    pub scroll_offset: usize,
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            versions: vec!["installed".to_string(), "latest".to_string()],
            ..Default::default()
        }
    }

    /// Open wizard for existing branch
    pub fn open_for_branch(&mut self, branch_name: &str) {
        self.visible = true;
        self.is_new_branch = false;
        self.branch_name = branch_name.to_string();
        self.step = WizardStep::AgentSelect;
        self.reset_selections();
    }

    /// Open wizard for new branch
    pub fn open_for_new_branch(&mut self) {
        self.visible = true;
        self.is_new_branch = true;
        self.step = WizardStep::BranchTypeSelect;
        self.reset_selections();
    }

    /// Reset all selections to default
    fn reset_selections(&mut self) {
        self.agent = CodingAgent::default();
        self.agent_index = 0;
        self.model = String::new();
        self.model_index = 0;
        self.reasoning_level = ReasoningLevel::default();
        self.reasoning_level_index = 1; // Medium
        self.version = "latest".to_string();
        self.version_index = 1;
        self.execution_mode = ExecutionMode::default();
        self.execution_mode_index = 0;
        self.skip_permissions = false;
        self.branch_type = BranchType::default();
        self.new_branch_name.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    /// Close wizard
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Go to next step
    pub fn next_step(&mut self) {
        self.step = match self.step {
            WizardStep::BranchTypeSelect => WizardStep::BranchNameInput,
            WizardStep::BranchNameInput => WizardStep::AgentSelect,
            WizardStep::AgentSelect => {
                // Set model based on selected agent
                let models = self.agent.models();
                if !models.is_empty() {
                    self.model = models[0].id.clone();
                }
                WizardStep::ModelSelect
            }
            WizardStep::ModelSelect => {
                // Skip to version select unless Codex
                if self.agent == CodingAgent::CodexCli {
                    WizardStep::ReasoningLevel
                } else {
                    WizardStep::VersionSelect
                }
            }
            WizardStep::ReasoningLevel => WizardStep::VersionSelect,
            WizardStep::VersionSelect => WizardStep::ExecutionMode,
            WizardStep::ExecutionMode => WizardStep::SkipPermissions,
            WizardStep::SkipPermissions => WizardStep::SkipPermissions, // Final step
        };
        self.scroll_offset = 0;
    }

    /// Go to previous step
    pub fn prev_step(&mut self) -> bool {
        let prev = match self.step {
            WizardStep::BranchTypeSelect => {
                self.close();
                return false;
            }
            WizardStep::BranchNameInput => WizardStep::BranchTypeSelect,
            WizardStep::AgentSelect => {
                if self.is_new_branch {
                    WizardStep::BranchNameInput
                } else {
                    self.close();
                    return false;
                }
            }
            WizardStep::ModelSelect => WizardStep::AgentSelect,
            WizardStep::ReasoningLevel => WizardStep::ModelSelect,
            WizardStep::VersionSelect => {
                if self.agent == CodingAgent::CodexCli {
                    WizardStep::ReasoningLevel
                } else {
                    WizardStep::ModelSelect
                }
            }
            WizardStep::ExecutionMode => WizardStep::VersionSelect,
            WizardStep::SkipPermissions => WizardStep::ExecutionMode,
        };
        self.step = prev;
        self.scroll_offset = 0;
        true
    }

    /// Select next item in current step
    pub fn select_next(&mut self) {
        match self.step {
            WizardStep::AgentSelect => {
                let max = CodingAgent::all().len().saturating_sub(1);
                if self.agent_index < max {
                    self.agent_index += 1;
                    self.agent = CodingAgent::all()[self.agent_index];
                }
            }
            WizardStep::ModelSelect => {
                let models = self.agent.models();
                let max = models.len().saturating_sub(1);
                if self.model_index < max {
                    self.model_index += 1;
                    self.model = models[self.model_index].id.clone();
                }
            }
            WizardStep::ReasoningLevel => {
                let max = ReasoningLevel::all().len().saturating_sub(1);
                if self.reasoning_level_index < max {
                    self.reasoning_level_index += 1;
                    self.reasoning_level = ReasoningLevel::all()[self.reasoning_level_index];
                }
            }
            WizardStep::VersionSelect => {
                let max = self.versions.len().saturating_sub(1);
                if self.version_index < max {
                    self.version_index += 1;
                    self.version = self.versions[self.version_index].clone();
                }
            }
            WizardStep::ExecutionMode => {
                let max = ExecutionMode::all().len().saturating_sub(1);
                if self.execution_mode_index < max {
                    self.execution_mode_index += 1;
                    self.execution_mode = ExecutionMode::all()[self.execution_mode_index];
                }
            }
            WizardStep::SkipPermissions => {
                self.skip_permissions = !self.skip_permissions;
            }
            WizardStep::BranchTypeSelect => {
                let types = BranchType::all();
                let current_idx = types.iter().position(|t| *t == self.branch_type).unwrap_or(0);
                if current_idx < types.len() - 1 {
                    self.branch_type = types[current_idx + 1];
                }
            }
            WizardStep::BranchNameInput => {
                // No selection in input mode
            }
        }
    }

    /// Select previous item in current step
    pub fn select_prev(&mut self) {
        match self.step {
            WizardStep::AgentSelect => {
                if self.agent_index > 0 {
                    self.agent_index -= 1;
                    self.agent = CodingAgent::all()[self.agent_index];
                }
            }
            WizardStep::ModelSelect => {
                if self.model_index > 0 {
                    self.model_index -= 1;
                    self.model = self.agent.models()[self.model_index].id.clone();
                }
            }
            WizardStep::ReasoningLevel => {
                if self.reasoning_level_index > 0 {
                    self.reasoning_level_index -= 1;
                    self.reasoning_level = ReasoningLevel::all()[self.reasoning_level_index];
                }
            }
            WizardStep::VersionSelect => {
                if self.version_index > 0 {
                    self.version_index -= 1;
                    self.version = self.versions[self.version_index].clone();
                }
            }
            WizardStep::ExecutionMode => {
                if self.execution_mode_index > 0 {
                    self.execution_mode_index -= 1;
                    self.execution_mode = ExecutionMode::all()[self.execution_mode_index];
                }
            }
            WizardStep::SkipPermissions => {
                self.skip_permissions = !self.skip_permissions;
            }
            WizardStep::BranchTypeSelect => {
                let types = BranchType::all();
                let current_idx = types.iter().position(|t| *t == self.branch_type).unwrap_or(0);
                if current_idx > 0 {
                    self.branch_type = types[current_idx - 1];
                }
            }
            WizardStep::BranchNameInput => {
                // No selection in input mode
            }
        }
    }

    /// Insert character in branch name input
    pub fn insert_char(&mut self, c: char) {
        if self.step == WizardStep::BranchNameInput {
            self.new_branch_name.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Delete character in branch name input
    pub fn delete_char(&mut self) {
        if self.step == WizardStep::BranchNameInput && self.cursor > 0 {
            self.cursor -= 1;
            self.new_branch_name.remove(self.cursor);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.cursor < self.new_branch_name.len() {
            self.cursor += 1;
        }
    }

    /// Get full branch name for new branch
    pub fn full_branch_name(&self) -> String {
        format!("{}{}", self.branch_type.prefix(), self.new_branch_name)
    }

    /// Check if wizard is complete
    pub fn is_complete(&self) -> bool {
        self.step == WizardStep::SkipPermissions
    }
}

/// Render wizard popup overlay
pub fn render_wizard(state: &WizardState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    // Calculate popup dimensions (60% of screen, centered)
    let popup_width = (area.width as f32 * 0.6) as u16;
    let popup_height = (area.height as f32 * 0.6) as u16;
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear background with dim overlay effect
    frame.render_widget(Clear, popup_area);

    // Popup border
    let title = match state.step {
        WizardStep::BranchTypeSelect => " Select Branch Type ",
        WizardStep::BranchNameInput => " Enter Branch Name ",
        WizardStep::AgentSelect => " Select Coding Agent ",
        WizardStep::ModelSelect => " Select Model ",
        WizardStep::ReasoningLevel => " Select Reasoning Level ",
        WizardStep::VersionSelect => " Select Version ",
        WizardStep::ExecutionMode => " Select Execution Mode ",
        WizardStep::SkipPermissions => " Skip Permissions? ",
    };

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let inner_area = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);

    // Render step content
    let content_area = Rect::new(
        inner_area.x + 1,
        inner_area.y + 1,
        inner_area.width.saturating_sub(2),
        inner_area.height.saturating_sub(4),
    );

    match state.step {
        WizardStep::BranchTypeSelect => render_branch_type_step(state, frame, content_area),
        WizardStep::BranchNameInput => render_branch_name_step(state, frame, content_area),
        WizardStep::AgentSelect => render_agent_step(state, frame, content_area),
        WizardStep::ModelSelect => render_model_step(state, frame, content_area),
        WizardStep::ReasoningLevel => render_reasoning_step(state, frame, content_area),
        WizardStep::VersionSelect => render_version_step(state, frame, content_area),
        WizardStep::ExecutionMode => render_execution_mode_step(state, frame, content_area),
        WizardStep::SkipPermissions => render_skip_permissions_step(state, frame, content_area),
    }

    // Footer with keybindings
    let footer_area = Rect::new(
        inner_area.x,
        inner_area.y + inner_area.height - 2,
        inner_area.width,
        1,
    );
    let footer_text = if state.step == WizardStep::BranchNameInput {
        "[Enter] Confirm  [Esc] Back"
    } else {
        "[Enter] Select  [Esc] Back  [Up/Down] Navigate"
    };
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, footer_area);
}

fn render_branch_type_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let types = BranchType::all();
    let items: Vec<ListItem> = types
        .iter()
        .map(|t| {
            let is_selected = *t == state.branch_type;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let text = format!("{}{:<12} {}", prefix, t.prefix(), t.description());
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_branch_name_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Label
            Constraint::Length(1), // Empty
            Constraint::Length(1), // Input
        ])
        .split(area);

    // Label
    let label = Paragraph::new(format!("Branch: {}", state.branch_type.prefix()))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    frame.render_widget(label, chunks[0]);

    // Input field
    let input_text = if state.new_branch_name.is_empty() {
        "Enter branch name...".to_string()
    } else {
        state.new_branch_name.clone()
    };
    let input_style = if state.new_branch_name.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };
    let input = Paragraph::new(input_text).style(input_style);
    frame.render_widget(input, chunks[2]);

    // Show cursor
    if !state.new_branch_name.is_empty() || state.cursor == 0 {
        frame.set_cursor_position((
            chunks[2].x + state.cursor as u16,
            chunks[2].y,
        ));
    }
}

fn render_agent_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show branch name if selecting for existing branch
    let start_y = if !state.is_new_branch {
        let branch_info = Paragraph::new(format!("Branch: {}", state.branch_name))
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
        frame.render_widget(branch_info, Rect::new(area.x, area.y, area.width, 1));
        2
    } else {
        0
    };

    let agents = CodingAgent::all();
    let items: Vec<ListItem> = agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let is_selected = i == state.agent_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default().fg(agent.color())
            };
            ListItem::new(format!("{}{}", prefix, agent.label())).style(style)
        })
        .collect();

    let list_area = Rect::new(area.x, area.y + start_y as u16, area.width, area.height.saturating_sub(start_y as u16));
    let list = List::new(items);
    frame.render_widget(list, list_area);
}

fn render_model_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let models = state.agent.models();
    let items: Vec<ListItem> = models
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let is_selected = i == state.model_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let desc = model.description.as_deref().unwrap_or("");
            let text = if desc.is_empty() {
                format!("{}{}", prefix, model.label)
            } else {
                format!("{}{:<20} {}", prefix, model.label, desc)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_reasoning_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let levels = ReasoningLevel::all();
    let items: Vec<ListItem> = levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            let is_selected = i == state.reasoning_level_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let text = format!("{}{:<10} {}", prefix, level.label(), level.description());
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_version_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let items: Vec<ListItem> = state
        .versions
        .iter()
        .enumerate()
        .map(|(i, version)| {
            let is_selected = i == state.version_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            ListItem::new(format!("{}{}", prefix, version)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_execution_mode_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let modes = ExecutionMode::all();
    let items: Vec<ListItem> = modes
        .iter()
        .enumerate()
        .map(|(i, mode)| {
            let is_selected = i == state.execution_mode_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let text = format!("{}{:<12} {}", prefix, mode.label(), mode.description());
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_skip_permissions_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let options = [("Yes", true), ("No", false)];
    let items: Vec<ListItem> = options
        .iter()
        .map(|(label, value)| {
            let is_selected = state.skip_permissions == *value;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let desc = if *value {
                "Skip permission prompts"
            } else {
                "Show permission prompts"
            };
            let text = format!("{}{:<6} {}", prefix, label, desc);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_open_for_branch() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test");
        assert!(state.visible);
        assert!(!state.is_new_branch);
        assert_eq!(state.branch_name, "feature/test");
        assert_eq!(state.step, WizardStep::AgentSelect);
    }

    #[test]
    fn test_wizard_open_for_new_branch() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        assert!(state.visible);
        assert!(state.is_new_branch);
        assert_eq!(state.step, WizardStep::BranchTypeSelect);
    }

    #[test]
    fn test_wizard_step_navigation() {
        let mut state = WizardState::new();
        state.open_for_branch("test");

        assert_eq!(state.step, WizardStep::AgentSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::ModelSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::VersionSelect);
    }

    #[test]
    fn test_wizard_codex_reasoning_step() {
        let mut state = WizardState::new();
        state.open_for_branch("test");
        state.agent = CodingAgent::CodexCli;
        state.agent_index = 1;

        state.next_step(); // ModelSelect
        state.next_step(); // ReasoningLevel (because Codex)
        assert_eq!(state.step, WizardStep::ReasoningLevel);
    }

    #[test]
    fn test_wizard_selection() {
        let mut state = WizardState::new();
        state.open_for_branch("test");

        state.select_next();
        assert_eq!(state.agent_index, 1);
        assert_eq!(state.agent, CodingAgent::CodexCli);

        state.select_prev();
        assert_eq!(state.agent_index, 0);
        assert_eq!(state.agent, CodingAgent::ClaudeCode);
    }
}
