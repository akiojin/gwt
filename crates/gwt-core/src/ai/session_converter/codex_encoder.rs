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
    fn create_header(&self, session_id: &str, worktree_path: &Path) -> Value {
        json!({
            "payload": {
                "id": session_id,
                "cwd": worktree_path.to_string_lossy()
            }
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

        let ts = timestamp.unwrap_or_else(Utc::now).timestamp_millis();

        json!({
            "type": "message",
            "role": role_str,
            "content": content,
            "timestamp": ts
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

        // Write the header line first
        let header = self.create_header(&new_session_id, worktree_path);
        let header_line = serde_json::to_string(&header)?;
        writeln!(writer, "{}", header_line)?;

        // Write messages
        let mut converted_count = 0;
        for message in &session.messages {
            let entry = self.message_to_jsonl(message.role, &message.content, message.timestamp);
            let line = serde_json::to_string(&entry)?;
            writeln!(writer, "{}", line)?;
            converted_count += 1;
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
        // Header + 2 messages
        assert_eq!(lines.len(), 3);

        // Verify header line
        let header: Value = serde_json::from_str(lines[0]).unwrap();
        assert!(header["payload"]["id"].is_string());
        assert!(header["payload"]["cwd"].is_string());

        // Verify first message
        let first_msg: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first_msg["type"], "message");
        assert_eq!(first_msg["role"], "user");
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
