//! Gemini CLI session encoder.
//!
//! Converts ParsedSession to Gemini CLI JSON format.
//! Gemini CLI stores sessions in `~/.gemini/sessions/{session-id}.json`

use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ConversionError, ConversionMetadata, ConversionResult, LossInfo, SessionEncoder};
use crate::ai::session_parser::{AgentType, MessageRole, ParsedSession};

/// Encoder for Gemini CLI JSON format.
pub struct GeminiEncoder {
    home_dir: PathBuf,
}

impl GeminiEncoder {
    /// Creates a new GeminiEncoder with the default home directory.
    pub fn new() -> Self {
        Self {
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Creates a new GeminiEncoder with a custom home directory.
    pub fn with_home_dir(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Returns the base directory for Gemini CLI sessions.
    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".gemini").join("sessions")
    }

    /// Converts a SessionMessage to a Gemini CLI message entry.
    fn message_to_entry(
        &self,
        role: MessageRole,
        content: &str,
        timestamp: Option<chrono::DateTime<Utc>>,
    ) -> Value {
        let role_str = match role {
            MessageRole::User => "user",
            MessageRole::Assistant => "model",
        };

        let ts = timestamp
            .unwrap_or_else(Utc::now)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();

        json!({
            "role": role_str,
            "parts": [
                {
                    "text": content
                }
            ],
            "timestamp": ts
        })
    }
}

impl Default for GeminiEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEncoder for GeminiEncoder {
    fn target_agent(&self) -> AgentType {
        AgentType::GeminiCli
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
            "messages": messages,
            "created_at": session.started_at
                .unwrap_or_else(Utc::now)
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string(),
            "updated_at": Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "metadata": {
                "converted_from": session.agent_type.display_name(),
                "original_session_id": session.session_id,
                "worktree": worktree_path.to_string_lossy()
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
            target_agent: AgentType::GeminiCli,
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
            session_id: "claude-session-789".to_string(),
            agent_type: AgentType::ClaudeCode,
            messages: vec![
                SessionMessage {
                    role: MessageRole::User,
                    content: "Explain async/await".to_string(),
                    timestamp: None,
                },
                SessionMessage {
                    role: MessageRole::Assistant,
                    content: "Async/await is a pattern for handling asynchronous operations."
                        .to_string(),
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
        let encoder = GeminiEncoder::with_home_dir(dir.path().to_path_buf());

        let session = sample_session();
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();

        let result = encoder.encode(&session, &worktree).unwrap();

        assert!(result.output_path.exists());
        assert_eq!(result.target_agent, AgentType::GeminiCli);
        assert!(!result.new_session_id.is_empty());

        // Tool executions should be noted as lost
        assert_eq!(result.loss_info.dropped_tool_results, 1);

        // Read and verify the output file
        let content = fs::read_to_string(&result.output_path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        assert!(parsed["id"].is_string());
        assert!(parsed["messages"].is_array());
        assert_eq!(parsed["messages"].as_array().unwrap().len(), 2);

        // Verify message structure (Gemini uses "model" instead of "assistant")
        let first_msg = &parsed["messages"][0];
        assert_eq!(first_msg["role"], "user");
        assert!(first_msg["parts"].is_array());
        assert!(first_msg["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("async"));

        // Verify metadata
        assert!(parsed["metadata"]["converted_from"].is_string());
        assert!(parsed["metadata"]["original_session_id"].is_string());
    }

    #[test]
    fn test_output_path() {
        let dir = tempdir().unwrap();
        let encoder = GeminiEncoder::with_home_dir(dir.path().to_path_buf());

        let worktree = PathBuf::from("/home/user/project");
        let path = encoder.output_path(&worktree, "test-session-id");

        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".gemini"));
        assert!(path_str.contains("sessions"));
        assert!(path_str.ends_with(".json"));
    }

    #[test]
    fn test_message_role_mapping() {
        let encoder = GeminiEncoder::new();

        let user_entry = encoder.message_to_entry(MessageRole::User, "Hello", None);
        assert_eq!(user_entry["role"], "user");

        let assistant_entry = encoder.message_to_entry(MessageRole::Assistant, "Hi", None);
        assert_eq!(assistant_entry["role"], "model");
    }
}
