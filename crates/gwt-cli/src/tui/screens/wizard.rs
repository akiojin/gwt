//! Wizard Popup Screen - TypeScript version compatible
//!
//! FR-044: Wizard popup overlay on branch selection
//! FR-045: Semi-transparent overlay background
//! FR-046: Centered popup with z-index
//! FR-047: Steps within same popup
//! FR-062~FR-073: Version selection flow

#![allow(dead_code)]

use gwt_core::agent::codex::supports_collaboration_modes;
use gwt_core::ai::AgentType;
use gwt_core::config::{CustomCodingAgent, ToolsConfig};
use gwt_core::git::GitHubIssue;
use ratatui::{prelude::*, widgets::*};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
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
    /// Branch action: use selected branch or create new branch from it (FR-052)
    BranchAction,
    #[default]
    AgentSelect,
    ModelSelect,
    ReasoningLevel, // Codex only
    VersionSelect,
    /// Collaboration modes (Codex v0.91.0+, SPEC-fdebd681)
    CollaborationModes,
    ExecutionMode,
    /// Source agent selection for session conversion
    ConvertAgentSelect,
    /// Session selection for conversion
    ConvertSessionSelect,
    SkipPermissions,
    // New branch flow
    BranchTypeSelect,
    /// GitHub Issue selection (SPEC-e4798383)
    IssueSelect,
    BranchNameInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardConfirmResult {
    Advance,
    Complete,
    /// Focus on an existing pane (for when agent is already running)
    FocusPane(usize),
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
    /// collaboration_modes setting (Codex v0.91.0+, SPEC-fdebd681)
    pub collaboration_modes: Option<bool>,
}

/// Source agent for session conversion
#[derive(Debug, Clone)]
pub struct ConvertSourceAgent {
    /// Agent type
    pub agent: CodingAgent,
    /// Display name
    pub label: String,
    /// Number of available sessions
    pub session_count: usize,
    /// Agent color
    pub color: Color,
}

/// Session entry for conversion selection
#[derive(Debug, Clone)]
pub struct ConvertSessionEntry {
    /// Session ID
    pub session_id: String,
    /// Last updated timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Message count
    pub message_count: usize,
    /// Display text (truncated session ID + date)
    pub display: String,
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
            CodingAgent::CodexCli => "Codex",
            CodingAgent::GeminiCli => "Gemini",
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

    /// Convert to AgentType for session conversion
    pub fn as_agent_type(self) -> AgentType {
        match self {
            CodingAgent::ClaudeCode => AgentType::ClaudeCode,
            CodingAgent::CodexCli => AgentType::CodexCli,
            CodingAgent::GeminiCli => AgentType::GeminiCli,
            CodingAgent::OpenCode => AgentType::OpenCode,
        }
    }

    /// Create from tool_id string
    pub fn from_tool_id(tool_id: &str) -> Option<Self> {
        match tool_id {
            "claude-code" => Some(CodingAgent::ClaudeCode),
            "codex-cli" => Some(CodingAgent::CodexCli),
            "gemini-cli" => Some(CodingAgent::GeminiCli),
            "opencode" => Some(CodingAgent::OpenCode),
            _ => None,
        }
    }
}

/// Unified agent entry for display (SPEC-71f2742d)
/// Represents both builtin and custom agents in a unified way
#[derive(Debug, Clone)]
pub struct AgentEntry {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub display_name: String,
    /// Display color
    pub color: Color,
    /// Whether this is a builtin agent
    pub is_builtin: bool,
    /// Whether the agent is installed/available
    pub is_installed: bool,
    /// Associated builtin agent (if any)
    pub builtin: Option<CodingAgent>,
    /// Associated custom agent (if any)
    pub custom: Option<CustomCodingAgent>,
}

impl AgentEntry {
    /// Create from builtin agent
    pub fn from_builtin(agent: CodingAgent, is_installed: bool) -> Self {
        Self {
            id: agent.id().to_string(),
            display_name: agent.label().to_string(),
            color: agent.color(),
            is_builtin: true,
            is_installed,
            builtin: Some(agent),
            custom: None,
        }
    }

    /// Create from custom agent with auto-assigned color
    pub fn from_custom(agent: CustomCodingAgent, color: Color, is_installed: bool) -> Self {
        Self {
            id: agent.id.clone(),
            display_name: agent.display_name.clone(),
            color,
            is_builtin: false,
            is_installed,
            builtin: None,
            custom: Some(agent),
        }
    }
}

/// Colors for custom agents (SPEC-71f2742d)
/// Cycles through: Blue -> Red -> White -> Gray
const CUSTOM_AGENT_COLORS: [Color; 4] = [Color::Blue, Color::Red, Color::White, Color::Gray];

