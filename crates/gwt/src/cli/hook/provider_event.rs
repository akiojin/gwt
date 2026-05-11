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
    let event = canonical_event(provider, native_event)
        .ok_or_else(|| HookError::InvalidEvent(format!("{provider}:{native_event}")))?;
    let raw = parse_input(input)?;
    let payload = normalize_payload(provider, native_event, raw);

    Ok(NormalizedProviderEvent {
        event: event.to_string(),
        payload,
    })
}

fn canonical_event(provider: &str, native_event: &str) -> Option<&'static str> {
    let provider = provider.trim().to_ascii_lowercase();
    match (provider.as_str(), native_event.trim()) {
        ("opencode", "session.created") => Some("SessionStart"),
        ("opencode", "message.updated") => Some("UserPromptSubmit"),
        ("opencode", "tool.execute.before") => Some("PreToolUse"),
        ("opencode", "tool.execute.after") => Some("PostToolUse"),
        ("opencode", "session.idle") => Some("Stop"),
        ("openclaw", "session_start") => Some("SessionStart"),
        ("openclaw", "before_prompt_build") => Some("UserPromptSubmit"),
        ("openclaw", "before_tool_call") => Some("PreToolUse"),
        ("openclaw", "after_tool_call") => Some("PostToolUse"),
        ("openclaw", "session_end") => Some("Stop"),
        ("hermes", "on_session_start") => Some("SessionStart"),
        ("hermes", "pre_llm_call") => Some("UserPromptSubmit"),
        ("hermes", "pre_tool_call") => Some("PreToolUse"),
        ("hermes", "post_tool_call") => Some("PostToolUse"),
        ("hermes", "on_session_end") => Some("Stop"),
        _ => None,
    }
}

fn parse_input(input: &str) -> Result<Value, HookError> {
    if input.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    Ok(serde_json::from_str(input)?)
}

fn normalize_payload(provider: &str, native_event: &str, raw: Value) -> Value {
    let mut out = raw.as_object().cloned().unwrap_or_else(|| {
        let mut map = Map::new();
        map.insert("raw".to_string(), raw.clone());
        map
    });

    insert_string_if_missing(&mut out, "provider", Some(provider));
    insert_string_if_missing(&mut out, "native_event", Some(native_event));
    insert_string_if_missing(&mut out, "tool_name", tool_name(&raw).as_deref());
    insert_value_if_missing(&mut out, "tool_input", tool_input(&raw));
    insert_string_if_missing(&mut out, "session_id", session_id(&raw).as_deref());
    insert_string_if_missing(&mut out, "cwd", cwd(&raw).as_deref());
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

fn tool_name(raw: &Value) -> Option<String> {
    string_at_any(
        raw,
        &[
            &["tool_name"],
            &["toolName"],
            &["tool", "name"],
            &["toolName"],
            &["name"],
        ],
    )
    .or_else(|| string_at_any(raw, &[&["tool"]]))
}

fn tool_input(raw: &Value) -> Option<Value> {
    value_at_any(
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
    )
    .cloned()
}

fn session_id(raw: &Value) -> Option<String> {
    string_at_any(
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
    )
}

fn cwd(raw: &Value) -> Option<String> {
    string_at_any(
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
    )
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
