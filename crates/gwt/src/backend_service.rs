//! Service layer for Agent Backend Override CRUD (SPEC-1921 FR-099 / FR-103).
//!
//! Mirrors [`crate::custom_agents_service`] for Backend Override profiles:
//!
//! - [`gwt_agent::backend_store`] for TOML persistence
//! - `gwt_ai::models_probe::list_model_ids_blocking` for the Claude Code
//!   `/v1/models` save-time probe (FR-061 carryover)
//! - on-wire profiles always go through [`redacted_for_wire`] so the api
//!   key never leaves the process unredacted (SPEC-1921 audit invariant).

use std::path::Path;

use gwt_agent::{backend_store, AgentBackendProfile, BuiltinAgentId, REDACTED_PLACEHOLDER};
use gwt_ai::models_probe::{list_model_ids_blocking, ProbeError};

/// Structured error variant exposed to the Settings UI.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackendServiceError {
    /// The supplied config path could not be read / written / parsed.
    #[error("storage error: {0}")]
    Storage(String),
    /// A backend with the requested id already exists for the given agent.
    #[error("a backend with id `{0}` already exists")]
    Duplicate(String),
    /// The payload failed validation (empty / non-matching id, invalid url, …).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// No matching backend was found for the given id.
    #[error("backend `{0}` not found")]
    NotFound(String),
    /// `/v1/models` probe failure, forwarded verbatim.
    #[error("probe error: {0}")]
    Probe(#[from] ProbeError),
}

impl From<String> for BackendServiceError {
    fn from(value: String) -> Self {
        Self::Storage(value)
    }
}

/// List every backend profile currently stored for the given built-in agent.
pub fn list_agent_backends(
    config_path: &Path,
    agent: BuiltinAgentId,
) -> Result<Vec<AgentBackendProfile>, BackendServiceError> {
    Ok(backend_store::load_backends_for_agent(config_path, agent)?)
}

/// SPEC-1921 FR-061 carry-over for Claude Code backends: probe the
/// configured `/v1/models` endpoint and return the discovered model IDs.
pub fn probe_claude_backend(base_url: &str, api_key: &str) -> Result<Vec<String>, ProbeError> {
    list_model_ids_blocking(base_url, api_key)
}

/// Persist a new Backend Override profile. Fails if the id already exists
/// for the given agent or the profile fails validation. Returns the
/// persisted profile so callers do not need a pre-save clone to echo back.
pub fn add_agent_backend(
    config_path: &Path,
    agent: BuiltinAgentId,
    profile: AgentBackendProfile,
) -> Result<AgentBackendProfile, BackendServiceError> {
    profile
        .validate(agent)
        .map_err(|err| BackendServiceError::InvalidInput(err.to_string()))?;

    let mut profiles = backend_store::load_backends_for_agent(config_path, agent)?;
    if profiles.iter().any(|p| p.id == profile.id) {
        return Err(BackendServiceError::Duplicate(profile.id));
    }
    profiles.push(profile.clone());
    backend_store::save_backends_for_agent(config_path, agent, &profiles)?;
    Ok(profile)
}

/// Update an existing Backend Override profile in place. The profile id
/// must match an existing entry; returns `NotFound` otherwise. Preserves
/// the previously persisted `api_key` when the caller passes
/// [`REDACTED_PLACEHOLDER`] (matching the Settings UI's redact-on-wire
/// contract).
pub fn update_agent_backend(
    config_path: &Path,
    agent: BuiltinAgentId,
    id: &str,
    mut patch: AgentBackendProfile,
) -> Result<AgentBackendProfile, BackendServiceError> {
    if patch.id != id {
        return Err(BackendServiceError::InvalidInput(format!(
            "patch id `{}` does not match target id `{}`",
            patch.id, id
        )));
    }
    let mut profiles = backend_store::load_backends_for_agent(config_path, agent)?;
    let Some(entry_idx) = profiles.iter().position(|p| p.id == id) else {
        return Err(BackendServiceError::NotFound(id.to_string()));
    };

    // Preserve the existing api_key when the caller sends the redaction
    // placeholder. Mirrors the secret-env handling in custom_agents_service.
    if patch.api_key == REDACTED_PLACEHOLDER {
        patch.api_key = profiles[entry_idx].api_key.clone();
    }

    patch
        .validate(agent)
        .map_err(|err| BackendServiceError::InvalidInput(err.to_string()))?;

    profiles[entry_idx] = patch.clone();
    backend_store::save_backends_for_agent(config_path, agent, &profiles)?;
    Ok(patch)
}

/// Remove the backend with the given id from the given agent's section.
/// Returns `NotFound` when no matching entry exists.
pub fn delete_agent_backend(
    config_path: &Path,
    agent: BuiltinAgentId,
    id: &str,
) -> Result<(), BackendServiceError> {
    let mut profiles = backend_store::load_backends_for_agent(config_path, agent)?;
    let original = profiles.len();
    profiles.retain(|p| p.id != id);
    if profiles.len() == original {
        return Err(BackendServiceError::NotFound(id.to_string()));
    }
    backend_store::save_backends_for_agent(config_path, agent, &profiles)?;
    Ok(())
}

