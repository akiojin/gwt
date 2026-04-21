//! WebSocket dispatch helpers for Custom Agent Settings requests.
//!
//! Error mapping: every [`CustomAgentsServiceError`] variant maps to a stable
//! [`CustomAgentErrorCode`] in [`BackendEvent::CustomAgentError`] so the
//! frontend can branch on failure type via the enum-serialized `code` field
//! instead of string matching on human-readable messages.

use std::path::{Path, PathBuf};

use gwt_agent::{redact_secrets_in_agent, CustomCodingAgent, PresetId};
use gwt_config::Settings;
use serde_json::Value;

use crate::{
    custom_agents_service::{
        add_from_preset, delete_custom_agent, list_custom_agents, list_presets, probe_backend,
        update_custom_agent, CustomAgentsServiceError,
    },
    protocol::{BackendEvent, CustomAgentErrorCode},
};

#[cfg(test)]
thread_local! {
    // Tests exercise dispatch helpers synchronously; spawned probe paths do
    // not read this override.
    static TEST_HOME_DIR_OVERRIDE: std::cell::RefCell<Option<Option<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

fn missing_home_dir_error() -> CustomAgentsServiceError {
    CustomAgentsServiceError::Storage(
        "unable to resolve home directory (`~/.gwt/config.toml`); \
         set HOME/USERPROFILE before managing custom agents"
            .to_string(),
    )
}

#[cfg(test)]
fn config_path_from_home(home: Option<PathBuf>) -> Result<PathBuf, CustomAgentsServiceError> {
    home.map(|home| Settings::global_config_path_for_home(&home))
        .ok_or_else(missing_home_dir_error)
}

#[cfg(test)]
fn test_home_dir_override() -> Option<Option<PathBuf>> {
    TEST_HOME_DIR_OVERRIDE.with(|override_home| override_home.borrow().clone())
}

#[cfg(test)]
fn set_test_home_dir_override(home: Option<Option<PathBuf>>) {
    TEST_HOME_DIR_OVERRIDE.with(|override_home| {
        *override_home.borrow_mut() = home;
    });
}

/// Resolve the custom-agent config file path. Returns `Storage` error when
/// the home directory cannot be discovered — silently falling back to
/// `./config.toml` would write `api_key` secrets into the current working
/// directory, which diverges from the app's canonical `~/.gwt/config.toml`
/// source of truth.
fn config_path() -> Result<PathBuf, CustomAgentsServiceError> {
    #[cfg(test)]
    if let Some(home) = test_home_dir_override() {
        return config_path_from_home(home);
    }

    Settings::global_config_path().ok_or_else(missing_home_dir_error)
}

/// Resolve the config path or produce a `CustomAgentError` event — eliminates
/// the four-way duplicate match at every dispatch helper entry.
fn with_config_path<F>(f: F) -> BackendEvent
where
    F: FnOnce(&Path) -> BackendEvent,
{
    match config_path() {
        Ok(path) => f(&path),
        Err(err) => error_to_event(err),
    }
}

/// Map a service-layer error to a `CustomAgentError` event.
fn error_to_event(err: CustomAgentsServiceError) -> BackendEvent {
    use CustomAgentsServiceError as E;
    let code = match &err {
        E::Storage(_) => CustomAgentErrorCode::Storage,
        E::Duplicate(_) => CustomAgentErrorCode::Duplicate,
        E::InvalidInput(_) => CustomAgentErrorCode::InvalidInput,
        E::NotFound(_) => CustomAgentErrorCode::NotFound,
        E::Probe(_) => CustomAgentErrorCode::Probe,
    };
    BackendEvent::CustomAgentError {
        code,
        message: err.to_string(),
    }
}

/// Mask secret env values on a copy of the agent so the clone is safe to
/// ship across the WebSocket (the original retains secrets for launch).
fn redacted_for_wire(agent: CustomCodingAgent) -> CustomCodingAgent {
    let mut wire = agent;
    redact_secrets_in_agent(&mut wire);
    wire
}

/// Respond to `FrontendEvent::ListCustomAgents`.
pub fn list_event() -> BackendEvent {
    with_config_path(|path| match list_custom_agents(path) {
        Ok(agents) => BackendEvent::CustomAgentList {
            agents: agents.into_iter().map(redacted_for_wire).collect(),
        },
        Err(err) => error_to_event(err),
    })
}

/// Respond to `FrontendEvent::ListCustomAgentPresets`.
pub fn list_presets_event() -> BackendEvent {
    BackendEvent::CustomAgentPresetList {
        presets: list_presets(),
    }
}

/// Respond to `FrontendEvent::AddCustomAgentFromPreset`.
pub fn add_from_preset_event(preset_id: PresetId, payload: Value) -> BackendEvent {
    with_config_path(|path| match add_from_preset(path, preset_id, &payload) {
        Ok(agent) => BackendEvent::CustomAgentSaved {
            agent: Box::new(redacted_for_wire(agent)),
        },
        Err(err) => error_to_event(err),
    })
}

/// Respond to `FrontendEvent::UpdateCustomAgent`.
pub fn update_event(agent: CustomCodingAgent) -> BackendEvent {
    with_config_path(|path| match update_custom_agent(path, agent.clone()) {
        Ok(saved) => BackendEvent::CustomAgentSaved {
            agent: Box::new(redacted_for_wire(saved)),
        },
        Err(err) => error_to_event(err),
    })
}

/// Respond to `FrontendEvent::DeleteCustomAgent`.
pub fn delete_event(agent_id: String) -> BackendEvent {
    with_config_path(|path| match delete_custom_agent(path, &agent_id) {
        Ok(()) => BackendEvent::CustomAgentDeleted { agent_id },
        Err(err) => error_to_event(err),
    })
}

/// Respond to `FrontendEvent::TestBackendConnection`. Does not require a
/// config path (the probe is pure network), so bypasses `with_config_path`.
pub fn test_connection_event(base_url: &str, api_key: &str) -> BackendEvent {
    match probe_backend(base_url, api_key) {
        Ok(models) => BackendEvent::BackendConnectionResult { models },
        Err(err) => error_to_event(CustomAgentsServiceError::from(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_input() -> ClaudeCodeOpenaiCompatInput {
        ClaudeCodeOpenaiCompatInput {
            id: "claude-proxy".to_string(),
            display_name: "Claude Proxy".to_string(),
            base_url: "http://proxy.local:32768".to_string(),
            api_key: "sk-real-secret".to_string(),
            default_model: "openai/gpt-oss-20b".to_string(),
        }
    }

    fn sample_agent(id: &str) -> CustomCodingAgent {
        gwt_agent::claude_code_openai_compat_preset(
            id,
            "Claude Proxy",
            "http://proxy.local:32768",
            "sk-real-secret",
            "openai/gpt-oss-20b",
        )
    }

    fn home_config_path(home: PathBuf) -> PathBuf {
        home.join(".gwt").join("config.toml")
    }

    struct HomeDirOverrideGuard;

    impl Drop for HomeDirOverrideGuard {
        fn drop(&mut self) {
            set_test_home_dir_override(None);
        }
    }

    fn override_home_dir(home: Option<PathBuf>) -> HomeDirOverrideGuard {
        set_test_home_dir_override(Some(home));
        HomeDirOverrideGuard
    }

    fn assert_storage_error(event: BackendEvent) {
        match event {
            BackendEvent::CustomAgentError { code, message } => {
                assert_eq!(code, CustomAgentErrorCode::Storage);
                assert!(
                    message.contains("unable to resolve home directory"),
                    "unexpected storage error message: {message}"
                );
            }
            other => panic!("expected CustomAgentError, got {other:?}"),
        }
    }

    #[test]
    fn error_to_event_preserves_code_per_variant() {
        let cases = [
            (
                CustomAgentsServiceError::Storage("x".into()),
                CustomAgentErrorCode::Storage,
            ),
            (
                CustomAgentsServiceError::Duplicate("x".into()),
                CustomAgentErrorCode::Duplicate,
            ),
            (
                CustomAgentsServiceError::InvalidInput("x".into()),
                CustomAgentErrorCode::InvalidInput,
            ),
            (
                CustomAgentsServiceError::NotFound("x".into()),
                CustomAgentErrorCode::NotFound,
            ),
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
            BackendEvent::CustomAgentError { code, .. } => {
                assert_eq!(code, CustomAgentErrorCode::Probe);
            }
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

    #[test]
    fn redacted_for_wire_masks_secret_env_entries() {
        use gwt_agent::REDACTED_PLACEHOLDER;
        let preset = gwt_agent::claude_code_openai_compat_preset(
            "preset-id",
            "Preset",
            "http://proxy.local:32768",
            "sk-real-secret",
            "openai/gpt-oss-20b",
        );
        let wired = redacted_for_wire(preset);
        assert_eq!(wired.env["ANTHROPIC_API_KEY"], REDACTED_PLACEHOLDER);
        // Non-secret entries pass through.
        assert_eq!(wired.env["ANTHROPIC_BASE_URL"], "http://proxy.local:32768");
    }

    #[test]
    fn list_event_returns_storage_error_when_config_path_is_unavailable() {
        let _guard = override_home_dir(None);

        assert_storage_error(list_event());
    }

    #[test]
    fn add_from_preset_event_returns_storage_error_when_config_path_is_unavailable() {
        let _guard = override_home_dir(None);

        assert_storage_error(add_from_preset_event(sample_input()));
    }

    #[test]
    fn update_event_returns_storage_error_when_config_path_is_unavailable() {
        let _guard = override_home_dir(None);

        assert_storage_error(update_event(sample_agent("claude-proxy")));
    }

    #[test]
    fn delete_event_returns_storage_error_when_config_path_is_unavailable() {
        let _guard = override_home_dir(None);

        assert_storage_error(delete_event("claude-proxy".to_string()));
    }

    #[test]
    fn list_event_uses_resolved_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = override_home_dir(Some(dir.path().to_path_buf()));

        match list_event() {
            BackendEvent::CustomAgentList { agents } => assert!(agents.is_empty()),
            other => panic!("expected CustomAgentList, got {other:?}"),
        }
    }

    #[test]
    fn add_from_preset_event_uses_resolved_config_path() {
        use gwt_agent::REDACTED_PLACEHOLDER;

        let dir = tempfile::tempdir().unwrap();
        let config_path = home_config_path(dir.path().to_path_buf());
        let _guard = override_home_dir(Some(dir.path().to_path_buf()));

        match add_from_preset_event(sample_input()) {
            BackendEvent::CustomAgentSaved { agent } => {
                assert_eq!(agent.id, "claude-proxy");
                assert_eq!(agent.env["ANTHROPIC_API_KEY"], REDACTED_PLACEHOLDER);
            }
            other => panic!("expected CustomAgentSaved, got {other:?}"),
        }

        let persisted =
            crate::custom_agents_service::list_custom_agents(&config_path).expect("reload");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].env["ANTHROPIC_API_KEY"], "sk-real-secret");
    }

    #[test]
    fn update_event_uses_resolved_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = home_config_path(dir.path().to_path_buf());
        let _guard = override_home_dir(Some(dir.path().to_path_buf()));

        add_from_preset_event(sample_input());
        let mut agent = crate::custom_agents_service::list_custom_agents(&config_path)
            .expect("reload")
            .pop()
            .expect("agent");
        agent.display_name = "Renamed Proxy".to_string();

        match update_event(agent) {
            BackendEvent::CustomAgentSaved { agent } => {
                assert_eq!(agent.display_name, "Renamed Proxy");
            }
            other => panic!("expected CustomAgentSaved, got {other:?}"),
        }

        let persisted =
            crate::custom_agents_service::list_custom_agents(&config_path).expect("reload");
        assert_eq!(persisted[0].display_name, "Renamed Proxy");
    }

    #[test]
    fn delete_event_uses_resolved_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = home_config_path(dir.path().to_path_buf());
        let _guard = override_home_dir(Some(dir.path().to_path_buf()));

        add_from_preset_event(sample_input());

        match delete_event("claude-proxy".to_string()) {
            BackendEvent::CustomAgentDeleted { agent_id } => assert_eq!(agent_id, "claude-proxy"),
            other => panic!("expected CustomAgentDeleted, got {other:?}"),
        }

        let persisted =
            crate::custom_agents_service::list_custom_agents(&config_path).expect("reload");
        assert!(persisted.is_empty());
    }
}
