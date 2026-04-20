//! Custom coding agent presets — factory helpers that seed well-known agent configurations.
//!
//! Presets are pure constructors: they return a `CustomCodingAgent` value that
//! callers persist through the ordinary custom-agent save path. Preset output is
//! not itself privileged at launch time; the seeded `env` table is applied
//! through `AgentLaunchBuilder` like any other custom-agent env set.

use std::collections::HashMap;

use crate::custom::{CustomAgentType, CustomCodingAgent};

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
}
