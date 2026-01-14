//! Wizard Popup Screen - TypeScript version compatible
//!
//! FR-044: Wizard popup overlay on branch selection
//! FR-045: Semi-transparent overlay background
//! FR-046: Centered popup with z-index
//! FR-047: Steps within same popup
//! FR-062~FR-073: Version selection flow

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

/// Version information from npm registry (FR-063)
#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub version: String,
    pub is_prerelease: bool,
    pub published_at: Option<String>,
}

/// Installed version information (FR-063a/b)
#[derive(Debug, Clone)]
pub struct InstalledVersionInfo {
    pub version: String,
    pub path: String,
}

/// Version option for display
#[derive(Debug, Clone)]
pub struct VersionOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

impl VersionOption {
    fn installed(version: &str, path: &str) -> Self {
        Self {
            // FR-063a: Display as "installed (X.Y.Z)" with path in description
            label: format!("installed ({})", version),
            value: "installed".to_string(),
            description: Some(path.to_string()),
        }
    }

    fn latest() -> Self {
        Self {
            label: "latest".to_string(),
            value: "latest".to_string(),
            description: Some("Always use the latest version".to_string()),
        }
    }

    fn from_version(v: &VersionInfo) -> Self {
        let label = if v.is_prerelease {
            format!("{} (pre)", v.version)
        } else {
            v.version.clone()
        };
        Self {
            label,
            value: v.version.clone(),
            description: v.published_at.as_ref().map(|d| {
                // Format: 2024-01-15T10:30:00Z -> 2024-01-15
                d.split('T').next().unwrap_or(d).to_string()
            }),
        }
    }
}

/// npm registry response structure
#[derive(Debug, Deserialize)]
struct NpmRegistryResponse {
    #[serde(rename = "dist-tags")]
    dist_tags: Option<HashMap<String, String>>,
    time: Option<HashMap<String, String>>,
    versions: Option<HashMap<String, serde_json::Value>>,
}

/// Check if version is prerelease
fn is_prerelease(version: &str) -> bool {
    // Prerelease versions contain - followed by alpha, beta, rc, canary, next, etc.
    version.contains("-alpha")
        || version.contains("-beta")
        || version.contains("-rc")
        || version.contains("-canary")
        || version.contains("-next")
        || version.contains("-dev")
        || version.contains("-pre")
}

/// Wizard step types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WizardStep {
    /// Quick Start: Show previous settings per agent (FR-050, SPEC-f47db390)
    QuickStart,
    #[default]
    AgentSelect,
    ModelSelect,
    ReasoningLevel, // Codex only
    VersionSelect,
    ExecutionMode,
    SkipPermissions,
    // New branch flow
    BranchTypeSelect,
    BranchNameInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardConfirmResult {
    Advance,
    Complete,
}

/// Quick Start option types (FR-050)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickStartAction {
    /// Resume with previous settings (uses sessionId)
    ResumeWithPrevious,
    /// Start new with previous settings (no sessionId)
    StartNewWithPrevious,
    /// Choose different settings
    ChooseDifferent,
}

/// Quick Start entry for a tool (FR-050)
#[derive(Debug, Clone)]
pub struct QuickStartEntry {
    /// Tool ID
    pub tool_id: String,
    /// Tool label (display name)
    pub tool_label: String,
    /// Model used
    pub model: Option<String>,
    /// Reasoning level (Codex only)
    pub reasoning_level: Option<String>,
    /// Tool version
    pub version: Option<String>,
    /// Session ID for resume
    pub session_id: Option<String>,
    /// Skip permissions setting
    pub skip_permissions: Option<bool>,
}

