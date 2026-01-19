//! Session parser infrastructure for agent histories.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod opencode;

pub use claude::ClaudeSessionParser;
pub use codex::CodexSessionParser;
pub use gemini::GeminiSessionParser;
pub use opencode::OpenCodeSessionParser;

const LARGE_SESSION_THRESHOLD: usize = 1000;
const SAMPLE_SEGMENT: usize = 20;
const MAX_SEARCH_DEPTH: usize = 6;

/// Supported agent types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    ClaudeCode,
    CodexCli,
    GeminiCli,
    OpenCode,
}

impl AgentType {
    pub fn display_name(&self) -> &'static str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
            AgentType::CodexCli => "Codex CLI",
            AgentType::GeminiCli => "Gemini CLI",
            AgentType::OpenCode => "OpenCode",
        }
    }

    pub fn from_tool_id(tool_id: &str) -> Option<Self> {
        let lower = tool_id.to_lowercase();
        if lower.contains("claude") {
            return Some(Self::ClaudeCode);
        }
        if lower.contains("codex") {
            return Some(Self::CodexCli);
        }
        if lower.contains("gemini") {
            return Some(Self::GeminiCli);
        }
        if lower.contains("opencode") || lower.contains("open-code") {
            return Some(Self::OpenCode);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// Single session message.
#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Tool execution entry.
#[derive(Debug, Clone)]
pub struct ToolExecution {
    pub tool_name: String,
    pub success: bool,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Parsed session output.
#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub session_id: String,
    pub agent_type: AgentType,
    pub messages: Vec<SessionMessage>,
    pub tool_executions: Vec<ToolExecution>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_updated_at: Option<DateTime<Utc>>,
    pub total_turns: usize,
}

pub trait SessionParser: Send + Sync {
    fn parse(&self, session_id: &str) -> Result<ParsedSession, SessionParseError>;
    fn agent_type(&self) -> AgentType;
    fn session_file_path(&self, session_id: &str) -> PathBuf;

    fn session_exists(&self, session_id: &str) -> bool {
        self.session_file_path(session_id).exists()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionParseError {
    #[error("Session file not found: {0}")]
    FileNotFound(String),
    #[error("Invalid session format: {0}")]
    InvalidFormat(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub(crate) fn parse_jsonl_session(
    path: &Path,
    session_id: &str,
    agent_type: AgentType,
) -> Result<ParsedSession, SessionParseError> {
    let file = fs::File::open(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SessionParseError::FileNotFound(path.display().to_string())
        } else {
            SessionParseError::IoError(err)
        }
    })?;
    let reader = BufReader::new(file);

    let mut messages = Vec::new();
    let mut tool_executions = Vec::new();
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut last_updated_at: Option<DateTime<Utc>> = None;
    let mut total_turns = 0usize;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if let Some(message) = extract_message(&value) {
            total_turns += 1;
            update_time_bounds(message.timestamp, &mut started_at, &mut last_updated_at);
            messages.push(message);
        }

        let tools = extract_tool_executions(&value);
        if !tools.is_empty() {
            for tool in tools {
                update_time_bounds(tool.timestamp, &mut started_at, &mut last_updated_at);
                tool_executions.push(tool);
            }
        }
    }

    let messages = sample_messages(messages, total_turns);

    Ok(ParsedSession {
        session_id: session_id.to_string(),
        agent_type,
        messages,
        tool_executions,
        started_at,
        last_updated_at,
        total_turns,
    })
}

pub(crate) fn parse_json_session(
    path: &Path,
    session_id: &str,
    agent_type: AgentType,
) -> Result<ParsedSession, SessionParseError> {
    let content = fs::read_to_string(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SessionParseError::FileNotFound(path.display().to_string())
        } else {
            SessionParseError::IoError(err)
        }
    })?;

    let root: Value = serde_json::from_str(&content)?;

    let entries = extract_entry_array(&root);
    let mut messages = Vec::new();
    let mut tool_executions = Vec::new();
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut last_updated_at: Option<DateTime<Utc>> = None;
    let mut total_turns = 0usize;

    for entry in entries {
        if let Some(message) = extract_message(entry) {
            total_turns += 1;
            update_time_bounds(message.timestamp, &mut started_at, &mut last_updated_at);
            messages.push(message);
        }
        let tools = extract_tool_executions(entry);
        if !tools.is_empty() {
            for tool in tools {
                update_time_bounds(tool.timestamp, &mut started_at, &mut last_updated_at);
                tool_executions.push(tool);
            }
        }
    }

    let root_tools = extract_tool_executions(&root);
    if !root_tools.is_empty() {
        for tool in root_tools {
            update_time_bounds(tool.timestamp, &mut started_at, &mut last_updated_at);
            tool_executions.push(tool);
        }
    }

    let messages = sample_messages(messages, total_turns);

    Ok(ParsedSession {
        session_id: session_id.to_string(),
        agent_type,
        messages,
        tool_executions,
        started_at,
        last_updated_at,
        total_turns,
    })
}

fn update_time_bounds(
    timestamp: Option<DateTime<Utc>>,
    started_at: &mut Option<DateTime<Utc>>,
    last_updated_at: &mut Option<DateTime<Utc>>,
) {
    if let Some(ts) = timestamp {
        let start = started_at.map(|current| current.min(ts)).unwrap_or(ts);
        let end = last_updated_at.map(|current| current.max(ts)).unwrap_or(ts);
        *started_at = Some(start);
        *last_updated_at = Some(end);
    }
}

fn sample_messages(mut messages: Vec<SessionMessage>, total_turns: usize) -> Vec<SessionMessage> {
    if total_turns <= LARGE_SESSION_THRESHOLD {
        return messages;
    }
    if messages.len() <= SAMPLE_SEGMENT * 3 {
        return messages;
    }

    let head = messages.drain(..SAMPLE_SEGMENT).collect::<Vec<_>>();
    let tail = messages
        .drain(messages.len().saturating_sub(SAMPLE_SEGMENT)..)
        .collect::<Vec<_>>();

    if messages.is_empty() {
        return [head, tail].concat();
    }

    let middle_start = messages.len() / 2;
    let middle_offset = SAMPLE_SEGMENT / 2;
    let start = middle_start.saturating_sub(middle_offset);
    let end = (start + SAMPLE_SEGMENT).min(messages.len());
    let middle = messages[start..end].to_vec();

    [head, middle, tail].concat()
}

fn extract_entry_array(root: &Value) -> Vec<&Value> {
    if let Some(arr) = root.as_array() {
        return arr.iter().collect();
    }

    for key in [
        "messages",
        "history",
        "turns",
        "events",
        "conversation",
        "items",
        "entries",
    ] {
        if let Some(arr) = root.get(key).and_then(|value| value.as_array()) {
            return arr.iter().collect();
        }
    }

    Vec::new()
}

fn extract_message(value: &Value) -> Option<SessionMessage> {
    extract_message_from(value)
        .or_else(|| value.get("payload").and_then(extract_message_from))
        .or_else(|| value.get("message").and_then(extract_message_from))
}

fn extract_message_from(value: &Value) -> Option<SessionMessage> {
    let role = extract_role(value)?;
    let content = extract_content(value)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let timestamp = extract_timestamp(value);
    Some(SessionMessage {
        role,
        content: trimmed.to_string(),
        timestamp,
    })
}

fn extract_tool_executions(value: &Value) -> Vec<ToolExecution> {
    let mut execs = Vec::new();

    execs.extend(extract_tool_executions_from(value));
    if let Some(payload) = value.get("payload") {
        execs.extend(extract_tool_executions_from(payload));
    }

    execs
}

fn extract_tool_executions_from(value: &Value) -> Vec<ToolExecution> {
    let mut execs = Vec::new();

    if let Some(calls) = value.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            if let Some(tool) = build_tool_execution(call) {
                execs.push(tool);
            }
        }
    }
    if let Some(calls) = value.get("toolCalls").and_then(|v| v.as_array()) {
        for call in calls {
            if let Some(tool) = build_tool_execution(call) {
                execs.push(tool);
            }
        }
    }

    if is_tool_event(value) {
        if let Some(tool) = build_tool_execution(value) {
            execs.push(tool);
        }
    }

    execs
}

