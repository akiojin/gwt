//! Service layer for Custom Agent CRUD operations exposed to the Settings UI.
//!
//! Single library surface that composes:
//!
//! - `gwt-agent::store` for TOML persistence
//! - `gwt-agent::presets` for preset catalog and seed dispatch
//! - `gwt-ai::models_probe::list_model_ids_blocking` for `/v1/models` probe

use std::path::Path;

use gwt_agent::{
    list_presets as agent_list_presets, load_custom_agents_from_path,
    load_stored_custom_agents_from_path, save_stored_custom_agents_to_path, seed_agent,
    CustomCodingAgent, PresetDefinition, PresetError, PresetId, StoredCustomAgent,
};
use gwt_ai::models_probe::{list_model_ids_blocking, ProbeError};
use serde_json::Value;

/// Return the catalog of built-in presets.
pub fn list_presets() -> Vec<PresetDefinition> {
    agent_list_presets()
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

impl From<PresetError> for CustomAgentsServiceError {
    fn from(value: PresetError) -> Self {
        Self::InvalidInput(value.to_string())
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

/// Persist a new custom agent seeded from the selected preset. Fails if the id
/// already exists or the preset payload fails validation.
/// Does NOT re-run the `/v1/models` probe; callers are expected to call
/// [`probe_backend`] first and only invoke this function once the Save
/// button's `last_probe_ok` gate is true (SPEC-1921 FR-061).
pub fn add_from_preset(
    config_path: &Path,
    preset_id: PresetId,
    payload: &Value,
) -> Result<CustomCodingAgent, CustomAgentsServiceError> {
    let agent = seed_agent(preset_id, payload)?;

    let mut entries = load_stored_custom_agents_from_path(config_path)?;
    if entries.iter().any(|entry| entry.agent.id == agent.id) {
        return Err(CustomAgentsServiceError::Duplicate(agent.id));
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

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_agent::ClaudeCodeOpenaiCompatInput;

    fn sample_input() -> ClaudeCodeOpenaiCompatInput {
        ClaudeCodeOpenaiCompatInput {
            id: "claude-code-openai".to_string(),
            display_name: "Claude Code (OpenAI-compat)".to_string(),
            base_url: "http://192.168.100.166:32768".to_string(),
            api_key: "sk_cwPkycrPTZBYQ8vFXsc3O0wkrvt36VSh".to_string(),
            default_model: "openai/gpt-oss-20b".to_string(),
        }
    }

    fn sample_payload(input: &ClaudeCodeOpenaiCompatInput) -> Value {
        serde_json::to_value(input).unwrap()
    }

    fn add_sample_from_preset(
        path: &Path,
        input: &ClaudeCodeOpenaiCompatInput,
    ) -> Result<CustomCodingAgent, CustomAgentsServiceError> {
        add_from_preset(
            path,
            PresetId::ClaudeCodeOpenaiCompat,
            &sample_payload(input),
        )
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

        let agent = add_sample_from_preset(&path, &input).expect("save");

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
    fn generic_add_from_preset_creates_entry_and_persists_all_env() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();
        let payload = sample_payload(&input);

        let agent =
            add_from_preset(&path, PresetId::ClaudeCodeOpenaiCompat, &payload).expect("save");

        assert_eq!(agent.id, input.id);
        assert_eq!(agent.env.len(), 13);
        assert_eq!(agent.env["ANTHROPIC_API_KEY"], input.api_key);
        assert_eq!(agent.env["ANTHROPIC_BASE_URL"], input.base_url);

        let reloaded = list_custom_agents(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(
            reloaded[0].env["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            input.default_model
        );
    }

    #[test]
    fn generic_add_from_preset_rejects_malformed_payload_as_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let payload = serde_json::json!({
            "id": "claude-code-openai"
        });

        let err = add_from_preset(&path, PresetId::ClaudeCodeOpenaiCompat, &payload).unwrap_err();

        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
        assert!(list_custom_agents(&path).unwrap().is_empty());
    }

    #[test]
    fn add_from_preset_rejects_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();

        add_sample_from_preset(&path, &input).expect("first save");
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::Duplicate(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.id = String::new();
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_id_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.id = "has spaces".to_string();
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_non_http_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.base_url = "ws://example.com".to_string();
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.api_key = String::new();
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_from_preset_rejects_empty_default_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut input = sample_input();
        input.default_model = String::new();
        let err = add_sample_from_preset(&path, &input).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::InvalidInput(_)));
    }

    #[test]
    fn update_custom_agent_modifies_existing_entry_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();
        let mut agent = add_sample_from_preset(&path, &input).unwrap();

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
        let mut agent =
            gwt_agent::claude_code_openai_compat_preset("missing", "X", "http://a", "k", "m");
        agent.id = "missing".to_string();
        let err = update_custom_agent(&path, agent).unwrap_err();
        assert!(matches!(err, CustomAgentsServiceError::NotFound(_)));
    }

    #[test]
    fn delete_custom_agent_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let input = sample_input();
        add_sample_from_preset(&path, &input).unwrap();

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
