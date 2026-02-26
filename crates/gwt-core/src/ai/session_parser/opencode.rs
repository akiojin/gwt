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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_parser(home: &std::path::Path) -> OpenCodeSessionParser {
        OpenCodeSessionParser::new(home.to_path_buf())
    }

    fn sessions_dir(home: &std::path::Path) -> std::path::PathBuf {
        home.join(".opencode").join("sessions")
    }

    // --- count_opencode_messages ---

    #[test]
    fn count_messages_from_top_level_array() {
        let json: serde_json::Value =
            serde_json::from_str(r#"[{"role":"user"},{"role":"assistant"}]"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 2);
    }

    #[test]
    fn count_messages_from_messages_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"messages":[{"role":"user"}]}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 1);
    }

    #[test]
    fn count_messages_from_history_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"history":[{"a":1},{"a":2},{"a":3}]}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 3);
    }

    #[test]
    fn count_messages_from_turns_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"turns":[{"x":1}]}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 1);
    }

    #[test]
    fn count_messages_from_events_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"events":[1,2,3,4]}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 4);
    }

    #[test]
    fn count_messages_from_conversation_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"conversation":[{"m":1},{"m":2}]}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 2);
    }

    #[test]
    fn count_messages_returns_zero_for_empty_object() {
        let json: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 0);
    }

    #[test]
    fn count_messages_returns_zero_for_non_array_messages() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"messages":"not an array"}"#).unwrap();
        assert_eq!(count_opencode_messages(&json), 0);
    }

    // --- agent_type ---

    #[test]
    fn agent_type_returns_opencode() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        assert_eq!(parser.agent_type(), AgentType::OpenCode);
    }

    // --- session_file_path ---

    #[test]
    fn session_file_path_returns_default_when_no_file_exists() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        let path = parser.session_file_path("abc-123");
        assert!(path.to_string_lossy().contains("abc-123.json"));
    }

    #[test]
    fn session_file_path_finds_existing_json_file() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();
        let file_path = sess_dir.join("my-sess.json");
        fs::write(&file_path, r#"{"messages":[]}"#).unwrap();

        let parser = make_parser(dir.path());
        let path = parser.session_file_path("my-sess");
        assert_eq!(path, file_path);
    }

    // --- list_sessions ---

    #[test]
    fn list_sessions_returns_empty_when_no_dir() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        assert!(parser.list_sessions(None).is_empty());
    }

    #[test]
    fn list_sessions_returns_empty_for_empty_dir() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        let parser = make_parser(dir.path());
        assert!(parser.list_sessions(None).is_empty());
    }

    #[test]
    fn list_sessions_finds_json_files() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(
            sess_dir.join("sess-1.json"),
            r#"[{"role":"user","content":"hi"}]"#,
        )
        .unwrap();
        fs::write(
            sess_dir.join("sess-2.jsonl"),
            r#"{"role":"user","content":"hello"}"#,
        )
        .unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn list_sessions_skips_non_json_files() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("sess-1.json"), r#"[]"#).unwrap();
        fs::write(sess_dir.join("readme.txt"), "not a session").unwrap();
        fs::write(sess_dir.join("data.csv"), "a,b,c").unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-1");
    }

    #[test]
    fn list_sessions_skips_directories() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();
        fs::create_dir_all(sess_dir.join("subdir.json")).unwrap();
        fs::write(sess_dir.join("real.json"), r#"[]"#).unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "real");
    }

    #[test]
    fn list_sessions_message_count_from_array() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(
            sess_dir.join("sess.json"),
            r#"[{"role":"user"},{"role":"assistant"},{"role":"user"}]"#,
        )
        .unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions[0].message_count, 3);
    }

    #[test]
    fn list_sessions_message_count_from_invalid_json() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("broken.json"), "this is not json").unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].message_count, 0);
    }

    #[test]
    fn list_sessions_sorted_newest_first() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        // Write files with slight delay to get different mtimes
        fs::write(sess_dir.join("old.json"), r#"[]"#).unwrap();
        // Force a different mtime by modifying after creation
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(sess_dir.join("new.json"), r#"[]"#).unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 2);
        // Newest first
        assert_eq!(sessions[0].session_id, "new");
        assert_eq!(sessions[1].session_id, "old");
    }

    // --- parse ---

    #[test]
    fn parse_returns_error_for_missing_file() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        let result = parser.parse("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn parse_reads_json_session() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        let content = r#"{
  "messages": [
    {"role": "user", "content": "Hello", "timestamp": 1700000000},
    {"role": "assistant", "content": "Hi there", "timestamp": 1700000001}
  ]
}"#;
        fs::write(sess_dir.join("test-sess.json"), content).unwrap();

        let parser = make_parser(dir.path());
        let parsed = parser.parse("test-sess").unwrap();
        assert_eq!(parsed.session_id, "test-sess");
        assert_eq!(parsed.agent_type, AgentType::OpenCode);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.total_turns, 2);
    }
}
