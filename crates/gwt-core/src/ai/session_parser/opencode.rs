//! OpenCode session parser.

use super::{
    find_session_file, parse_json_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};

pub struct OpenCodeSessionParser {
    home_dir: PathBuf,
}

impl OpenCodeSessionParser {
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    pub fn with_default_home() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".opencode").join("sessions")
    }
}

impl SessionParser for OpenCodeSessionParser {
    fn parse(&self, session_id: &str) -> Result<super::ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        parse_json_session(&path, session_id, AgentType::OpenCode)
    }

    fn agent_type(&self) -> AgentType {
        AgentType::OpenCode
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        let root = self.base_dir();
        if let Some(found) = find_session_file(&root, session_id, &["json", "jsonl"]) {
            return found;
        }
        root.join(format!("{}.json", session_id))
    }

    fn list_sessions(&self, _worktree_path: Option<&Path>) -> Vec<SessionListEntry> {
        let root = self.base_dir();
        if !root.exists() {
            return vec![];
        }

        let mut entries = Vec::new();
        let dir_entries = match fs::read_dir(&root) {
            Ok(e) => e,
            Err(_) => return entries,
        };

        for entry in dir_entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "json" && ext != "jsonl" {
                continue;
            }

            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if session_id.is_empty() {
                continue;
            }

            let last_updated = fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from);

            // For JSON files, count messages
            let message_count = fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                .map(|json| count_opencode_messages(&json))
                .unwrap_or(0);

            entries.push(SessionListEntry {
                session_id,
                last_updated,
                message_count,
                file_path: path,
            });
        }

        // Sort by newest first
        entries.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        entries
    }
}

fn count_opencode_messages(json: &serde_json::Value) -> usize {
    if let Some(arr) = json.as_array() {
        return arr.len();
    }
    for key in ["messages", "history", "turns", "events", "conversation"] {
        if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
            return arr.len();
        }
    }
    0
}
