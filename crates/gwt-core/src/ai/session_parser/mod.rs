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

/// Session list entry for UI display.
#[derive(Debug, Clone)]
pub struct SessionListEntry {
    pub session_id: String,
    pub last_updated: Option<DateTime<Utc>>,
    pub message_count: usize,
    pub file_path: PathBuf,
}

pub trait SessionParser: Send + Sync {
    fn parse(&self, session_id: &str) -> Result<ParsedSession, SessionParseError>;
    fn agent_type(&self) -> AgentType;
    fn session_file_path(&self, session_id: &str) -> PathBuf;

    fn session_exists(&self, session_id: &str) -> bool {
        self.session_file_path(session_id).exists()
    }

    /// List all available sessions, optionally filtered by worktree path.
    /// Results are sorted by last_updated (newest first).
    fn list_sessions(&self, worktree_path: Option<&Path>) -> Vec<SessionListEntry>;
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
        return !status.eq_ignore_ascii_case("error") && !status.eq_ignore_ascii_case("failed");
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

pub(crate) fn find_session_file(
    root: &Path,
    session_id: &str,
    extensions: &[&str],
) -> Option<PathBuf> {
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

    // --- AgentType tests ---

    #[test]
    fn agent_type_display_names() {
        assert_eq!(AgentType::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(AgentType::CodexCli.display_name(), "Codex CLI");
        assert_eq!(AgentType::GeminiCli.display_name(), "Gemini CLI");
        assert_eq!(AgentType::OpenCode.display_name(), "OpenCode");
    }

    #[test]
    fn agent_type_from_tool_id_claude() {
        assert_eq!(
            AgentType::from_tool_id("claude-code"),
            Some(AgentType::ClaudeCode)
        );
        assert_eq!(
            AgentType::from_tool_id("CLAUDE"),
            Some(AgentType::ClaudeCode)
        );
    }

    #[test]
    fn agent_type_from_tool_id_codex() {
        assert_eq!(
            AgentType::from_tool_id("codex-cli"),
            Some(AgentType::CodexCli)
        );
        assert_eq!(
            AgentType::from_tool_id("CODEX"),
            Some(AgentType::CodexCli)
        );
    }

    #[test]
    fn agent_type_from_tool_id_gemini() {
        assert_eq!(
            AgentType::from_tool_id("gemini-cli"),
            Some(AgentType::GeminiCli)
        );
    }

    #[test]
    fn agent_type_from_tool_id_opencode() {
        assert_eq!(
            AgentType::from_tool_id("opencode"),
            Some(AgentType::OpenCode)
        );
        assert_eq!(
            AgentType::from_tool_id("open-code"),
            Some(AgentType::OpenCode)
        );
    }

    #[test]
    fn agent_type_from_tool_id_unknown() {
        assert_eq!(AgentType::from_tool_id("unknown-tool"), None);
        assert_eq!(AgentType::from_tool_id(""), None);
    }

    // --- extract_role / map_role ---

    #[test]
    fn map_role_user_variants() {
        assert_eq!(map_role("user"), Some(MessageRole::User));
        assert_eq!(map_role("human"), Some(MessageRole::User));
        assert_eq!(map_role("USER"), Some(MessageRole::User));
    }

    #[test]
    fn map_role_assistant_variants() {
        assert_eq!(map_role("assistant"), Some(MessageRole::Assistant));
        assert_eq!(map_role("ai"), Some(MessageRole::Assistant));
        assert_eq!(map_role("model"), Some(MessageRole::Assistant));
    }

    #[test]
    fn map_role_unknown() {
        assert_eq!(map_role("system"), None);
        assert_eq!(map_role("tool"), None);
        assert_eq!(map_role(""), None);
    }

    // --- extract_timestamp / parse_timestamp ---

    #[test]
    fn parse_timestamp_from_rfc3339_string() {
        let value = serde_json::json!("2026-01-15T10:30:00Z");
        let ts = parse_timestamp(&value);
        assert!(ts.is_some());
    }

    #[test]
    fn parse_timestamp_from_unix_seconds() {
        let value = serde_json::json!(1700000000);
        let ts = parse_timestamp(&value);
        assert!(ts.is_some());
    }

    #[test]
    fn parse_timestamp_from_unix_millis() {
        let value = serde_json::json!(1700000000000i64);
        let ts = parse_timestamp(&value);
        assert!(ts.is_some());
    }

    #[test]
    fn parse_timestamp_from_string_number() {
        let value = serde_json::json!("1700000000");
        let ts = parse_timestamp(&value);
        assert!(ts.is_some());
    }

    #[test]
    fn parse_timestamp_returns_none_for_zero() {
        let value = serde_json::json!(0);
        assert!(parse_timestamp(&value).is_none());
    }

    #[test]
    fn parse_timestamp_returns_none_for_negative() {
        let value = serde_json::json!(-100);
        assert!(parse_timestamp(&value).is_none());
    }

    #[test]
    fn parse_timestamp_returns_none_for_invalid_string() {
        let value = serde_json::json!("not-a-date");
        assert!(parse_timestamp(&value).is_none());
    }

    // --- extract_tool_success ---

    #[test]
    fn tool_success_from_success_field() {
        let value = serde_json::json!({"success": true});
        assert!(extract_tool_success(&value));

        let value = serde_json::json!({"success": false});
        assert!(!extract_tool_success(&value));
    }

    #[test]
    fn tool_success_from_status_field() {
        let value = serde_json::json!({"status": "completed"});
        assert!(extract_tool_success(&value));

        let value = serde_json::json!({"status": "error"});
        assert!(!extract_tool_success(&value));

        let value = serde_json::json!({"status": "FAILED"});
        assert!(!extract_tool_success(&value));
    }

    #[test]
    fn tool_success_default_when_no_error() {
        let value = serde_json::json!({"tool": "read_file"});
        assert!(extract_tool_success(&value));
    }

    #[test]
    fn tool_success_false_when_error_present() {
        let value = serde_json::json!({"error": "something went wrong"});
        assert!(!extract_tool_success(&value));
    }

    // --- is_tool_event ---

    #[test]
    fn is_tool_event_from_type_field() {
        let value = serde_json::json!({"type": "tool_use"});
        assert!(is_tool_event(&value));

        let value = serde_json::json!({"type": "function_call"});
        assert!(is_tool_event(&value));
    }

    #[test]
    fn is_tool_event_from_tool_field() {
        let value = serde_json::json!({"tool": "bash"});
        assert!(is_tool_event(&value));
    }

    #[test]
    fn is_tool_event_from_tool_name_field() {
        let value = serde_json::json!({"tool_name": "read_file"});
        assert!(is_tool_event(&value));
    }

    #[test]
    fn is_tool_event_false_for_message() {
        let value = serde_json::json!({"type": "message", "role": "user"});
        assert!(!is_tool_event(&value));
    }

    // --- extract_content ---

    #[test]
    fn extract_content_from_content_string() {
        let value = serde_json::json!({"content": "Hello world"});
        assert_eq!(extract_content(&value), Some("Hello world".to_string()));
    }

    #[test]
    fn extract_content_from_text_field() {
        let value = serde_json::json!({"text": "Some text"});
        assert_eq!(extract_content(&value), Some("Some text".to_string()));
    }

    #[test]
    fn extract_content_from_message_field() {
        let value = serde_json::json!({"message": "A message"});
        assert_eq!(extract_content(&value), Some("A message".to_string()));
    }

    #[test]
    fn extract_content_from_content_array() {
        let value = serde_json::json!({"content": [{"text": "part1"}, {"text": "part2"}]});
        let content = extract_content(&value).unwrap();
        assert!(content.contains("part1"));
        assert!(content.contains("part2"));
    }

    #[test]
    fn extract_content_empty_returns_some_empty() {
        // extract_content returns the raw string; filtering happens in extract_message_from
        let value = serde_json::json!({"content": ""});
        assert_eq!(extract_content(&value), Some("".to_string()));
    }

    #[test]
    fn extract_message_filters_empty_content() {
        // extract_message_from trims and rejects empty content
        let value = serde_json::json!({"role": "user", "content": "   "});
        assert!(extract_message(&value).is_none());
    }

    // --- value_to_text ---

    #[test]
    fn value_to_text_number() {
        let value = serde_json::json!(42);
        assert_eq!(value_to_text(&value), Some("42".to_string()));
    }

    #[test]
    fn value_to_text_empty_array_returns_none() {
        let value = serde_json::json!([]);
        assert!(value_to_text(&value).is_none());
    }

    #[test]
    fn value_to_text_null_returns_none() {
        let value = serde_json::json!(null);
        assert!(value_to_text(&value).is_none());
    }

    #[test]
    fn value_to_text_object_with_text_key() {
        let value = serde_json::json!({"text": "hello"});
        assert_eq!(value_to_text(&value), Some("hello".to_string()));
    }

    #[test]
    fn value_to_text_object_with_content_key() {
        let value = serde_json::json!({"content": "world"});
        assert_eq!(value_to_text(&value), Some("world".to_string()));
    }

    // --- extract_entry_array ---

    #[test]
    fn extract_entry_array_from_top_level_array() {
        let root = serde_json::json!([{"role": "user"}, {"role": "assistant"}]);
        assert_eq!(extract_entry_array(&root).len(), 2);
    }

    #[test]
    fn extract_entry_array_from_messages_key() {
        let root = serde_json::json!({"messages": [{"role": "user"}]});
        assert_eq!(extract_entry_array(&root).len(), 1);
    }

    #[test]
    fn extract_entry_array_from_history_key() {
        let root = serde_json::json!({"history": [1, 2, 3]});
        assert_eq!(extract_entry_array(&root).len(), 3);
    }

    #[test]
    fn extract_entry_array_empty_object() {
        let root = serde_json::json!({});
        assert!(extract_entry_array(&root).is_empty());
    }

    #[test]
    fn extract_entry_array_tries_keys_in_order() {
        // If both "messages" and "history" exist, "messages" wins
        let root =
            serde_json::json!({"messages": [1, 2], "history": [3, 4, 5]});
        assert_eq!(extract_entry_array(&root).len(), 2);
    }

    // --- extract_message ---

    #[test]
    fn extract_message_from_direct_fields() {
        let value =
            serde_json::json!({"role": "user", "content": "Hello", "timestamp": 1700000000});
        let msg = extract_message(&value).unwrap();
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "Hello");
        assert!(msg.timestamp.is_some());
    }

    #[test]
    fn extract_message_from_payload() {
        let value = serde_json::json!({
            "payload": {"role": "assistant", "content": "Response"}
        });
        let msg = extract_message(&value).unwrap();
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.content, "Response");
    }

