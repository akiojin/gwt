//! Core agent types for identification, display, and status tracking.

use serde::{Deserialize, Serialize};

/// Identifies a coding agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Gemini,
    OpenCode,
    Copilot,
    Custom(String),
}

impl AgentId {
    /// Canonical command name used to invoke this agent.
    pub fn command(&self) -> &str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Copilot => "gh",
            Self::Custom(name) => name,
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Copilot => "GitHub Copilot",
            Self::Custom(name) => name,
        }
    }

    /// npm package name (if distributed via npm).
    pub fn package_name(&self) -> Option<&str> {
        match self {
            Self::ClaudeCode => Some("@anthropic-ai/claude-code"),
            Self::Codex => Some("@openai/codex"),
            Self::Gemini => Some("@anthropic-ai/gemini-cli"),
            Self::OpenCode => None,
            Self::Copilot => None,
            Self::Custom(_) => None,
        }
    }

    /// Default UI color for this agent.
    pub fn default_color(&self) -> AgentColor {
        match self {
            Self::ClaudeCode => AgentColor::Yellow,
            Self::Codex => AgentColor::Cyan,
            Self::Gemini => AgentColor::Magenta,
            Self::OpenCode => AgentColor::Green,
            Self::Copilot => AgentColor::Blue,
            Self::Custom(_) => AgentColor::Gray,
        }
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Normalize a raw agent identifier string (command name, display name, or
/// persisted `agent_id`) back into an [`AgentId`].
///
/// 既知のエージェントは表記揺れを吸収して確定する (`"claude"`,
/// `"ClaudeCode"`, `"claude-code"` → `ClaudeCode`)。空文字または
/// 空白のみの入力は `None`。それ以外の未知文字列は `Custom(trimmed)`。
///
/// SPEC #2133 FR-012 / gwt-core::BoardEntry::origin_agent_id の
/// 正規化で使用される。
pub fn resolve_agent_id(raw: &str) -> Option<AgentId> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "claude" | "claudecode" | "claude-code" | "claude code" => Some(AgentId::ClaudeCode),
        "codex" => Some(AgentId::Codex),
        "gemini" | "gemini cli" | "gemini-cli" => Some(AgentId::Gemini),
        "opencode" | "open-code" => Some(AgentId::OpenCode),
        "gh" | "copilot" | "github copilot" | "github-copilot" => Some(AgentId::Copilot),
        _ => Some(AgentId::Custom(trimmed.to_string())),
    }
}

/// Static information about an agent, combining identity with presentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub display_name: String,
    pub command: String,
    pub package_name: Option<String>,
    pub color: AgentColor,
}

impl AgentInfo {
    /// Build an `AgentInfo` from an `AgentId` using defaults.
    pub fn from_id(id: AgentId) -> Self {
        Self {
            display_name: id.display_name().to_string(),
            command: id.command().to_string(),
            package_name: id.package_name().map(|s| s.to_string()),
            color: id.default_color(),
            id,
        }
    }
}

/// UI color for agent display.
///
/// Wire 表現は snake_case 小文字固定 (`"yellow"` / `"cyan"` など)。
/// フロント側の CSS 変数名 (`--agent-*`) と 1 対 1 対応させ、色値の
/// ハードコードを `crates/gwt/web/index.html` に持ち込まないための
/// 制約。See SPEC #2133 FR-001 / FR-002.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentColor {
    Green,
    Blue,
    Cyan,
    Yellow,
    Magenta,
    Gray,
}

/// Runtime status of an agent process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    #[default]
    Unknown,
    Running,
    WaitingInput,
    Stopped,
}

/// Session start mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    #[default]
    Normal,
    Continue,
    Resume,
}

/// Runtime target for launching an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchRuntimeTarget {
    #[default]
    Host,
    Docker,
}

/// Non-persisted lifecycle intent for a Docker launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockerLifecycleIntent {
    #[default]
    Connect,
    Start,
    Restart,
    Recreate,
    CreateAndStart,
}

