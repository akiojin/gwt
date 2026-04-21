//! Custom coding agent presets — factory helpers that seed well-known agent configurations.
//!
//! Presets are pure constructors: they return a `CustomCodingAgent` value that
//! callers persist through the ordinary custom-agent save path. Preset output is
//! not itself privileged at launch time; the seeded `env` table is applied
//! through `AgentLaunchBuilder` like any other custom-agent env set.

use std::{collections::HashMap, fmt};

use crate::custom::{CustomAgentType, CustomCodingAgent};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// Stable identifier for a built-in Custom Agent preset. Keep this set small:
/// every id is a frontend-visible contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetId {
    /// Claude Code routed through an Anthropic Messages API compatible proxy
    /// that speaks `/v1/models`. SPEC-1921 FR-062.
    ClaudeCodeOpenaiCompat,
}

impl PresetId {
    /// Return the transport string used by the Settings UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            PresetId::ClaudeCodeOpenaiCompat => "claude_code_openai_compat",
        }
    }
}

impl fmt::Display for PresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata that the Settings UI shows in the "Add from preset" picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetDefinition {
    /// Stable id used by the add-from-preset request.
    pub id: PresetId,
    /// Display label rendered in the picker.
    pub label: &'static str,
    /// Short description rendered below the label in the picker.
    pub description: &'static str,
}

impl PresetDefinition {
    const fn catalog() -> [PresetDefinition; 1] {
        [PresetDefinition {
            id: PresetId::ClaudeCodeOpenaiCompat,
            label: "Claude Code (OpenAI-compat backend)",
            description: concat!(
                "Route Claude Code to an Anthropic Messages API compatible ",
                "proxy backed by an OpenAI-compatible upstream."
            ),
        }]
    }
}

/// Return the catalog of built-in Custom Agent presets.
pub fn list_presets() -> Vec<PresetDefinition> {
    PresetDefinition::catalog().to_vec()
}

/// Input payload for adding a custom agent from the
/// `ClaudeCodeOpenaiCompat` preset. SPEC-1921 FR-060 / FR-062.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaudeCodeOpenaiCompatInput {
    /// TOML key / stable id for the new custom agent. Must match
    /// `CustomCodingAgent::validate()` (alphanumeric + `-`).
    pub id: String,
    /// Human-readable name shown in the agent picker.
    pub display_name: String,
    /// Upstream base URL (http/https).
    pub base_url: String,
    /// API key forwarded as `Bearer <api_key>` during `/v1/models` probe and
    /// injected as `ANTHROPIC_API_KEY` at launch.
    pub api_key: String,
    /// Model ID chosen from the probe-populated dropdown.
    pub default_model: String,
}

/// Error returned by preset payload parsing, validation, or seed construction.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PresetError {
    /// The payload could not be deserialized as the selected preset's input.
    #[error("invalid payload for preset `{preset_id}`: {message}")]
    InvalidPayload {
        preset_id: PresetId,
        message: String,
    },
    /// The deserialized payload failed semantic validation.
    #[error("invalid input for preset `{preset_id}`: {message}")]
    InvalidInput {
        preset_id: PresetId,
        message: String,
    },
    /// The preset factory returned an invalid custom-agent definition.
    #[error("preset `{preset_id}` produced an invalid agent id: {agent_id}")]
    InvalidAgent {
        preset_id: PresetId,
        agent_id: String,
    },
}

trait PresetFactory {
    type Input: DeserializeOwned;

    const ID: PresetId;

    fn validate(input: &Self::Input) -> Result<(), PresetError>;

    fn build(input: Self::Input) -> CustomCodingAgent;

    fn parse_input(payload: &Value) -> Result<Self::Input, PresetError> {
        serde_json::from_value(payload.clone()).map_err(|err| PresetError::InvalidPayload {
            preset_id: Self::ID,
            message: err.to_string(),
        })
    }

    fn seed(payload: &Value) -> Result<CustomCodingAgent, PresetError> {
        let input = Self::parse_input(payload)?;
        Self::validate(&input)?;
        let agent = Self::build(input);
        if agent.validate() {
            Ok(agent)
        } else {
            Err(PresetError::InvalidAgent {
                preset_id: Self::ID,
                agent_id: agent.id,
            })
        }
    }
}

struct ClaudeCodeOpenaiCompatPreset;

impl PresetFactory for ClaudeCodeOpenaiCompatPreset {
    type Input = ClaudeCodeOpenaiCompatInput;

    const ID: PresetId = PresetId::ClaudeCodeOpenaiCompat;

    fn validate(input: &Self::Input) -> Result<(), PresetError> {
        validate_claude_code_openai_compat_input(input)
    }

