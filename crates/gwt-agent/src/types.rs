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
            Self::ClaudeCode => AgentColor::Green,
            Self::Codex => AgentColor::Blue,
            Self::Gemini => AgentColor::Cyan,
            Self::OpenCode => AgentColor::Yellow,
            Self::Copilot => AgentColor::Magenta,
            Self::Custom(_) => AgentColor::Gray,
        }
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
        assert_eq!(AgentId::ClaudeCode.default_color(), AgentColor::Green);
        assert_eq!(AgentId::Codex.default_color(), AgentColor::Blue);
        assert_eq!(AgentId::Custom("x".into()).default_color(), AgentColor::Gray);
    }

    #[test]
    fn agent_info_from_id() {
        let info = AgentInfo::from_id(AgentId::ClaudeCode);
        assert_eq!(info.display_name, "Claude Code");
        assert_eq!(info.command, "claude");
        assert_eq!(info.color, AgentColor::Green);
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
}