/// Coding agent types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
            CodingAgent::ClaudeCode => Color::Yellow, // Yellow (#f6e05e)
            CodingAgent::CodexCli => Color::Cyan,     // Cyan (#4fd1c5)
            CodingAgent::GeminiCli => Color::Magenta, // Magenta (#d53f8c)
            CodingAgent::OpenCode => Color::Green,    // Green (#48bb78)
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

    /// Get npm package name for this agent (FR-063)
    pub fn npm_package(&self) -> &'static str {
        match self {
            CodingAgent::ClaudeCode => "@anthropic-ai/claude-code",
            CodingAgent::CodexCli => "@openai/codex",
            CodingAgent::GeminiCli => "@google/gemini-cli",
            CodingAgent::OpenCode => "opencode-ai",
        }
    }

    /// Get command name for version detection (FR-063a)
    pub fn command_name(&self) -> &'static str {
        match self {
            CodingAgent::ClaudeCode => "claude",
            CodingAgent::CodexCli => "codex",
            CodingAgent::GeminiCli => "gemini",
            CodingAgent::OpenCode => "opencode",
        }
    }
}

/// Fetch package versions from npm registry (FR-063, FR-064)
/// Returns up to 10 recent versions sorted by publish date
pub fn fetch_package_versions(package_name: &str) -> Vec<VersionInfo> {
    const TIMEOUT_SECS: u64 = 3;
    const LIMIT: usize = 10;

    let url = format!(
        "https://registry.npmjs.org/{}",
        urlencoding::encode(package_name)
    );

    // Create agent with timeout config
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(TIMEOUT_SECS)))
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let result = agent.get(&url).call();

    let response = match result {
        Ok(resp) => resp,
        Err(_) => return vec![],
    };

    // Read body as string and parse JSON
    let body = match response.into_body().read_to_string() {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let data: NpmRegistryResponse = match serde_json::from_str(&body) {
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

    // Take top N versions
    versions_with_time
        .into_iter()
        .take(LIMIT)
        .map(|(version, published_at)| VersionInfo {
            is_prerelease: is_prerelease(&version),
            version,
            published_at: Some(published_at),
        })
        .collect()
}

/// Detect installed version of an agent (FR-063a, FR-063b)
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
        // Extract version number (semver pattern)
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
        &[
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::XHigh,
        ]
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
        &[
            ExecutionMode::Normal,
            ExecutionMode::Continue,
            ExecutionMode::Resume,
        ]
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
        &[
            BranchType::Feature,
            BranchType::Bugfix,
            BranchType::Hotfix,
            BranchType::Release,
        ]
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
    /// Version options (FR-063: dynamic list)
    pub version_options: Vec<VersionOption>,
    /// Selected version index
    pub version_index: usize,
    /// Whether versions have been fetched for current agent
    pub versions_fetched: bool,
    /// Cached installed versions per agent (FR-017)
    pub installed_cache: HashMap<CodingAgent, Option<InstalledVersionInfo>>,
    /// Selected execution mode
    pub execution_mode: ExecutionMode,
    /// Selected execution mode index
    pub execution_mode_index: usize,
    /// Skip permissions
    pub skip_permissions: bool,
    /// Scroll offset for popup content
    pub scroll_offset: usize,
    // Quick Start (FR-050, SPEC-f47db390)
    /// Quick Start entries per tool
    pub quick_start_entries: Vec<QuickStartEntry>,
    /// Selected Quick Start index (flattened: each tool has 2 options + 1 choose different)
    pub quick_start_index: usize,
    /// Whether Quick Start should be shown (has previous history)
    pub has_quick_start: bool,
    /// FR-074: Block first Enter in VersionSelect to prevent auto-advance
    pub block_next_enter: bool,
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            version_options: vec![VersionOption::latest()],
            ..Default::default()
        }
    }

    /// Open wizard for existing branch (FR-050)
    /// If history entries are provided, shows Quick Start first
    pub fn open_for_branch(&mut self, branch_name: &str, history: Vec<QuickStartEntry>) {
        self.visible = true;
        self.is_new_branch = false;
        self.branch_name = branch_name.to_string();
        self.reset_selections();

        // FR-050: Show Quick Start if history exists
        if history.is_empty() {
            self.step = WizardStep::AgentSelect;
            self.has_quick_start = false;
            self.quick_start_entries.clear();
        } else {
            self.step = WizardStep::QuickStart;
            self.has_quick_start = true;
            self.quick_start_entries = history;
            self.quick_start_index = 0;
        }
    }

    /// Open wizard for new branch
    pub fn open_for_new_branch(&mut self) {
        self.visible = true;
        self.is_new_branch = true;
        self.step = WizardStep::BranchTypeSelect;
        self.reset_selections();
        self.has_quick_start = false;
        self.quick_start_entries.clear();
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
        self.version_options = vec![VersionOption::latest()];
        self.version_index = 0;
        self.versions_fetched = false;
        self.execution_mode = ExecutionMode::default();
        self.execution_mode_index = 0;
        self.skip_permissions = false;
        self.branch_type = BranchType::default();
        self.new_branch_name.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.quick_start_index = 0;
    }

    /// Get the total number of Quick Start options (FR-050)
    /// Each tool has 2 options (Resume, Start New) + 1 "Choose different settings"
    pub fn quick_start_option_count(&self) -> usize {
        if self.quick_start_entries.is_empty() {
            0
        } else {
            // Each entry has 2 options, plus 1 "Choose different" at the end
            self.quick_start_entries.len() * 2 + 1
        }
    }

    /// Get the selected Quick Start action and tool index (FR-050)
    /// Returns (action, tool_index) or None if "Choose different" is selected
    pub fn selected_quick_start_action(&self) -> Option<(QuickStartAction, usize)> {
        if self.quick_start_entries.is_empty() {
            return None;
        }

        let entries_count = self.quick_start_entries.len();
        let choose_different_index = entries_count * 2;

        if self.quick_start_index >= choose_different_index {
            // "Choose different settings" selected
            return None;
        }

        // Each tool has 2 options: Resume (even index), Start New (odd index)
        let tool_index = self.quick_start_index / 2;
        let is_resume = self.quick_start_index.is_multiple_of(2);

        let action = if is_resume {
            QuickStartAction::ResumeWithPrevious
        } else {
            QuickStartAction::StartNewWithPrevious
        };

        Some((action, tool_index))
    }

    /// Apply Quick Start selection to wizard state (FR-050)
    pub fn apply_quick_start_selection(&mut self, tool_index: usize, action: QuickStartAction) {
        if let Some(entry) = self.quick_start_entries.get(tool_index) {
            // Map tool_id to CodingAgent
            self.agent = match entry.tool_id.as_str() {
                "claude-code" => CodingAgent::ClaudeCode,
                "codex-cli" => CodingAgent::CodexCli,
                "gemini-cli" => CodingAgent::GeminiCli,
                "opencode" => CodingAgent::OpenCode,
                _ => CodingAgent::ClaudeCode,
            };

            // Set model if available
            if let Some(model) = &entry.model {
                self.model = model.clone();
            }

            // Set reasoning level for Codex
            if let Some(level) = &entry.reasoning_level {
                self.reasoning_level = match level.as_str() {
                    "low" => ReasoningLevel::Low,
                    "medium" => ReasoningLevel::Medium,
                    "high" => ReasoningLevel::High,
                    "xhigh" => ReasoningLevel::XHigh,
                    _ => ReasoningLevel::Medium,
                };
            }

            // Set version if available
            if let Some(version) = &entry.version {
                self.version = version.clone();
            }

            // Set skip permissions
            self.skip_permissions = entry.skip_permissions.unwrap_or(false);

            // Set execution mode based on action
            self.execution_mode = match action {
                QuickStartAction::ResumeWithPrevious => ExecutionMode::Resume,
                QuickStartAction::StartNewWithPrevious => ExecutionMode::Normal,
                QuickStartAction::ChooseDifferent => ExecutionMode::Normal,
            };
        }
    }

    /// Fetch versions for current agent (FR-063, FR-064)
    /// Called when entering VersionSelect step
    pub fn fetch_versions_for_agent(&mut self) {
        if self.versions_fetched {
            return;
        }

        let mut options = Vec::new();

        // Check installed version (FR-063a, FR-063b)
        let installed = if let Some(cached) = self.installed_cache.get(&self.agent) {
            cached.clone()
        } else {
            let result = detect_installed_version(self.agent);
            self.installed_cache.insert(self.agent, result.clone());
            result
        };

        // Add installed option first if available
        if let Some(info) = installed {
            options.push(VersionOption::installed(&info.version, &info.path));
        }

        // Add "latest" option
        options.push(VersionOption::latest());

        // Fetch from npm registry (FR-063, FR-064)
        let npm_versions = fetch_package_versions(self.agent.npm_package());
        for v in npm_versions {
            options.push(VersionOption::from_version(&v));
        }

        self.version_options = options;
        self.version_index = 0;
        if !self.version_options.is_empty() {
            self.version = self.version_options[0].value.clone();
        }
        self.versions_fetched = true;
    }

    /// Prefetch installed versions for all agents at startup (FR-017)
    pub fn prefetch_installed_versions(&mut self) {
        for agent in CodingAgent::all() {
            if !self.installed_cache.contains_key(agent) {
                let result = detect_installed_version(*agent);
                self.installed_cache.insert(*agent, result);
            }
        }
    }

    /// Close wizard
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Go to next step
    pub fn next_step(&mut self) {
        self.step = match self.step {
            WizardStep::QuickStart => {
                // FR-050: Handle Quick Start selection
                if let Some((action, tool_index)) = self.selected_quick_start_action() {
                    match action {
                        QuickStartAction::ResumeWithPrevious
                        | QuickStartAction::StartNewWithPrevious => {
                            // Apply settings from history and skip to completion
                            self.apply_quick_start_selection(tool_index, action);
                            WizardStep::SkipPermissions
                        }
                        QuickStartAction::ChooseDifferent => {
                            // Go to agent selection for manual configuration
                            WizardStep::AgentSelect
                        }
                    }
                } else {
                    // No history, go to agent selection
                    WizardStep::AgentSelect
                }
            }
            WizardStep::BranchTypeSelect => WizardStep::BranchNameInput,
            WizardStep::BranchNameInput => WizardStep::AgentSelect,
            WizardStep::AgentSelect => {
                // Set model based on selected agent
                let models = self.agent.models();
                if !models.is_empty() {
                    self.model = models[0].id.clone();
                }
                // Reset version fetch when agent changes
                self.versions_fetched = false;
                WizardStep::ModelSelect
            }
            WizardStep::ModelSelect => {
                // Skip to version select unless Codex
                if self.agent == CodingAgent::CodexCli {
                    WizardStep::ReasoningLevel
                } else {
                    // Fetch versions when entering VersionSelect (FR-063)
                    self.fetch_versions_for_agent();
                    // FR-074: Block first Enter to prevent auto-advance
                    self.block_next_enter = true;
                    WizardStep::VersionSelect
                }
            }
            WizardStep::ReasoningLevel => {
                // Fetch versions when entering VersionSelect (FR-063)
                self.fetch_versions_for_agent();
                // FR-074: Block first Enter to prevent auto-advance
                self.block_next_enter = true;
                WizardStep::VersionSelect
            }
            WizardStep::VersionSelect => WizardStep::ExecutionMode,
            WizardStep::ExecutionMode => WizardStep::SkipPermissions,
            WizardStep::SkipPermissions => WizardStep::SkipPermissions, // Final step
        };
        self.scroll_offset = 0;
    }

    /// Go to previous step
    pub fn prev_step(&mut self) -> bool {
        let prev = match self.step {
            WizardStep::QuickStart => {
                // FR-050: Escape in Quick Start closes wizard
                self.close();
                return false;
            }
            WizardStep::BranchTypeSelect => {
                self.close();
                return false;
            }
            WizardStep::BranchNameInput => WizardStep::BranchTypeSelect,
            WizardStep::AgentSelect => {
                if self.is_new_branch {
                    WizardStep::BranchNameInput
                } else if self.has_quick_start {
                    // FR-050: Go back to Quick Start if available
                    WizardStep::QuickStart
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

    pub fn confirm(&mut self) -> WizardConfirmResult {
        if self.step == WizardStep::QuickStart {
            if let Some((action, tool_index)) = self.selected_quick_start_action() {
                match action {
                    QuickStartAction::ResumeWithPrevious
                    | QuickStartAction::StartNewWithPrevious => {
                        self.apply_quick_start_selection(tool_index, action);
                        self.step = WizardStep::SkipPermissions;
                        self.scroll_offset = 0;
                        return WizardConfirmResult::Complete;
                    }
                    QuickStartAction::ChooseDifferent => {
                        self.step = WizardStep::AgentSelect;
                        self.scroll_offset = 0;
                        return WizardConfirmResult::Advance;
                    }
                }
            } else {
                self.step = WizardStep::AgentSelect;
                self.scroll_offset = 0;
                return WizardConfirmResult::Advance;
            }
        }

        if self.is_complete() {
            WizardConfirmResult::Complete
        } else {
            self.next_step();
            WizardConfirmResult::Advance
        }
    }

    /// Select next item in current step
    pub fn select_next(&mut self) {
        match self.step {
            WizardStep::QuickStart => {
                // FR-050: Navigate Quick Start options
                let max = self.quick_start_option_count().saturating_sub(1);
                if self.quick_start_index < max {
                    self.quick_start_index += 1;
                }
            }
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
                let max = self.version_options.len().saturating_sub(1);
                if self.version_index < max {
                    self.version_index += 1;
                    self.version = self.version_options[self.version_index].value.clone();
                    // FR-062: Scroll to keep cursor in view
                    self.ensure_version_visible();
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
                let current_idx = types
                    .iter()
                    .position(|t| *t == self.branch_type)
                    .unwrap_or(0);
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
            WizardStep::QuickStart => {
                // FR-050: Navigate Quick Start options
                if self.quick_start_index > 0 {
                    self.quick_start_index -= 1;
                }
            }
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
                    self.version = self.version_options[self.version_index].value.clone();
                    // FR-062: Scroll to keep cursor in view
                    self.ensure_version_visible();
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
                let current_idx = types
                    .iter()
                    .position(|t| *t == self.branch_type)
                    .unwrap_or(0);
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

    /// Ensure version selection is visible (FR-062)
    /// Adjusts scroll_offset to keep version_index within visible area
    fn ensure_version_visible(&mut self) {
        const VISIBLE_ITEMS: usize = 8; // Approximate visible items in popup

        // Scroll down if cursor is below visible area
        if self.version_index >= self.scroll_offset + VISIBLE_ITEMS {
            self.scroll_offset = self.version_index.saturating_sub(VISIBLE_ITEMS - 1);
        }
        // Scroll up if cursor is above visible area
        if self.version_index < self.scroll_offset {
            self.scroll_offset = self.version_index;
        }
    }

    /// Get visible item count for wizard popup (FR-060)
    pub fn visible_items_count(&self, available_height: usize) -> usize {
        available_height.saturating_sub(2).max(3) // Leave room for header/footer
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

    // Render dark overlay background (FR-048 alternative implementation)
    let overlay = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(overlay, area);

    // Calculate popup dimensions (60% of screen, min 40x15) per FR-045
    let popup_width = ((area.width as f32 * 0.6) as u16).max(40);
    let popup_height = ((area.height as f32 * 0.6) as u16).max(15);
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear popup area
    frame.render_widget(Clear, popup_area);

    // Popup border with close hint (FR-047)
    let title = match state.step {
        WizardStep::QuickStart => " Quick Start ",
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
        .title_top(
            Line::from(title).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .title_top(Line::from(" [ESC] ").right_aligned());

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
        WizardStep::QuickStart => render_quick_start_step(state, frame, content_area),
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

/// Render Quick Start step (FR-050, SPEC-f47db390)
fn render_quick_start_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();

    // Show branch name
    let branch_info = Paragraph::new(format!("Branch: {}", state.branch_name)).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let branch_area = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(branch_info, branch_area);

    let list_area = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );

    // Build options list per tool (FR-050)
    for (tool_idx, entry) in state.quick_start_entries.iter().enumerate() {
        // Tool header with agent color
        let agent_color = match entry.tool_id.as_str() {
            "claude-code" => Color::Yellow,
            "codex-cli" => Color::Cyan,
            "gemini-cli" => Color::Magenta,
            "opencode" => Color::Green,
            _ => Color::White,
        };

        // Build tool info string with model (FR-011: Codex shows reasoning level)
        let tool_info = if entry.tool_id == "codex-cli" {
            if let Some(level) = &entry.reasoning_level {
                format!(
                    "{} ({}, Reasoning: {})",
                    entry.tool_label,
                    entry.model.as_deref().unwrap_or("default"),
                    level
                )
            } else {
                format!(
                    "{} ({})",
                    entry.tool_label,
                    entry.model.as_deref().unwrap_or("default")
                )
            }
        } else {
            format!(
                "{} ({})",
                entry.tool_label,
                entry.model.as_deref().unwrap_or("default")
            )
        };

        // Tool header line
        items.push(ListItem::new(tool_info).style(Style::default().fg(agent_color)));

        // Resume option (show session ID if available)
        let resume_idx = tool_idx * 2;
        let resume_selected = state.quick_start_index == resume_idx;
        let resume_prefix = if resume_selected { "> " } else { "  " };
        let resume_style = if resume_selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
        };
        let resume_text = if let Some(sid) = &entry.session_id {
            format!(
                "{}Resume with previous settings ({}...)",
                resume_prefix,
                &sid[..sid.len().min(8)]
            )
        } else {
            format!("{}Resume with previous settings", resume_prefix)
        };
        items.push(ListItem::new(resume_text).style(resume_style));

        // Start new option (FR-011: no session ID shown)
        let new_idx = tool_idx * 2 + 1;
        let new_selected = state.quick_start_index == new_idx;
        let new_prefix = if new_selected { "> " } else { "  " };
        let new_style = if new_selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
        };
        items.push(
            ListItem::new(format!("{}Start new with previous settings", new_prefix))
                .style(new_style),
        );

        // Empty line between tools
        if tool_idx < state.quick_start_entries.len() - 1 {
            items.push(ListItem::new(""));
        }
    }

    // Separator
    items.push(
        ListItem::new("─".repeat(list_area.width as usize))
            .style(Style::default().fg(Color::DarkGray)),
    );

    // "Choose different settings" option
    let choose_different_idx = state.quick_start_entries.len() * 2;
    let choose_selected = state.quick_start_index >= choose_different_idx;
    let choose_prefix = if choose_selected { "> " } else { "  " };
    let choose_style = if choose_selected {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        Style::default()
    };
    items.push(
        ListItem::new(format!("{}Choose different settings...", choose_prefix)).style(choose_style),
    );

    let list = List::new(items);
    frame.render_widget(list, list_area);
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
    let label = Paragraph::new(format!("Branch: {}", state.branch_type.prefix())).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
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
        frame.set_cursor_position((chunks[2].x + state.cursor as u16, chunks[2].y));
    }
}

fn render_agent_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show branch name if selecting for existing branch
    let start_y = if !state.is_new_branch {
        let branch_info = Paragraph::new(format!("Branch: {}", state.branch_name)).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
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

    let list_area = Rect::new(
        area.x,
        area.y + start_y as u16,
        area.width,
        area.height.saturating_sub(start_y as u16),
    );
    let list = List::new(items);
    frame.render_widget(list, list_area);
}

fn render_model_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let models = state.agent.models();
    let available_width = area.width as usize;

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
                // Dynamic width calculation to prevent text cutoff
                let label_width = model.label.len().min(25);
                let separator = " - ";
                let prefix_len = 2; // "> " or "  "
                let max_desc_width =
                    available_width.saturating_sub(prefix_len + label_width + separator.len());

                let truncated_desc = if desc.len() > max_desc_width && max_desc_width > 3 {
                    format!("{}...", &desc[..max_desc_width.saturating_sub(3)])
                } else if max_desc_width == 0 {
                    String::new()
                } else {
                    desc.to_string()
                };

                if truncated_desc.is_empty() {
                    format!("{}{}", prefix, model.label)
                } else {
                    format!("{}{}{}{}", prefix, model.label, separator, truncated_desc)
                }
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
    let available_width = area.width as usize;
    let available_height = area.height as usize;

    // FR-060: Calculate visible items based on available height
    let visible_count = available_height.min(state.version_options.len());
    let scroll_offset = state.scroll_offset;

    // FR-060: Show scroll indicator if there are more items
    let has_more_above = scroll_offset > 0;
    let has_more_below = scroll_offset + visible_count < state.version_options.len();

    // Reserve space for scroll indicators
    let list_height = if has_more_above || has_more_below {
        visible_count.saturating_sub(1)
    } else {
        visible_count
    };

    let items: Vec<ListItem> = state
        .version_options
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(list_height)
        .map(|(i, opt)| {
            let is_selected = i == state.version_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };

            // Format: label - description (if fits)
            let text = if let Some(desc) = &opt.description {
                let label_width = opt.label.len().min(20);
                let separator = " - ";
                let prefix_len = 2;
                let max_desc_width =
                    available_width.saturating_sub(prefix_len + label_width + separator.len());

                let truncated_desc = if desc.len() > max_desc_width && max_desc_width > 3 {
                    format!("{}...", &desc[..max_desc_width.saturating_sub(3)])
                } else if max_desc_width == 0 {
                    String::new()
                } else {
                    desc.to_string()
                };

                if truncated_desc.is_empty() {
                    format!("{}{}", prefix, opt.label)
                } else {
                    format!("{}{}{}{}", prefix, opt.label, separator, truncated_desc)
                }
            } else {
                format!("{}{}", prefix, opt.label)
            };

            ListItem::new(text).style(style)
        })
        .collect();

    // Render scroll indicator at top if needed (FR-060)
    let mut y_offset = 0;
    if has_more_above {
        let indicator =
            Paragraph::new("  ^ more above ^").style(Style::default().fg(Color::DarkGray));
        let indicator_area = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget(indicator, indicator_area);
        y_offset = 1;
    }

    // Render list
    let list_area = Rect::new(area.x, area.y + y_offset, area.width, list_height as u16);
    let list = List::new(items);
    frame.render_widget(list, list_area);

    // Render scroll indicator at bottom if needed (FR-060)
    if has_more_below {
        let indicator =
            Paragraph::new("  v more below v").style(Style::default().fg(Color::DarkGray));
        let indicator_area = Rect::new(
            area.x,
            area.y + y_offset + list_height as u16,
            area.width,
            1,
        );
        frame.render_widget(indicator, indicator_area);
    }
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
        state.open_for_branch("feature/test", vec![]);
        assert!(state.visible);
        assert!(!state.is_new_branch);
        assert_eq!(state.branch_name, "feature/test");
        // No history, so should start at AgentSelect
        assert_eq!(state.step, WizardStep::AgentSelect);
    }

    #[test]
    fn test_wizard_open_for_branch_with_history() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "claude-code".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: Some("1.0.0".to_string()),
            session_id: Some("abc123".to_string()),
            skip_permissions: None,
        }];
        state.open_for_branch("feature/test", history);
        assert!(state.visible);
        assert!(!state.is_new_branch);
        assert_eq!(state.branch_name, "feature/test");
        // With history, should start at QuickStart
        assert_eq!(state.step, WizardStep::QuickStart);
        assert!(state.has_quick_start);
        assert_eq!(state.quick_start_entries.len(), 1);
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
    fn test_opencode_model_options_include_default_and_custom() {
        let options = CodingAgent::OpenCode.models();
        assert!(!options.is_empty());
        assert!(options.iter().any(|option| option.is_default));
        assert!(options.iter().any(|option| option.id == "__custom__"));
    }

    #[test]
    fn test_wizard_step_navigation() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![]);

        assert_eq!(state.step, WizardStep::AgentSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::ModelSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::VersionSelect);
    }

    #[test]
    fn test_wizard_codex_reasoning_step() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![]);
        state.agent = CodingAgent::CodexCli;
        state.agent_index = 1;

        state.next_step(); // ModelSelect
        state.next_step(); // ReasoningLevel (because Codex)
        assert_eq!(state.step, WizardStep::ReasoningLevel);
    }

    #[test]
    fn test_wizard_selection() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![]);

        state.select_next();
        assert_eq!(state.agent_index, 1);
        assert_eq!(state.agent, CodingAgent::CodexCli);

        state.select_prev();
        assert_eq!(state.agent_index, 0);
        assert_eq!(state.agent, CodingAgent::ClaudeCode);
    }

    #[test]
    fn test_quick_start_resume_skips_to_completion() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "claude-code".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: Some("1.0.0".to_string()),
            session_id: Some("abc123".to_string()),
            skip_permissions: Some(true),
        }];
        state.open_for_branch("feature/test", history);

        // Should start at QuickStart
        assert_eq!(state.step, WizardStep::QuickStart);

        // Index 0 = Resume with previous settings (first tool)
        state.quick_start_index = 0;

        // Confirm should complete immediately
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Complete);
        assert_eq!(state.step, WizardStep::SkipPermissions);

        // Settings should be applied
        assert_eq!(state.agent, CodingAgent::ClaudeCode);
        assert_eq!(state.model, "sonnet");
        assert_eq!(state.version, "1.0.0");
        assert_eq!(state.execution_mode, ExecutionMode::Resume);
        assert!(state.skip_permissions);
    }

    #[test]
    fn test_quick_start_new_skips_to_completion() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex CLI".to_string(),
            model: Some("o3-mini".to_string()),
            reasoning_level: Some("high".to_string()),
            version: Some("2.0.0".to_string()),
            session_id: Some("xyz789".to_string()),
            skip_permissions: Some(false),
        }];
        state.open_for_branch("feature/test", history);

        assert_eq!(state.step, WizardStep::QuickStart);

        // Index 1 = Start new with previous settings (first tool)
        state.quick_start_index = 1;

        // Confirm should complete immediately
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Complete);
        assert_eq!(state.step, WizardStep::SkipPermissions);

        // Settings should be applied with Normal mode (not Resume)
        assert_eq!(state.agent, CodingAgent::CodexCli);
        assert_eq!(state.model, "o3-mini");
        assert_eq!(state.reasoning_level, ReasoningLevel::High);
        assert_eq!(state.execution_mode, ExecutionMode::Normal);
        assert!(!state.skip_permissions);
    }

    #[test]
    fn test_quick_start_choose_different_goes_to_agent_select() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "claude-code".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: Some("1.0.0".to_string()),
            session_id: Some("abc123".to_string()),
            skip_permissions: None,
        }];
        state.open_for_branch("feature/test", history);

        assert_eq!(state.step, WizardStep::QuickStart);

        // Index 2 = "Choose different settings..."
        state.quick_start_index = 2;

        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::AgentSelect);
    }
}
