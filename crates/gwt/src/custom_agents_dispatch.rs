//! WebSocket dispatch helpers for Custom Agent Settings requests.
//!
//! Extracted from `main.rs` so the 6 request variants have a single owner
//! instead of being inlined alongside the rest of the frontend event router.
//! Each helper takes strongly-typed request data and returns a
//! [`BackendEvent`] reply that the caller wraps in a client-targeted
//! `OutboundEvent`.
//!
//! Error mapping: every `CustomAgentsServiceError` variant maps to a stable
//! `code` string in [`BackendEvent::CustomAgentError`] so the frontend can
//! branch on failure type without string matching on messages.

use std::path::PathBuf;

use gwt_agent::CustomCodingAgent;
use gwt_config::Settings;

use crate::{
    custom_agents_service::{
        add_from_claude_code_openai_compat_preset, delete_custom_agent, list_custom_agents,
        list_presets, probe_backend, update_custom_agent, ClaudeCodeOpenaiCompatInput,
        CustomAgentsServiceError,
    },
    protocol::BackendEvent,
};

/// Resolve the custom-agent config file path. Returns `Storage` error when
/// the home directory cannot be discovered — silently falling back to
/// `./config.toml` would write `api_key` secrets into the current working
/// directory, which diverges from the app's canonical `~/.gwt/config.toml`
/// source of truth.
pub fn config_path() -> Result<PathBuf, CustomAgentsServiceError> {
    Settings::global_config_path().ok_or_else(|| {
        CustomAgentsServiceError::Storage(
            "unable to resolve home directory (`~/.gwt/config.toml`); \
             set HOME/USERPROFILE before managing custom agents"
                .to_string(),
        )
    })
}

/// Map a service-layer error to the `CustomAgentError` backend event with
/// a stable `code` string.
pub fn error_to_event(err: CustomAgentsServiceError) -> BackendEvent {
    use CustomAgentsServiceError as E;
    let code = match &err {
        E::Storage(_) => "storage",
        E::Duplicate(_) => "duplicate",
        E::InvalidInput(_) => "invalid_input",
        E::NotFound(_) => "not_found",
        E::Probe(_) => "probe",
    };
    BackendEvent::CustomAgentError {
        code: code.to_string(),
        message: err.to_string(),
    }
}

/// Respond to `FrontendEvent::ListCustomAgents`.
pub fn list_event() -> BackendEvent {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => return error_to_event(err),
    };
    match list_custom_agents(&path) {
        Ok(agents) => BackendEvent::CustomAgentList { agents },
        Err(err) => error_to_event(err),
    }
}

/// Respond to `FrontendEvent::ListCustomAgentPresets`.
pub fn list_presets_event() -> BackendEvent {
    BackendEvent::CustomAgentPresetList {
        presets: list_presets(),
    }
}

/// Respond to `FrontendEvent::AddCustomAgentFromPreset`.
pub fn add_from_preset_event(input: ClaudeCodeOpenaiCompatInput) -> BackendEvent {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => return error_to_event(err),
    };
    match add_from_claude_code_openai_compat_preset(&path, &input) {
        Ok(agent) => BackendEvent::CustomAgentSaved {
            agent: Box::new(agent),
        },
        Err(err) => error_to_event(err),
    }
}

/// Respond to `FrontendEvent::UpdateCustomAgent`.
pub fn update_event(agent: CustomCodingAgent) -> BackendEvent {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => return error_to_event(err),
    };
    let saved = agent.clone();
    match update_custom_agent(&path, agent) {
        Ok(()) => BackendEvent::CustomAgentSaved {
            agent: Box::new(saved),
        },
        Err(err) => error_to_event(err),
    }
}

/// Respond to `FrontendEvent::DeleteCustomAgent`.
pub fn delete_event(agent_id: String) -> BackendEvent {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => return error_to_event(err),
    };
    match delete_custom_agent(&path, &agent_id) {
        Ok(()) => BackendEvent::CustomAgentDeleted { agent_id },
        Err(err) => error_to_event(err),
    }
}

/// Respond to `FrontendEvent::TestBackendConnection`.
pub fn test_connection_event(base_url: &str, api_key: &str) -> BackendEvent {
    match probe_backend(base_url, api_key) {
        Ok(models) => BackendEvent::BackendConnectionResult { models },
        Err(err) => BackendEvent::CustomAgentError {
            code: "probe".to_string(),
            message: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_to_event_preserves_code_per_variant() {
        let cases = [
            (CustomAgentsServiceError::Storage("x".into()), "storage"),
            (CustomAgentsServiceError::Duplicate("x".into()), "duplicate"),
            (
                CustomAgentsServiceError::InvalidInput("x".into()),
                "invalid_input",
            ),
            (CustomAgentsServiceError::NotFound("x".into()), "not_found"),
        ];
        for (err, expected_code) in cases {
            match error_to_event(err) {
                BackendEvent::CustomAgentError { code, .. } => assert_eq!(code, expected_code),
                other => panic!("expected CustomAgentError, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_connection_event_invalid_scheme_returns_probe_error_code() {
        match test_connection_event("ws://example.com", "k") {
            BackendEvent::CustomAgentError { code, .. } => assert_eq!(code, "probe"),
            other => panic!("expected CustomAgentError, got {other:?}"),
        }
    }

    #[test]
    fn list_presets_event_returns_preset_list_variant() {
        match list_presets_event() {
            BackendEvent::CustomAgentPresetList { presets } => {
                assert!(!presets.is_empty());
            }
            other => panic!("expected CustomAgentPresetList, got {other:?}"),
        }
    }
}