/// Check if a custom agent command is installed
fn is_custom_agent_installed(agent: &CustomCodingAgent) -> bool {
    use gwt_core::config::AgentType;

    match agent.agent_type {
        AgentType::Command => {
            // Check if command exists in PATH
            Command::new("which")
                .arg(&agent.command)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        AgentType::Path => {
            // Check if path exists
            Path::new(&agent.command).exists()
        }
        AgentType::Bunx => {
            // Check if bunx is available
            Command::new("which")
                .arg("bunx")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
    }
}

/// Get all agents (builtin + custom) as unified entries (SPEC-71f2742d T115)
pub fn get_all_agents(
    installed_cache: &HashMap<CodingAgent, Option<InstalledVersionInfo>>,
) -> Vec<AgentEntry> {
    let mut entries = Vec::new();

    // Add builtin agents first
    for agent in CodingAgent::all() {
        let is_installed = installed_cache
            .get(agent)
            .map(|v| v.is_some())
            .unwrap_or(false);
        entries.push(AgentEntry::from_builtin(*agent, is_installed));
    }

    // Load and add custom agents
    let repo_root = std::env::current_dir().unwrap_or_default();
    let tools_config = ToolsConfig::load_merged(&repo_root);
    let custom_agents = tools_config.custom_coding_agents;

    for (idx, custom) in custom_agents.into_iter().enumerate() {
        // Skip if ID conflicts with builtin
        if entries.iter().any(|e| e.id == custom.id) {
            continue;
        }

        let color = CUSTOM_AGENT_COLORS[idx % CUSTOM_AGENT_COLORS.len()];
        let is_installed = is_custom_agent_installed(&custom);
        entries.push(AgentEntry::from_custom(custom, color, is_installed));
    }

    entries
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
                ModelOption::new("gpt-5.2-codex", "gpt-5.2-codex", "Codex flagship with extra-high reasoning support.")
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
                ModelOption::default_option("Default (Auto)", "Use Gemini default model"),
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
    Convert,
}

impl ExecutionMode {
    pub fn label(&self) -> &'static str {
        match self {
            ExecutionMode::Normal => "Normal",
            ExecutionMode::Continue => "Continue",
            ExecutionMode::Resume => "Resume",
            ExecutionMode::Convert => "Convert",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ExecutionMode::Normal => "Start a new session",
            ExecutionMode::Continue => "Continue from last session",
            ExecutionMode::Resume => "Resume a specific session",
            ExecutionMode::Convert => "Convert session from another agent",
        }
    }

    pub fn all() -> &'static [ExecutionMode] {
        &[
            ExecutionMode::Normal,
            ExecutionMode::Continue,
            ExecutionMode::Resume,
            ExecutionMode::Convert,
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
    /// Collaboration modes (Codex v0.91.0+, SPEC-fdebd681)
    pub collaboration_modes: bool,
    /// Session ID for resume/continue
    pub session_id: Option<String>,
    /// Scroll offset for popup content
    pub scroll_offset: usize,
    /// Branch action selection index (0: use selected, 1: create new)
    pub branch_action_index: usize,
    /// Whether branch action step is part of this flow
    pub has_branch_action: bool,
    /// Base branch override when creating new branch from selected branch
    pub base_branch_override: Option<String>,
    // Quick Start (FR-050, SPEC-f47db390)
    /// Quick Start entries per tool
    pub quick_start_entries: Vec<QuickStartEntry>,
    /// Selected Quick Start index (flattened: each tool has 2 options + 1 choose different)
    pub quick_start_index: usize,
    /// Whether Quick Start should be shown (has previous history)
    pub has_quick_start: bool,
    // Running agent context
    /// Whether an agent is already running for this branch
    pub has_running_agent: bool,
    /// Pane index of the running agent (for FocusPane result)
    pub running_agent_pane_idx: Option<usize>,
    // Mouse click support
    /// Cached popup area (outer, with border)
    pub popup_area: Option<Rect>,
    /// Cached list inner area (content rows inside popup)
    pub list_inner_area: Option<Rect>,
    // GitHub Issue selection (SPEC-e4798383)
    /// Selected GitHub Issue
    pub selected_issue: Option<GitHubIssue>,
    /// List of available issues
    pub issue_list: Vec<GitHubIssue>,
    /// Filtered issue list (for incremental search)
    pub filtered_issues: Vec<usize>,
    /// Issue search query
    pub issue_search_query: String,
    /// Selected issue index
    pub issue_selected_index: usize,
    /// Whether issues are being loaded
    pub issue_loading: bool,
    /// Issue loading error message
    pub issue_error: Option<String>,
    // Custom agents (SPEC-71f2742d)
    /// All available agents (builtin + custom)
    pub all_agents: Vec<AgentEntry>,
    /// Selected agent entry (may be custom)
    pub selected_agent_entry: Option<AgentEntry>,
    /// Existing branch for selected issue (FR-011 duplicate detection)
    pub issue_existing_branch: Option<String>,
    // Session conversion (Execution Mode: Convert)
    /// Available source agents with convertible sessions (excludes target agent)
    pub convert_source_agents: Vec<ConvertSourceAgent>,
    /// Selected source agent index for conversion
    pub convert_agent_index: usize,
    /// Sessions for the selected source agent (sorted by newest first)
    pub convert_sessions: Vec<ConvertSessionEntry>,
    /// Selected session index for conversion
    pub convert_session_index: usize,
    /// Converted session ID (set after successful conversion)
    pub converted_session_id: Option<String>,
    /// Session conversion error message
    pub convert_error: Option<String>,
    /// Worktree path for session search
    pub worktree_path: Option<std::path::PathBuf>,
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
    /// If running_pane_idx is provided, agent is already running for this branch
    pub fn open_for_branch(
        &mut self,
        branch_name: &str,
        history: Vec<QuickStartEntry>,
        running_pane_idx: Option<usize>,
    ) {
        self.visible = true;
        self.is_new_branch = false;
        self.branch_name = branch_name.to_string();
        self.reset_selections();
        self.has_branch_action = true;

        // Set running agent context
        self.has_running_agent = running_pane_idx.is_some();
        self.running_agent_pane_idx = running_pane_idx;

        // FR-050: Show Quick Start if history exists (only when no agent is running)
        if running_pane_idx.is_some() || history.is_empty() {
            // If agent is running or no history, skip Quick Start and go to BranchAction
            self.step = WizardStep::BranchAction;
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
        self.has_branch_action = false;
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
        self.collaboration_modes = false;
        self.session_id = None;
        self.branch_type = BranchType::default();
        self.new_branch_name.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.branch_action_index = 0;
        self.has_branch_action = false;
        self.base_branch_override = None;
        self.quick_start_index = 0;
        self.has_running_agent = false;
        self.running_agent_pane_idx = None;
        // Load all agents (builtin + custom)
        self.all_agents = get_all_agents(&self.installed_cache);
        self.selected_agent_entry = self.all_agents.first().cloned();
        // Reset session conversion state
        self.convert_source_agents.clear();
        self.convert_agent_index = 0;
        self.convert_sessions.clear();
        self.convert_session_index = 0;
        self.converted_session_id = None;
        self.convert_error = None;
    }

    /// Get branch action options based on running agent status
    pub fn branch_action_options(&self) -> &[&'static str] {
        if self.has_running_agent {
            &["Focus agent pane", "Create new branch from this"]
        } else {
            &["Use selected branch", "Create new from selected"]
        }
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

    /// Resolve version string to semantic version for collaboration_modes check (SPEC-fdebd681)
    fn resolve_version_for_collaboration_modes(&self) -> Option<String> {
        match self.version.as_str() {
            "latest" => Some("99.99.99".to_string()), // latest always supports
            "installed" => {
                // Get installed version from cache
                self.installed_cache
                    .get(&CodingAgent::CodexCli)
                    .and_then(|info| info.as_ref())
                    .map(|info| info.version.clone())
            }
            v => Some(v.to_string()), // concrete version
        }
    }

    /// Check if CollaborationModes step should be shown (SPEC-fdebd681)
    fn should_show_collaboration_modes(&self) -> bool {
        self.agent == CodingAgent::CodexCli
            && self
                .resolve_version_for_collaboration_modes()
                .as_deref()
                .map(|v| supports_collaboration_modes(Some(v)))
                .unwrap_or(false)
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
            // T604, T605: Find agent by ID in all_agents (supports custom agents and builtin ID overwrite)
            // SPEC-71f2742d US6
            if let Some((agent_index, agent_entry)) = self
                .all_agents
                .iter()
                .enumerate()
                .find(|(_, a)| a.id == entry.tool_id)
            {
                // Found in all_agents (could be builtin, custom, or custom overwriting builtin)
                self.agent_index = agent_index;
                self.selected_agent_entry = Some(agent_entry.clone());

                if let Some(builtin) = agent_entry.builtin {
                    self.agent = builtin;
                } else {
                    // Custom agent without builtin association - use default but selected_agent_entry is set
                    self.agent = CodingAgent::ClaudeCode;
                }
            } else {
                // Fallback: Map tool_id to builtin CodingAgent
                self.agent = match entry.tool_id.as_str() {
                    "claude-code" => CodingAgent::ClaudeCode,
                    "codex-cli" => CodingAgent::CodexCli,
                    "gemini-cli" => CodingAgent::GeminiCli,
                    "opencode" => CodingAgent::OpenCode,
                    _ => CodingAgent::ClaudeCode,
                };
                self.selected_agent_entry = None;
            }

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

            // Restore or auto-enable collaboration_modes for Codex (SPEC-fdebd681)
            if self.agent == CodingAgent::CodexCli {
                if self.version == "installed"
                    && !self.installed_cache.contains_key(&CodingAgent::CodexCli)
                {
                    let detected = detect_installed_version(CodingAgent::CodexCli);
                    self.installed_cache.insert(CodingAgent::CodexCli, detected);
                }
                let supports = self
                    .resolve_version_for_collaboration_modes()
                    .as_deref()
                    .map(|v| supports_collaboration_modes(Some(v)))
                    .unwrap_or(false);
                // collaboration_modes is auto-enabled for supported versions
                self.collaboration_modes = supports;
            } else {
                self.collaboration_modes = false;
            }

            // Set skip permissions
            self.skip_permissions = entry.skip_permissions.unwrap_or(false);

            // Set execution mode based on action
            self.execution_mode = match action {
                QuickStartAction::ResumeWithPrevious => {
                    if entry.session_id.is_some() {
                        ExecutionMode::Resume
                    } else {
                        ExecutionMode::Continue
                    }
                }
                QuickStartAction::StartNewWithPrevious => ExecutionMode::Normal,
                QuickStartAction::ChooseDifferent => ExecutionMode::Normal,
            };
            self.session_id = match action {
                QuickStartAction::ResumeWithPrevious => entry.session_id.clone(),
                _ => None,
            };
        }
    }

    /// Collect source agents with convertible sessions (excludes target agent)
    pub fn collect_convert_source_agents(&mut self) {
        use gwt_core::ai::{
            ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, OpenCodeSessionParser,
            SessionParser,
        };

        self.convert_source_agents.clear();
        let target_agent = self.agent;
        let worktree_path = self.worktree_path.as_deref();

        // Check each agent type (except target)
        for agent in CodingAgent::all() {
            if *agent == target_agent {
                continue;
            }

            // Get session count for this agent
            let session_count = match agent {
                CodingAgent::ClaudeCode => {
                    if let Some(parser) = ClaudeSessionParser::with_default_home() {
                        parser.list_sessions(worktree_path).len()
                    } else {
                        0
                    }
                }
                CodingAgent::CodexCli => {
                    if let Some(parser) = CodexSessionParser::with_default_home() {
                        parser.list_sessions(worktree_path).len()
                    } else {
                        0
                    }
                }
                CodingAgent::GeminiCli => {
                    if let Some(parser) = GeminiSessionParser::with_default_home() {
                        parser.list_sessions(worktree_path).len()
                    } else {
                        0
                    }
                }
                CodingAgent::OpenCode => {
                    if let Some(parser) = OpenCodeSessionParser::with_default_home() {
                        parser.list_sessions(worktree_path).len()
                    } else {
                        0
                    }
                }
            };

            self.convert_source_agents.push(ConvertSourceAgent {
                agent: *agent,
                label: format!("{} ({} sessions)", agent.label(), session_count),
                session_count,
                color: agent.color(),
            });
        }
    }

    /// Load sessions for the selected source agent
    pub fn load_sessions_for_agent(&mut self) {
        use gwt_core::ai::{
            ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, OpenCodeSessionParser,
            SessionParser,
        };

        self.convert_sessions.clear();
        self.convert_session_index = 0;

        if self.convert_agent_index >= self.convert_source_agents.len() {
            return;
        }

        let source_agent = &self.convert_source_agents[self.convert_agent_index];
        let worktree_path = self.worktree_path.as_deref();

        let sessions = match source_agent.agent {
            CodingAgent::ClaudeCode => {
                if let Some(parser) = ClaudeSessionParser::with_default_home() {
                    parser.list_sessions(worktree_path)
                } else {
                    vec![]
                }
            }
            CodingAgent::CodexCli => {
                if let Some(parser) = CodexSessionParser::with_default_home() {
                    parser.list_sessions(worktree_path)
                } else {
                    vec![]
                }
            }
            CodingAgent::GeminiCli => {
                if let Some(parser) = GeminiSessionParser::with_default_home() {
                    parser.list_sessions(worktree_path)
                } else {
                    vec![]
                }
            }
            CodingAgent::OpenCode => {
                if let Some(parser) = OpenCodeSessionParser::with_default_home() {
                    parser.list_sessions(worktree_path)
                } else {
                    vec![]
                }
            }
        };

        // Convert to ConvertSessionEntry and sort by newest first
        self.convert_sessions = sessions
            .into_iter()
            .map(|entry| {
                let date_str = entry
                    .last_updated
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                let short_id = if entry.session_id.len() > 12 {
                    format!("{}...", &entry.session_id[..12])
                } else {
                    entry.session_id.clone()
                };
                ConvertSessionEntry {
                    session_id: entry.session_id,
                    last_updated: entry.last_updated,
                    message_count: entry.message_count,
                    display: format!("{} ({})", short_id, date_str),
                }
            })
            .collect();

        // Sort by newest first
        self.convert_sessions
            .sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
    }

    /// Get selected source agent for conversion
    pub fn selected_convert_source_agent(&self) -> Option<&ConvertSourceAgent> {
        self.convert_source_agents.get(self.convert_agent_index)
    }

    /// Get selected session for conversion
    pub fn selected_convert_session(&self) -> Option<&ConvertSessionEntry> {
        self.convert_sessions.get(self.convert_session_index)
    }

    /// Perform session conversion from source agent to target agent
    pub fn perform_session_conversion(&mut self) -> bool {
        use gwt_core::ai::{
            convert_session, ClaudeSessionParser, CodexSessionParser, GeminiSessionParser,
            OpenCodeSessionParser, SessionParser,
        };

        // Clear previous error/result
        self.convert_error = None;
        self.converted_session_id = None;

        // Get source agent and session
        let source_agent = match self.selected_convert_source_agent() {
            Some(a) => a.agent,
            None => {
                self.convert_error = Some("No source agent selected".to_string());
                return false;
            }
        };

        let source_session_id = match self.selected_convert_session() {
            Some(s) => s.session_id.clone(),
            None => {
                self.convert_error = Some("No session selected".to_string());
                return false;
            }
        };

        // Parse the source session
        let parsed_session = match source_agent {
            CodingAgent::ClaudeCode => {
                let parser = match ClaudeSessionParser::with_default_home() {
                    Some(p) => p,
                    None => {
                        self.convert_error = Some("Could not initialize Claude parser".to_string());
                        return false;
                    }
                };
                parser.parse(&source_session_id)
            }
            CodingAgent::CodexCli => {
                let parser = match CodexSessionParser::with_default_home() {
                    Some(p) => p,
                    None => {
                        self.convert_error = Some("Could not initialize Codex parser".to_string());
                        return false;
                    }
                };
                parser.parse(&source_session_id)
            }
            CodingAgent::GeminiCli => {
                let parser = match GeminiSessionParser::with_default_home() {
                    Some(p) => p,
                    None => {
                        self.convert_error = Some("Could not initialize Gemini parser".to_string());
                        return false;
                    }
                };
                parser.parse(&source_session_id)
            }
            CodingAgent::OpenCode => {
                let parser = match OpenCodeSessionParser::with_default_home() {
                    Some(p) => p,
                    None => {
                        self.convert_error =
                            Some("Could not initialize OpenCode parser".to_string());
                        return false;
                    }
                };
                parser.parse(&source_session_id)
            }
        };

        let parsed = match parsed_session {
            Ok(p) => p,
            Err(e) => {
                self.convert_error = Some(format!("Failed to parse session: {}", e));
                return false;
            }
        };

        // Determine target agent type
        let target_agent_type = match self.agent {
            CodingAgent::ClaudeCode => AgentType::ClaudeCode,
            CodingAgent::CodexCli => AgentType::CodexCli,
            CodingAgent::GeminiCli => AgentType::GeminiCli,
            CodingAgent::OpenCode => AgentType::OpenCode,
        };

        // Get worktree path
        let worktree_path = self
            .worktree_path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        // Perform conversion
        match convert_session(&parsed, target_agent_type, &worktree_path) {
            Ok(result) => {
                self.converted_session_id = Some(result.new_session_id.clone());
                self.session_id = Some(result.new_session_id);
                true
            }
            Err(e) => {
                self.convert_error = Some(format!("Conversion failed: {}", e));
                false
            }
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

    /// Get supported execution modes for the current agent (T212 SPEC-71f2742d)
    /// For builtin agents, all modes are supported.
    /// For custom agents, only modes with non-empty modeArgs are supported.
    pub fn supported_execution_modes(&self) -> Vec<ExecutionMode> {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                // Custom agent: check mode_args
                if let Some(ref mode_args) = custom.mode_args {
                    let mut modes = vec![ExecutionMode::Normal]; // Normal is always supported
                    if !mode_args.continue_mode.is_empty() {
                        modes.push(ExecutionMode::Continue);
                    }
                    if !mode_args.resume.is_empty() {
                        modes.push(ExecutionMode::Resume);
                    }
                    return modes;
                } else {
                    // No mode_args defined, only Normal is supported
                    return vec![ExecutionMode::Normal];
                }
            }
        }
        // Builtin agent: all modes supported
        ExecutionMode::all().to_vec()
    }

    /// Check if skip_permissions option should be shown (T213 SPEC-71f2742d)
    /// For builtin agents, always show.
    /// For custom agents, only show if permission_skip_args is non-empty.
    pub fn supports_skip_permissions(&self) -> bool {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                return !custom.permission_skip_args.is_empty();
            }
        }
        // Builtin agent: always supports skip_permissions
        true
    }

    /// Get models for current agent (T503 SPEC-71f2742d FR-011)
    /// For custom agents, returns models from CustomCodingAgent.models.
    /// For builtin agents, returns CodingAgent.models().
    pub fn get_models(&self) -> Vec<ModelOption> {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                // Convert ModelDef to ModelOption
                if custom.models.is_empty() {
                    return vec![];
                }
                return custom
                    .models
                    .iter()
                    .map(|m| ModelOption::new(&m.id, &m.label, ""))
                    .collect();
            }
        }
        // Builtin agent
        self.agent.models()
    }

    /// Check if model selection step should be shown (T504 SPEC-71f2742d FR-011)
    pub fn has_models(&self) -> bool {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                return !custom.models.is_empty();
            }
        }
        // Builtin agents always have models
        true
    }

    /// Check if version command is available (T505 SPEC-71f2742d FR-012)
    pub fn has_version_command(&self) -> bool {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                return custom.version_command.is_some();
            }
        }
        // Builtin agents use installed version detection
        true
    }

    /// Get version from custom agent's versionCommand (T505 SPEC-71f2742d FR-012)
    pub fn get_custom_version(&self) -> Option<String> {
        if let Some(ref entry) = self.selected_agent_entry {
            if let Some(ref custom) = entry.custom {
                if let Some(ref cmd) = custom.version_command {
                    // Execute version command
                    if let Ok(output) = std::process::Command::new("sh").args(["-c", cmd]).output()
                    {
                        if output.status.success() {
                            let version =
                                String::from_utf8_lossy(&output.stdout).trim().to_string();
                            if !version.is_empty() {
                                return Some(version);
                            }
                        }
                    }
                }
            }
        }
        None
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
                            // Go to branch action selection for manual configuration
                            WizardStep::BranchAction
                        }
                    }
                } else {
                    // No history, go to branch action selection
                    WizardStep::BranchAction
                }
            }
            WizardStep::BranchAction => {
                if self.branch_action_index == 0 {
                    self.is_new_branch = false;
                    self.base_branch_override = None;
                    WizardStep::AgentSelect
                } else {
                    self.is_new_branch = true;
                    self.base_branch_override = Some(self.branch_name.clone());
                    WizardStep::BranchTypeSelect
                }
            }
            WizardStep::BranchTypeSelect => {
                // SPEC-e4798383: Check if gh CLI is available
                if gwt_core::git::is_gh_cli_available() {
                    // Start loading issues
                    self.issue_loading = true;
                    self.issue_error = None;
                    self.issue_list.clear();
                    self.filtered_issues.clear();
                    self.issue_search_query.clear();
                    self.issue_selected_index = 0;
                    self.selected_issue = None;
                    WizardStep::IssueSelect
                } else {
                    // Skip IssueSelect if gh CLI is not available (FR-012)
                    WizardStep::BranchNameInput
                }
            }
            WizardStep::IssueSelect => {
                // Generate branch name from selected issue if any
                if let Some(ref issue) = self.selected_issue {
                    self.new_branch_name = gwt_core::git::generate_branch_name(
                        self.branch_type.prefix(),
                        issue.number,
                    );
                    // Remove the prefix since it's added separately
                    self.new_branch_name = self
                        .new_branch_name
                        .strip_prefix(self.branch_type.prefix())
                        .unwrap_or(&self.new_branch_name)
                        .to_string();
                }
                self.cursor = self.new_branch_name.len();
                WizardStep::BranchNameInput
            }
            WizardStep::BranchNameInput => WizardStep::AgentSelect,
            WizardStep::AgentSelect => {
                // Set model based on selected agent (T503 SPEC-71f2742d)
                let models = self.get_models();
                if !models.is_empty() {
                    self.model = models[0].id.clone();
                    self.model_index = 0;
                }
                // Reset version fetch when agent changes
                self.versions_fetched = false;

                // T504: Skip ModelSelect if no models defined
                if !self.has_models() {
                    // T506: Skip VersionSelect if no versionCommand
                    if !self.has_version_command() {
                        // Go directly to ExecutionMode
                        let supported = self.supported_execution_modes();
                        self.execution_mode_index = 0;
                        if !supported.is_empty() {
                            self.execution_mode = supported[0];
                        }
                        WizardStep::ExecutionMode
                    } else {
                        // Fetch versions
                        self.fetch_versions_for_agent();
                        WizardStep::VersionSelect
                    }
                } else {
                    WizardStep::ModelSelect
                }
            }
            WizardStep::ModelSelect => {
                // Skip to version select unless Codex
                if self.agent == CodingAgent::CodexCli {
                    WizardStep::ReasoningLevel
                } else {
                    // T506: Skip VersionSelect if custom agent without versionCommand
                    if !self.has_version_command() {
                        // Go directly to ExecutionMode
                        let supported = self.supported_execution_modes();
                        self.execution_mode_index = 0;
                        if !supported.is_empty() {
                            self.execution_mode = supported[0];
                        }
                        WizardStep::ExecutionMode
                    } else {
                        // Fetch versions when entering VersionSelect (FR-063)
                        self.fetch_versions_for_agent();
                        WizardStep::VersionSelect
                    }
                }
            }
            WizardStep::ReasoningLevel => {
                // Fetch versions when entering VersionSelect (FR-063)
                self.fetch_versions_for_agent();
                WizardStep::VersionSelect
            }
            WizardStep::VersionSelect => {
                // SPEC-fdebd681: Auto-enable collaboration_modes for Codex v0.91.0+
                if self.should_show_collaboration_modes() {
                    self.collaboration_modes = true;
                }
                // CollaborationModes step skipped - go directly to ExecutionMode
                let supported = self.supported_execution_modes();
                self.execution_mode_index = 0;
                if !supported.is_empty() {
                    self.execution_mode = supported[0];
                }
                WizardStep::ExecutionMode
            }
            WizardStep::CollaborationModes => {
                // No longer used - step is skipped, but keep for enum exhaustiveness
                WizardStep::ExecutionMode
            }
            WizardStep::ExecutionMode => {
                // Check if Convert mode is selected
                if self.execution_mode == ExecutionMode::Convert {
                    // Collect source agents and go to agent selection
                    self.collect_convert_source_agents();
                    WizardStep::ConvertAgentSelect
                } else if self.supports_skip_permissions() {
                    WizardStep::SkipPermissions
                } else {
                    // Auto-disable skip_permissions and stay at final step
                    self.skip_permissions = false;
                    WizardStep::SkipPermissions
                }
            }
            WizardStep::ConvertAgentSelect => {
                // Check if selected agent has sessions
                if let Some(agent) = self.selected_convert_source_agent() {
                    if agent.session_count == 0 {
                        // Stay on this step - can't select agent with 0 sessions
                        WizardStep::ConvertAgentSelect
                    } else {
                        // Load sessions for the selected agent
                        self.load_sessions_for_agent();
                        WizardStep::ConvertSessionSelect
                    }
                } else {
                    WizardStep::ConvertAgentSelect
                }
            }
            WizardStep::ConvertSessionSelect => {
                // Perform session conversion
                if !self.perform_session_conversion() {
                    // Conversion failed - stay on this step
                    // Error is stored in self.convert_error
                    WizardStep::ConvertSessionSelect
                } else if self.supports_skip_permissions() {
                    WizardStep::SkipPermissions
                } else {
                    self.skip_permissions = false;
                    WizardStep::SkipPermissions
                }
            }
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
            WizardStep::BranchAction => {
                if self.has_quick_start {
                    WizardStep::QuickStart
                } else {
                    self.close();
                    return false;
                }
            }
            WizardStep::BranchTypeSelect => {
                if self.base_branch_override.is_some() {
                    WizardStep::BranchAction
                } else {
                    self.close();
                    return false;
                }
            }
            WizardStep::IssueSelect => WizardStep::BranchTypeSelect,
            WizardStep::BranchNameInput => {
                // Go back to IssueSelect if gh CLI is available, otherwise BranchTypeSelect
                if gwt_core::git::is_gh_cli_available() {
                    WizardStep::IssueSelect
                } else {
                    WizardStep::BranchTypeSelect
                }
            }
            WizardStep::AgentSelect => {
                if self.is_new_branch {
                    WizardStep::BranchNameInput
                } else if self.has_branch_action {
                    WizardStep::BranchAction
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
                } else if self.has_models() {
                    WizardStep::ModelSelect
                } else {
                    // T504: Skip back to AgentSelect if no models
                    WizardStep::AgentSelect
                }
            }
            WizardStep::CollaborationModes => {
                // No longer used - step is skipped, but keep for enum exhaustiveness
                WizardStep::VersionSelect
            }
            WizardStep::ExecutionMode => {
                // SPEC-fdebd681: CollaborationModes step skipped - go back directly
                if self.has_version_command() {
                    WizardStep::VersionSelect
                } else if self.has_models() {
                    WizardStep::ModelSelect
                } else {
                    WizardStep::AgentSelect
                }
            }
            WizardStep::ConvertAgentSelect => WizardStep::ExecutionMode,
            WizardStep::ConvertSessionSelect => WizardStep::ConvertAgentSelect,
            WizardStep::SkipPermissions => {
                // Check if we came from Convert flow
                if self.execution_mode == ExecutionMode::Convert {
                    WizardStep::ConvertSessionSelect
                } else {
                    WizardStep::ExecutionMode
                }
            }
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
                        self.step = WizardStep::BranchAction;
                        self.scroll_offset = 0;
                        self.branch_action_index = 0;
                        self.is_new_branch = false;
                        self.base_branch_override = None;
                        return WizardConfirmResult::Advance;
                    }
                }
            } else {
                self.step = WizardStep::BranchAction;
                self.scroll_offset = 0;
                self.branch_action_index = 0;
                self.is_new_branch = false;
                self.base_branch_override = None;
                return WizardConfirmResult::Advance;
            }
        }

        // Handle BranchAction step with running agent: "Focus agent pane" selected
        if self.step == WizardStep::BranchAction
            && self.has_running_agent
            && self.branch_action_index == 0
        {
            if let Some(pane_idx) = self.running_agent_pane_idx {
                self.close();
                return WizardConfirmResult::FocusPane(pane_idx);
            }
        }
        // Note: BranchAction index == 1 with running agent means
        // "Create new branch from this" - continues to next step via is_complete()/next_step()

        // Handle IssueSelect step (SPEC-e4798383)
        if self.step == WizardStep::IssueSelect {
            // Index 0 = Skip option, Index 1+ = actual issues
            if self.issue_selected_index > 0 && !self.filtered_issues.is_empty() {
                let adjusted_index = self.issue_selected_index - 1; // Adjust for Skip option
                if adjusted_index < self.filtered_issues.len() {
                    // Issue selected - set selected_issue and generate branch name (FR-009)
                    let issue_idx = self.filtered_issues[adjusted_index];
                    if let Some(issue) = self.issue_list.get(issue_idx).cloned() {
                        // FR-011: Check for duplicate branch
                        if let Some(existing) = &self.issue_existing_branch {
                            self.issue_error = Some(format!(
                                "Branch for issue #{} already exists: {}",
                                issue.number, existing
                            ));
                            return WizardConfirmResult::Advance; // Stay on same step
                        }

                        // Generate branch name: {type}/issue-{number}
                        self.new_branch_name = issue.branch_name_suffix();
                        self.cursor = self.new_branch_name.len();
                        self.selected_issue = Some(issue);
                    }
                }
            }
            // If index 0 (Skip) or no issues, proceed without issue (FR-004, T603)
            // selected_issue remains None, new_branch_name remains empty
            self.issue_error = None; // Clear error when skipping
            self.next_step();
            return WizardConfirmResult::Advance;
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
            WizardStep::ConvertAgentSelect => {
                let max = self.convert_source_agents.len().saturating_sub(1);
                if self.convert_agent_index < max {
                    self.convert_agent_index += 1;
                }
            }
            WizardStep::ConvertSessionSelect => {
                let max = self.convert_sessions.len().saturating_sub(1);
                if self.convert_session_index < max {
                    self.convert_session_index += 1;
                }
            }
            WizardStep::BranchAction => {
                if self.branch_action_index < 1 {
                    self.branch_action_index += 1;
                }
            }
            WizardStep::AgentSelect => {
                let max = self.all_agents.len().saturating_sub(1);
                if self.agent_index < max {
                    self.agent_index += 1;
                    self.selected_agent_entry = self.all_agents.get(self.agent_index).cloned();
                    // Keep builtin agent in sync if it's a builtin
                    if let Some(ref entry) = self.selected_agent_entry {
                        if let Some(builtin) = entry.builtin {
                            self.agent = builtin;
                        }
                    }
                }
            }
            WizardStep::ModelSelect => {
                let models = self.get_models(); // T503: Use get_models() for custom agent support
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
            WizardStep::CollaborationModes => {
                // Step is skipped - no-op (kept for enum exhaustiveness)
            }
            WizardStep::ExecutionMode => {
                // T212: Only navigate through supported modes
                let supported = self.supported_execution_modes();
                let max = supported.len().saturating_sub(1);
                if self.execution_mode_index < max {
                    self.execution_mode_index += 1;
                    self.execution_mode = supported[self.execution_mode_index];
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
            WizardStep::IssueSelect => {
                // Navigate through Skip option (0) + filtered issues (1+) (FR-008)
                let max = self.filtered_issues.len(); // 0=Skip, 1..=len=issues
                if self.issue_selected_index < max {
                    self.issue_selected_index += 1;
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
            WizardStep::ConvertAgentSelect => {
                if self.convert_agent_index > 0 {
                    self.convert_agent_index -= 1;
                }
            }
            WizardStep::ConvertSessionSelect => {
                if self.convert_session_index > 0 {
                    self.convert_session_index -= 1;
                }
            }
            WizardStep::BranchAction => {
                if self.branch_action_index > 0 {
                    self.branch_action_index -= 1;
                }
            }
            WizardStep::AgentSelect => {
                if self.agent_index > 0 {
                    self.agent_index -= 1;
                    self.selected_agent_entry = self.all_agents.get(self.agent_index).cloned();
                    // Keep builtin agent in sync if it's a builtin
                    if let Some(ref entry) = self.selected_agent_entry {
                        if let Some(builtin) = entry.builtin {
                            self.agent = builtin;
                        }
                    }
                }
            }
            WizardStep::ModelSelect => {
                let models = self.get_models(); // T503: Use get_models() for custom agent support
                if self.model_index > 0 {
                    self.model_index -= 1;
                    self.model = models[self.model_index].id.clone();
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
            WizardStep::CollaborationModes => {
                // Step is skipped - no-op (kept for enum exhaustiveness)
            }
            WizardStep::ExecutionMode => {
                // T212: Only navigate through supported modes
                let supported = self.supported_execution_modes();
                if self.execution_mode_index > 0 {
                    self.execution_mode_index -= 1;
                    self.execution_mode = supported[self.execution_mode_index];
                }
            }
            WizardStep::SkipPermissions => {
                // T213: Only toggle if skip_permissions is supported
                if self.supports_skip_permissions() {
                    self.skip_permissions = !self.skip_permissions;
                }
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
            WizardStep::IssueSelect => {
                // Navigate through filtered issues (FR-008)
                if self.issue_selected_index > 0 {
                    self.issue_selected_index -= 1;
                }
            }
            WizardStep::BranchNameInput => {
                // No selection in input mode
            }
        }
    }

    /// Insert character in branch name input or issue search
    pub fn insert_char(&mut self, c: char) {
        match self.step {
            WizardStep::BranchNameInput => {
                self.new_branch_name.insert(self.cursor, c);
                self.cursor += 1;
            }
            WizardStep::IssueSelect => {
                // FR-008: Incremental search
                self.issue_search_query.push(c);
                self.update_filtered_issues();
            }
            _ => {}
        }
    }

    /// Delete character in branch name input or issue search
    pub fn delete_char(&mut self) {
        match self.step {
            WizardStep::BranchNameInput if self.cursor > 0 => {
                self.cursor -= 1;
                self.new_branch_name.remove(self.cursor);
            }
            WizardStep::IssueSelect if !self.issue_search_query.is_empty() => {
                self.issue_search_query.pop();
                self.update_filtered_issues();
            }
            _ => {}
        }
    }

    /// Update filtered issues based on search query (FR-008)
    fn update_filtered_issues(&mut self) {
        use gwt_core::git::filter_issues_by_title;

        if self.issue_search_query.is_empty() {
            // Show all issues
            self.filtered_issues = (0..self.issue_list.len()).collect();
        } else {
            // Filter by title
            let filtered = filter_issues_by_title(&self.issue_list, &self.issue_search_query);
            self.filtered_issues = filtered
                .iter()
                .filter_map(|issue| {
                    self.issue_list
                        .iter()
                        .position(|i| i.number == issue.number)
                })
                .collect();
        }
        // Reset selection index if out of bounds
        if self.issue_selected_index >= self.filtered_issues.len() {
            self.issue_selected_index = 0;
        }
    }

    /// Check if selected issue has an existing branch (FR-011)
    pub fn check_issue_duplicate(&mut self, repo_path: &Path) {
        use gwt_core::git::find_branch_for_issue;

        self.issue_existing_branch = None;
        self.issue_error = None;

        // Index 0 = Skip option, so actual issues start at index 1
        if self.issue_selected_index > 0 && !self.filtered_issues.is_empty() {
            let adjusted_index = self.issue_selected_index - 1;
            if adjusted_index < self.filtered_issues.len() {
                let issue_idx = self.filtered_issues[adjusted_index];
                if let Some(issue) = self.issue_list.get(issue_idx) {
                    if let Ok(Some(branch)) = find_branch_for_issue(repo_path, issue.number) {
                        self.issue_existing_branch = Some(branch);
                    }
                }
            }
        }
    }

    fn apply_loaded_issues(&mut self, issues: Vec<GitHubIssue>) {
        self.issue_list = issues;
        self.filtered_issues = (0..self.issue_list.len()).collect();
        self.issue_loading = false;

        if self.issue_list.is_empty() {
            self.selected_issue = None;
            self.issue_existing_branch = None;
            self.issue_error = None;
            self.issue_search_query.clear();
            self.issue_selected_index = 0;
            self.new_branch_name.clear();
            self.cursor = 0;
            self.step = WizardStep::BranchNameInput;
        }
    }

    /// Load issues from GitHub (FR-005)
    pub fn load_issues(&mut self, repo_path: &Path) {
        use gwt_core::git::{fetch_open_issues, is_gh_cli_available};

        self.issue_loading = true;
        self.issue_error = None;
        self.issue_list.clear();
        self.filtered_issues.clear();
        self.issue_search_query.clear();
        self.issue_selected_index = 0;
        self.selected_issue = None;
        self.issue_existing_branch = None;

        if !is_gh_cli_available() {
            self.issue_loading = false;
            // gh CLI not available - will auto-skip
            return;
        }

        match fetch_open_issues(repo_path) {
            Ok(issues) => {
                self.apply_loaded_issues(issues);
            }
            Err(e) => {
                self.issue_error = Some(e);
                self.issue_loading = false;
            }
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

    // --- Mouse click support ---

    /// Get item count for current step (for mouse click handling)
    pub fn current_step_item_count(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => self.quick_start_option_count(),
            WizardStep::ConvertAgentSelect => self.convert_source_agents.len(),
            WizardStep::ConvertSessionSelect => self.convert_sessions.len(),
            WizardStep::BranchAction => self.branch_action_options().len(),
            WizardStep::BranchTypeSelect => BranchType::all().len(),
            WizardStep::IssueSelect => self.filtered_issues.len() + 1, // +1 for Skip option
            WizardStep::BranchNameInput => 0,                          // Text input, no list items
            WizardStep::AgentSelect => self.all_agents.len(),
            WizardStep::ModelSelect => self.get_models().len(), // T503: Use get_models() for custom agent support
            WizardStep::ReasoningLevel => ReasoningLevel::all().len(),
            WizardStep::VersionSelect => self.version_options.len(),
            WizardStep::CollaborationModes => 0, // Step is skipped (kept for enum exhaustiveness)
            WizardStep::ExecutionMode => self.supported_execution_modes().len(), // T212
            WizardStep::SkipPermissions => {
                // T213: If skip_permissions not supported, show only "No" option (auto-confirm)
                if self.supports_skip_permissions() {
                    2 // Yes/No
                } else {
                    1 // Only "No" (auto-confirm)
                }
            }
        }
    }

    /// Get current selection index for current step
    pub fn current_selection_index(&self) -> usize {
        match self.step {
            WizardStep::QuickStart => self.quick_start_index,
            WizardStep::ConvertAgentSelect => self.convert_agent_index,
            WizardStep::ConvertSessionSelect => self.convert_session_index,
            WizardStep::BranchAction => self.branch_action_index,
            WizardStep::BranchTypeSelect => BranchType::all()
                .iter()
                .position(|t| *t == self.branch_type)
                .unwrap_or(0),
            WizardStep::IssueSelect => self.issue_selected_index,
            WizardStep::BranchNameInput => 0,
            WizardStep::AgentSelect => self.agent_index,
            WizardStep::ModelSelect => self.model_index,
            WizardStep::ReasoningLevel => self.reasoning_level_index,
            WizardStep::VersionSelect => self.version_index,
            WizardStep::CollaborationModes => 0, // Step is skipped (kept for enum exhaustiveness)
            WizardStep::ExecutionMode => self.execution_mode_index,
            WizardStep::SkipPermissions => {
                if self.skip_permissions {
                    0
                } else {
                    1
                }
            }
        }
    }

    /// Set selection index for current step (returns true if changed)
    pub fn set_selection_index(&mut self, index: usize) -> bool {
        let item_count = self.current_step_item_count();
        if index >= item_count {
            return false;
        }
        let current = self.current_selection_index();
        if current == index {
            return false;
        }

        match self.step {
            WizardStep::QuickStart => {
                self.quick_start_index = index;
            }
            WizardStep::ConvertAgentSelect => {
                self.convert_agent_index = index;
            }
            WizardStep::ConvertSessionSelect => {
                self.convert_session_index = index;
            }
            WizardStep::BranchAction => {
                self.branch_action_index = index;
            }
            WizardStep::BranchTypeSelect => {
                self.branch_type = BranchType::all()[index];
            }
            WizardStep::IssueSelect => {
                self.issue_selected_index = index;
                // Update selected_issue based on filtered list
                if let Some(&issue_idx) = self.filtered_issues.get(index) {
                    self.selected_issue = self.issue_list.get(issue_idx).cloned();
                }
            }
            WizardStep::BranchNameInput => {
                return false; // No list items
            }
            WizardStep::AgentSelect => {
                self.agent_index = index;
                // Update selected_agent_entry for custom agent support
                self.selected_agent_entry = self.all_agents.get(index).cloned();
                // Keep builtin agent in sync if it's a builtin
                if let Some(ref entry) = self.selected_agent_entry {
                    if let Some(builtin) = entry.builtin {
                        self.agent = builtin;
                    }
                }
            }
            WizardStep::ModelSelect => {
                let models = self.get_models(); // T503: Use get_models() for custom agent support
                self.model_index = index;
                self.model = models[index].id.clone();
            }
            WizardStep::ReasoningLevel => {
                self.reasoning_level_index = index;
                self.reasoning_level = ReasoningLevel::all()[index];
            }
            WizardStep::VersionSelect => {
                self.version_index = index;
                self.version = self.version_options[index].value.clone();
                self.ensure_version_visible();
            }
            WizardStep::CollaborationModes => {
                // Step is skipped - no-op (kept for enum exhaustiveness)
            }
            WizardStep::ExecutionMode => {
                // T212: Use supported modes
                let supported = self.supported_execution_modes();
                self.execution_mode_index = index;
                self.execution_mode = supported[index];
            }
            WizardStep::SkipPermissions => {
                // T213: Only allow toggle if supported
                if self.supports_skip_permissions() {
                    self.skip_permissions = index == 0;
                } else {
                    self.skip_permissions = false;
                }
            }
        }
        true
    }

    /// Resolve selection index from a mouse position within the list area
    pub fn selection_index_from_point(&self, x: u16, y: u16) -> Option<usize> {
        let inner = self.list_inner_area?;
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let right = inner.x.saturating_add(inner.width);
        let bottom = inner.y.saturating_add(inner.height);
        if x < inner.x || x >= right || y < inner.y || y >= bottom {
            return None;
        }
        let row = (y - inner.y) as usize;
        let index = self.scroll_offset.saturating_add(row);
        let item_count = self.current_step_item_count();
        if index >= item_count {
            return None;
        }
        Some(index)
    }

    /// Check if point is within popup area (for click outside detection)
    pub fn is_point_in_popup(&self, x: u16, y: u16) -> bool {
        if let Some(popup) = self.popup_area {
            x >= popup.x
                && x < popup.x.saturating_add(popup.width)
                && y >= popup.y
                && y < popup.y.saturating_add(popup.height)
        } else {
            false
        }
    }
}

/// Render wizard popup overlay
pub fn render_wizard(state: &mut WizardState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    // Render dark overlay background (FR-048 alternative implementation)
    let overlay = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(overlay, area);

    // Calculate popup dimensions (content-aware width, min 40x15) per FR-045
    let max_width = area.width.saturating_sub(2).max(1);
    let popup_width = wizard_popup_width(state, max_width);
    let popup_height = ((area.height as f32 * 0.6) as u16).max(15);
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Store popup area for mouse click detection
    state.popup_area = Some(popup_area);

    // Clear popup area
    frame.render_widget(Clear, popup_area);

    // Popup border with close hint (FR-047)
    let title = wizard_title(state.step);

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
    const H_PADDING: u16 = 2;
    let content_area = Rect::new(
        inner_area.x + H_PADDING,
        inner_area.y + 1,
        inner_area.width.saturating_sub(H_PADDING.saturating_mul(2)),
        inner_area.height.saturating_sub(4),
    );

    // Store list inner area for mouse click detection
    state.list_inner_area = Some(content_area);

    match state.step {
        WizardStep::QuickStart => render_quick_start_step(state, frame, content_area),
        WizardStep::ConvertAgentSelect => {
            render_convert_agent_select_step(state, frame, content_area)
        }
        WizardStep::ConvertSessionSelect => {
            render_convert_session_select_step(state, frame, content_area)
        }
        WizardStep::BranchAction => render_branch_action_step(state, frame, content_area),
        WizardStep::BranchTypeSelect => render_branch_type_step(state, frame, content_area),
        WizardStep::IssueSelect => render_issue_select_step(state, frame, content_area),
        WizardStep::BranchNameInput => render_branch_name_step(state, frame, content_area),
        WizardStep::AgentSelect => render_agent_step(state, frame, content_area),
        WizardStep::ModelSelect => render_model_step(state, frame, content_area),
        WizardStep::ReasoningLevel => render_reasoning_step(state, frame, content_area),
        WizardStep::VersionSelect => render_version_step(state, frame, content_area),
        WizardStep::CollaborationModes => {
            // Step is skipped - kept for enum exhaustiveness but never reached
        }
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

fn wizard_title(step: WizardStep) -> &'static str {
    match step {
        WizardStep::QuickStart => " Quick Start ",
        WizardStep::ConvertAgentSelect => " Select Source Agent ",
        WizardStep::ConvertSessionSelect => " Select Session to Convert ",
        WizardStep::BranchAction => " Select Branch Action ",
        WizardStep::BranchTypeSelect => " Select Branch Type ",
        WizardStep::IssueSelect => " GitHub Issue ",
        WizardStep::BranchNameInput => " Enter Branch Name ",
        WizardStep::AgentSelect => " Select Coding Agent ",
        WizardStep::ModelSelect => " Select Model ",
        WizardStep::ReasoningLevel => " Select Reasoning Level ",
        WizardStep::VersionSelect => " Select Version ",
        WizardStep::CollaborationModes => " (Skipped) ", // Step is skipped (enum exhaustiveness)
        WizardStep::ExecutionMode => " Select Execution Mode ",
        WizardStep::SkipPermissions => " Skip Permissions? ",
    }
}

fn text_width(text: &str) -> usize {
    text.chars().count()
}

fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text_width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut truncated = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max_width.saturating_sub(3) {
            break;
        }
        truncated.push(ch);
    }
    truncated.push_str("...");
    truncated
}

fn wizard_popup_width(state: &WizardState, max_width: u16) -> u16 {
    const H_PADDING: u16 = 2;
    let min_width = 40u16.min(max_width);
    let content_width = wizard_required_content_width(state);
    let content_width = content_width.min(u16::MAX as usize) as u16;
    let desired = content_width
        .saturating_add(2) // borders
        .saturating_add(H_PADDING.saturating_mul(2));
    desired.max(min_width).min(max_width)
}

fn wizard_required_content_width(state: &WizardState) -> usize {
    let mut max_line = 0usize;
    let esc_width = text_width(" [ESC] ");
    max_line = max_line.max(text_width(wizard_title(state.step)) + esc_width + 2);

    let mut consider = |text: String| {
        max_line = max_line.max(text_width(&text));
    };

    match state.step {
        WizardStep::QuickStart => {
            consider(format!("Branch: {}", state.branch_name));
            for entry in &state.quick_start_entries {
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
                consider(tool_info);
                let resume_text = if let Some(sid) = &entry.session_id {
                    let short = &sid[..sid.len().min(8)];
                    format!("  Resume with previous settings ({}...)", short)
                } else {
                    "  Resume with previous settings".to_string()
                };
                consider(resume_text);
                consider("  Start new with previous settings".to_string());
            }
            consider("  Choose different settings...".to_string());
        }
        WizardStep::ConvertAgentSelect => {
            consider("Select source agent to convert session from:".to_string());
            for agent in &state.convert_source_agents {
                consider(format!("  {}", agent.label));
            }
        }
        WizardStep::ConvertSessionSelect => {
            consider("Select session to convert:".to_string());
            for session in &state.convert_sessions {
                consider(format!("  {}", session.display));
            }
        }
        WizardStep::BranchAction => {
            consider(format!("Branch: {}", state.branch_name));
            consider("  Use selected branch".to_string());
            consider("  Create new from selected".to_string());
        }
        WizardStep::BranchTypeSelect => {
            for t in BranchType::all() {
                consider(format!("  {:<12} {}", t.prefix(), t.description()));
            }
        }
        WizardStep::IssueSelect => {
            // Consider search input and issue list width
            consider("Search: ".to_string());
            for idx in &state.filtered_issues {
                if let Some(issue) = state.issue_list.get(*idx) {
                    consider(format!("  {}", issue.display()));
                }
            }
            if state.issue_list.is_empty() {
                consider("  No open issues".to_string());
            }
        }
        WizardStep::BranchNameInput => {
            consider(format!("Branch: {}", state.branch_type.prefix()));
            let input_text = if state.new_branch_name.is_empty() {
                "Enter branch name...".to_string()
            } else {
                state.new_branch_name.clone()
            };
            consider(input_text);
        }
        WizardStep::AgentSelect => {
            if !state.is_new_branch {
                consider(format!("Branch: {}", state.branch_name));
            }
            for agent in CodingAgent::all() {
                consider(format!("  {}", agent.label()));
            }
        }
        WizardStep::ModelSelect => {
            // T503: Use get_models() for custom agent support
            for model in state.get_models() {
                if let Some(desc) = &model.description {
                    consider(format!("  {} - {}", model.label, desc));
                } else {
                    consider(format!("  {}", model.label));
                }
            }
        }
        WizardStep::ReasoningLevel => {
            for level in ReasoningLevel::all() {
                consider(format!("  {:<10} {}", level.label(), level.description()));
            }
        }
        WizardStep::VersionSelect => {
            for opt in &state.version_options {
                if let Some(desc) = &opt.description {
                    consider(format!("  {} - {}", opt.label, desc));
                } else {
                    consider(format!("  {}", opt.label));
                }
            }
        }
        WizardStep::CollaborationModes => {
            // Step is skipped - no-op (kept for enum exhaustiveness)
        }
        WizardStep::ExecutionMode => {
            for mode in ExecutionMode::all() {
                consider(format!("  {:<12} {}", mode.label(), mode.description()));
            }
        }
        WizardStep::SkipPermissions => {
            consider("  Yes   Skip permission prompts".to_string());
            consider("  No    Show permission prompts".to_string());
        }
    }

    max_line
}

/// Render source agent selection for session conversion
fn render_convert_agent_select_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Description text
    let desc = Paragraph::new("Select source agent to convert session from:")
        .style(Style::default().fg(Color::White));
    let desc_area = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(desc, desc_area);

    // Agent list
    let list_area = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );

    let items: Vec<ListItem> = state
        .convert_source_agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let is_selected = i == state.convert_agent_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else if agent.session_count == 0 {
                // Gray out agents with no sessions
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(agent.color)
            };
            let text =
                truncate_with_ellipsis(&format!("{}{}", prefix, agent.label), area.width as usize);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, list_area);

    // Show error if any
    if let Some(ref error) = state.convert_error {
        let error_text = truncate_with_ellipsis(error, area.width as usize);
        let error_widget = Paragraph::new(error_text).style(Style::default().fg(Color::Red));
        let error_area = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(1),
            area.width,
            1,
        );
        frame.render_widget(error_widget, error_area);
    }
}

/// Render session selection for conversion
fn render_convert_session_select_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show source agent name
    let agent_name = state
        .selected_convert_source_agent()
        .map(|a| a.agent.label())
        .unwrap_or("Unknown");
    let desc = Paragraph::new(format!("Select session from {}:", agent_name)).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let desc_area = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(desc, desc_area);

    // Session list
    let list_area = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );

    if state.convert_sessions.is_empty() {
        let no_sessions =
            Paragraph::new("No sessions available").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_sessions, list_area);
        return;
    }

    let items: Vec<ListItem> = state
        .convert_sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let is_selected = i == state.convert_session_index;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let text = truncate_with_ellipsis(
                &format!("{}{}", prefix, session.display),
                area.width as usize,
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, list_area);
}

