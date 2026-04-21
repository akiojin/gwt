//! Service layer for Custom Agent CRUD operations exposed to the Settings UI.
//!
//! Single library surface that composes:
//!
//! - `gwt-agent::store` for TOML persistence
//! - `gwt-agent::presets::claude_code_openai_compat_preset` for preset seeding
//! - `gwt-ai::models_probe::list_model_ids_blocking` for `/v1/models` probe

use std::path::Path;

use gwt_agent::{
    claude_code_openai_compat_preset, load_custom_agents_from_path,
    load_stored_custom_agents_from_path, save_stored_custom_agents_to_path, CustomCodingAgent,
    StoredCustomAgent,
};
use gwt_ai::models_probe::{is_valid_base_url, list_model_ids_blocking, ProbeError};
use serde::{Deserialize, Serialize};

/// Stable identifier for a built-in preset. Keep this set small — every new
/// id is a frontend-visible contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetId {
    /// Claude Code routed through an Anthropic Messages API compatible proxy
    /// that speaks `/v1/models`. SPEC-1921 FR-062.
    ClaudeCodeOpenaiCompat,
}

/// Metadata that the Settings UI shows in the "Add from preset" picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetDefinition {
    /// Stable id used by the `AddFromPreset` request.
    pub id: PresetId,
    /// Display label rendered in the picker.
    pub label: &'static str,
    /// Short description rendered below the label in the picker.
    pub description: &'static str,
}

