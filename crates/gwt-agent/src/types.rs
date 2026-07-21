//! Core agent types for identification, display, and status tracking.

use serde::{Deserialize, Serialize};

/// Identifies a coding agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Antigravity,
    Gemini,
    OpenCode,
    OpenClaw,
    Hermes,
    Copilot,
    Custom(String),
}

impl AgentId {
    /// Canonical command name used to invoke this agent.
    pub fn command(&self) -> &str {
        self.builtin_descriptor()
            .map(|descriptor| descriptor.command)
            .unwrap_or_else(|| match self {
                Self::Custom(name) => name,
                _ => unreachable!("all non-custom agents must have descriptors"),
            })
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &str {
        self.builtin_descriptor()
            .map(|descriptor| descriptor.display_name)
            .unwrap_or_else(|| match self {
                Self::Custom(name) => name,
                _ => unreachable!("all non-custom agents must have descriptors"),
            })
    }

    /// npm package name (if distributed via npm).
    pub fn package_name(&self) -> Option<&str> {
        self.builtin_descriptor()
            .and_then(|descriptor| descriptor.package_name)
    }

    /// Default UI color for this agent.
    pub fn default_color(&self) -> AgentColor {
        self.builtin_descriptor()
            .map(|descriptor| descriptor.color)
            .unwrap_or(AgentColor::Gray)
    }

    pub fn builtin_descriptor(&self) -> Option<&'static BuiltinAgentDescriptor> {
        builtin_agent_descriptors()
            .iter()
            .find(|descriptor| descriptor.id == *self)
    }

    /// Whether this agent's CLI exposes an interactive session picker when its
    /// resume command is invoked without a session id (e.g. `claude --resume`
    /// shows the picker, `codex resume` shows the picker). Used by the Launch
    /// Wizard to gate the Execution Mode `Resume` option.
    ///
    /// SPEC-2014 2026-05-18 amendment FR-C.
    pub fn supports_resume_picker(&self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex)
    }

    /// Whether this agent can continue the latest session without an explicit
    /// session id.
    pub fn supports_continue_latest(&self) -> bool {
        matches!(
            self,
            Self::ClaudeCode | Self::Codex | Self::Antigravity | Self::OpenCode | Self::Hermes
        )
    }

    /// Whether this agent can resume a specific saved session id.
    pub fn supports_resume_session_id(&self) -> bool {
        matches!(
            self,
            Self::ClaudeCode
                | Self::Codex
                | Self::Antigravity
                | Self::OpenCode
                | Self::OpenClaw
                | Self::Hermes
        )
    }

    /// Whether this agent exposes a Fast mode launch setting that can be
    /// enabled before the agent starts.
    ///
    /// SPEC-2014 2026-05-27 amendment: Claude Code supports Fast mode through
    /// session-local settings, while Codex supports it through its service tier
    /// config override.
    pub fn supports_fast_mode(&self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex)
    }

    /// Whether this agent supports selecting an upstream provider at launch
    /// (Hermes `--provider`). SPEC-3152.
    pub fn supports_provider_selection(&self) -> bool {
        matches!(self, Self::Hermes)
    }

    /// Whether this agent supports selecting a named config profile at launch
    /// (Hermes `--profile`). SPEC-3152.
    pub fn supports_profile_selection(&self) -> bool {
        matches!(self, Self::Hermes)
    }

    /// Whether this agent takes a free-text model string rather than a fixed
    /// gwt model list, because the available models depend on the chosen
    /// provider (Hermes `--model`, OpenCode `--model provider/model`).
    /// SPEC-3152 / SPEC-3151 FR-008.
    pub fn supports_freetext_model(&self) -> bool {
        matches!(self, Self::Hermes | Self::OpenCode)
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Metadata for a built-in coding agent.
///
/// Keep agent identity, presentation, detection, and cache keys in this
/// descriptor so new built-ins do not require synchronized table edits across
/// every consumer.
#[derive(Debug, Clone)]
pub struct BuiltinAgentDescriptor {
    pub id: AgentId,
    pub command: &'static str,
    pub display_name: &'static str,
    pub package_name: Option<&'static str>,
    pub color: AgentColor,
    pub aliases: &'static [&'static str],
    pub cache_key: &'static str,
    pub version_flag: &'static str,
    pub version_prefix_args: &'static [&'static str],
}