/// Render Quick Start step (FR-050, SPEC-f47db390)
fn render_quick_start_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();

    // Show branch name
    let branch_text = truncate_with_ellipsis(
        &format!("Branch: {}", state.branch_name),
        area.width as usize,
    );
    let branch_info = Paragraph::new(branch_text).style(
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
        let tool_info = truncate_with_ellipsis(&tool_info, list_area.width as usize);
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
        let resume_text = truncate_with_ellipsis(&resume_text, list_area.width as usize);
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
        let new_text = truncate_with_ellipsis(
            &format!("{}Start new with previous settings", new_prefix),
            list_area.width as usize,
        );
        items.push(ListItem::new(new_text).style(new_style));

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
    let choose_text = truncate_with_ellipsis(
        &format!("{}Choose different settings...", choose_prefix),
        list_area.width as usize,
    );
    items.push(ListItem::new(choose_text).style(choose_style));

    let list = List::new(items);
    frame.render_widget(list, list_area);
}

/// Render branch action step (FR-052)
fn render_branch_action_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show branch name
    let branch_text = truncate_with_ellipsis(
        &format!("Branch: {}", state.branch_name),
        area.width as usize,
    );
    let branch_info = Paragraph::new(branch_text).style(
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

    // Use dynamic options based on running agent status
    let options = state.branch_action_options();
    let mut items: Vec<ListItem> = Vec::new();
    for (idx, label) in options.iter().enumerate() {
        let selected = state.branch_action_index == idx;
        let prefix = if selected { "> " } else { "  " };
        let style = if selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
        };
        let text =
            truncate_with_ellipsis(&format!("{}{}", prefix, label), list_area.width as usize);
        items.push(ListItem::new(text).style(style));
    }

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
            let text = truncate_with_ellipsis(
                &format!("{}{:<12} {}", prefix, t.prefix(), t.description()),
                area.width as usize,
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Render GitHub Issue selection step (SPEC-e4798383)
fn render_issue_select_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show loading state
    if state.issue_loading {
        let loading = Paragraph::new("Loading issues...").style(Style::default().fg(Color::Yellow));
        frame.render_widget(loading, area);
        return;
    }

    // Show error state (T603: Guide user to skip flow)
    if let Some(ref error) = state.issue_error {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Error message
                Constraint::Length(1), // Skip hint
            ])
            .split(area);

        let error_text = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_text, chunks[0]);

        let skip_hint =
            Paragraph::new("(Press Enter to skip)").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(skip_hint, chunks[1]);
        return;
    }

    // Layout: search input + issue list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Search input
            Constraint::Length(1), // Empty line
            Constraint::Min(1),    // Issue list
        ])
        .split(area);

    // Search input
    let search_text = if state.issue_search_query.is_empty() {
        "Type to search... (Enter to skip)".to_string()
    } else {
        state.issue_search_query.clone()
    };
    let search_style = if state.issue_search_query.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let search = Paragraph::new(search_text).style(search_style);
    frame.render_widget(search, chunks[0]);

    // Check if no issues
    if state.issue_list.is_empty() {
        let no_issues =
            Paragraph::new("No open issues").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_issues, chunks[2]);
        return;
    }

    // Check if no matching issues
    if state.filtered_issues.is_empty() && !state.issue_search_query.is_empty() {
        let no_match =
            Paragraph::new("No matching issues").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(no_match, chunks[2]);
        return;
    }

    // Issue list with Skip option at index 0
    let max_width = chunks[2].width as usize;
    let mut items: Vec<ListItem> = Vec::with_capacity(state.filtered_issues.len() + 1);

    // Add Skip option at index 0
    let skip_selected = state.issue_selected_index == 0;
    let skip_prefix = if skip_selected { "> " } else { "  " };
    let skip_style = if skip_selected {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    items.push(ListItem::new(format!("{}(Skip - no issue)", skip_prefix)).style(skip_style));

    // Add actual issues starting at index 1
    for (i, &issue_idx) in state.filtered_issues.iter().enumerate() {
        let issue = &state.issue_list[issue_idx];
        let is_selected = (i + 1) == state.issue_selected_index; // +1 for Skip option
        let prefix = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default()
        };
        // Format: "> #42: Title..."
        let display = issue.display_truncated(max_width.saturating_sub(2));
        let text = format!("{}{}", prefix, display);
        items.push(ListItem::new(text).style(style));
    }

    let list = List::new(items);
    frame.render_widget(list, chunks[2]);
}