/// Replace the `api_key` with [`REDACTED_PLACEHOLDER`] so the wire payload
/// never carries the plaintext secret.
pub fn redacted_for_wire(mut profile: AgentBackendProfile) -> AgentBackendProfile {
    if !profile.api_key.is_empty() {
        profile.api_key = REDACTED_PLACEHOLDER.to_string();
    }
    profile
}

/// Convenience for list views: mask every backend's api_key.
pub fn redacted_list_for_wire(profiles: Vec<AgentBackendProfile>) -> Vec<AgentBackendProfile> {
    profiles.into_iter().map(redacted_for_wire).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc_profile(id: &str) -> AgentBackendProfile {
        AgentBackendProfile {
            id: id.into(),
            display_name: "LM Studio".into(),
            base_url: "http://127.0.0.1:1234".into(),
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
    fn list_returns_empty_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let claude = list_agent_backends(&path, BuiltinAgentId::ClaudeCode).unwrap();
        let codex = list_agent_backends(&path, BuiltinAgentId::Codex).unwrap();
        assert!(claude.is_empty());
        assert!(codex.is_empty());
    }

    #[test]
    fn add_then_list_round_trips_claude_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let saved = add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio"))
            .expect("save");
        assert_eq!(saved.id, "lmstudio");
        assert_eq!(saved.api_key, "sk-test");

        let listed = list_agent_backends(&path, BuiltinAgentId::ClaudeCode).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "lmstudio");
        // Codex section is untouched.
        let codex = list_agent_backends(&path, BuiltinAgentId::Codex).expect("list codex");
        assert!(codex.is_empty());
    }

    #[test]
    fn add_rejects_invalid_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut bad = cc_profile("BAD");
        bad.id = "BAD".into(); // uppercase rejected
        let err = add_agent_backend(&path, BuiltinAgentId::ClaudeCode, bad).unwrap_err();
        assert!(matches!(err, BackendServiceError::InvalidInput(_)));
    }

    #[test]
    fn add_rejects_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("dup")).unwrap();
        let err =
            add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("dup")).unwrap_err();
        assert!(matches!(err, BackendServiceError::Duplicate(_)));
    }

    #[test]
    fn update_preserves_api_key_when_caller_sends_redacted_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).unwrap();

        let mut patch = cc_profile("lmstudio");
        patch.api_key = REDACTED_PLACEHOLDER.to_string();
        patch.display_name = "Updated".into();
        let saved = update_agent_backend(&path, BuiltinAgentId::ClaudeCode, "lmstudio", patch)
            .expect("update");
        assert_eq!(
            saved.api_key, "sk-test",
            "redacted placeholder must restore the original key"
        );
        assert_eq!(saved.display_name, "Updated");

        let listed = list_agent_backends(&path, BuiltinAgentId::ClaudeCode).expect("list");
        assert_eq!(listed[0].api_key, "sk-test");
        assert_eq!(listed[0].display_name, "Updated");
    }

    #[test]
    fn update_rejects_id_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).unwrap();
        let err = update_agent_backend(
            &path,
            BuiltinAgentId::ClaudeCode,
            "lmstudio",
            cc_profile("renamed"),
        )
        .unwrap_err();
        assert!(matches!(err, BackendServiceError::InvalidInput(_)));
    }

    #[test]
    fn update_rejects_missing_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let err = update_agent_backend(
            &path,
            BuiltinAgentId::ClaudeCode,
            "ghost",
            cc_profile("ghost"),
        )
        .unwrap_err();
        assert!(matches!(err, BackendServiceError::NotFound(_)));
    }

    #[test]
    fn delete_returns_not_found_for_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let err = delete_agent_backend(&path, BuiltinAgentId::ClaudeCode, "ghost").unwrap_err();
        assert!(matches!(err, BackendServiceError::NotFound(_)));
    }

    #[test]
    fn delete_removes_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        add_agent_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).unwrap();
        delete_agent_backend(&path, BuiltinAgentId::ClaudeCode, "lmstudio").unwrap();
        let listed = list_agent_backends(&path, BuiltinAgentId::ClaudeCode).expect("list");
        assert!(listed.is_empty());
    }

    #[test]
    fn redacted_for_wire_masks_api_key_only() {
        let profile = cc_profile("lmstudio");
        let redacted = redacted_for_wire(profile.clone());
        assert_eq!(redacted.api_key, REDACTED_PLACEHOLDER);
        // Other fields untouched.
        assert_eq!(redacted.id, profile.id);
        assert_eq!(redacted.display_name, profile.display_name);
        assert_eq!(redacted.base_url, profile.base_url);
        assert_eq!(redacted.model, profile.model);
    }

    #[test]
    fn redacted_for_wire_leaves_empty_api_key_alone() {
        let mut profile = cc_profile("lmstudio");
        profile.api_key = String::new();
        let redacted = redacted_for_wire(profile);
        // No artificial placeholder for an explicitly empty key.
        assert_eq!(redacted.api_key, "");
    }

    #[test]
    fn codex_backend_round_trips_with_extension_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut profile = codex_profile("llmlb");
        profile.wire_api = Some("responses".into());
        profile.provider_id = Some("custom-provider".into());

        add_agent_backend(&path, BuiltinAgentId::Codex, profile.clone()).unwrap();
        let listed = list_agent_backends(&path, BuiltinAgentId::Codex).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].wire_api.as_deref(), Some("responses"));
        assert_eq!(listed[0].provider_id.as_deref(), Some("custom-provider"));
    }
}
