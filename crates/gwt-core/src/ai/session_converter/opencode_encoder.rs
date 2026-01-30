//! OpenCode session encoder.
//!
//! Converts ParsedSession to OpenCode JSON format.
//! OpenCode stores sessions in `~/.opencode/sessions/{session-id}.json`

use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ConversionError, ConversionMetadata, ConversionResult, LossInfo, SessionEncoder};
use crate::ai::session_parser::{AgentType, MessageRole, ParsedSession};

/// Encoder for OpenCode JSON format.
pub struct OpenCodeEncoder {
    home_dir: PathBuf,
}

impl OpenCodeEncoder {
    /// Creates a new OpenCodeEncoder with the default home directory.
    pub fn new() -> Self {
        Self {
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Creates a new OpenCodeEncoder with a custom home directory.
    pub fn with_home_dir(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Returns the base directory for OpenCode sessions.
    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".opencode").join("sessions")
    }

    /// Converts a SessionMessage to an OpenCode message entry.
    fn message_to_entry(
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
            "role": role_str,
            "content": content,
            "timestamp": ts
        })
    }
}

impl Default for OpenCodeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEncoder for OpenCodeEncoder {
    fn target_agent(&self) -> AgentType {
        AgentType::OpenCode
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

        // Tool executions cannot be fully preserved
        if !session.tool_executions.is_empty() {
            loss_info.dropped_tool_results = session.tool_executions.len();
            loss_info
                .lost_metadata_fields
                .push("tool_execution_details".to_string());
        }

        // Build the messages array
        let messages: Vec<Value> = session
            .messages
            .iter()
            .map(|msg| self.message_to_entry(msg.role, &msg.content, msg.timestamp))
            .collect();

        let converted_count = messages.len();

        // Build the session object
        let session_obj = json!({
            "id": new_session_id,
            "history": messages,
            "created_at": session.started_at.unwrap_or_else(Utc::now).timestamp_millis(),
            "updated_at": Utc::now().timestamp_millis(),
            "project_path": worktree_path.to_string_lossy(),
            "metadata": {
                "converted_from": session.agent_type.display_name(),
                "original_session_id": session.session_id
            }
        });

        // Write the JSON file
        let content = serde_json::to_string_pretty(&session_obj)?;
        fs::write(&output_path, content)?;

        // Calculate dropped messages
        loss_info.dropped_messages = session.messages.len().saturating_sub(converted_count);
        loss_info.build_summary();

        let converted_at = Utc::now();
        let metadata = ConversionMetadata::new(session, &loss_info, converted_at);

        Ok(ConversionResult {
            output_path,
            new_session_id,
            target_agent: AgentType::OpenCode,
            loss_info,
            metadata,
        })
    }

    fn output_path(&self, _worktree_path: &Path, session_id: &str) -> PathBuf {
        self.base_dir().join(format!("{}.json", session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::session_parser::{SessionMessage, ToolExecution};
    use tempfile::tempdir;

    fn sample_session() -> ParsedSession {
        ParsedSession {
            session_id: "gemini-session-abc".to_string(),
            agent_type: AgentType::GeminiCli,
            messages: vec![
                SessionMessage {
                    role: MessageRole::User,
                    content: "How do I use iterators in Rust?".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::Assistant,
                    content: "Iterators in Rust provide a way to process sequences...".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::User,
                    content: "Show me an example".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::Assistant,
                    content: "Here's an example: vec![1, 2, 3].iter().map(|x| x * 2)".to_string(),
                    timestamp: None,
                },
            ],
            tool_executions: vec![ToolExecution {
                tool_name: "execute_code".to_string(),
                success: true,
                timestamp: None,
            }],
            started_at: None,
            last_updated_at: None,
            total_turns: 4,
        }
    }

    #[test]
    fn test_encode_session() {
        let dir = tempdir().unwrap();
        let encoder = OpenCodeEncoder::with_home_dir(dir.path().to_path_buf());

        let session = sample_session();
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();

        let result = encoder.encode(&session, &worktree).unwrap();

        assert!(result.output_path.exists());
        assert_eq!(result.target_agent, AgentType::OpenCode);
        assert!(!result.new_session_id.is_empty());

        // Tool executions should be noted as lost
        assert_eq!(result.loss_info.dropped_tool_results, 1);

        // Read and verify the output file
        let content = fs::read_to_string(&result.output_path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert!(parsed["id"].is_string());
        // OpenCode uses "history" key instead of "messages"
        assert!(parsed["history"].is_array());
        assert_eq!(parsed["history"].as_array().unwrap().len(), 4);

        // Verify message structure
        let first_msg = &parsed["history"][0];
        assert_eq!(first_msg["role"], "user");
        assert!(first_msg["content"].as_str().unwrap().contains("iterators"));

        // Verify metadata
        assert_eq!(parsed["metadata"]["converted_from"], "Gemini CLI");
        assert_eq!(
            parsed["metadata"]["original_session_id"],
            "gemini-session-abc"
        );

        // Verify project_path
        assert!(parsed["project_path"].is_string());
    }

    #[test]
    fn test_output_path() {
        let dir = tempdir().unwrap();
        let encoder = OpenCodeEncoder::with_home_dir(dir.path().to_path_buf());

        let worktree = PathBuf::from("/home/user/project");
        let path = encoder.output_path(&worktree, "test-session-id");

        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".opencode"));
        assert!(path_str.contains("sessions"));
        assert!(path_str.ends_with(".json"));
    }

    #[test]
    fn test_timestamp_format() {
        let encoder = OpenCodeEncoder::new();

        let entry = encoder.message_to_entry(MessageRole::User, "Test", None);

        // Should be a number (milliseconds timestamp)
        assert!(entry["timestamp"].is_number());
    }
}