    #[test]
    fn extract_message_from_nested_message() {
        let value = serde_json::json!({
            "message": {"role": "user", "content": "Nested"}
        });
        let msg = extract_message(&value).unwrap();
        assert_eq!(msg.content, "Nested");
    }

    #[test]
    fn extract_message_returns_none_for_empty_content() {
        let value = serde_json::json!({"role": "user", "content": ""});
        assert!(extract_message(&value).is_none());
    }

    #[test]
    fn extract_message_returns_none_for_no_role() {
        let value = serde_json::json!({"content": "No role here"});
        assert!(extract_message(&value).is_none());
    }

    // --- extract_tool_executions ---

    #[test]
    fn extract_tool_executions_from_tool_calls() {
        let value = serde_json::json!({
            "tool_calls": [
                {"name": "bash", "timestamp": 1700000000},
                {"name": "read_file", "timestamp": 1700000001}
            ]
        });
        let tools = extract_tool_executions(&value);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_name, "bash");
        assert_eq!(tools[1].tool_name, "read_file");
    }

    #[test]
    fn extract_tool_executions_from_camel_case() {
        let value = serde_json::json!({
            "toolCalls": [{"name": "edit"}]
        });
        let tools = extract_tool_executions(&value);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "edit");
    }

    #[test]
    fn extract_tool_executions_from_payload() {
        let value = serde_json::json!({
            "payload": {
                "tool_calls": [{"name": "bash"}]
            }
        });
        let tools = extract_tool_executions(&value);
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn extract_tool_executions_empty_for_message() {
        let value = serde_json::json!({"role": "user", "content": "Hi"});
        let tools = extract_tool_executions(&value);
        assert!(tools.is_empty());
    }

    // --- find_session_file ---

    #[test]
    fn find_session_file_by_stem() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let file = root.join("abc.jsonl");
        fs::write(&file, "{}").unwrap();

        let found = find_session_file(root, "abc", &["jsonl"]);
        assert_eq!(found, Some(file));
    }

    #[test]
    fn find_session_file_in_nested_dir() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let file = sub.join("deep-session.json");
        fs::write(&file, "{}").unwrap();

        let found = find_session_file(root, "deep-session", &["json"]);
        assert_eq!(found, Some(file));
    }

    #[test]
    fn find_session_file_returns_none_for_missing() {
        let dir = tempdir().unwrap();
        let found = find_session_file(dir.path(), "nonexistent", &["json"]);
        assert!(found.is_none());
    }

    #[test]
    fn find_session_file_returns_none_for_nonexistent_root() {
        let found = find_session_file(
            std::path::Path::new("/nonexistent/path"),
            "sess",
            &["json"],
        );
        assert!(found.is_none());
    }

    #[test]
    fn find_session_file_filters_by_extension() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let txt_file = root.join("sess.txt");
        fs::write(&txt_file, "text").unwrap();

        let found = find_session_file(root, "sess", &["json", "jsonl"]);
        assert!(found.is_none());
    }

    #[test]
    fn find_session_file_matches_partial_in_filename() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let file = root.join("project-abc-123.json");
        fs::write(&file, "{}").unwrap();

        let found = find_session_file(root, "abc-123", &["json"]);
        assert_eq!(found, Some(file));
    }

    // --- sample_messages edge cases ---

    #[test]
    fn sample_messages_returns_all_when_under_threshold() {
        let messages = (0..100)
            .map(|i| SessionMessage {
                role: MessageRole::User,
                content: format!("msg-{}", i),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let sampled = sample_messages(messages.clone(), 100);
        assert_eq!(sampled.len(), 100);
    }

    #[test]
    fn sample_messages_returns_all_when_few_messages_but_many_turns() {
        // total_turns > threshold but messages.len() <= SAMPLE_SEGMENT * 3
        let messages = (0..50)
            .map(|i| SessionMessage {
                role: MessageRole::User,
                content: format!("msg-{}", i),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let sampled = sample_messages(messages.clone(), 1500);
        assert_eq!(sampled.len(), 50);
    }

    #[test]
    fn sample_messages_preserves_head_and_tail() {
        let messages = (0..200)
            .map(|i| SessionMessage {
                role: MessageRole::User,
                content: format!("msg-{}", i),
                timestamp: None,
            })
            .collect::<Vec<_>>();
        let sampled = sample_messages(messages, 1500);
        // Head: first SAMPLE_SEGMENT messages
        assert_eq!(sampled[0].content, "msg-0");
        // Tail: last few messages
        let last = &sampled[sampled.len() - 1];
        assert_eq!(last.content, "msg-199");
    }

    // --- update_time_bounds ---

    #[test]
    fn update_time_bounds_sets_initial_values() {
        let mut started = None;
        let mut ended = None;
        let ts = Utc::now();
        update_time_bounds(Some(ts), &mut started, &mut ended);
        assert_eq!(started, Some(ts));
        assert_eq!(ended, Some(ts));
    }

    #[test]
    fn update_time_bounds_expands_range() {
        let t1 = Utc.timestamp_opt(1700000000, 0).single().unwrap();
        let t2 = Utc.timestamp_opt(1700000100, 0).single().unwrap();
        let t3 = Utc.timestamp_opt(1699999900, 0).single().unwrap();

        let mut started = Some(t1);
        let mut ended = Some(t1);

        update_time_bounds(Some(t2), &mut started, &mut ended);
        assert_eq!(started, Some(t1));
        assert_eq!(ended, Some(t2));

        update_time_bounds(Some(t3), &mut started, &mut ended);
        assert_eq!(started, Some(t3));
        assert_eq!(ended, Some(t2));
    }

    #[test]
    fn update_time_bounds_noop_for_none() {
        let mut started = None;
        let mut ended = None;
        update_time_bounds(None, &mut started, &mut ended);
        assert!(started.is_none());
        assert!(ended.is_none());
    }

    // --- parse_jsonl_session edge cases ---

    #[test]
    fn parse_jsonl_session_file_not_found() {
        let result = parse_jsonl_session(
            std::path::Path::new("/nonexistent/file.jsonl"),
            "sess",
            AgentType::ClaudeCode,
        );
        assert!(matches!(result, Err(SessionParseError::FileNotFound(_))));
    }

    #[test]
    fn parse_jsonl_session_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        fs::write(&path, "").unwrap();

        let parsed = parse_jsonl_session(&path, "empty", AgentType::ClaudeCode).unwrap();
        assert_eq!(parsed.messages.len(), 0);
        assert_eq!(parsed.total_turns, 0);
    }

    #[test]
    fn parse_jsonl_session_skips_invalid_json_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mixed.jsonl");
        let content = "not json\n{\"role\":\"user\",\"content\":\"Hello\"}\n{invalid}";
        fs::write(&path, content).unwrap();

        let parsed = parse_jsonl_session(&path, "mixed", AgentType::ClaudeCode).unwrap();
        assert_eq!(parsed.messages.len(), 1);
    }

    #[test]
    fn parse_jsonl_session_blank_lines_ignored() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("blanks.jsonl");
        let content = "\n\n{\"role\":\"user\",\"content\":\"Hi\"}\n\n";
        fs::write(&path, content).unwrap();

        let parsed = parse_jsonl_session(&path, "blanks", AgentType::ClaudeCode).unwrap();
        assert_eq!(parsed.total_turns, 1);
    }

    // --- parse_json_session edge cases ---

    #[test]
    fn parse_json_session_file_not_found() {
        let result = parse_json_session(
            std::path::Path::new("/nonexistent/file.json"),
            "sess",
            AgentType::GeminiCli,
        );
        assert!(matches!(result, Err(SessionParseError::FileNotFound(_))));
    }

    #[test]
    fn parse_json_session_invalid_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "not json at all").unwrap();

        let result = parse_json_session(&path, "bad", AgentType::GeminiCli);
        assert!(matches!(result, Err(SessionParseError::JsonError(_))));
    }

    #[test]
    fn parse_json_session_empty_messages() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.json");
        fs::write(&path, r#"{"messages":[]}"#).unwrap();

        let parsed = parse_json_session(&path, "empty", AgentType::OpenCode).unwrap();
        assert_eq!(parsed.messages.len(), 0);
        assert_eq!(parsed.total_turns, 0);
    }

    #[test]
    fn parse_json_session_with_root_tool_calls() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tools.json");
        let content = r#"{
  "messages": [{"role": "user", "content": "Do it"}],
  "tool_calls": [
    {"name": "bash", "timestamp": 1700000000},
    {"name": "edit", "success": true}
  ]
}"#;
        fs::write(&path, content).unwrap();

        let parsed = parse_json_session(&path, "tools", AgentType::OpenCode).unwrap();
        assert_eq!(parsed.tool_executions.len(), 2);
    }

    // --- extract_tool_name ---

    #[test]
    fn extract_tool_name_from_name_field() {
        let value = serde_json::json!({"name": "read_file"});
        assert_eq!(extract_tool_name(&value), Some("read_file".to_string()));
    }

    #[test]
    fn extract_tool_name_from_tool_field() {
        let value = serde_json::json!({"tool": "bash"});
        assert_eq!(extract_tool_name(&value), Some("bash".to_string()));
    }

    #[test]
    fn extract_tool_name_from_function_name() {
        let value = serde_json::json!({"function": {"name": "search"}});
        assert_eq!(extract_tool_name(&value), Some("search".to_string()));
    }

    #[test]
    fn extract_tool_name_returns_none_for_empty() {
        let value = serde_json::json!({});
        assert_eq!(extract_tool_name(&value), None);
    }

    // --- extract_string_field ---

    #[test]
    fn extract_string_field_first_match() {
        let value = serde_json::json!({"name": "a", "tool": "b"});
        assert_eq!(
            extract_string_field(&value, &["tool", "name"]),
            Some("b".to_string())
        );
    }

    #[test]
    fn extract_string_field_from_number() {
        let value = serde_json::json!({"id": 42});
        assert_eq!(
            extract_string_field(&value, &["id"]),
            Some("42".to_string())
        );
    }

    #[test]
    fn extract_string_field_skips_empty_strings() {
        let value = serde_json::json!({"name": "", "fallback": "ok"});
        assert_eq!(
            extract_string_field(&value, &["name", "fallback"]),
            Some("ok".to_string())
        );
    }

    #[test]
    fn extract_string_field_returns_none_when_no_match() {
        let value = serde_json::json!({"other": "value"});
        assert_eq!(extract_string_field(&value, &["name", "tool"]), None);
    }
}
