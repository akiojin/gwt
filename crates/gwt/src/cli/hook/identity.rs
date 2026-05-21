use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use serde::Deserialize;

use super::{HookError, HookEvent};

const CODEX_THREAD_ID_ENV: &str = "CODEX_THREAD_ID";
const CODEX_PLACEHOLDER_SESSION_ID: &str = "agent-session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GwtSessionId(String);

impl GwtSessionId {
    pub(crate) fn from_env() -> Option<Self> {
        let value = std::env::var_os(GWT_SESSION_ID_ENV)?;
        Self::parse(value.to_string_lossy().as_ref())
    }

    pub(crate) fn required_from_env(event: &str) -> Result<Self, HookError> {
        let Some(session_id) = Self::from_env() else {
            eprintln!("gwtd hook runtime-state: missing GWT_SESSION_ID event={event}");
            return Err(HookError::InvalidEvent(format!(
                "missing GWT_SESSION_ID for managed hook event {event}"
            )));
        };
        Ok(session_id)
    }

    fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        (!value.is_empty()).then(|| Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Non-empty provider session id extracted from a raw hook payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookSessionId(String);

impl HookSessionId {
    fn parse(value: Option<&str>) -> Option<Self> {
        value
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| Self(id.to_string()))
    }

    fn is_codex_placeholder(&self) -> bool {
        self.0 == CODEX_PLACEHOLDER_SESSION_ID
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookAgentSessionId {
    Provided(HookSessionId),
    MissingRequiredForCodex,
    MissingOptional,
}

/// Raw Claude Code / Codex hook payload. Fields are optional only at this
/// boundary so malformed provider payloads can still be parsed and logged.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawHookEvent {
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    session_id: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
}

impl RawHookEvent {
    pub(crate) fn read_from_str(input: &str) -> Result<Option<Self>, HookError> {
        if input.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(input)?))
    }

    pub(crate) fn session_id(&self) -> Option<HookSessionId> {
        HookSessionId::parse(self.session_id.as_deref())
    }

    pub(crate) fn tool_name(&self) -> Option<&str> {
        self.tool_name.as_deref()
    }

    pub(crate) fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
}

impl From<RawHookEvent> for HookEvent {
    fn from(raw: RawHookEvent) -> Self {
        Self {
            tool_name: raw.tool_name,
            tool_input: raw.tool_input,
            transcript_path: raw.transcript_path,
            cwd: raw.cwd,
        }
    }
}

pub(crate) fn resolve_hook_agent_session_id(
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) -> HookAgentSessionId {
    if session.map(is_codex_session).unwrap_or(false) {
        if let Some(session_id) = codex_thread_id_from_env() {
            return HookAgentSessionId::Provided(session_id);
        }
        if let Some(session_id) = hook_event
            .and_then(RawHookEvent::session_id)
            .filter(|session_id| !session_id.is_codex_placeholder())
        {
            return HookAgentSessionId::Provided(session_id);
        }
        return HookAgentSessionId::MissingRequiredForCodex;
    }
    if let Some(session_id) = hook_event.and_then(RawHookEvent::session_id) {
        return HookAgentSessionId::Provided(session_id);
    }
    HookAgentSessionId::MissingOptional
}

fn is_codex_session(session: &Session) -> bool {
    matches!(&session.agent_id, gwt_agent::AgentId::Codex)
}

fn codex_thread_id_from_env() -> Option<HookSessionId> {
    let value = std::env::var_os(CODEX_THREAD_ID_ENV)?;
    HookSessionId::parse(Some(value.to_string_lossy().as_ref()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use gwt_agent::{AgentId, Session};

    use super::*;

    struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self { saved: Vec::new() }
        }

        fn set(&mut self, key: &'static str, value: impl Into<OsString>) {
            if !self.saved.iter().any(|(saved_key, _)| *saved_key == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::set_var(key, value.into());
        }

        fn remove(&mut self, key: &'static str) {
            if !self.saved.iter().any(|(saved_key, _)| *saved_key == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::remove_var(key);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.iter().rev() {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    fn codex_session() -> Session {
        Session::new("/tmp/worktree", "work/recover", AgentId::Codex)
    }

    fn raw_event(session_id: &str) -> RawHookEvent {
        RawHookEvent {
            tool_name: None,
            tool_input: None,
            session_id: Some(session_id.to_string()),
            transcript_path: None,
            cwd: None,
        }
    }

    #[test]
    fn codex_hook_identity_prefers_thread_id_env_over_placeholder_payload() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut env = EnvGuard::new();
        env.set(CODEX_THREAD_ID_ENV, "019e4646-9d79-79f0-b74a-df9f74f9f0fd");
        let session = codex_session();

        let resolved = resolve_hook_agent_session_id(
            Some(&session),
            Some(&raw_event(CODEX_PLACEHOLDER_SESSION_ID)),
        );

        assert!(matches!(
            resolved,
            HookAgentSessionId::Provided(id)
                if id.as_str() == "019e4646-9d79-79f0-b74a-df9f74f9f0fd"
        ));
    }

    #[test]
    fn codex_hook_identity_rejects_placeholder_without_thread_id_env() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut env = EnvGuard::new();
        env.remove(CODEX_THREAD_ID_ENV);
        let session = codex_session();

        let resolved = resolve_hook_agent_session_id(
            Some(&session),
            Some(&raw_event(CODEX_PLACEHOLDER_SESSION_ID)),
        );

        assert_eq!(resolved, HookAgentSessionId::MissingRequiredForCodex);
    }

    #[test]
    fn codex_hook_identity_uses_raw_payload_when_thread_id_env_is_missing() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut env = EnvGuard::new();
        env.remove(CODEX_THREAD_ID_ENV);
        let session = codex_session();

        let resolved = resolve_hook_agent_session_id(
            Some(&session),
            Some(&raw_event("019e4646-9d79-79f0-b74a-df9f74f9f0fd")),
        );

        assert!(matches!(
            resolved,
            HookAgentSessionId::Provided(id)
                if id.as_str() == "019e4646-9d79-79f0-b74a-df9f74f9f0fd"
        ));
    }
}
