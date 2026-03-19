//! Gemini CLI session parser.

use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};

use super::{
    find_session_file, parse_json_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};

pub struct GeminiSessionParser {
    home_dir: PathBuf,
}

impl GeminiSessionParser {
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    pub fn with_default_home() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".gemini").join("sessions")
    }
}

impl SessionParser for GeminiSessionParser {
    fn parse(&self, session_id: &str) -> Result<super::ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        parse_json_session(&path, session_id, AgentType::GeminiCli)
    }

    fn agent_type(&self) -> AgentType {
        AgentType::GeminiCli
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

            // For JSON files, count top-level array items or message entries
            let message_count = fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                .map(|json| count_messages(&json))
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

fn count_messages(json: &serde_json::Value) -> usize {
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
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn make_parser(home: &std::path::Path) -> GeminiSessionParser {
        GeminiSessionParser::new(home.to_path_buf())
    }

    fn sessions_dir(home: &std::path::Path) -> std::path::PathBuf {
        home.join(".gemini").join("sessions")
    }

    // --- count_messages ---

    #[test]
    fn count_messages_from_top_level_array() {
        let json: serde_json::Value =
            serde_json::from_str(r#"[{"role":"user"},{"role":"model"}]"#).unwrap();
        assert_eq!(count_messages(&json), 2);
    }

    #[test]
    fn count_messages_from_messages_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"messages":[{"role":"user"}]}"#).unwrap();
        assert_eq!(count_messages(&json), 1);
    }

    #[test]
    fn count_messages_from_history_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"history":[{"a":1},{"a":2},{"a":3}]}"#).unwrap();
        assert_eq!(count_messages(&json), 3);
    }

    #[test]
    fn count_messages_from_turns_key() {
        let json: serde_json::Value = serde_json::from_str(r#"{"turns":[{"x":1}]}"#).unwrap();
        assert_eq!(count_messages(&json), 1);
    }

    #[test]
    fn count_messages_from_events_key() {
        let json: serde_json::Value = serde_json::from_str(r#"{"events":[1,2,3,4,5]}"#).unwrap();
        assert_eq!(count_messages(&json), 5);
    }

    #[test]
    fn count_messages_from_conversation_key() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"conversation":[{"m":1}]}"#).unwrap();
        assert_eq!(count_messages(&json), 1);
    }

    #[test]
    fn count_messages_returns_zero_for_empty_object() {
        let json: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(count_messages(&json), 0);
    }

    #[test]
    fn count_messages_returns_zero_for_non_array_value() {
        let json: serde_json::Value = serde_json::from_str(r#"{"messages":"string"}"#).unwrap();
        assert_eq!(count_messages(&json), 0);
    }

    #[test]
    fn count_messages_empty_array() {
        let json: serde_json::Value = serde_json::from_str(r#"[]"#).unwrap();
        assert_eq!(count_messages(&json), 0);
    }

    // --- agent_type ---

    #[test]
    fn agent_type_returns_gemini_cli() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        assert_eq!(parser.agent_type(), AgentType::GeminiCli);
    }

    // --- session_file_path ---

    #[test]
    fn session_file_path_returns_default_when_no_file_exists() {
        let dir = tempdir().unwrap();
        let parser = make_parser(dir.path());
        let path = parser.session_file_path("test-id");
        assert!(path.to_string_lossy().ends_with("test-id.json"));
    }

    #[test]
    fn session_file_path_finds_existing_json_file() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();
        let file_path = sess_dir.join("my-session.json");
        fs::write(&file_path, r#"{"messages":[]}"#).unwrap();

        let parser = make_parser(dir.path());
        let path = parser.session_file_path("my-session");
        assert_eq!(path, file_path);
    }

    #[test]
    fn session_file_path_finds_existing_jsonl_file() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();
        let file_path = sess_dir.join("gemini-sess.jsonl");
        fs::write(&file_path, r#"{"role":"user","content":"hi"}"#).unwrap();

        let parser = make_parser(dir.path());
        let path = parser.session_file_path("gemini-sess");
        assert_eq!(path, file_path);
    }

    // --- list_sessions ---

    #[test]
    fn list_sessions_returns_empty_when_dir_missing() {
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
    fn list_sessions_collects_json_and_jsonl_files() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("a.json"), r#"[]"#).unwrap();
        fs::write(sess_dir.join("b.jsonl"), r#"{"role":"user"}"#).unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn list_sessions_skips_non_session_files() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("session.json"), r#"[]"#).unwrap();
        fs::write(sess_dir.join("notes.txt"), "notes").unwrap();
        fs::write(sess_dir.join("config.yaml"), "key: val").unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session");
    }

    #[test]
    fn list_sessions_ignores_worktree_path_parameter() {
        // Gemini parser ignores worktree_path filtering (flat session dir)
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("s1.json"), r#"[]"#).unwrap();

        let parser = make_parser(dir.path());
        let worktree = std::path::PathBuf::from("/some/worktree");
        let sessions = parser.list_sessions(Some(&worktree));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn list_sessions_sorted_newest_first() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("old.json"), r#"[]"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(sess_dir.join("new.json"), r#"[]"#).unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions[0].session_id, "new");
    }

    #[test]
    fn list_sessions_message_count_from_messages_key() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(
            sess_dir.join("s.json"),
            r#"{"messages":[{"role":"user"},{"role":"model"}]}"#,
        )
        .unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions[0].message_count, 2);
    }

    #[test]
    fn list_sessions_message_count_zero_for_invalid_json() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        fs::write(sess_dir.join("bad.json"), "not valid json!").unwrap();

        let parser = make_parser(dir.path());
        let sessions = parser.list_sessions(None);
        assert_eq!(sessions[0].message_count, 0);
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
    fn parse_reads_json_session_with_messages() {
        let dir = tempdir().unwrap();
        let sess_dir = sessions_dir(dir.path());
        fs::create_dir_all(&sess_dir).unwrap();

        let content = r#"{
  "messages": [
    {"role": "user", "content": "Explain this code", "timestamp": 1700000000},
    {"role": "assistant", "content": "This code does...", "timestamp": 1700000001}
  ]
}"#;
        fs::write(sess_dir.join("gemini-test.json"), content).unwrap();

        let parser = make_parser(dir.path());
        let parsed = parser.parse("gemini-test").unwrap();
        assert_eq!(parsed.session_id, "gemini-test");
        assert_eq!(parsed.agent_type, AgentType::GeminiCli);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.total_turns, 2);
    }
}
