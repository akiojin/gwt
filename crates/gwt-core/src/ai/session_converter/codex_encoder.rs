//! Codex CLI session encoder.
//!
//! Converts ParsedSession to Codex CLI JSONL format.
//! Codex CLI stores sessions in `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`

use chrono::{Datelike, Utc};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use super::{ConversionError, ConversionMetadata, ConversionResult, LossInfo, SessionEncoder};
use crate::ai::session_parser::{AgentType, MessageRole, ParsedSession};

/// Encoder for Codex CLI JSONL format.
pub struct CodexEncoder {
    home_dir: PathBuf,
}

impl CodexEncoder {
    /// Creates a new CodexEncoder with the default home directory.
    pub fn new() -> Self {
        Self {
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Creates a new CodexEncoder with a custom home directory.
    pub fn with_home_dir(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Returns the base directory for Codex CLI sessions.
    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".codex").join("sessions")
    }

    /// Creates the metadata header for a Codex session file.
    fn create_header(
        &self,
        session_id: &str,
        worktree_path: &Path,
        timestamp: &str,
    ) -> Value {
        let mut payload = serde_json::Map::new();

        payload.insert("id".to_string(), json!(session_id));
        payload.insert("timestamp".to_string(), json!(timestamp));
        payload.insert(
            "cwd".to_string(),
            json!(worktree_path.to_string_lossy().to_string()),
        );

        payload.insert("originator".to_string(), json!("codex_cli_rs"));
        payload.insert("cli_version".to_string(), json!("0.0.0"));
        payload.insert("source".to_string(), json!("cli"));
        payload.insert("model_provider".to_string(), json!("openai"));
        payload.insert("base_instructions".to_string(), json!({ "text": "" }));
        payload.insert("instructions".to_string(), json!(""));

        if let Some(template) = self.load_template_payload() {
            apply_template_fields(&mut payload, &template);
        }

        json!({
            "timestamp": timestamp,
            "type": "session_meta",
            "payload": payload
        })
    }

    /// Converts a SessionMessage to a Codex CLI JSONL entry.
    fn message_to_jsonl(
        &self,
        role: MessageRole,
        content: &str,
        timestamp: Option<chrono::DateTime<Utc>>,
    ) -> Value {
        let role_str = match role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };

        let ts = format_codex_timestamp(timestamp.unwrap_or_else(Utc::now));
        let content_type = match role {
            MessageRole::User => "input_text",
            MessageRole::Assistant => "output_text",
        };

        json!({
            "timestamp": ts,
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": role_str,
                "content": [{
                    "type": content_type,
                    "text": content
                }]
            }
        })
    }

    fn user_event_to_jsonl(&self, content: &str, timestamp: &str) -> Value {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": content,
                "images": []
            }
        })
    }

    /// Generates a Codex-style session filename.
    fn generate_filename(&self, session_id: &str) -> String {
        // Codex uses format: rollout-{uuid}.jsonl
        format!("rollout-{}.jsonl", session_id)
    }
}

impl Default for CodexEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEncoder for CodexEncoder {
    fn target_agent(&self) -> AgentType {
        AgentType::CodexCli
    }

    fn encode(
        &self,
        session: &ParsedSession,
        worktree_path: &Path,
    ) -> Result<ConversionResult, ConversionError> {
        let new_session_id = self.generate_session_id();
        let output_path = self.output_path(worktree_path, &new_session_id);

        // Ensure the output directory exists
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Track loss info
        let mut loss_info = LossInfo::default();

        // Codex format doesn't preserve full tool execution details
        if !session.tool_executions.is_empty() {
            loss_info.dropped_tool_results = session.tool_executions.len();
            loss_info
                .lost_metadata_fields
                .push("tool_execution_details".to_string());
        }

        // Write the JSONL file
        let file = File::create(&output_path)?;
        let mut writer = BufWriter::new(file);

        let header_timestamp = format_codex_timestamp(Utc::now());

        // Write the header line first
        let header = self.create_header(&new_session_id, worktree_path, &header_timestamp);
        let header_line = serde_json::to_string(&header)?;
        writeln!(writer, "{}", header_line)?;

        // Write messages
        let mut converted_count = 0;
        for message in &session.messages {
            let entry = self.message_to_jsonl(message.role, &message.content, message.timestamp);
            let line = serde_json::to_string(&entry)?;
            writeln!(writer, "{}", line)?;
            converted_count += 1;

            if matches!(message.role, MessageRole::User) {
                let ts = format_codex_timestamp(message.timestamp.unwrap_or_else(Utc::now));
                let event = self.user_event_to_jsonl(&message.content, &ts);
                let event_line = serde_json::to_string(&event)?;
                writeln!(writer, "{}", event_line)?;
            }
        }

        // Calculate dropped messages
        loss_info.dropped_messages = session.messages.len().saturating_sub(converted_count);
        loss_info.build_summary();

        let converted_at = Utc::now();
        let metadata = ConversionMetadata::new(session, &loss_info, converted_at);

        Ok(ConversionResult {
            output_path,
            new_session_id,
            target_agent: AgentType::CodexCli,
            loss_info,
            metadata,
        })
    }

    fn output_path(&self, _worktree_path: &Path, session_id: &str) -> PathBuf {
        // Codex uses date-based directory structure
        let now = Utc::now();
        self.base_dir()
            .join(format!("{:04}", now.year()))
            .join(format!("{:02}", now.month()))
            .join(format!("{:02}", now.day()))
            .join(self.generate_filename(session_id))
    }
}

