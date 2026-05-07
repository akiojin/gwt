use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use serde::Deserialize;

use super::{HookError, HookEvent};

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
    if let Some(session_id) = hook_event.and_then(RawHookEvent::session_id) {
        return HookAgentSessionId::Provided(session_id);
    }
    if session.map(is_codex_session).unwrap_or(false) {
        return HookAgentSessionId::MissingRequiredForCodex;
    }
    HookAgentSessionId::MissingOptional
}

fn is_codex_session(session: &Session) -> bool {
    matches!(&session.agent_id, gwt_agent::AgentId::Codex)
}