const BUILTIN_AGENT_DESCRIPTORS: &[BuiltinAgentDescriptor] = &[
    BuiltinAgentDescriptor {
        id: AgentId::ClaudeCode,
        command: "claude",
        display_name: "Claude Code",
        package_name: Some("@anthropic-ai/claude-code"),
        color: AgentColor::Yellow,
        aliases: &["claude", "claudecode", "claude-code", "claude code"],
        cache_key: "claude-code",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::Codex,
        command: "codex",
        display_name: "Codex",
        package_name: Some("@openai/codex"),
        color: AgentColor::Cyan,
        aliases: &["codex", "codex-cli", "codex cli", "codexcli"],
        cache_key: "codex",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::Antigravity,
        command: "agy",
        display_name: "Antigravity CLI",
        package_name: None,
        color: AgentColor::Green,
        aliases: &["agy", "antigravity", "antigravity cli", "antigravity-cli"],
        cache_key: "antigravity",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::Gemini,
        command: "gemini",
        display_name: "Gemini CLI (legacy)",
        package_name: Some("@google/gemini-cli"),
        color: AgentColor::Magenta,
        aliases: &["gemini", "gemini cli", "gemini-cli", "gemini cli legacy"],
        cache_key: "gemini",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::OpenCode,
        command: "opencode",
        display_name: "OpenCode",
        // SPEC-3151: OpenCode ships on npm as `opencode-ai` (bin `opencode`),
        // so versioned launches route through the bunx/npx package runner like
        // Codex/Claude Code instead of requiring a native binary in PATH.
        package_name: Some("opencode-ai"),
        color: AgentColor::Green,
        aliases: &["opencode", "open-code"],
        cache_key: "opencode",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::OpenClaw,
        command: "openclaw",
        display_name: "OpenClaw",
        package_name: None,
        color: AgentColor::Blue,
        aliases: &["openclaw", "open-claw"],
        cache_key: "openclaw",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::Hermes,
        command: "hermes",
        display_name: "Hermes Agent",
        package_name: None,
        color: AgentColor::Magenta,
        aliases: &["hermes", "hermes agent", "hermes-agent"],
        cache_key: "hermes",
        version_flag: "--version",
        version_prefix_args: &[],
    },
    BuiltinAgentDescriptor {
        id: AgentId::Copilot,
        command: "gh",
        display_name: "GitHub Copilot",
        package_name: None,
        color: AgentColor::Blue,
        aliases: &["gh", "copilot", "github copilot", "github-copilot"],
        cache_key: "copilot",
        version_flag: "--version",
        version_prefix_args: &["copilot"],
    },
];

pub fn builtin_agent_descriptors() -> &'static [BuiltinAgentDescriptor] {
    BUILTIN_AGENT_DESCRIPTORS
}

pub fn builtin_agent_descriptor_for_command(
    command: &str,
) -> Option<&'static BuiltinAgentDescriptor> {
    builtin_agent_descriptors()
        .iter()
        .find(|descriptor| descriptor.command == command)
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
    if let Some(descriptor) = builtin_agent_descriptors().iter().find(|descriptor| {
        descriptor.aliases.iter().any(|alias| *alias == lower)
            || descriptor.command == lower
            || descriptor.display_name.eq_ignore_ascii_case(trimmed)
    }) {
        Some(descriptor.id.clone())
    } else {
        Some(AgentId::Custom(trimmed.to_string()))
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
            package_name: id.package_name().map(std::string::ToString::to_string),
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
    #[serde(alias = "running", alias = "Running")]
    Running,
    #[serde(alias = "idle", alias = "Idle")]
    Idle,
    #[serde(
        rename = "Waiting",
        alias = "waiting",
        alias = "Waiting",
        alias = "waiting_input",
        alias = "WaitingInput"
    )]
    WaitingInput,
    #[serde(alias = "stopped", alias = "Stopped")]
    Stopped,
    #[serde(alias = "interrupted", alias = "Interrupted")]
    Interrupted,
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

