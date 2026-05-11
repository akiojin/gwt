//! Provider-owned hook adapters for non-Claude/Codex agents.
//!
//! OpenCode, OpenClaw, and Hermes expose different native hook names and
//! payload shapes. This adapter normalizes those native events into gwt's
//! canonical hook vocabulary, then delegates to the existing event dispatcher.

use std::path::Path;

use serde_json::{Map, Value};

use super::{event_dispatcher, HookError, HookOutput};

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedProviderEvent {
    pub event: String,
    pub payload: Value,
}

pub fn handle_with_input(
    provider: &str,
    native_event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Result<HookOutput, HookError> {
    let normalized = normalize_provider_payload(provider, native_event, input)?;
    let payload = serde_json::to_string(&normalized.payload)?;
    event_dispatcher::handle_with_input(&normalized.event, &payload, worktree_root, current_session)
}

pub fn normalize_provider_payload(
    provider: &str,
    native_event: &str,
    input: &str,
) -> Result<NormalizedProviderEvent, HookError> {
    let provider_kind = ProviderKind::parse(provider)
        .ok_or_else(|| HookError::InvalidEvent(format!("{provider}:{native_event}")))?;
    let event = provider_kind
        .canonical_event(native_event)
        .ok_or_else(|| HookError::InvalidEvent(format!("{provider}:{native_event}")))?;
    let raw = parse_input(input)?;
    let payload = normalize_payload(provider_kind, native_event, raw);

    Ok(NormalizedProviderEvent {
        event: event.to_string(),
        payload,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    OpenCode,
    OpenClaw,
    Hermes,
}

impl ProviderKind {
    fn parse(provider: &str) -> Option<Self> {
        match provider.trim().to_ascii_lowercase().as_str() {
            "opencode" => Some(Self::OpenCode),
            "openclaw" => Some(Self::OpenClaw),
            "hermes" => Some(Self::Hermes),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::OpenClaw => "openclaw",
            Self::Hermes => "hermes",
        }
    }

    fn canonical_event(self, native_event: &str) -> Option<&'static str> {
        match (self, native_event.trim()) {
            (Self::OpenCode, "session.created") => Some("SessionStart"),
            (Self::OpenCode, "message.updated") => Some("UserPromptSubmit"),
            (Self::OpenCode, "tool.execute.before") => Some("PreToolUse"),
            (Self::OpenCode, "tool.execute.after") => Some("PostToolUse"),
            (Self::OpenCode, "session.idle") => Some("Stop"),
            (Self::OpenClaw, "session_start") => Some("SessionStart"),
            (Self::OpenClaw, "before_prompt_build") => Some("UserPromptSubmit"),
            (Self::OpenClaw, "before_tool_call") => Some("PreToolUse"),
            (Self::OpenClaw, "after_tool_call") => Some("PostToolUse"),
            (Self::OpenClaw, "session_end") => Some("Stop"),
            (Self::Hermes, "on_session_start") => Some("SessionStart"),
            (Self::Hermes, "pre_llm_call") => Some("UserPromptSubmit"),
            (Self::Hermes, "pre_tool_call") => Some("PreToolUse"),
            (Self::Hermes, "post_tool_call") => Some("PostToolUse"),
            (Self::Hermes, "on_session_end") => Some("Stop"),
            _ => None,
        }
    }
}

fn parse_input(input: &str) -> Result<Value, HookError> {
    if input.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    Ok(serde_json::from_str(input)?)
}

fn normalize_payload(provider: ProviderKind, native_event: &str, raw: Value) -> Value {
    let mut out = raw.as_object().cloned().unwrap_or_else(|| {
        let mut map = Map::new();
        map.insert("raw".to_string(), raw.clone());
        map
    });

    insert_string_if_missing(&mut out, "provider", Some(provider.as_str()));
    insert_string_if_missing(&mut out, "native_event", Some(native_event));
    insert_string_if_missing(&mut out, "tool_name", tool_name(provider, &raw).as_deref());
    insert_value_if_missing(&mut out, "tool_input", tool_input(provider, &raw));
    insert_string_if_missing(
        &mut out,
        "session_id",
        session_id(provider, &raw).as_deref(),
    );
    insert_string_if_missing(&mut out, "cwd", cwd(provider, &raw).as_deref());
    insert_string_if_missing(
        &mut out,
        "transcript_path",
        string_at_any(&raw, &[&["transcript_path"], &["transcriptPath"]]).as_deref(),
    );

    Value::Object(out)
}

fn insert_string_if_missing(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if map.contains_key(key) {
        return;
    }
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    map.insert(key.to_string(), Value::String(value.to_string()));
}

fn insert_value_if_missing(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if map.contains_key(key) {
        return;
    }
    if let Some(value) = value {
        map.insert(key.to_string(), value);
    }
}

fn tool_name(provider: ProviderKind, raw: &Value) -> Option<String> {
    match provider {
        ProviderKind::OpenCode => string_at_any(
            raw,
            &[
                &["tool_name"],
                &["toolName"],
                &["tool", "name"],
                &["name"],
                &["input", "tool"],
                &["input", "toolName"],
                &["output", "tool"],
                &["output", "toolName"],
            ],
        ),
        ProviderKind::OpenClaw => string_at_any(
            raw,
            &[
                &["tool_name"],
                &["toolName"],
                &["tool", "name"],
                &["event", "toolName"],
                &["event", "tool_name"],
            ],
        ),
        ProviderKind::Hermes => string_at_any(
            raw,
            &[
                &["tool_name"],
                &["toolName"],
                &["tool", "name"],
                &["name"],
                &["tool"],
            ],
        ),
    }
}

fn tool_input(provider: ProviderKind, raw: &Value) -> Option<Value> {
    match provider {
        ProviderKind::OpenCode => value_at_any(
            raw,
            &[
                &["tool_input"],
                &["toolInput"],
                &["output", "args"],
                &["input", "args"],
                &["output", "params"],
                &["input", "params"],
                &["output", "input"],
                &["input", "input"],
                &["args"],
                &["arguments"],
                &["params"],
                &["tool", "input"],
                &["tool", "args"],
                &["tool", "arguments"],
                &["tool", "params"],
            ],
        ),
        ProviderKind::OpenClaw => value_at_any(
            raw,
            &[
                &["tool_input"],
                &["toolInput"],
                &["event", "params"],
                &["event", "args"],
                &["event", "toolInput"],
                &["event", "tool_input"],
                &["params"],
                &["args"],
                &["tool", "input"],
            ],
        ),
        ProviderKind::Hermes => value_at_any(
            raw,
            &[
                &["tool_input"],
                &["toolInput"],
                &["input"],
                &["args"],
                &["arguments"],
                &["params"],
                &["tool", "input"],
                &["tool", "args"],
                &["tool", "arguments"],
                &["tool", "params"],
            ],
        ),
    }
    .cloned()
}

fn session_id(provider: ProviderKind, raw: &Value) -> Option<String> {
    match provider {
        ProviderKind::OpenCode => string_at_any(
            raw,
            &[
                &["session_id"],
                &["sessionId"],
                &["sessionID"],
                &["session", "id"],
                &["session", "session_id"],
                &["input", "sessionID"],
                &["input", "sessionId"],
                &["input", "session_id"],
                &["context", "sessionID"],
                &["context", "sessionId"],
            ],
        ),
        ProviderKind::OpenClaw => string_at_any(
            raw,
            &[
                &["session_id"],
                &["sessionId"],
                &["sessionID"],
                &["session", "id"],
                &["event", "sessionId"],
                &["event", "session_id"],
                &["ctx", "sessionId"],
                &["ctx", "sessionKey"],
            ],
        ),
        ProviderKind::Hermes => string_at_any(
            raw,
            &[
                &["session_id"],
                &["sessionId"],
                &["sessionID"],
                &["session", "id"],
                &["session", "session_id"],
                &["ctx", "sessionId"],
                &["context", "sessionId"],
            ],
        ),
    }
}

fn cwd(provider: ProviderKind, raw: &Value) -> Option<String> {
    match provider {
        ProviderKind::OpenCode => string_at_any(
            raw,
            &[
                &["cwd"],
                &["working_directory"],
                &["workingDirectory"],
                &["directory"],
                &["input", "cwd"],
                &["context", "directory"],
                &["context", "worktree"],
                &["context", "project", "root"],
            ],
        ),
        ProviderKind::OpenClaw => string_at_any(
            raw,
            &[
                &["cwd"],
                &["working_directory"],
                &["workingDirectory"],
                &["directory"],
                &["event", "cwd"],
                &["ctx", "cwd"],
                &["ctx", "workspaceRoot"],
            ],
        ),
        ProviderKind::Hermes => string_at_any(
            raw,
            &[
                &["cwd"],
                &["working_directory"],
                &["workingDirectory"],
                &["directory"],
                &["project", "root"],
                &["workspace", "root"],
                &["session", "cwd"],
            ],
        ),
    }
}

fn string_at_any(raw: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value_at(raw, path))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_at_any<'a>(raw: &'a Value, paths: &[&[&str]]) -> Option<&'a Value> {
    paths.iter().find_map(|path| value_at(raw, path))
}