/// Session-level workflow policy bypass for ownerless operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowBypass {
    Release,
    Chore,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_command_returns_expected() {
        assert_eq!(AgentId::ClaudeCode.command(), "claude");
        assert_eq!(AgentId::Codex.command(), "codex");
        assert_eq!(AgentId::Gemini.command(), "gemini");
        assert_eq!(AgentId::OpenCode.command(), "opencode");
        assert_eq!(AgentId::Copilot.command(), "gh");
        assert_eq!(AgentId::Custom("aider".into()).command(), "aider");
    }

    #[test]
    fn agent_id_display_name_returns_expected() {
        assert_eq!(AgentId::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(AgentId::Copilot.display_name(), "GitHub Copilot");
        assert_eq!(AgentId::Custom("aider".into()).display_name(), "aider");
    }

    #[test]
    fn agent_id_package_name() {
        assert_eq!(
            AgentId::ClaudeCode.package_name(),
            Some("@anthropic-ai/claude-code")
        );
        assert_eq!(AgentId::OpenCode.package_name(), None);
        assert_eq!(AgentId::Custom("x".into()).package_name(), None);
    }

    #[test]
    fn agent_id_default_color() {
        assert_eq!(AgentId::ClaudeCode.default_color(), AgentColor::Yellow);
        assert_eq!(AgentId::Codex.default_color(), AgentColor::Cyan);
        assert_eq!(AgentId::Gemini.default_color(), AgentColor::Magenta);
        assert_eq!(
            AgentId::Custom("x".into()).default_color(),
            AgentColor::Gray
        );
    }

    #[test]
    fn agent_info_from_id() {
        let info = AgentInfo::from_id(AgentId::ClaudeCode);
        assert_eq!(info.display_name, "Claude Code");
        assert_eq!(info.command, "claude");
        assert_eq!(info.color, AgentColor::Yellow);
        assert_eq!(info.package_name, Some("@anthropic-ai/claude-code".into()));
    }

    #[test]
    fn agent_status_default_is_unknown() {
        assert_eq!(AgentStatus::default(), AgentStatus::Unknown);
    }

    #[test]
    fn session_mode_default_is_normal() {
        assert_eq!(SessionMode::default(), SessionMode::Normal);
    }

    #[test]
    fn agent_id_display_trait() {
        assert_eq!(format!("{}", AgentId::ClaudeCode), "Claude Code");
        assert_eq!(format!("{}", AgentId::Custom("aider".into())), "aider");
    }

    #[test]
    fn resolve_agent_id_maps_known_identifiers() {
        let claude_inputs = ["claude", "ClaudeCode", "Claude Code", "claude-code"];
        for raw in claude_inputs {
            assert_eq!(resolve_agent_id(raw), Some(AgentId::ClaudeCode), "{raw}");
        }
        assert_eq!(resolve_agent_id("codex"), Some(AgentId::Codex));
        assert_eq!(resolve_agent_id("Codex"), Some(AgentId::Codex));
        assert_eq!(resolve_agent_id("gemini"), Some(AgentId::Gemini));
        assert_eq!(resolve_agent_id("Gemini CLI"), Some(AgentId::Gemini));
        assert_eq!(resolve_agent_id("opencode"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("OpenCode"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("open-code"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("gh"), Some(AgentId::Copilot));
        assert_eq!(resolve_agent_id("copilot"), Some(AgentId::Copilot));
        assert_eq!(resolve_agent_id("GitHub Copilot"), Some(AgentId::Copilot));
    }

    #[test]
    fn resolve_agent_id_returns_none_for_empty() {
        assert_eq!(resolve_agent_id(""), None);
        assert_eq!(resolve_agent_id("   "), None);
        assert_eq!(resolve_agent_id("\t\n"), None);
    }

    #[test]
    fn resolve_agent_id_falls_back_to_custom() {
        assert_eq!(
            resolve_agent_id("my-aider"),
            Some(AgentId::Custom("my-aider".into()))
        );
        assert_eq!(
            resolve_agent_id("  aider  "),
            Some(AgentId::Custom("aider".into())),
            "trims whitespace"
        );
        assert_eq!(
            resolve_agent_id("unknown-cli"),
            Some(AgentId::Custom("unknown-cli".into()))
        );
    }

    #[test]
    fn resolve_agent_id_does_not_infer_known_agents_from_custom_names() {
        let custom_inputs = ["my-claude-wrapper", "codex-wrapper", "opencode-mentor"];
        for raw in custom_inputs {
            assert_eq!(
                resolve_agent_id(raw),
                Some(AgentId::Custom(raw.into())),
                "{raw}"
            );
        }
    }

    #[test]
    fn resolve_agent_id_feeds_default_color() {
        // FR-012 → resolve_agent_id → default_color 連携の確認
        let cases = [
            ("claude", AgentColor::Yellow),
            ("codex", AgentColor::Cyan),
            ("gemini", AgentColor::Magenta),
            ("opencode", AgentColor::Green),
            ("gh", AgentColor::Blue),
            ("my-custom", AgentColor::Gray),
        ];
        for (raw, expected) in cases {
            let color = resolve_agent_id(raw)
                .map(|id| id.default_color())
                .expect("non-empty input");
            assert_eq!(color, expected, "{raw}");
        }
    }

    #[test]
    fn agent_id_serde_roundtrip() {
        let ids = vec![
            AgentId::ClaudeCode,
            AgentId::Codex,
            AgentId::Custom("test".into()),
        ];
        for id in ids {
            let json = serde_json::to_string(&id).unwrap();
            let parsed: AgentId = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, id);
        }
    }

    #[test]
    fn agent_color_serde_roundtrip() {
        let color = AgentColor::Cyan;
        let json = serde_json::to_string(&color).unwrap();
        let parsed: AgentColor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, color);
    }

    #[test]
    fn agent_color_serializes_as_snake_case() {
        // CSS variable names (--agent-claude など) と 1 対 1 対応させるため、
        // wire 表現は snake_case 小文字固定。
        let pairs = [
            (AgentColor::Yellow, "\"yellow\""),
            (AgentColor::Cyan, "\"cyan\""),
            (AgentColor::Magenta, "\"magenta\""),
            (AgentColor::Green, "\"green\""),
            (AgentColor::Blue, "\"blue\""),
            (AgentColor::Gray, "\"gray\""),
        ];
        for (color, expected) in pairs {
            let json = serde_json::to_string(&color).unwrap();
            assert_eq!(json, expected, "serialize form for {color:?}");
            let parsed: AgentColor = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, color);
        }
    }

    #[test]
    fn workflow_bypass_serde_roundtrip() {
        for bypass in [WorkflowBypass::Release, WorkflowBypass::Chore] {
            let json = serde_json::to_string(&bypass).unwrap();
            let parsed: WorkflowBypass = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, bypass);
        }
    }
}