/// Windows Host shell used to wrap interactive launch commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsShellKind {
    CommandPrompt,
    WindowsPowerShell,
    #[serde(rename = "power_shell_7", alias = "power_shell7")]
    PowerShell7,
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
        assert_eq!(AgentId::Antigravity.command(), "agy");
        assert_eq!(AgentId::Gemini.command(), "gemini");
        assert_eq!(AgentId::OpenCode.command(), "opencode");
        assert_eq!(AgentId::OpenClaw.command(), "openclaw");
        assert_eq!(AgentId::Hermes.command(), "hermes");
        assert_eq!(AgentId::Copilot.command(), "gh");
        assert_eq!(AgentId::Custom("aider".into()).command(), "aider");
    }

    #[test]
    fn agent_id_display_name_returns_expected() {
        assert_eq!(AgentId::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(AgentId::Antigravity.display_name(), "Antigravity CLI");
        assert_eq!(AgentId::Gemini.display_name(), "Gemini CLI (legacy)");
        assert_eq!(AgentId::OpenCode.display_name(), "OpenCode");
        assert_eq!(AgentId::OpenClaw.display_name(), "OpenClaw");
        assert_eq!(AgentId::Hermes.display_name(), "Hermes Agent");
        assert_eq!(AgentId::Copilot.display_name(), "GitHub Copilot");
        assert_eq!(AgentId::Custom("aider".into()).display_name(), "aider");
    }

    #[test]
    fn agent_id_package_name() {
        assert_eq!(
            AgentId::ClaudeCode.package_name(),
            Some("@anthropic-ai/claude-code")
        );
        assert_eq!(AgentId::Antigravity.package_name(), None);
        assert_eq!(AgentId::Gemini.package_name(), Some("@google/gemini-cli"));
        assert_eq!(AgentId::OpenCode.package_name(), Some("opencode-ai"));
        assert_eq!(AgentId::OpenClaw.package_name(), None);
        assert_eq!(AgentId::Hermes.package_name(), None);
        assert_eq!(AgentId::Custom("x".into()).package_name(), None);
    }

    #[test]
    fn agent_id_default_color() {
        assert_eq!(AgentId::ClaudeCode.default_color(), AgentColor::Yellow);
        assert_eq!(AgentId::Codex.default_color(), AgentColor::Cyan);
        assert_eq!(AgentId::Antigravity.default_color(), AgentColor::Green);
        assert_eq!(AgentId::Gemini.default_color(), AgentColor::Magenta);
        assert_eq!(AgentId::OpenCode.default_color(), AgentColor::Green);
        assert_eq!(AgentId::OpenClaw.default_color(), AgentColor::Blue);
        assert_eq!(AgentId::Hermes.default_color(), AgentColor::Magenta);
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
    fn builtin_agent_descriptors_drive_agent_info_contract() {
        let descriptors = builtin_agent_descriptors();
        assert_eq!(descriptors.len(), 8);

        for descriptor in descriptors {
            let info = AgentInfo::from_id(descriptor.id.clone());
            assert_eq!(info.command, descriptor.command, "{:?}", descriptor.id);
            assert_eq!(
                info.display_name, descriptor.display_name,
                "{:?}",
                descriptor.id
            );
            assert_eq!(info.package_name.as_deref(), descriptor.package_name);
            assert_eq!(info.color, descriptor.color);
            assert_eq!(
                resolve_agent_id(descriptor.command),
                Some(descriptor.id.clone()),
                "{} must resolve through the same descriptor registry",
                descriptor.command
            );
        }
    }

    #[test]
    fn legacy_writer_codex_cli_id_resolves_to_builtin_codex() {
        assert_eq!(resolve_agent_id("codex-cli"), Some(AgentId::Codex));
    }

    #[test]
    fn antigravity_descriptor_uses_native_agy_without_npm_package() {
        let descriptor =
            builtin_agent_descriptor_for_command("agy").expect("Antigravity must be built in");

        assert_eq!(descriptor.command, "agy");
        assert_eq!(descriptor.display_name, "Antigravity CLI");
        assert_eq!(descriptor.package_name, None);
        assert_eq!(descriptor.cache_key, "antigravity");
        assert_eq!(descriptor.version_flag, "--version");
        assert!(descriptor.aliases.contains(&"antigravity"));
        assert!(descriptor.aliases.contains(&"antigravity-cli"));
    }

    #[test]
    fn agent_status_default_is_unknown() {
        assert_eq!(AgentStatus::default(), AgentStatus::Unknown);
    }

    #[test]
    fn agent_status_waiting_preserves_wire_contract_and_legacy_aliases() {
        let json = serde_json::to_string(&AgentStatus::WaitingInput).unwrap();
        assert_eq!(json, "\"Waiting\"");

        for raw in [
            "\"Waiting\"",
            "\"waiting\"",
            "\"waiting_input\"",
            "\"WaitingInput\"",
        ] {
            let parsed: AgentStatus = serde_json::from_str(raw).unwrap();
            assert_eq!(parsed, AgentStatus::WaitingInput, "{raw}");
        }
    }

    #[test]
    fn agent_status_idle_preserves_wire_contract_and_aliases() {
        let json = serde_json::to_string(&AgentStatus::Idle).unwrap();
        assert_eq!(json, "\"Idle\"");

        for raw in ["\"Idle\"", "\"idle\""] {
            let parsed: AgentStatus = serde_json::from_str(raw).unwrap();
            assert_eq!(parsed, AgentStatus::Idle, "{raw}");
        }
    }

    #[test]
    fn session_mode_default_is_normal() {
        assert_eq!(SessionMode::default(), SessionMode::Normal);
    }

    #[test]
    fn supports_resume_picker_only_for_picker_capable_builtins() {
        // SPEC-2014 2026-05-18 amendment FR-C: Claude Code と Codex のみ
        // interactive picker (`claude --resume` / `codex resume`) を持つ。
        assert!(AgentId::ClaudeCode.supports_resume_picker());
        assert!(AgentId::Codex.supports_resume_picker());
        for non_picker in [
            AgentId::Antigravity,
            AgentId::Gemini,
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
            AgentId::Copilot,
            AgentId::Custom("aider".into()),
        ] {
            assert!(
                !non_picker.supports_resume_picker(),
                "{non_picker:?} should not advertise picker support"
            );
        }
    }

    #[test]
    fn supports_continue_latest_only_for_agents_with_latest_session_args() {
        for supported in [
            AgentId::ClaudeCode,
            AgentId::Codex,
            resolve_agent_id("agy").expect("Antigravity must resolve"),
            AgentId::OpenCode,
            AgentId::Hermes,
        ] {
            assert!(
                supported.supports_continue_latest(),
                "{supported:?} should support latest-session continue"
            );
        }
        for unsupported in [
            AgentId::Gemini,
            AgentId::OpenClaw,
            AgentId::Copilot,
            AgentId::Custom("aider".into()),
        ] {
            assert!(
                !unsupported.supports_continue_latest(),
                "{unsupported:?} should not advertise latest-session continue"
            );
        }
    }

    #[test]
    fn supports_resume_session_id_matches_agents_with_specific_resume_args() {
        for supported in [
            AgentId::ClaudeCode,
            AgentId::Codex,
            resolve_agent_id("agy").expect("Antigravity must resolve"),
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
        ] {
            assert!(
                supported.supports_resume_session_id(),
                "{supported:?} should support explicit session resume"
            );
        }
        for unsupported in [
            AgentId::Gemini,
            AgentId::Copilot,
            AgentId::Custom("aider".into()),
        ] {
            assert!(
                !unsupported.supports_resume_session_id(),
                "{unsupported:?} should not advertise explicit session resume"
            );
        }
    }

    #[test]
    fn supports_fast_mode_only_for_fast_capable_builtins() {
        assert!(AgentId::ClaudeCode.supports_fast_mode());
        assert!(AgentId::Codex.supports_fast_mode());
        for unsupported in [
            AgentId::Antigravity,
            AgentId::Gemini,
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
            AgentId::Copilot,
            AgentId::Custom("aider".into()),
        ] {
            assert!(
                !unsupported.supports_fast_mode(),
                "{unsupported:?} should not advertise Fast mode support"
            );
        }
    }

    #[test]
    fn hermes_advertises_provider_profile_and_freetext_model() {
        assert!(AgentId::Hermes.supports_provider_selection());
        assert!(AgentId::Hermes.supports_profile_selection());
        assert!(AgentId::Hermes.supports_freetext_model());
        // SPEC-3151 FR-008: OpenCode also takes a free-text `provider/model`
        // string, but it does not expose Hermes-style provider/profile flags.
        assert!(AgentId::OpenCode.supports_freetext_model());
        assert!(!AgentId::OpenCode.supports_provider_selection());
        assert!(!AgentId::OpenCode.supports_profile_selection());
        for other in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Gemini] {
            assert!(!other.supports_provider_selection());
            assert!(!other.supports_profile_selection());
            assert!(!other.supports_freetext_model());
        }
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
        assert_eq!(
            resolve_agent_id("Gemini CLI (legacy)"),
            Some(AgentId::Gemini)
        );
        assert_eq!(
            resolve_agent_id("agy").map(|id| id.command().to_string()),
            Some("agy".into())
        );
        assert_eq!(
            resolve_agent_id("Antigravity CLI").map(|id| id.display_name().to_string()),
            Some("Antigravity CLI".into())
        );
        assert_eq!(resolve_agent_id("opencode"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("OpenCode"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("open-code"), Some(AgentId::OpenCode));
        assert_eq!(resolve_agent_id("openclaw"), Some(AgentId::OpenClaw));
        assert_eq!(resolve_agent_id("OpenClaw"), Some(AgentId::OpenClaw));
        assert_eq!(resolve_agent_id("open-claw"), Some(AgentId::OpenClaw));
        assert_eq!(resolve_agent_id("hermes"), Some(AgentId::Hermes));
        assert_eq!(resolve_agent_id("Hermes Agent"), Some(AgentId::Hermes));
        assert_eq!(resolve_agent_id("hermes-agent"), Some(AgentId::Hermes));
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
        let custom_inputs = [
            "my-claude-wrapper",
            "codex-wrapper",
            "gemini-helper",
            "antigravity-helper",
            "opencode-mentor",
            "openclaw-mentor",
            "hermes-helper",
            "copilot-mentor",
            "github-copilot-wrapper",
        ];
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
            ("agy", AgentColor::Green),
            ("antigravity", AgentColor::Green),
            ("gemini", AgentColor::Magenta),
            ("opencode", AgentColor::Green),
            ("openclaw", AgentColor::Blue),
            ("hermes", AgentColor::Magenta),
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
            AgentId::Antigravity,
            AgentId::Gemini,
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
            AgentId::Copilot,
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

    #[test]
    fn windows_shell_kind_wire_values_are_stable() {
        assert_eq!(
            serde_json::to_string(&WindowsShellKind::CommandPrompt).unwrap(),
            "\"command_prompt\""
        );
        assert_eq!(
            serde_json::from_str::<WindowsShellKind>("\"windows_power_shell\"").unwrap(),
            WindowsShellKind::WindowsPowerShell
        );
        assert_eq!(
            serde_json::from_str::<WindowsShellKind>("\"power_shell_7\"").unwrap(),
            WindowsShellKind::PowerShell7
        );
        assert_eq!(
            serde_json::from_str::<WindowsShellKind>("\"power_shell7\"").unwrap(),
            WindowsShellKind::PowerShell7
        );
    }
}
