//! Claude Code session encoder.
//!
//! Converts ParsedSession to Claude Code JSONL format.
//! Claude Code stores sessions in `~/.claude/projects/{encoded-path}/{session-id}.jsonl`

use chrono::Utc;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use super::{ConversionError, ConversionMetadata, ConversionResult, LossInfo, SessionEncoder};
use crate::ai::session_parser::{AgentType, MessageRole, ParsedSession};
use crate::ai::claude_paths::encode_claude_project_path;

/// Encoder for Claude Code JSONL format.
pub struct ClaudeEncoder {
    home_dir: PathBuf,
}

impl ClaudeEncoder {
    /// Creates a new ClaudeEncoder with the default home directory.
    pub fn new() -> Self {
        Self {
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Creates a new ClaudeEncoder with a custom home directory.
    pub fn with_home_dir(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Returns the base directory for Claude Code projects.
    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".claude").join("projects")
    }

    /// Converts a SessionMessage to a Claude Code JSONL entry.
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

        let ts = timestamp
            .unwrap_or_else(Utc::now)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();

        json!({
            "type": role_str,
            "message": {
                "role": role_str,
                "content": content
            },
            "timestamp": ts
        })
    }
}

impl Default for ClaudeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEncoder for ClaudeEncoder {
    fn target_agent(&self) -> AgentType {
        AgentType::ClaudeCode
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

        // Tool executions cannot be fully preserved in Claude Code format
        // as they're embedded differently
        if !session.tool_executions.is_empty() {
            loss_info.dropped_tool_results = session.tool_executions.len();
            loss_info
                .lost_metadata_fields
                .push("tool_execution_details".to_string());
        }

        // Write the JSONL file
        let file = File::create(&output_path)?;
        let mut writer = BufWriter::new(file);

        let mut converted_count = 0;
        for message in &session.messages {
            let entry = self.message_to_jsonl(message.role, &message.content, message.timestamp);
            let line = serde_json::to_string(&entry)?;
            writeln!(writer, "{}", line)?;
            converted_count += 1;
        }

        // Calculate dropped messages (if any failed to convert)
        loss_info.dropped_messages = session.messages.len().saturating_sub(converted_count);
        loss_info.build_summary();

        let converted_at = Utc::now();
        let metadata = ConversionMetadata::new(session, &loss_info, converted_at);

        Ok(ConversionResult {
            output_path,
            new_session_id,
            target_agent: AgentType::ClaudeCode,
            loss_info,
            metadata,
        })
    }

    fn output_path(&self, worktree_path: &Path, session_id: &str) -> PathBuf {
        let encoded = encode_claude_project_path(worktree_path);
        self.base_dir()
            .join(encoded)
            .join(format!("{}.jsonl", session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::session_parser::{SessionMessage, ToolExecution};
    use tempfile::tempdir;

    fn sample_session() -> ParsedSession {
        ParsedSession {
            session_id: "old-session-123".to_string(),
            agent_type: AgentType::CodexCli,
            messages: vec![
                SessionMessage {
                    role: MessageRole::User,
                    content: "Hello, please help me".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::Assistant,
                    content: "Of course! How can I assist you?".to_string(),
                    timestamp: None,
                },
            ],
            tool_executions: vec![ToolExecution {
                tool_name: "read_file".to_string(),
                success: true,
                timestamp: None,
            }],
            started_at: None,
            last_updated_at: None,
            total_turns: 2,
        }
    }

    #[test]
    fn test_encode_session() {
        let dir = tempdir().unwrap();
        let encoder = ClaudeEncoder::with_home_dir(dir.path().to_path_buf());

        let session = sample_session();
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();

        let result = encoder.encode(&session, &worktree).unwrap();

        assert!(result.output_path.exists());
        assert_eq!(result.target_agent, AgentType::ClaudeCode);
        assert!(!result.new_session_id.is_empty());

        // Tool executions should be noted as lost
        assert_eq!(result.loss_info.dropped_tool_results, 1);

        // Read and verify the output file
        let content = fs::read_to_string(&result.output_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Verify first line is valid JSON with expected structure
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["type"], "user");
        assert!(first["message"]["content"]
            .as_str()
            .unwrap()
            .contains("Hello"));
    }

    #[test]
    fn test_output_path() {
        let dir = tempdir().unwrap();
        let encoder = ClaudeEncoder::with_home_dir(dir.path().to_path_buf());

        let worktree = PathBuf::from("/home/user/project");
        let path = encoder.output_path(&worktree, "test-session-id");

        assert!(path.to_string_lossy().contains(".claude"));
        assert!(path.to_string_lossy().contains("projects"));
        assert!(path.to_string_lossy().ends_with(".jsonl"));
    }
}
