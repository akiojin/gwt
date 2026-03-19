//! Structured logging for Project Mode operations
//!
//! Provides JSON Lines formatted log entries for Lead, Coordinator,
//! Developer, and Session events. Log files are written to `~/.gwt/logs/agent.jsonl`.

#![allow(dead_code)]

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Log entry categories
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogCategory {
    LeadLlm,
    Coordinator,
    Developer,
    Session,
}

/// A structured log entry for Project Mode operations
#[derive(Debug, Clone, Serialize)]
pub struct AgentLogEntry {
    pub timestamp: DateTime<Utc>,
    pub category: LogCategory,
    pub session_id: String,
    pub message: String,
    pub metadata: Option<serde_json::Value>,
}

impl AgentLogEntry {
    pub fn new(category: LogCategory, session_id: &str, message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            category,
            session_id: session_id.to_string(),
            message: message.into(),
            metadata: None,
        }
    }
}

/// Format a log entry as JSON Lines
pub fn format_jsonl(entry: &AgentLogEntry) -> String {
    serde_json::to_string(entry).unwrap_or_default()
}

/// Get the log file path for agent operations
pub fn agent_log_path() -> PathBuf {
    let home = dirs::home_dir().expect("failed to determine home directory");
    home.join(".gwt").join("logs").join("agent.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_jsonl_produces_valid_json() {
        let entry = AgentLogEntry::new(LogCategory::LeadLlm, "sess-001", "Test message");
        let line = format_jsonl(&entry);

        // Must be valid JSON
        let parsed: serde_json::Value =
            serde_json::from_str(&line).expect("format_jsonl should produce valid JSON");

        assert_eq!(parsed["session_id"], "sess-001");
        assert_eq!(parsed["message"], "Test message");
        assert_eq!(parsed["category"], "lead_llm");
        assert!(parsed["metadata"].is_null());
    }

    #[test]
    fn agent_log_path_ends_with_agent_jsonl() {
        let path = agent_log_path();
        assert!(
            path.ends_with("agent.jsonl"),
            "expected path to end with agent.jsonl, got: {:?}",
            path
        );
    }

    #[test]
    fn agent_log_entry_new_sets_correct_fields() {
        let entry = AgentLogEntry::new(LogCategory::Developer, "sess-002", "dev task started");

        assert_eq!(entry.session_id, "sess-002");
        assert_eq!(entry.message, "dev task started");
        assert!(entry.metadata.is_none());
        // Timestamp should be recent (within last 5 seconds)
        let now = Utc::now();
        let diff = now.signed_duration_since(entry.timestamp);
        assert!(diff.num_seconds() < 5);
    }

    #[test]
    fn log_category_serializes_correctly() {
        let cases = vec![
            (LogCategory::LeadLlm, "lead_llm"),
            (LogCategory::Coordinator, "coordinator"),
            (LogCategory::Developer, "developer"),
            (LogCategory::Session, "session"),
        ];

        for (category, expected) in cases {
            let json = serde_json::to_string(&category).expect("serialize category");
            assert_eq!(json, format!("\"{}\"", expected));
        }
    }

    #[test]
    fn format_jsonl_with_metadata() {
        let mut entry = AgentLogEntry::new(LogCategory::Session, "sess-003", "session started");
        entry.metadata = Some(serde_json::json!({ "agent_type": "claude" }));

        let line = format_jsonl(&entry);
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");

        assert_eq!(parsed["metadata"]["agent_type"], "claude");
    }

    #[test]
    fn agent_log_path_contains_gwt_logs_directory() {
        let path = agent_log_path();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains(".gwt/logs"),
            "expected path to contain .gwt/logs, got: {}",
            path_str
        );
    }
}