impl PresetDefinition {
    fn catalog() -> [PresetDefinition; 1] {
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

/// Return the catalog of built-in presets.
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

/// Structured error variant exposed to the Settings UI.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CustomAgentsServiceError {
    /// The supplied config path could not be read / written / parsed.
    #[error("storage error: {0}")]
    Storage(String),
    /// A custom agent with the requested id already exists.
    #[error("a custom agent with id `{0}` already exists")]
    Duplicate(String),
    /// The payload failed validation (empty / non-matching id, invalid url, …).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// No matching custom agent was found for the given id.
    #[error("custom agent `{0}` not found")]
    NotFound(String),
    /// `/v1/models` probe failure, forwarded verbatim.
    #[error("probe error: {0}")]
    Probe(#[from] ProbeError),
}

impl From<String> for CustomAgentsServiceError {
    fn from(value: String) -> Self {
        Self::Storage(value)
    }
}

/// List every custom agent currently stored in the given config file.
pub fn list_custom_agents(
    config_path: &Path,
) -> Result<Vec<CustomCodingAgent>, CustomAgentsServiceError> {
    Ok(load_custom_agents_from_path(config_path)?)
}

/// Probe an OpenAI-compatible `/v1/models` endpoint. Returns the discovered
/// model IDs verbatim, or a [`ProbeError`] wrapped in the service error enum.
/// SPEC-1921 FR-061.
pub fn probe_backend(base_url: &str, api_key: &str) -> Result<Vec<String>, ProbeError> {
    list_model_ids_blocking(base_url, api_key)
}

/// Persist a new custom agent seeded from the Claude Code (OpenAI-compat
/// backend) preset. Fails if the id already exists or fails validation.
/// Does NOT re-run the `/v1/models` probe; callers are expected to call
/// [`probe_backend`] first and only invoke this function once the Save
/// button's `last_probe_ok` gate is true (SPEC-1921 FR-061).
pub fn add_from_claude_code_openai_compat_preset(
    config_path: &Path,
    input: &ClaudeCodeOpenaiCompatInput,
) -> Result<CustomCodingAgent, CustomAgentsServiceError> {
    validate_preset_input(input)?;

    let mut entries = load_stored_custom_agents_from_path(config_path)?;
    if entries.iter().any(|entry| entry.agent.id == input.id) {
        return Err(CustomAgentsServiceError::Duplicate(input.id.clone()));
    }

    let agent = claude_code_openai_compat_preset(
        input.id.clone(),
        input.display_name.clone(),
        input.base_url.clone(),
        input.api_key.clone(),
        input.default_model.clone(),
    );
    if !agent.validate() {
        return Err(CustomAgentsServiceError::InvalidInput(format!(
            "preset produced an invalid agent id: {}",
            input.id
        )));
    }

    entries.push(StoredCustomAgent::new(agent.clone()));
    save_stored_custom_agents_to_path(config_path, &entries)?;
    Ok(agent)
}

/// Update an existing custom agent in place. The agent id must match an
/// existing entry; returns `NotFound` otherwise. Preserves any sibling
/// TOML tables (e.g. `models`) via the stored `raw` table. Returns the
/// persisted agent so callers do not need a pre-save clone to echo back.
pub fn update_custom_agent(
    config_path: &Path,
    updated: CustomCodingAgent,
) -> Result<CustomCodingAgent, CustomAgentsServiceError> {
    if !updated.validate() {
        return Err(CustomAgentsServiceError::InvalidInput(format!(
            "invalid agent id or fields: {}",
            updated.id
        )));
    }
    let mut entries = load_stored_custom_agents_from_path(config_path)?;
    let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.agent.id == updated.id)
    else {
        return Err(CustomAgentsServiceError::NotFound(updated.id));
    };
    entry.agent = updated;
    let saved = entry.agent.clone();
    save_stored_custom_agents_to_path(config_path, &entries)?;
    Ok(saved)
}

/// Remove the custom agent with the given id. Returns `NotFound` if no
/// matching entry exists.
pub fn delete_custom_agent(
    config_path: &Path,
    agent_id: &str,
) -> Result<(), CustomAgentsServiceError> {
    let mut entries = load_stored_custom_agents_from_path(config_path)?;
    let original_len = entries.len();
    entries.retain(|entry| entry.agent.id != agent_id);
    if entries.len() == original_len {
        return Err(CustomAgentsServiceError::NotFound(agent_id.to_string()));
    }
    save_stored_custom_agents_to_path(config_path, &entries)?;
    Ok(())
}

fn require_non_empty(field: &str, value: &str) -> Result<(), CustomAgentsServiceError> {
    if value.trim().is_empty() {
        Err(CustomAgentsServiceError::InvalidInput(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_preset_input(
    input: &ClaudeCodeOpenaiCompatInput,
) -> Result<(), CustomAgentsServiceError> {
    require_non_empty("id", &input.id)?;
    if !input.id.chars().all(|c| c.is_alphanumeric() || c == '-') {
        return Err(CustomAgentsServiceError::InvalidInput(format!(
            "id `{}` contains invalid characters (allowed: alphanumeric, `-`)",
            input.id
        )));
    }
    require_non_empty("display_name", &input.display_name)?;
    if !is_valid_base_url(&input.base_url) {
        return Err(CustomAgentsServiceError::InvalidInput(format!(
            "base_url must start with http:// or https://, got: {}",
            input.base_url
        )));
    }
    require_non_empty("api_key", &input.api_key)?;
    require_non_empty("default_model", &input.default_model)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> ClaudeCodeOpenaiCompatInput {
        ClaudeCodeOpenaiCompatInput {
            id: "claude-code-openai".to_string(),
            display_name: "Claude Code (OpenAI-compat)".to_string(),
            base_url: "http://192.168.100.166:32768".to_string(),
            api_key: "sk_cwPkycrPTZBYQ8vFXsc3O0wkrvt36VSh".to_string(),
            default_model: "openai/gpt-oss-20b".to_string(),
        }
    }

    #[test]
    fn list_presets_includes_claude_code_openai_compat() {
        let presets = list_presets();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].id, PresetId::ClaudeCodeOpenaiCompat);
        assert!(presets[0].label.contains("OpenAI-compat"));
    }

    #[test]
    fn add_from_preset_creates_entry_and_persists_all_env() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();

        let agent = add_from_claude_code_openai_compat_preset(&path, &input).expect("save");

        assert_eq!(agent.id, input.id);
        assert_eq!(agent.env.len(), 13);
        assert_eq!(agent.env["ANTHROPIC_API_KEY"], input.api_key);
        assert_eq!(agent.env["ANTHROPIC_BASE_URL"], input.base_url);

        let reloaded = list_custom_agents(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].env.len(), 13);
        assert_eq!(
            reloaded[0].env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            input.default_model
        );
    }

    #[test]
    fn add_from_preset_rejects_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();

        add_from_claude_code_openai_compat_preset(&path, &input).expect("first save");
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::Duplicate(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.id = String::new();
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_id_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.id = "has spaces".to_string();
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_non_http_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.base_url = "ws://example.com".to_string();
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.api_key = String::new();
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_default_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.default_model = String::new();
        let err = add_from_claude_code_openai_compat_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn update_custom_agent_modifies_existing_entry_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();
        let mut agent = add_from_claude_code_openai_compat_preset(&path, &input).unwrap();

        agent.display_name = "Renamed Claude".to_string();
        agent
            .env
            .insert("CUSTOM_EXTRA".to_string(), "value".to_string());
        update_custom_agent(&path, agent).expect("update");

        let reloaded = list_custom_agents(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].display_name, "Renamed Claude");
        assert_eq!(
            reloaded[0].env.get("CUSTOM_EXTRA").map(String::as_str),
            Some("value")
        );
    }

    #[test]
    fn update_custom_agent_returns_not_found_for_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut agent = claude_code_openai_compat_preset("missing", "X", "http://a", "k", "m");
        agent.id = "missing".to_string();
        let err = update_custom_agent(&path, agent).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::NotFound(_)));
    }

    #[test]
    fn delete_custom_agent_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();
        add_from_claude_code_openai_compat_preset(&path, &input).unwrap();

        delete_custom_agent(&path, &input.id).expect("delete");
        let reloaded = list_custom_agents(&path).unwrap();
        assert!(reloaded.is_empty());
    }

    #[test]
    fn delete_custom_agent_returns_not_found_for_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let err = delete_custom_agent(&path, "ghost").unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::NotFound(_)));
    }

    #[test]
    fn list_custom_agents_returns_empty_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let agents = list_custom_agents(&path).expect("list");
        assert!(agents.is_empty());
    }

    #[test]
    fn probe_backend_rejects_non_http_scheme() {
        let err = probe_backend("ws://example.com", "k").unwrap_err();
        assert!(matches!(err, ProbeError::InvalidUrl(_)));
    }

    #[test]
    fn preset_id_serializes_as_snake_case() {
        let json = serde_json::to_string(&PresetId::ClaudeCodeOpenaiCompat).unwrap();
        assert_eq!(json, "\"claude_code_openai_compat\"");
    }
}