fn format_codex_timestamp(timestamp: chrono::DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn apply_template_fields(payload: &mut serde_json::Map<String, Value>, template: &Value) {
    let Some(template_obj) = template.as_object() else {
        return;
    };

    if let Some(originator) = template_obj.get("originator").and_then(|v| v.as_str()) {
        payload.insert("originator".to_string(), json!(originator));
    }
    if let Some(cli_version) = template_obj.get("cli_version").and_then(|v| v.as_str()) {
        payload.insert("cli_version".to_string(), json!(cli_version));
    }
    if let Some(source) = template_obj.get("source").and_then(|v| v.as_str()) {
        payload.insert("source".to_string(), json!(source));
    }
    if let Some(provider) = template_obj.get("model_provider").and_then(|v| v.as_str()) {
        payload.insert("model_provider".to_string(), json!(provider));
    }
    if let Some(base_instructions) = template_obj.get("base_instructions") {
        payload.insert("base_instructions".to_string(), base_instructions.clone());
    }
    if let Some(instructions) = template_obj.get("instructions") {
        payload.insert("instructions".to_string(), instructions.clone());
    }
    if let Some(git) = template_obj.get("git") {
        payload.insert("git".to_string(), git.clone());
    }
}

impl CodexEncoder {
    fn load_template_payload(&self) -> Option<Value> {
        let base_dir = self.base_dir();
        if !base_dir.exists() {
            return None;
        }

        let mut stack = vec![base_dir];
        let mut latest: Option<(std::time::SystemTime, Value)> = None;

        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let metadata = entry.metadata().ok()?;
                if metadata.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !metadata.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
                if ext != "jsonl" && ext != "json" {
                    continue;
                }
                let modified = match metadata.modified() {
                    Ok(modified) => modified,
                    Err(_) => continue,
                };
                let content = match fs::read_to_string(&path) {
                    Ok(content) => content,
                    Err(_) => continue,
                };
                let line = match content.lines().find(|line| !line.trim().is_empty()) {
                    Some(line) => line,
                    None => continue,
                };
                let value: Value = match serde_json::from_str(line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
                    continue;
                }
                let payload = match value.get("payload") {
                    Some(payload) => payload.clone(),
                    None => continue,
                };
                let originator = payload
                    .get("originator")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if originator == "gwt" {
                    continue;
                }
                let instructions = payload
                    .get("instructions")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let base_text = payload
                    .get("base_instructions")
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if instructions.is_empty() && base_text.is_empty() {
                    continue;
                }

                let should_replace = latest
                    .as_ref()
                    .map(|(prev, _)| modified > *prev)
                    .unwrap_or(true);
                if should_replace {
                    latest = Some((modified, payload));
                }
            }
        }

        latest.map(|(_, payload)| payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::session_parser::{SessionMessage, ToolExecution};
    use tempfile::tempdir;

    fn sample_session() -> ParsedSession {
        ParsedSession {
            session_id: "old-session-456".to_string(),
            agent_type: AgentType::ClaudeCode,
            messages: vec![
                SessionMessage {
                    role: MessageRole::User,
                    content: "What is Rust?".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::Assistant,
                    content: "Rust is a systems programming language.".to_string(),
                    timestamp: None,
                },
            ],
            tool_executions: vec![
                ToolExecution {
                    tool_name: "bash".to_string(),
                    success: true,
                    timestamp: None,
                },
                ToolExecution {
                    tool_name: "write_file".to_string(),
                    success: false,
                    timestamp: None,
                },
            ],
            started_at: None,
            last_updated_at: None,
            total_turns: 2,
        }
    }

    #[test]
    fn test_encode_session() {
        let dir = tempdir().unwrap();
        let encoder = CodexEncoder::with_home_dir(dir.path().to_path_buf());

        let session = sample_session();
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();

        let result = encoder.encode(&session, &worktree).unwrap();

        assert!(result.output_path.exists());
        assert_eq!(result.target_agent, AgentType::CodexCli);
        assert!(!result.new_session_id.is_empty());

        // Tool executions should be noted as lost
        assert_eq!(result.loss_info.dropped_tool_results, 2);

        // Read and verify the output file
        let content = fs::read_to_string(&result.output_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // Session meta + 2 messages + user event
        assert_eq!(lines.len(), 4);

        // Verify header line
        let header: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header["type"], "session_meta");
        assert!(header["payload"]["id"].is_string());
        assert!(header["payload"]["cwd"].is_string());

        // Verify first message
        let first_msg: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first_msg["type"], "response_item");
        assert_eq!(first_msg["payload"]["type"], "message");
        assert_eq!(first_msg["payload"]["role"], "user");

        let event_msg: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(event_msg["type"], "event_msg");
        assert_eq!(event_msg["payload"]["type"], "user_message");

        let second_msg: Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(second_msg["type"], "response_item");
        assert_eq!(second_msg["payload"]["role"], "assistant");
    }

    #[test]
    fn test_output_path_date_structure() {
        let dir = tempdir().unwrap();
        let encoder = CodexEncoder::with_home_dir(dir.path().to_path_buf());

        let worktree = PathBuf::from("/home/user/project");
        let path = encoder.output_path(&worktree, "test-session-id");

        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".codex"));
        assert!(path_str.contains("sessions"));
        // Should contain year/month/day structure
        assert!(path_str.contains("rollout-"));
        assert!(path_str.ends_with(".jsonl"));
    }

    #[test]
    fn test_generate_filename() {
        let encoder = CodexEncoder::new();
        let filename = encoder.generate_filename("abc-123");
        assert_eq!(filename, "rollout-abc-123.jsonl");
    }
}