fn value_at<'a>(raw: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = raw;
    for key in path {
        current = current.as_object()?.get(*key)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_native_payload_shape_promotes_tool_and_workspace_fields() {
        let normalized = normalize_provider_payload(
            "opencode",
            "tool.execute.before",
            r#"{
              "input": {
                "tool": "Bash",
                "args": { "command": "pwd" },
                "sessionID": "oc-1"
              },
              "context": {
                "directory": "/repo"
              }
            }"#,
        )
        .expect("normalize opencode native-like payload");

        assert_eq!(normalized.event, "PreToolUse");
        assert_eq!(normalized.payload["tool_name"], "Bash");
        assert_eq!(normalized.payload["tool_input"]["command"], "pwd");
        assert_eq!(normalized.payload["session_id"], "oc-1");
        assert_eq!(normalized.payload["cwd"], "/repo");
    }

    #[test]
    fn openclaw_native_payload_shape_promotes_event_and_context_fields() {
        let normalized = normalize_provider_payload(
            "openclaw",
            "before_tool_call",
            r#"{
              "event": {
                "toolName": "Bash",
                "params": { "command": "git status" },
                "sessionId": "claw-1"
              },
              "ctx": {
                "workspaceRoot": "/repo"
              }
            }"#,
        )
        .expect("normalize openclaw native-like payload");

        assert_eq!(normalized.event, "PreToolUse");
        assert_eq!(normalized.payload["tool_name"], "Bash");
        assert_eq!(normalized.payload["tool_input"]["command"], "git status");
        assert_eq!(normalized.payload["session_id"], "claw-1");
        assert_eq!(normalized.payload["cwd"], "/repo");
    }
}