    fn build(input: Self::Input) -> CustomCodingAgent {
        claude_code_openai_compat_preset(
            input.id,
            input.display_name,
            input.base_url,
            input.api_key,
            input.default_model,
        )
    }
}

/// Seed a built-in preset by stable id and opaque JSON payload.
pub fn seed_agent(preset_id: PresetId, payload: &Value) -> Result<CustomCodingAgent, PresetError> {
    match preset_id {
        PresetId::ClaudeCodeOpenaiCompat => ClaudeCodeOpenaiCompatPreset::seed(payload),
    }
}

fn require_non_empty(preset_id: PresetId, field: &str, value: &str) -> Result<(), PresetError> {
    if value.trim().is_empty() {
        Err(PresetError::InvalidInput {
            preset_id,
            message: format!("{field} must not be empty"),
        })
    } else {
        Ok(())
    }
}

fn is_valid_base_url(base_url: &str) -> bool {
    let lower = base_url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn validate_claude_code_openai_compat_input(
    input: &ClaudeCodeOpenaiCompatInput,
) -> Result<(), PresetError> {
    let preset_id = PresetId::ClaudeCodeOpenaiCompat;
    require_non_empty(preset_id, "id", &input.id)?;
    if !input.id.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(PresetError::InvalidInput {
            preset_id,
            message: format!(
                "id `{}` contains invalid characters (allowed: alphanumeric, `-`)",
                input.id
            ),
        });
    }
    require_non_empty(preset_id, "display_name", &input.display_name)?;
    if !is_valid_base_url(&input.base_url) {
        return Err(PresetError::InvalidInput {
            preset_id,
            message: format!(
                "base_url must start with http:// or https://, got: {}",
                input.base_url
            ),
        });
    }
    require_non_empty(preset_id, "api_key", &input.api_key)?;
    require_non_empty(preset_id, "default_model", &input.default_model)?;
    Ok(())
}