fn build_tool_execution(value: &Value) -> Option<ToolExecution> {
    let tool_name = extract_tool_name(value)?;
    let timestamp = extract_timestamp(value);
    let success = extract_tool_success(value);
    Some(ToolExecution {
        tool_name,
        success,
        timestamp,
    })
}

fn extract_tool_name(value: &Value) -> Option<String> {
    if let Some(name) = extract_string_field(
        value,
        &["tool", "tool_name", "toolName", "name", "function_name"],
    ) {
        return Some(name);
    }
    if let Some(function) = value.get("function") {
        if let Some(name) = extract_string_field(function, &["name"]) {
            return Some(name);
        }
    }
    None
}

fn extract_tool_success(value: &Value) -> bool {
    if let Some(success) = value.get("success").and_then(|v| v.as_bool()) {
        return success;
    }
    if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
        return !status.eq_ignore_ascii_case("error")
            && !status.eq_ignore_ascii_case("failed");
    }
    value.get("error").is_none()
}

fn is_tool_event(value: &Value) -> bool {
    if let Some(kind) = value.get("type").and_then(|v| v.as_str()) {
        let lower = kind.to_lowercase();
        return lower.contains("tool") || lower.contains("function");
    }
    value.get("tool").is_some()
        || value.get("tool_name").is_some()
        || value.get("toolName").is_some()
}

