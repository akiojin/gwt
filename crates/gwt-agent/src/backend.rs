//! Backend Override profile for built-in coding agents (SPEC-1921 US-17 / FR-098).
//!
//! `AgentBackendProfile` represents a saved redirect from a built-in agent
//! (Claude Code, Codex) to a non-default LLM endpoint such as LM Studio,
//! llmlb, or a self-hosted OpenAI-compatible gateway. Stored under
//! `[builtinAgents.<agent>.backends.<id>]` in `~/.gwt/config.toml`.
//!
//! The profile carries a common base (`id`, `display_name`, `base_url`,
//! `api_key`, `model`) plus agent-specific optional extensions. Each
//! extension MUST be empty for the wrong agent; this is enforced by
//! [`AgentBackendProfile::validate`].

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Subset of [`crate::types::AgentId`] for built-ins that support backend
/// override. Keep this set small: every variant is a TOML-section key and a
/// frontend-visible contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinAgentId {
    ClaudeCode,
    Codex,
}

impl BuiltinAgentId {
    /// Stable camelCase string used as a TOML section key and a transport
    /// identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            BuiltinAgentId::ClaudeCode => "claudeCode",
            BuiltinAgentId::Codex => "codex",
        }
    }

    /// Reverse mapping from the stable camelCase string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claudeCode" => Some(BuiltinAgentId::ClaudeCode),
            "codex" => Some(BuiltinAgentId::Codex),
            _ => None,
        }
    }

    /// Embed into the broader [`crate::types::AgentId`] enum.
    pub fn to_agent_id(self) -> crate::types::AgentId {
        match self {
            BuiltinAgentId::ClaudeCode => crate::types::AgentId::ClaudeCode,
            BuiltinAgentId::Codex => crate::types::AgentId::Codex,
        }
    }
}

/// A saved Backend Override profile attached to a built-in agent.
///
/// The same struct represents both Claude Code and Codex profiles; the
/// `agent` association is implicit in the TOML section path
/// (`[builtinAgents.<agent>.backends.<id>]`) and validated through
/// [`AgentBackendProfile::validate`].
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackendProfile {
    /// Stable id (`[a-z0-9-]+`). Unique within the owning built-in agent's
    /// backend list.
    #[serde(default)]
    pub id: String,
    /// Human-readable label shown in the Backend picker.
    pub display_name: String,
    /// Upstream base URL (must begin with `http://` or `https://`).
    pub base_url: String,
    /// API key. Persisted under audit-redaction rules; the on-wire shape
    /// uses [`crate::audit::REDACTED_PLACEHOLDER`] when returned to the
    /// frontend.
    pub api_key: String,
    /// Default model id reported to the agent. For Claude Code, this is
    /// fanned out to all four `ANTHROPIC_DEFAULT_*_MODEL` / subagent roles
    /// unless an explicit role override is set.
    pub model: String,

    // ---- Claude Code-specific extensions (optional, MUST be None for Codex)
    /// Overrides `ANTHROPIC_DEFAULT_HAIKU_MODEL` env var. Defaults to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haiku_model: Option<String>,
    /// Overrides `ANTHROPIC_DEFAULT_OPUS_MODEL`. Defaults to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opus_model: Option<String>,
    /// Overrides `ANTHROPIC_DEFAULT_SONNET_MODEL`. Defaults to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sonnet_model: Option<String>,
    /// Overrides `CLAUDE_CODE_SUBAGENT_MODEL`. Defaults to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,

    // ---- Codex-specific extensions (optional, MUST be None for Claude Code)
    /// Codex `wire_api` (`"chat"` or `"responses"`). Defaults to `"responses"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_api: Option<String>,
    /// Codex `env_key` (env var name carrying the API key). Defaults to
    /// `GWT_CODEX_BACKEND_API_KEY_<UPPER>` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_key: Option<String>,
    /// Codex `model_provider` id. Defaults to `gwt-<profile.id>` to avoid
    /// colliding with reserved built-in provider ids (`openai`, `ollama`,
    /// `lmstudio`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// Codex provider-level static HTTP headers.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub http_headers: HashMap<String, String>,
    /// Codex provider-level URL query params.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub query_params: HashMap<String, String>,
}

