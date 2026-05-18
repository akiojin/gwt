//! WebSocket dispatch helpers for Agent Backend Override (SPEC-1921
//! FR-099 / FR-103). Mirrors [`crate::custom_agents_dispatch`] for the
//! `[builtinAgents.<agent>.backends.<id>]` surface.

use std::path::{Path, PathBuf};

use gwt_agent::{AgentBackendProfile, BuiltinAgentId};
use gwt_config::Settings;

use crate::{
    backend_service::{
        add_agent_backend, delete_agent_backend, list_agent_backends, probe_claude_backend,
        redacted_for_wire, redacted_list_for_wire, update_agent_backend, BackendServiceError,
    },
    protocol::{BackendEvent, CustomAgentErrorCode},
};

fn missing_home_dir_error() -> BackendServiceError {
    BackendServiceError::Storage(
        "unable to resolve home directory (`~/.gwt/config.toml`); \
         set HOME/USERPROFILE before managing agent backends"
            .to_string(),
    )
}

fn config_path() -> Result<PathBuf, BackendServiceError> {
    Settings::global_config_path().ok_or_else(missing_home_dir_error)
}

fn with_config_path<F>(agent: BuiltinAgentId, f: F) -> BackendEvent
where
    F: FnOnce(&Path) -> BackendEvent,
{
    match config_path() {
        Ok(path) => f(&path),
        Err(err) => error_to_event(agent, err),
    }
}

fn error_to_event(agent: BuiltinAgentId, err: BackendServiceError) -> BackendEvent {
    use BackendServiceError as E;
    let code = match &err {
        E::Storage(_) => CustomAgentErrorCode::Storage,
        E::Duplicate(_) => CustomAgentErrorCode::Duplicate,
        E::InvalidInput(_) => CustomAgentErrorCode::InvalidInput,
        E::NotFound(_) => CustomAgentErrorCode::NotFound,
        E::Probe(_) => CustomAgentErrorCode::Probe,
    };
    BackendEvent::AgentBackendError {
        agent,
        code,
        message: err.to_string(),
    }
}

/// Respond to `FrontendEvent::ListAgentBackends`.
pub fn list_event(agent: BuiltinAgentId) -> BackendEvent {
    with_config_path(agent, |path| match list_agent_backends(path, agent) {
        Ok(backends) => BackendEvent::AgentBackendList {
            agent,
            backends: redacted_list_for_wire(backends),
        },
        Err(err) => error_to_event(agent, err),
    })
}

/// Respond to `FrontendEvent::AddAgentBackend`.
pub fn add_event(agent: BuiltinAgentId, profile: AgentBackendProfile) -> BackendEvent {
    with_config_path(agent, |path| {
        match add_agent_backend(path, agent, profile.clone()) {
            Ok(saved) => BackendEvent::AgentBackendSaved {
                agent,
                profile: Box::new(redacted_for_wire(saved)),
            },
            Err(err) => error_to_event(agent, err),
        }
    })
}

/// Respond to `FrontendEvent::UpdateAgentBackend`.
pub fn update_event(
    agent: BuiltinAgentId,
    id: String,
    profile: AgentBackendProfile,
) -> BackendEvent {
    with_config_path(agent, |path| {
        match update_agent_backend(path, agent, &id, profile.clone()) {
            Ok(saved) => BackendEvent::AgentBackendSaved {
                agent,
                profile: Box::new(redacted_for_wire(saved)),
            },
            Err(err) => error_to_event(agent, err),
        }
    })
}

/// Respond to `FrontendEvent::DeleteAgentBackend`.
pub fn delete_event(agent: BuiltinAgentId, id: String) -> BackendEvent {
    with_config_path(agent, |path| match delete_agent_backend(path, agent, &id) {
        Ok(()) => BackendEvent::AgentBackendDeleted { agent, id },
        Err(err) => error_to_event(agent, err),
    })
}

/// Respond to `FrontendEvent::TestAgentBackendConnection`. Currently only
/// the Claude Code probe path is supported; Codex backends reuse the same
/// `/v1/models` shape because LM Studio / llmlb / OpenAI-compat proxies
/// expose it identically (FR-103 investigation 2026-05-18).
pub fn test_connection_event(agent: BuiltinAgentId, base_url: &str, api_key: &str) -> BackendEvent {
    match probe_claude_backend(base_url, api_key) {
        Ok(models) => BackendEvent::BackendConnectionResult { models },
        Err(err) => error_to_event(agent, BackendServiceError::from(err)),
    }
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

    #[test]
    fn error_to_event_preserves_code_per_variant() {
        for (err, expected_code) in [
            (
                BackendServiceError::Storage("x".into()),
                CustomAgentErrorCode::Storage,
            ),
            (
                BackendServiceError::Duplicate("dup".into()),
                CustomAgentErrorCode::Duplicate,
            ),
            (
                BackendServiceError::InvalidInput("bad".into()),
                CustomAgentErrorCode::InvalidInput,
            ),
            (
                BackendServiceError::NotFound("x".into()),
                CustomAgentErrorCode::NotFound,
            ),
        ] {
            let event = error_to_event(BuiltinAgentId::ClaudeCode, err);
            match event {
                BackendEvent::AgentBackendError { code, .. } => assert_eq!(code, expected_code),
                other => panic!("expected AgentBackendError, got {other:?}"),
            }
        }
    }

    #[test]
    fn redacted_for_wire_masks_api_key() {
        let wired = redacted_for_wire(cc_profile("lmstudio"));
        assert_ne!(wired.api_key, "sk-test");
        assert_eq!(wired.id, "lmstudio");
    }
}