fn extract_role(value: &Value) -> Option<MessageRole> {
    if let Some(role) = extract_string_field(value, &["role", "speaker", "author", "from"]) {
        return map_role(&role);
    }
    if let Some(kind) = extract_string_field(value, &["type"]) {
        return map_role(&kind);
    }
    None
}

fn map_role(value: &str) -> Option<MessageRole> {
    let lower = value.to_lowercase();
    if lower.contains("user") || lower.contains("human") {
        Some(MessageRole::User)
    } else if lower.contains("assistant") || lower.contains("ai") || lower.contains("model") {
        Some(MessageRole::Assistant)
    } else {
        None
    }
}

fn extract_content(value: &Value) -> Option<String> {
    if let Some(content) = value.get("content") {
        if let Some(text) = value_to_text(content) {
            return Some(text);
        }
    }
    if let Some(text) = extract_string_field(value, &["text", "message", "prompt", "response"]) {
        return Some(text);
    }
    if let Some(input) = value.get("input") {
        if let Some(text) = value_to_text(input) {
            return Some(text);
        }
    }
    if let Some(output) = value.get("output") {
        if let Some(text) = value_to_text(output) {
            return Some(text);
        }
    }
    None
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(num) => Some(num.to_string()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = value_to_text(item) {
                    if !text.trim().is_empty() {
                        parts.push(text);
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Object(map) => {
            if let Some(text) = map
                .get("text")
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
            {
                return Some(text);
            }
            if let Some(text) = map
                .get("content")
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
            {
                return Some(text);
            }
            None
        }
        _ => None,
    }
}

fn extract_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    for key in [
        "timestamp",
        "ts",
        "time",
        "created_at",
        "createdAt",
        "updated_at",
        "updatedAt",
        "timestamp_ms",
        "timestampMs",
    ] {
        if let Some(val) = value.get(key) {
            if let Some(dt) = parse_timestamp(val) {
                return Some(dt);
            }
        }
    }
    None
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(num) = value.as_i64() {
        return timestamp_from_i64(num);
    }
    if let Some(num) = value.as_u64() {
        return timestamp_from_i64(num as i64);
    }
    if let Some(text) = value.as_str() {
        if let Ok(num) = text.parse::<i64>() {
            return timestamp_from_i64(num);
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    None
}

fn timestamp_from_i64(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    if value > 1_000_000_000_000 {
        Utc.timestamp_millis_opt(value).single()
    } else {
        Utc.timestamp_opt(value, 0).single()
    }
}

fn extract_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(val) = value.get(*key) {
            if let Some(text) = val.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            } else if let Some(num) = val.as_i64() {
                return Some(num.to_string());
            } else if let Some(num) = val.as_u64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

pub(crate) fn find_session_file(root: &Path, session_id: &str, extensions: &[&str]) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_SEARCH_DEPTH {
            continue;
        }
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extensions.is_empty() && !extensions.iter().any(|&e| e.eq_ignore_ascii_case(ext)) {
                continue;
            }
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            if stem == session_id || file_name.contains(session_id) {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_jsonl_session_basic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sess-1.jsonl");
        let content = r#"{"type":"message","role":"user","content":"Hello","timestamp":1700000000}
{"type":"message","role":"assistant","content":"Hi","timestamp":1700000001}
{"type":"tool_use","name":"read_file","timestamp":1700000002}"#;
        fs::write(&path, content).unwrap();

        let parsed = parse_jsonl_session(&path, "sess-1", AgentType::CodexCli).unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.tool_executions.len(), 1);
        assert_eq!(parsed.total_turns, 2);
    }

    #[test]
    fn test_parse_json_session_basic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sess-2.json");
        let content = r#"{
  "messages": [
    {"role": "user", "content": "Do thing", "timestamp": 1700000100},
    {"role": "assistant", "content": "Done", "timestamp": 1700000200}
  ],
  "tool_calls": [
    {"name": "bash", "timestamp": 1700000150}
  ]
}"#;
        fs::write(&path, content).unwrap();

        let parsed = parse_json_session(&path, "sess-2", AgentType::GeminiCli).unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.tool_executions.len(), 1);
        assert_eq!(parsed.total_turns, 2);
    }

    #[test]
    fn test_sample_messages_long_session() {
        let messages = (0..1200)
            .map(|i| SessionMessage {
                role: MessageRole::User,
                content: format!("msg-{}", i),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let sampled = sample_messages(messages, 1200);
        assert!(sampled.len() <= SAMPLE_SEGMENT * 3);
    }
}