impl AgentBackendProfile {
    /// Validate id, base_url, and model invariants plus agent-specific field
    /// constraints (Claude Code MUST NOT carry Codex fields and vice versa).
    pub fn validate(&self, agent: BuiltinAgentId) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("id must not be empty");
        }
        if !self
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("id must match [a-z0-9-]+");
        }
        if self.display_name.trim().is_empty() {
            return Err("display_name must not be empty");
        }
        let url_lower = self.base_url.trim().to_ascii_lowercase();
        if !(url_lower.starts_with("http://") || url_lower.starts_with("https://")) {
            return Err("base_url must begin with http:// or https://");
        }
        if self.model.trim().is_empty() {
            return Err("model must not be empty");
        }

        match agent {
            BuiltinAgentId::ClaudeCode => {
                if self.wire_api.is_some()
                    || self.env_key.is_some()
                    || self.provider_id.is_some()
                    || !self.http_headers.is_empty()
                    || !self.query_params.is_empty()
                {
                    return Err("Claude Code backend must not carry Codex-specific fields");
                }
            }
            BuiltinAgentId::Codex => {
                if self.haiku_model.is_some()
                    || self.opus_model.is_some()
                    || self.sonnet_model.is_some()
                    || self.subagent_model.is_some()
                {
                    return Err("Codex backend must not carry Claude Code-specific fields");
                }
            }
        }

        Ok(())
    }

    /// Effective `ANTHROPIC_DEFAULT_HAIKU_MODEL` for a Claude Code launch:
    /// `haiku_model` if set, otherwise `model`.
    pub fn effective_haiku_model(&self) -> &str {
        self.haiku_model.as_deref().unwrap_or(&self.model)
    }

    /// Effective `ANTHROPIC_DEFAULT_OPUS_MODEL`.
    pub fn effective_opus_model(&self) -> &str {
        self.opus_model.as_deref().unwrap_or(&self.model)
    }

    /// Effective `ANTHROPIC_DEFAULT_SONNET_MODEL`.
    pub fn effective_sonnet_model(&self) -> &str {
        self.sonnet_model.as_deref().unwrap_or(&self.model)
    }

    /// Effective `CLAUDE_CODE_SUBAGENT_MODEL`.
    pub fn effective_subagent_model(&self) -> &str {
        self.subagent_model.as_deref().unwrap_or(&self.model)
    }

    /// Effective Codex `wire_api`. Defaults to `"responses"` per
    /// FR-103 / Codex CLI conventions.
    pub fn effective_wire_api(&self) -> &str {
        self.wire_api.as_deref().unwrap_or("responses")
    }

    /// Effective Codex `provider_id`. Defaults to `gwt-<id>` so the generated
    /// provider entry never collides with built-in reserved ids.
    pub fn effective_provider_id(&self) -> String {
        self.provider_id
            .clone()
            .unwrap_or_else(|| format!("gwt-{}", self.id))
    }

    /// Effective Codex `env_key`. Defaults to
    /// `GWT_CODEX_BACKEND_API_KEY_<UPPER>` so multiple Codex backends can
    /// coexist without overlapping the OPENAI_API_KEY namespace.
    pub fn effective_env_key(&self) -> String {
        self.env_key.clone().unwrap_or_else(|| {
            let upper: String = self
                .id
                .chars()
                .map(|c| {
                    if c == '-' {
                        '_'
                    } else {
                        c.to_ascii_uppercase()
                    }
                })
                .collect();
            format!("GWT_CODEX_BACKEND_API_KEY_{upper}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc_profile(id: &str) -> AgentBackendProfile {
        AgentBackendProfile {
            id: id.into(),
            display_name: "LM Studio".into(),
            base_url: "http://192.168.100.166:32768".into(),
            api_key: "sk-test".into(),
            model: "openai/gpt-oss-20b".into(),
            ..Default::default()
        }
    }

    fn codex_profile(id: &str) -> AgentBackendProfile {
        AgentBackendProfile {
            id: id.into(),
            display_name: "llmlb".into(),
            base_url: "http://127.0.0.1:8080".into(),
            api_key: "sk-codex".into(),
            model: "local/qwen3-coder".into(),
            ..Default::default()
        }
    }

    #[test]
    fn builtin_agent_id_round_trip() {
        assert_eq!(BuiltinAgentId::ClaudeCode.as_str(), "claudeCode");
        assert_eq!(BuiltinAgentId::Codex.as_str(), "codex");
        assert_eq!(
            BuiltinAgentId::parse("claudeCode"),
            Some(BuiltinAgentId::ClaudeCode)
        );
        assert_eq!(BuiltinAgentId::parse("codex"), Some(BuiltinAgentId::Codex));
        assert_eq!(BuiltinAgentId::parse("gemini"), None);
    }

    #[test]
    fn builtin_agent_id_maps_to_agent_id() {
        assert_eq!(
            BuiltinAgentId::ClaudeCode.to_agent_id(),
            crate::types::AgentId::ClaudeCode
        );
        assert_eq!(
            BuiltinAgentId::Codex.to_agent_id(),
            crate::types::AgentId::Codex
        );
    }

    #[test]
    fn validate_accepts_minimal_claude_code_profile() {
        cc_profile("lmstudio")
            .validate(BuiltinAgentId::ClaudeCode)
            .expect("minimal Claude Code profile should validate");
    }

    #[test]
    fn validate_accepts_minimal_codex_profile() {
        codex_profile("llmlb")
            .validate(BuiltinAgentId::Codex)
            .expect("minimal Codex profile should validate");
    }

    #[test]
    fn validate_rejects_empty_id() {
        let mut p = cc_profile("");
        p.id = String::new();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("id must not be empty")
        );
    }

    #[test]
    fn validate_rejects_uppercase_id() {
        let mut p = cc_profile("LMStudio");
        p.id = "LMStudio".into();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("id must match [a-z0-9-]+")
        );
    }

    #[test]
    fn validate_rejects_id_with_spaces() {
        let mut p = cc_profile("ls");
        p.id = "lm studio".into();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("id must match [a-z0-9-]+")
        );
    }

    #[test]
    fn validate_rejects_empty_display_name() {
        let mut p = cc_profile("x");
        p.display_name = String::new();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("display_name must not be empty")
        );
    }

    #[test]
    fn validate_rejects_non_http_base_url() {
        let mut p = cc_profile("x");
        p.base_url = "ftp://example.com".into();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("base_url must begin with http:// or https://")
        );
    }

    #[test]
    fn validate_accepts_https_base_url() {
        let mut p = cc_profile("x");
        p.base_url = "https://proxy.example.com:443/v1".into();
        p.validate(BuiltinAgentId::ClaudeCode)
            .expect("https accepted");
    }

    #[test]
    fn validate_rejects_empty_model() {
        let mut p = cc_profile("x");
        p.model = "  ".into();
        assert_eq!(
            p.validate(BuiltinAgentId::ClaudeCode),
            Err("model must not be empty")
        );
    }

    #[test]
    fn validate_rejects_claude_code_profile_with_codex_fields() {
        let mut p = cc_profile("x");
        p.wire_api = Some("chat".into());
        assert!(p.validate(BuiltinAgentId::ClaudeCode).is_err());

        let mut p2 = cc_profile("x");
        p2.env_key = Some("MY_KEY".into());
        assert!(p2.validate(BuiltinAgentId::ClaudeCode).is_err());

        let mut p3 = cc_profile("x");
        p3.http_headers.insert("X-K".into(), "v".into());
        assert!(p3.validate(BuiltinAgentId::ClaudeCode).is_err());
    }

    #[test]
    fn validate_rejects_codex_profile_with_claude_code_fields() {
        let mut p = codex_profile("x");
        p.opus_model = Some("opus".into());
        assert!(p.validate(BuiltinAgentId::Codex).is_err());

        let mut p2 = codex_profile("x");
        p2.subagent_model = Some("sub".into());
        assert!(p2.validate(BuiltinAgentId::Codex).is_err());
    }

    #[test]
    fn claude_code_extension_defaults_fan_out_model() {
        let p = cc_profile("x");
        assert_eq!(p.effective_haiku_model(), "openai/gpt-oss-20b");
        assert_eq!(p.effective_opus_model(), "openai/gpt-oss-20b");
        assert_eq!(p.effective_sonnet_model(), "openai/gpt-oss-20b");
        assert_eq!(p.effective_subagent_model(), "openai/gpt-oss-20b");
    }

    #[test]
    fn claude_code_extension_overrides_per_role() {
        let mut p = cc_profile("x");
        p.haiku_model = Some("haiku".into());
        p.opus_model = Some("opus".into());
        p.sonnet_model = Some("sonnet".into());
        p.subagent_model = Some("sub".into());
        assert_eq!(p.effective_haiku_model(), "haiku");
        assert_eq!(p.effective_opus_model(), "opus");
        assert_eq!(p.effective_sonnet_model(), "sonnet");
        assert_eq!(p.effective_subagent_model(), "sub");
        p.validate(BuiltinAgentId::ClaudeCode).expect("valid");
    }

    #[test]
    fn codex_extension_defaults() {
        let p = codex_profile("llmlb");
        assert_eq!(p.effective_wire_api(), "responses");
        assert_eq!(p.effective_provider_id(), "gwt-llmlb");
        assert_eq!(p.effective_env_key(), "GWT_CODEX_BACKEND_API_KEY_LLMLB");
    }

    #[test]
    fn codex_extension_overrides() {
        let mut p = codex_profile("llmlb");
        p.wire_api = Some("chat".into());
        p.provider_id = Some("custom-provider".into());
        p.env_key = Some("MY_CUSTOM_KEY".into());
        assert_eq!(p.effective_wire_api(), "chat");
        assert_eq!(p.effective_provider_id(), "custom-provider");
        assert_eq!(p.effective_env_key(), "MY_CUSTOM_KEY");
        p.validate(BuiltinAgentId::Codex).expect("valid");
    }

    #[test]
    fn codex_env_key_default_uppercases_and_dashes_become_underscores() {
        let mut p = codex_profile("multi-host-llm");
        p.id = "multi-host-llm".into();
        assert_eq!(
            p.effective_env_key(),
            "GWT_CODEX_BACKEND_API_KEY_MULTI_HOST_LLM"
        );
    }

    #[test]
    fn toml_camel_case_round_trip_claude_code() {
        let mut p = cc_profile("lmstudio");
        p.opus_model = Some("openai/gpt-oss-120b".into());
        let serialized = toml::to_string(&p).expect("serialize");
        // SPEC-1921 FR-098: canonical camelCase keys on disk.
        assert!(serialized.contains("displayName = \"LM Studio\""));
        assert!(serialized.contains("baseUrl = \"http://192.168.100.166:32768\""));
        assert!(serialized.contains("apiKey = \"sk-test\""));
        assert!(serialized.contains("model = \"openai/gpt-oss-20b\""));
        assert!(serialized.contains("opusModel = \"openai/gpt-oss-120b\""));
        // Codex-only fields must not bleed into a Claude Code profile.
        assert!(!serialized.contains("wireApi"));
        assert!(!serialized.contains("providerId"));

        let parsed: AgentBackendProfile = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(parsed, p);
    }

    #[test]
    fn toml_camel_case_round_trip_codex() {
        let mut p = codex_profile("llmlb");
        p.wire_api = Some("responses".into());
        p.http_headers.insert("X-Bearer".into(), "v".into());
        p.query_params
            .insert("api-version".into(), "2024-01".into());
        let serialized = toml::to_string(&p).expect("serialize");
        assert!(serialized.contains("wireApi = \"responses\""));
        assert!(serialized.contains("[httpHeaders]"));
        assert!(serialized.contains("[queryParams]"));
        // Claude Code-only fields must not appear.
        assert!(!serialized.contains("haikuModel"));
        assert!(!serialized.contains("subagentModel"));

        let parsed: AgentBackendProfile = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(parsed, p);
    }

    #[test]
    fn toml_missing_optional_fields_deserialize_to_defaults() {
        let toml_text = r#"
displayName = "Basic"
baseUrl = "http://127.0.0.1:1234"
apiKey = "k"
model = "m"
"#;
        let parsed: AgentBackendProfile = toml::from_str(toml_text).expect("deserialize");
        assert_eq!(parsed.id, "");
        assert_eq!(parsed.display_name, "Basic");
        assert!(parsed.haiku_model.is_none());
        assert!(parsed.wire_api.is_none());
        assert!(parsed.http_headers.is_empty());
        assert!(parsed.query_params.is_empty());
    }
}