/// Seed a `CustomCodingAgent` that routes Claude Code's Anthropic Messages API
/// traffic through an OpenAI-compatible upstream (local LLM runtime, self-hosted
/// gateway, etc.).
///
/// SPEC-1921 FR-062: the preset populates all four model-role env vars
/// (`ANTHROPIC_DEFAULT_HAIKU_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`,
/// `ANTHROPIC_DEFAULT_SONNET_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL`) with the
/// single `default_model` ID chosen in the Settings form.
pub fn claude_code_openai_compat_preset(
    id: impl Into<String>,
    display_name: impl Into<String>,
    base_url: impl Into<String>,
    api_key: impl Into<String>,
    default_model: impl Into<String>,
) -> CustomCodingAgent {
    let base_url = base_url.into();
    let api_key = api_key.into();
    let default_model = default_model.into();

    let mut env = HashMap::with_capacity(13);
    env.insert("ANTHROPIC_API_KEY".to_string(), api_key);
    env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);
    env.insert(
        "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
        default_model.clone(),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
        default_model.clone(),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
        default_model.clone(),
    );
    env.insert("CLAUDE_CODE_SUBAGENT_MODEL".to_string(), default_model);
    env.insert(
        "CLAUDE_CODE_ATTRIBUTION_HEADER".to_string(),
        "0".to_string(),
    );
    env.insert("DISABLE_TELEMETRY".to_string(), "1".to_string());
    env.insert("CLAUDE_CODE_NO_FLICKER".to_string(), "1".to_string());
    env.insert("DISABLE_ERROR_REPORTING".to_string(), "1".to_string());
    env.insert("DISABLE_FEEDBACK_COMMAND".to_string(), "1".to_string());
    env.insert(
        "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY".to_string(),
        "1".to_string(),
    );
    env.insert(
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_string(),
        "1".to_string(),
    );

    CustomCodingAgent {
        id: id.into(),
        display_name: display_name.into(),
        agent_type: CustomAgentType::Bunx,
        command: "@anthropic-ai/claude-code@latest".to_string(),
        default_args: vec![],
        mode_args: None,
        skip_permissions_args: vec!["--dangerously-skip-permissions".to_string()],
        env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preset_has_expected_shape() {
        let preset = claude_code_openai_compat_preset(
            "claude-code-openai",
            "Claude Code (OpenAI-compat)",
            "http://192.168.100.166:32768",
            "sk-test-123",
            "openai/gpt-oss-20b",
        );

        assert_eq!(preset.id, "claude-code-openai");
        assert_eq!(preset.display_name, "Claude Code (OpenAI-compat)");
        assert_eq!(preset.agent_type, CustomAgentType::Bunx);
        assert_eq!(preset.command, "@anthropic-ai/claude-code@latest");
        assert!(preset.default_args.is_empty());
        assert!(preset.mode_args.is_none());
        assert_eq!(
            preset.skip_permissions_args,
            vec!["--dangerously-skip-permissions".to_string()]
        );
    }

    #[test]
    fn preset_env_contains_thirteen_entries() {
        let preset = claude_code_openai_compat_preset("x", "X", "http://a", "k", "m");
        assert_eq!(
            preset.env.len(),
            13,
            "preset must seed exactly 13 env vars (FR-062)"
        );
    }

    #[test]
    fn preset_env_includes_all_required_keys() {
        let preset = claude_code_openai_compat_preset(
            "x",
            "X",
            "http://proxy.local:32768",
            "sk-test-123",
            "openai/gpt-oss-20b",
        );
        let expected_keys = [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "DISABLE_TELEMETRY",
            "CLAUDE_CODE_NO_FLICKER",
            "DISABLE_ERROR_REPORTING",
            "DISABLE_FEEDBACK_COMMAND",
            "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        ];
        for key in expected_keys {
            assert!(
                preset.env.contains_key(key),
                "preset env is missing key {key}"
            );
        }
    }

    #[test]
    fn preset_propagates_base_url_and_api_key_verbatim() {
        let preset = claude_code_openai_compat_preset(
            "x",
            "X",
            "http://192.168.100.166:32768",
            "sk_cwPkycrPTZBYQ8vFXsc3O0wkrvt36VSh",
            "openai/gpt-oss-20b",
        );
        assert_eq!(
            preset.env["ANTHROPIC_BASE_URL"],
            "http://192.168.100.166:32768"
        );
        assert_eq!(
            preset.env["ANTHROPIC_API_KEY"],
            "sk_cwPkycrPTZBYQ8vFXsc3O0wkrvt36VSh"
        );
    }

    #[test]
    fn preset_default_model_propagates_to_all_four_roles() {
        let preset = claude_code_openai_compat_preset("x", "X", "http://a", "k", "my-custom-model");
        assert_eq!(
            preset.env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "my-custom-model"
        );
        assert_eq!(
            preset.env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            "my-custom-model"
        );
        assert_eq!(
            preset.env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            "my-custom-model"
        );
        assert_eq!(preset.env["CLAUDE_CODE_SUBAGENT_MODEL"], "my-custom-model");
    }

    #[test]
    fn preset_attribution_and_telemetry_flags_are_off() {
        let preset = claude_code_openai_compat_preset("x", "X", "http://a", "k", "m");
        assert_eq!(preset.env["CLAUDE_CODE_ATTRIBUTION_HEADER"], "0");
        assert_eq!(preset.env["DISABLE_TELEMETRY"], "1");
        assert_eq!(preset.env["CLAUDE_CODE_NO_FLICKER"], "1");
        assert_eq!(preset.env["DISABLE_ERROR_REPORTING"], "1");
        assert_eq!(preset.env["DISABLE_FEEDBACK_COMMAND"], "1");
        assert_eq!(preset.env["CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY"], "1");
        assert_eq!(preset.env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    }

    #[test]
    fn preset_id_passes_custom_agent_validate() {
        let preset = claude_code_openai_compat_preset(
            "claude-code-openai",
            "Claude Code (OpenAI-compat)",
            "https://example.com",
            "key",
            "model-a",
        );
        assert!(preset.validate());
    }

    #[test]
    fn seed_agent_dispatches_by_preset_id_and_payload() {
        let payload = json!({
            "id": "claude-code-openai",
            "display_name": "Claude Code (OpenAI-compat)",
            "base_url": "https://proxy.example.com",
            "api_key": "sk-test-123",
            "default_model": "openai/gpt-oss-20b"
        });

        let preset = seed_agent(PresetId::ClaudeCodeOpenaiCompat, &payload).expect("seed preset");

        assert_eq!(preset.id, "claude-code-openai");
        assert_eq!(preset.command, "@anthropic-ai/claude-code@latest");
        assert_eq!(preset.env.len(), 13);
        assert_eq!(
            preset.env["ANTHROPIC_BASE_URL"],
            "https://proxy.example.com"
        );
        assert_eq!(preset.env["ANTHROPIC_API_KEY"], "sk-test-123");
        assert_eq!(
            preset.env["CLAUDE_CODE_SUBAGENT_MODEL"],
            "openai/gpt-oss-20b"
        );
    }

    #[test]
    fn seed_agent_rejects_malformed_payload() {
        let payload = json!({
            "id": "claude-code-openai"
        });

        let err = seed_agent(PresetId::ClaudeCodeOpenaiCompat, &payload).unwrap_err();

        assert!(matches!(err, PresetError::InvalidPayload { .. }));
    }
}