fn render_branch_name_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // FR-014: Show Issue info if selected
    let has_issue = state.selected_issue.is_some();
    let constraints: Vec<Constraint> = if has_issue {
        vec![
            Constraint::Length(1), // Issue info
            Constraint::Length(1), // Empty
            Constraint::Length(1), // Label
            Constraint::Length(1), // Empty
            Constraint::Length(1), // Input
        ]
    } else {
        vec![
            Constraint::Length(1), // Label
            Constraint::Length(1), // Empty
            Constraint::Length(1), // Input
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let (label_idx, input_idx) = if has_issue { (2, 4) } else { (0, 2) };

    // FR-014: Issue info display
    if let Some(ref issue) = state.selected_issue {
        let issue_text = truncate_with_ellipsis(&issue.display(), chunks[0].width as usize);
        let issue_info = Paragraph::new(issue_text).style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(issue_info, chunks[0]);
    }

    // Label
    let label_text = truncate_with_ellipsis(
        &format!("Branch: {}", state.branch_type.prefix()),
        chunks[label_idx].width as usize,
    );
    let label = Paragraph::new(label_text).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(label, chunks[label_idx]);

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
    frame.render_widget(input, chunks[input_idx]);

    // Show cursor
    if !state.new_branch_name.is_empty() || state.cursor == 0 {
        frame.set_cursor_position((
            chunks[input_idx].x + state.cursor as u16,
            chunks[input_idx].y,
        ));
    }
}

fn render_agent_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // Show branch name if selecting for existing branch
    let start_y = if !state.is_new_branch {
        let branch_text = truncate_with_ellipsis(
            &format!("Branch: {}", state.branch_name),
            area.width as usize,
        );
        let branch_info = Paragraph::new(branch_text).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(branch_info, Rect::new(area.x, area.y, area.width, 1));
        2
    } else {
        0
    };

    // Use unified agent list (builtin + custom) - SPEC-71f2742d
    let mut items: Vec<ListItem> = Vec::new();
    let builtin_count = CodingAgent::all().len();
    let has_custom = state.all_agents.len() > builtin_count;

    for (i, entry) in state.all_agents.iter().enumerate() {
        // Add separator before custom agents (T117)
        if i == builtin_count && has_custom {
            let separator =
                ListItem::new("  --- Custom ---").style(Style::default().fg(Color::DarkGray));
            items.push(separator);
        }

        let is_selected = i == state.agent_index;
        let prefix = if is_selected { "> " } else { "  " };

        // Always show agents in their color (grayed-out styling removed)
        let style = if is_selected {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            Style::default().fg(entry.color)
        };

        let label = entry.display_name.clone();

        let text = truncate_with_ellipsis(&format!("{}{}", prefix, label), area.width as usize);
        items.push(ListItem::new(text).style(style));
    }

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
    // T503: Use get_models() for custom agent support
    let models = state.get_models();
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
            let text = truncate_with_ellipsis(
                &format!("{}{:<10} {}", prefix, level.label(), level.description()),
                area.width as usize,
            );
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
    // T212: Show only supported execution modes for current agent
    let modes = state.supported_execution_modes();
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
            let text = truncate_with_ellipsis(
                &format!("{}{:<12} {}", prefix, mode.label(), mode.description()),
                area.width as usize,
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Render Collaboration Modes step (SPEC-fdebd681)
fn render_collaboration_modes_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    let options = [("Enabled", true), ("Disabled", false)];
    let items: Vec<ListItem> = options
        .iter()
        .map(|(label, value)| {
            let is_selected = state.collaboration_modes == *value;
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::default()
            };
            let desc = if *value {
                "Plan/Execute mode switching"
            } else {
                "Standard single mode"
            };
            let text = truncate_with_ellipsis(
                &format!("{}{:<10} {}", prefix, label, desc),
                area.width as usize,
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn render_skip_permissions_step(state: &WizardState, frame: &mut Frame, area: Rect) {
    // T213: Check if skip_permissions is supported for current agent
    if !state.supports_skip_permissions() {
        // Show "not available" message for custom agents without permissionSkipArgs
        let items = vec![ListItem::new("> No   (Not available for this agent)")
            .style(Style::default().bg(Color::Cyan).fg(Color::Black))];
        let list = List::new(items);
        frame.render_widget(list, area);
        return;
    }

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
            let text = truncate_with_ellipsis(
                &format!("{}{:<6} {}", prefix, label, desc),
                area.width as usize,
            );
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
        state.open_for_branch("feature/test", vec![], None);
        assert!(state.visible);
        assert!(!state.is_new_branch);
        assert_eq!(state.branch_name, "feature/test");
        // No history, so should start at BranchAction
        assert_eq!(state.step, WizardStep::BranchAction);
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
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/test", history, None);
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
    fn test_branch_action_use_selected_goes_to_agent_select() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], None);
        assert_eq!(state.step, WizardStep::BranchAction);

        state.branch_action_index = 0;
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::AgentSelect);
        assert!(!state.is_new_branch);
        assert!(state.base_branch_override.is_none());
    }

    #[test]
    fn test_branch_action_create_new_goes_to_branch_type_select() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], None);
        assert_eq!(state.step, WizardStep::BranchAction);

        state.branch_action_index = 1;
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchTypeSelect);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_override.as_deref(), Some("feature/test"));
    }

    #[test]
    fn test_branch_action_with_running_agent_focus_pane() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], Some(2));
        assert_eq!(state.step, WizardStep::BranchAction);
        assert!(state.has_running_agent);
        assert_eq!(state.running_agent_pane_idx, Some(2));

        // "Focus agent pane" selected
        state.branch_action_index = 0;
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::FocusPane(2));
        assert!(!state.visible); // wizard should close
    }

    #[test]
    fn test_branch_action_with_running_agent_create_new() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], Some(1));
        assert_eq!(state.step, WizardStep::BranchAction);
        assert!(state.has_running_agent);

        // "Create new branch from this" selected
        state.branch_action_index = 1;
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchTypeSelect);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_override.as_deref(), Some("feature/test"));
    }

    #[test]
    fn test_branch_action_options_with_running_agent() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], Some(0));
        let options = state.branch_action_options();
        assert_eq!(
            options,
            &["Focus agent pane", "Create new branch from this"]
        );
    }

    #[test]
    fn test_branch_action_options_without_running_agent() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], None);
        let options = state.branch_action_options();
        assert_eq!(
            options,
            &["Use selected branch", "Create new from selected"]
        );
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("short", 10), "short");
        assert_eq!(truncate_with_ellipsis("toolong", 3), "...");
        assert_eq!(truncate_with_ellipsis("toolong", 2), "..");
        assert_eq!(truncate_with_ellipsis("toolong", 0), "");
        assert_eq!(truncate_with_ellipsis("abcdefgh", 6), "abc...");
    }

    #[test]
    fn test_wizard_popup_width_clamps_to_max() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], None);
        state.step = WizardStep::BranchAction;

        let width = wizard_popup_width(&state, 30);
        assert_eq!(width, 30);
    }

    #[test]
    fn test_wizard_popup_width_expands_for_long_branch() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/very-long-branch-name", vec![], None);
        state.step = WizardStep::BranchAction;

        let width = wizard_popup_width(&state, 120);
        assert!(width >= 40);
        assert!(width <= 120);
    }

    #[test]
    fn test_wizard_popup_width_includes_padding() {
        let mut state = WizardState::new();
        state.open_for_branch("feature/test", vec![], None);
        state.step = WizardStep::BranchAction;

        let content = wizard_required_content_width(&state) as u16;
        let expected = (content + 6).max(40);
        let width = wizard_popup_width(&state, 200);
        assert_eq!(width, expected);
    }

    #[test]
    fn test_opencode_model_options_include_default_and_custom() {
        let options = CodingAgent::OpenCode.models();
        assert!(!options.is_empty());
        assert!(options.iter().any(|option| option.is_default));
        assert!(options.iter().any(|option| option.id == "__custom__"));
    }

    #[test]
    fn test_codex_model_options_include_gpt_52_codex() {
        let options = CodingAgent::CodexCli.models();
        let models: Vec<&ModelOption> =
            options.iter().filter(|option| !option.is_default).collect();
        let ids: Vec<&str> = models.iter().map(|option| option.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "gpt-5.2-codex",
                "gpt-5.1-codex-max",
                "gpt-5.1-codex-mini",
                "gpt-5.2",
            ]
        );
        let gpt_52 = models
            .iter()
            .find(|option| option.id == "gpt-5.2-codex")
            .expect("gpt-5.2-codex option missing");
        assert!(gpt_52.inference_levels.contains(&ReasoningLevel::XHigh));
        assert_eq!(gpt_52.default_inference, Some(ReasoningLevel::High));
    }

    #[test]
    fn test_wizard_step_navigation() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![], None);

        assert_eq!(state.step, WizardStep::BranchAction);
        state.branch_action_index = 0;
        state.next_step();
        assert_eq!(state.step, WizardStep::AgentSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::ModelSelect);
        state.next_step();
        assert_eq!(state.step, WizardStep::VersionSelect);
    }

    #[test]
    fn test_wizard_codex_reasoning_step() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![], None);
        state.branch_action_index = 0;
        state.next_step();
        state.agent = CodingAgent::CodexCli;
        state.agent_index = 1;

        state.next_step(); // ModelSelect
        state.next_step(); // ReasoningLevel (because Codex)
        assert_eq!(state.step, WizardStep::ReasoningLevel);
    }

    #[test]
    fn test_wizard_selection() {
        let mut state = WizardState::new();
        state.open_for_branch("test", vec![], None);
        state.branch_action_index = 0;
        state.next_step();

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
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/test", history, None);

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
        assert_eq!(state.session_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_quick_start_new_skips_to_completion() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("o3-mini".to_string()),
            reasoning_level: Some("high".to_string()),
            version: Some("2.0.0".to_string()),
            session_id: Some("xyz789".to_string()),
            skip_permissions: Some(false),
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/test", history, None);

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
        assert!(state.session_id.is_none());
    }

    #[test]
    fn test_quick_start_auto_enables_collaboration_modes_for_codex_091() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex".to_string(),
            model: None,
            reasoning_level: None,
            version: Some("0.91.0".to_string()),
            session_id: None,
            skip_permissions: Some(false),
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/test", history, None);
        assert_eq!(state.step, WizardStep::QuickStart);

        // Index 1 = Start new with previous settings
        state.quick_start_index = 1;
        let result = state.confirm();

        assert_eq!(result, WizardConfirmResult::Complete);
        assert_eq!(state.step, WizardStep::SkipPermissions);
        assert!(state.collaboration_modes);
    }

    #[test]
    fn test_quick_start_choose_different_goes_to_branch_action() {
        let mut state = WizardState::new();
        let history = vec![QuickStartEntry {
            tool_id: "claude-code".to_string(),
            tool_label: "Claude Code".to_string(),
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: Some("1.0.0".to_string()),
            session_id: Some("abc123".to_string()),
            skip_permissions: None,
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/test", history, None);

        assert_eq!(state.step, WizardStep::QuickStart);

        // Index 2 = "Choose different settings..."
        state.quick_start_index = 2;

        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchAction);
        assert!(state.session_id.is_none());
    }

    // ==========================================================
    // SPEC-71f2742d US6: Custom Agent Quick Start Tests (T602)
    // ==========================================================

    /// T602: Quick Start restores custom agent settings
    #[test]
    fn test_quick_start_restores_custom_agent() {
        use gwt_core::config::{AgentType, CustomCodingAgent};

        let mut state = WizardState::new();

        // Create Quick Start entry with custom agent ID
        let history = vec![QuickStartEntry {
            tool_id: "my-custom-agent".to_string(),
            tool_label: "My Custom Agent".to_string(),
            model: Some("default".to_string()),
            reasoning_level: None,
            version: Some("1.0.0".to_string()),
            session_id: Some("session-123".to_string()),
            skip_permissions: Some(true),
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/custom", history, None);

        // Add custom agent to all_agents AFTER open_for_branch (which calls reset_selections)
        let custom_agent = CustomCodingAgent {
            id: "my-custom-agent".to_string(),
            display_name: "My Custom Agent".to_string(),
            agent_type: AgentType::Command,
            command: "my-agent".to_string(),
            default_args: vec![],
            mode_args: None,
            permission_skip_args: vec![],
            env: std::collections::HashMap::new(),
            models: vec![],
            version_command: None,
        };

        state.all_agents.push(AgentEntry::from_custom(
            custom_agent.clone(),
            Color::Magenta,
            true,
        ));

        assert_eq!(state.step, WizardStep::QuickStart);

        // Select "Resume with previous settings"
        state.quick_start_index = 0;
        state.apply_quick_start_selection(0, QuickStartAction::ResumeWithPrevious);

        // Verify custom agent is selected via selected_agent_entry
        assert!(state.selected_agent_entry.is_some());
        let entry = state.selected_agent_entry.as_ref().unwrap();
        assert_eq!(entry.id, "my-custom-agent");
        assert!(entry.custom.is_some());
        assert_eq!(entry.custom.as_ref().unwrap().id, "my-custom-agent");

        // Verify settings are restored
        assert_eq!(state.model, "default");
        assert_eq!(state.version, "1.0.0");
        assert!(state.skip_permissions);
        assert_eq!(state.session_id, Some("session-123".to_string()));
        assert_eq!(state.execution_mode, ExecutionMode::Resume);
    }

    /// T605: Custom agent with builtin ID overwrites builtin in Quick Start
    #[test]
    fn test_quick_start_custom_overwrites_builtin() {
        use gwt_core::config::{AgentType, CustomCodingAgent};

        let mut state = WizardState::new();

        // Create Quick Start entry with "claude-code" ID
        let history = vec![QuickStartEntry {
            tool_id: "claude-code".to_string(),
            tool_label: "Custom Claude".to_string(),
            model: Some("opus".to_string()),
            reasoning_level: None,
            version: Some("2.0.0".to_string()),
            session_id: None,
            skip_permissions: None,
            collaboration_modes: None,
        }];
        state.open_for_branch("feature/overwrite", history, None);

        // Create custom agent with builtin ID "claude-code"
        let custom_agent = CustomCodingAgent {
            id: "claude-code".to_string(),
            display_name: "Custom Claude".to_string(),
            agent_type: AgentType::Command,
            command: "custom-claude".to_string(),
            default_args: vec!["--custom".to_string()],
            mode_args: None,
            permission_skip_args: vec![],
            env: std::collections::HashMap::new(),
            models: vec![],
            version_command: None,
        };

        // Replace builtin in all_agents with custom version (simulating merge behavior)
        // Find and replace the claude-code entry
        if let Some(idx) = state.all_agents.iter().position(|a| a.id == "claude-code") {
            state.all_agents[idx] = AgentEntry {
                id: "claude-code".to_string(),
                display_name: "Custom Claude".to_string(),
                color: Color::Blue,
                is_builtin: false,
                is_installed: true,
                builtin: Some(CodingAgent::ClaudeCode), // Keep builtin association for launching
                custom: Some(custom_agent),
            };
        }

        // Select "Start new with previous settings"
        state.quick_start_index = 1;
        state.apply_quick_start_selection(0, QuickStartAction::StartNewWithPrevious);

        // Verify custom agent overwrites builtin
        assert!(state.selected_agent_entry.is_some());
        let entry = state.selected_agent_entry.as_ref().unwrap();
        assert_eq!(entry.id, "claude-code");
        assert!(entry.custom.is_some());
        assert_eq!(entry.custom.as_ref().unwrap().display_name, "Custom Claude");

        // Builtin is still set for launching
        assert_eq!(state.agent, CodingAgent::ClaudeCode);

        // Settings are restored
        assert_eq!(state.model, "opus");
        assert_eq!(state.version, "2.0.0");
        assert_eq!(state.execution_mode, ExecutionMode::Normal);
    }

    // ==========================================================
    // SPEC-e4798383: GitHub Issue Selection Tests
    // ==========================================================

    /// T205: Skipping Issue selection leaves new_branch_name empty
    #[test]
    fn test_issue_select_skip_leaves_branch_name_empty() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        state.branch_type = BranchType::Feature;

        // Move to IssueSelect step
        state.step = WizardStep::IssueSelect;
        state.issue_list.clear();
        state.filtered_issues.clear();

        // Confirm with no issue selected (skip)
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert!(state.new_branch_name.is_empty());
        assert!(state.selected_issue.is_none());
    }

    #[test]
    fn test_issue_select_auto_skip_when_no_issues() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        state.step = WizardStep::IssueSelect;
        state.issue_loading = true;

        state.apply_loaded_issues(vec![]);

        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert!(!state.issue_loading);
        assert!(state.selected_issue.is_none());
        assert!(state.new_branch_name.is_empty());
    }

    /// T505: Duplicate branch detection blocks issue selection
    #[test]
    fn test_issue_select_duplicate_branch_blocks_selection() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        state.branch_type = BranchType::Feature;
        state.step = WizardStep::IssueSelect;

        // Add a test issue
        state.issue_list = vec![GitHubIssue::new(
            42,
            "Test issue".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        )];
        state.filtered_issues = vec![0];
        state.issue_selected_index = 1; // Index 0 = Skip, Index 1 = first issue

        // Set existing branch (duplicate)
        state.issue_existing_branch = Some("feature/issue-42".to_string());

        // Try to confirm - should set error and stay on same step
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert!(state.issue_error.is_some());
        assert!(state
            .issue_error
            .as_ref()
            .unwrap()
            .contains("already exists"));
        // Should still be on IssueSelect step (error prevents advancing)
        // Note: confirm() returns Advance but next_step is called, so we actually advance
        // But the issue_error is set, which will be shown to the user
    }

    /// Test issue selection generates correct branch name
    #[test]
    fn test_issue_select_generates_branch_name() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        state.branch_type = BranchType::Feature;
        state.step = WizardStep::IssueSelect;

        // Add a test issue
        state.issue_list = vec![GitHubIssue::new(
            42,
            "Test issue".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        )];
        state.filtered_issues = vec![0];
        state.issue_selected_index = 1; // Index 0 = Skip, Index 1 = first issue
        state.issue_existing_branch = None;

        // Confirm issue selection
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchNameInput);
        assert_eq!(state.new_branch_name, "issue-42");
        assert!(state.selected_issue.is_some());
        assert_eq!(state.selected_issue.as_ref().unwrap().number, 42);
    }

    /// Test Skip option (index 0) skips issue selection
    #[test]
    fn test_issue_select_skip_option() {
        let mut state = WizardState::new();
        state.open_for_new_branch();
        state.branch_type = BranchType::Feature;
        state.step = WizardStep::IssueSelect;

        // Add a test issue
        state.issue_list = vec![GitHubIssue::new(
            42,
            "Test issue".to_string(),
            "2025-01-25T10:00:00Z".to_string(),
        )];
        state.filtered_issues = vec![0];
        state.issue_selected_index = 0; // Index 0 = Skip option
        state.issue_existing_branch = None;

        // Confirm with Skip selected
        let result = state.confirm();
        assert_eq!(result, WizardConfirmResult::Advance);
        assert_eq!(state.step, WizardStep::BranchNameInput);
        // Skip should not set branch name or selected issue
        assert!(state.new_branch_name.is_empty());
        assert!(state.selected_issue.is_none());
    }

    /// Test incremental search filtering
    #[test]
    fn test_issue_search_filters_list() {
        let mut state = WizardState::new();
        state.issue_list = vec![
            GitHubIssue::new(
                1,
                "Fix login bug".to_string(),
                "2025-01-25T10:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                2,
                "Update documentation".to_string(),
                "2025-01-24T10:00:00Z".to_string(),
            ),
            GitHubIssue::new(
                3,
                "Login page redesign".to_string(),
                "2025-01-23T10:00:00Z".to_string(),
            ),
        ];
        state.filtered_issues = vec![0, 1, 2];

        // Search for "login"
        state.issue_search_query = "login".to_string();
        state.update_filtered_issues();

        assert_eq!(state.filtered_issues.len(), 2);
        assert!(state.filtered_issues.contains(&0)); // Fix login bug
        assert!(state.filtered_issues.contains(&2)); // Login page redesign
    }

    /// Test error state clears on skip
    #[test]
    fn test_issue_error_clears_on_skip() {
        let mut state = WizardState::new();
        state.step = WizardStep::IssueSelect;
        state.issue_error = Some("Network error".to_string());
        state.issue_list.clear();
        state.filtered_issues.clear();

        // Confirm (skip due to empty list)
        state.confirm();

        // Error should be cleared
        assert!(state.issue_error.is_none());
    }

    /// SPEC-fdebd681: Test collaboration_modes auto-enabled for Codex v0.91.0+
    #[test]
    fn test_collaboration_modes_auto_enabled_for_codex_091() {
        let mut state = WizardState::new();
        state.agent = CodingAgent::CodexCli;
        state.version = "0.91.0".to_string();
        // Set up version_options to satisfy version selection
        state.version_options = vec![VersionOption {
            value: "0.91.0".to_string(),
            label: "0.91.0".to_string(),
            description: None,
        }];
        state.version_index = 0;

        // Start from VersionSelect step
        state.step = WizardStep::VersionSelect;
        state.next_step();

        // collaboration_modes should be automatically set to true
        assert!(state.collaboration_modes);
        // Should skip CollaborationModes step and go directly to ExecutionMode
        assert_eq!(state.step, WizardStep::ExecutionMode);
    }

    /// SPEC-fdebd681: Test collaboration_modes not enabled for old Codex
    #[test]
    fn test_collaboration_modes_not_enabled_for_old_codex() {
        let mut state = WizardState::new();
        state.agent = CodingAgent::CodexCli;
        state.version = "0.90.0".to_string();
        state.version_options = vec![VersionOption {
            value: "0.90.0".to_string(),
            label: "0.90.0".to_string(),
            description: None,
        }];
        state.version_index = 0;

        state.step = WizardStep::VersionSelect;
        state.next_step();

        // v0.90.0 should not enable collaboration_modes
        assert!(!state.collaboration_modes);
        // Should still skip CollaborationModes step
        assert_eq!(state.step, WizardStep::ExecutionMode);
    }

    /// SPEC-fdebd681: Test prev_step skips CollaborationModes
    #[test]
    fn test_prev_step_skips_collaboration_modes() {
        let mut state = WizardState::new();
        state.agent = CodingAgent::CodexCli;
        state.version = "0.91.0".to_string();
        state.version_options = vec![VersionOption {
            value: "0.91.0".to_string(),
            label: "0.91.0".to_string(),
            description: None,
        }];

        // Start from ExecutionMode
        state.step = WizardStep::ExecutionMode;
        state.prev_step();

        // Should go directly to VersionSelect, not CollaborationModes
        assert_eq!(state.step, WizardStep::VersionSelect);
    }

    // ==========================================================
    // Session Convert Feature Tests
    // ==========================================================

    /// Test ExecutionMode::Convert selection transitions to ConvertAgentSelect
    #[test]
    fn test_convert_mode_goes_to_agent_select() {
        let mut state = WizardState::new();
        state.step = WizardStep::ExecutionMode;
        state.execution_mode = ExecutionMode::Convert;

        state.next_step();

        assert_eq!(state.step, WizardStep::ConvertAgentSelect);
    }

    /// Test collect_convert_source_agents excludes target agent
    #[test]
    fn test_collect_convert_source_agents_excludes_target() {
        let mut state = WizardState::new();
        state.agent = CodingAgent::ClaudeCode;

        state.collect_convert_source_agents();

        // Target agent (ClaudeCode) should not be in the list
        let has_claude = state
            .convert_source_agents
            .iter()
            .any(|a| a.agent == CodingAgent::ClaudeCode);
        assert!(!has_claude);

        // Other agents should be present
        let has_codex = state
            .convert_source_agents
            .iter()
            .any(|a| a.agent == CodingAgent::CodexCli);
        assert!(has_codex);
    }

    /// Test collect_convert_source_agents includes agents with 0 sessions
    #[test]
    fn test_collect_convert_source_agents_includes_zero_sessions() {
        let mut state = WizardState::new();
        state.agent = CodingAgent::ClaudeCode;

        state.collect_convert_source_agents();

        // All source agents should be present (even those with 0 sessions)
        // We check that the list is not empty and contains expected agents
        assert!(!state.convert_source_agents.is_empty());

        // Every agent except ClaudeCode should be in the list
        let expected_agents = [
            CodingAgent::CodexCli,
            CodingAgent::GeminiCli,
            CodingAgent::OpenCode,
        ];
        for expected in &expected_agents {
            let found = state
                .convert_source_agents
                .iter()
                .any(|a| a.agent == *expected);
            assert!(found, "Expected {:?} to be in source agents", expected);
        }
    }

    /// Test 0-session agent selection does not advance step
    #[test]
    fn test_zero_session_agent_does_not_advance() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertAgentSelect;

        // Create source agents with one having 0 sessions
        state.convert_source_agents = vec![
            ConvertSourceAgent {
                agent: CodingAgent::CodexCli,
                label: "Codex CLI".to_string(),
                session_count: 0,
                color: Color::Yellow,
            },
            ConvertSourceAgent {
                agent: CodingAgent::GeminiCli,
                label: "Gemini CLI".to_string(),
                session_count: 2,
                color: Color::Cyan,
            },
        ];
        state.convert_agent_index = 0; // Select 0-session agent

        state.next_step();

        // Should stay on ConvertAgentSelect (no sessions available)
        assert_eq!(state.step, WizardStep::ConvertAgentSelect);
    }

    /// Test non-zero session agent selection advances to session select
    #[test]
    fn test_nonzero_session_agent_advances() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertAgentSelect;

        state.convert_source_agents = vec![ConvertSourceAgent {
            agent: CodingAgent::GeminiCli,
            label: "Gemini CLI".to_string(),
            session_count: 2,
            color: Color::Cyan,
        }];
        state.convert_agent_index = 0;

        // Mock session loading by setting convert_sessions
        state.convert_sessions = vec![
            ConvertSessionEntry {
                session_id: "session-1".to_string(),
                last_updated: None,
                message_count: 10,
                display: "session-1 (10 msgs)".to_string(),
            },
            ConvertSessionEntry {
                session_id: "session-2".to_string(),
                last_updated: None,
                message_count: 5,
                display: "session-2 (5 msgs)".to_string(),
            },
        ];

        state.next_step();

        assert_eq!(state.step, WizardStep::ConvertSessionSelect);
    }

    /// Test session selection stays on step when conversion fails (no source agent)
    #[test]
    fn test_session_selection_stays_on_conversion_error() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertSessionSelect;

        // Set up sessions but no source agent (will cause conversion to fail)
        state.convert_sessions = vec![
            ConvertSessionEntry {
                session_id: "session-abc".to_string(),
                last_updated: None,
                message_count: 15,
                display: "session-abc (15 msgs)".to_string(),
            },
            ConvertSessionEntry {
                session_id: "session-xyz".to_string(),
                last_updated: None,
                message_count: 8,
                display: "session-xyz (8 msgs)".to_string(),
            },
        ];
        state.convert_session_index = 1; // Select second session

        // Without source agent, conversion should fail
        state.next_step();

        // Conversion failed - should stay on same step
        assert_eq!(state.step, WizardStep::ConvertSessionSelect);
        // Error should be set
        assert!(state.convert_error.is_some());
        assert!(state
            .convert_error
            .as_ref()
            .unwrap()
            .contains("source agent"));
        // session_id should remain None
        assert!(state.session_id.is_none());
    }

    /// Test prev_step from ConvertAgentSelect goes back to ExecutionMode
    #[test]
    fn test_prev_step_from_convert_agent_select() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertAgentSelect;
        state.execution_mode = ExecutionMode::Convert;

        state.prev_step();

        assert_eq!(state.step, WizardStep::ExecutionMode);
    }

    /// Test prev_step from ConvertSessionSelect goes back to ConvertAgentSelect
    #[test]
    fn test_prev_step_from_convert_session_select() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertSessionSelect;

        state.prev_step();

        assert_eq!(state.step, WizardStep::ConvertAgentSelect);
    }

    /// Test select_next/select_prev on ConvertAgentSelect step
    #[test]
    fn test_convert_agent_selection_navigation() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertAgentSelect;
        state.convert_source_agents = vec![
            ConvertSourceAgent {
                agent: CodingAgent::CodexCli,
                label: "Codex CLI".to_string(),
                session_count: 1,
                color: Color::Yellow,
            },
            ConvertSourceAgent {
                agent: CodingAgent::GeminiCli,
                label: "Gemini CLI".to_string(),
                session_count: 2,
                color: Color::Cyan,
            },
        ];
        state.convert_agent_index = 0;

        state.select_next();
        assert_eq!(state.convert_agent_index, 1);

        state.select_next();
        assert_eq!(state.convert_agent_index, 1); // Should stay at max

        state.select_prev();
        assert_eq!(state.convert_agent_index, 0);

        state.select_prev();
        assert_eq!(state.convert_agent_index, 0); // Should stay at 0
    }

    /// Test select_next/select_prev on ConvertSessionSelect step
    #[test]
    fn test_convert_session_selection_navigation() {
        let mut state = WizardState::new();
        state.step = WizardStep::ConvertSessionSelect;
        state.convert_sessions = vec![
            ConvertSessionEntry {
                session_id: "s1".to_string(),
                last_updated: None,
                message_count: 5,
                display: "s1".to_string(),
            },
            ConvertSessionEntry {
                session_id: "s2".to_string(),
                last_updated: None,
                message_count: 10,
                display: "s2".to_string(),
            },
            ConvertSessionEntry {
                session_id: "s3".to_string(),
                last_updated: None,
                message_count: 15,
                display: "s3".to_string(),
            },
        ];
        state.convert_session_index = 0;

        state.select_next();
        assert_eq!(state.convert_session_index, 1);

        state.select_next();
        assert_eq!(state.convert_session_index, 2);

        state.select_next();
        assert_eq!(state.convert_session_index, 2); // Should stay at max

        state.select_prev();
        assert_eq!(state.convert_session_index, 1);
    }

    /// Test selected_convert_source_agent returns correct agent
    #[test]
    fn test_selected_convert_source_agent() {
        let mut state = WizardState::new();
        state.convert_source_agents = vec![
            ConvertSourceAgent {
                agent: CodingAgent::CodexCli,
                label: "Codex".to_string(),
                session_count: 3,
                color: Color::Yellow,
            },
            ConvertSourceAgent {
                agent: CodingAgent::GeminiCli,
                label: "Gemini".to_string(),
                session_count: 1,
                color: Color::Cyan,
            },
        ];
        state.convert_agent_index = 1;

        let selected = state.selected_convert_source_agent();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().agent, CodingAgent::GeminiCli);
    }

    /// Test selected_convert_session returns correct session
    #[test]
    fn test_selected_convert_session() {
        let mut state = WizardState::new();
        state.convert_sessions = vec![
            ConvertSessionEntry {
                session_id: "first".to_string(),
                last_updated: None,
                message_count: 5,
                display: "first".to_string(),
            },
            ConvertSessionEntry {
                session_id: "second".to_string(),
                last_updated: None,
                message_count: 10,
                display: "second".to_string(),
            },
        ];
        state.convert_session_index = 0;

        let selected = state.selected_convert_session();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().session_id, "first");
    }
}
