//! Codex CLI session parser.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};

use super::{
    find_session_file, parse_jsonl_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};

pub struct CodexSessionParser {
    home_dir: PathBuf,
}

impl CodexSessionParser {
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    pub fn with_default_home() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".codex").join("sessions")
    }
}

impl SessionParser for CodexSessionParser {
    fn parse(&self, session_id: &str) -> Result<super::ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        parse_jsonl_session(&path, session_id, AgentType::CodexCli)
    }

    fn agent_type(&self) -> AgentType {
        AgentType::CodexCli
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        let root = self.base_dir();
        if let Some(found) = find_session_file(&root, session_id, &["jsonl", "json"]) {
            return found;
        }
        if let Some(found) = find_codex_session_file(&root, session_id) {
            return found;
        }
        root.join(format!("{}.jsonl", session_id))
    }

    fn list_sessions(&self, worktree_path: Option<&Path>) -> Vec<SessionListEntry> {
        let root = self.base_dir();
        if !root.exists() {
            return vec![];
        }

        let mut entries = Vec::new();
        let mut stack = vec![root.clone()];

        while let Some(dir) = stack.pop() {
            let dir_entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in dir_entries.flatten() {
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.is_dir() {
                    stack.push(path);
                    continue;
                }

                if !metadata.is_file() {
                    continue;
                }

                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext != "jsonl" && ext != "json" {
                    continue;
                }

                // Try to extract session ID from file content (Codex stores ID in payload.id)
                let session_id = match parse_codex_session_id(&path) {
                    Some(id) => id,
                    None => {
                        // Fall back to filename
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string()
                    }
                };

                if session_id.is_empty() {
                    continue;
                }

                // If worktree_path specified, check if session is related
                if let Some(wt_path) = worktree_path {
                    // Check if session file path or content contains worktree reference
                    if !self.is_session_for_worktree(&path, wt_path) {
                        continue;
                    }
                }

                let last_updated = metadata.modified().ok().map(DateTime::<Utc>::from);

                let message_count = fs::read_to_string(&path)
                    .map(|content| content.lines().filter(|l| !l.trim().is_empty()).count())
                    .unwrap_or(0);

                entries.push(SessionListEntry {
                    session_id,
                    last_updated,
                    message_count,
                    file_path: path,
                });
            }
        }

        // Sort by newest first
        entries.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        entries
    }
}

impl CodexSessionParser {
    fn is_session_for_worktree(&self, session_path: &Path, worktree_path: &Path) -> bool {
        // Check if session file contains reference to worktree path (in payload.cwd)
        let file = match fs::File::open(session_path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let reader = BufReader::new(file);

        for line in reader.lines().take(10) {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(payload) = value.get("payload") {
                    if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                        if cwd.contains(&worktree_path.to_string_lossy().to_string()) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }
}

fn find_codex_session_file(root: &Path, session_id: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if metadata.is_file() {
                let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
                if ext != "jsonl" && ext != "json" {
                    continue;
                }
                if let Some(id) = parse_codex_session_id(&path) {
                    if id == session_id {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn parse_codex_session_id(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        let payload = value.get("payload")?;
        let id = payload.get("id")?.as_str()?;
        if !id.trim().is_empty() {
            return Some(id.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_session_file_path_finds_payload_id() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("random.jsonl");
        let content = r#"{"payload":{"id":"sess-123","cwd":"/repo/wt"}}"#;
        fs::write(&path, content).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let resolved = parser.session_file_path("sess-123");
        assert_eq!(resolved, path);
    }

    // --- agent_type ---

    #[test]
    fn agent_type_returns_codex_cli() {
        let dir = tempdir().unwrap();
        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        assert_eq!(parser.agent_type(), AgentType::CodexCli);
    }

    // --- session_file_path edge cases ---

    #[test]
    fn session_file_path_returns_default_when_no_session_dir() {
        let dir = tempdir().unwrap();
        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let path = parser.session_file_path("nonexistent");
        assert!(path.to_string_lossy().ends_with("nonexistent.jsonl"));
    }

    #[test]
    fn session_file_path_finds_by_filename_match() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("my-sess.jsonl");
        fs::write(&path, r#"{"type":"message"}"#).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let resolved = parser.session_file_path("my-sess");
        assert_eq!(resolved, path);
    }

    // --- list_sessions ---

    #[test]
    fn list_sessions_returns_empty_when_no_dir() {
        let dir = tempdir().unwrap();
        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        assert!(parser.list_sessions(None).is_empty());
    }

    #[test]
    fn list_sessions_collects_from_nested_dirs() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        let sub = sessions.join("sub");
        fs::create_dir_all(&sub).unwrap();

        fs::write(
            sessions.join("root.jsonl"),
            r#"{"payload":{"id":"sess-1"}}"#,
        )
        .unwrap();
        fs::write(sub.join("nested.jsonl"), r#"{"payload":{"id":"sess-2"}}"#).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let all = parser.list_sessions(None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_sessions_skips_non_session_files() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();

        fs::write(sessions.join("sess.jsonl"), r#"{"payload":{"id":"s1"}}"#).unwrap();
        fs::write(sessions.join("readme.txt"), "not a session").unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let all = parser.list_sessions(None);
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn list_sessions_sorted_newest_first() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();

        fs::write(sessions.join("old.jsonl"), r#"{"payload":{"id":"old"}}"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(sessions.join("new.jsonl"), r#"{"payload":{"id":"new"}}"#).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let all = parser.list_sessions(None);
        assert_eq!(all[0].session_id, "new");
    }

    // --- is_session_for_worktree ---

    #[test]
    fn is_session_for_worktree_matches_cwd() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("sess.jsonl");
        fs::write(
            &path,
            r#"{"payload":{"id":"s1","cwd":"/repo/worktrees/feature-x"}}"#,
        )
        .unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let wt_path = std::path::PathBuf::from("/repo/worktrees/feature-x");
        assert!(parser.is_session_for_worktree(&path, &wt_path));
    }

    #[test]
    fn is_session_for_worktree_no_match() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("sess.jsonl");
        fs::write(&path, r#"{"payload":{"id":"s1","cwd":"/other/path"}}"#).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let wt_path = std::path::PathBuf::from("/repo/worktrees/feature-x");
        assert!(!parser.is_session_for_worktree(&path, &wt_path));
    }

    #[test]
    fn is_session_for_worktree_nonexistent_file() {
        let dir = tempdir().unwrap();
        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let wt_path = std::path::PathBuf::from("/repo");
        assert!(!parser.is_session_for_worktree(&dir.path().join("nonexistent.jsonl"), &wt_path));
    }

    // --- parse_codex_session_id ---

    #[test]
    fn parse_codex_session_id_extracts_from_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        fs::write(&path, r#"{"payload":{"id":"my-session-id","cwd":"/repo"}}"#).unwrap();

        let id = parse_codex_session_id(&path);
        assert_eq!(id, Some("my-session-id".to_string()));
    }

    #[test]
    fn parse_codex_session_id_returns_none_for_no_payload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        fs::write(&path, r#"{"type":"message","content":"hi"}"#).unwrap();

        let id = parse_codex_session_id(&path);
        assert!(id.is_none());
    }

    #[test]
    fn parse_codex_session_id_returns_none_for_empty_id() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        fs::write(&path, r#"{"payload":{"id":"","cwd":"/repo"}}"#).unwrap();

        let id = parse_codex_session_id(&path);
        assert!(id.is_none());
    }

    #[test]
    fn parse_codex_session_id_returns_none_for_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        fs::write(&path, "").unwrap();

        let id = parse_codex_session_id(&path);
        assert!(id.is_none());
    }

    // --- parse ---

    #[test]
    fn parse_returns_error_for_missing_file() {
        let dir = tempdir().unwrap();
        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let result = parser.parse("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn parse_reads_jsonl_session() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();

        let content = r#"{"payload":{"id":"test-sess","cwd":"/repo"}}
{"type":"message","role":"user","content":"Hello","timestamp":1700000000}
{"type":"message","role":"assistant","content":"Hi","timestamp":1700000001}"#;
        fs::write(sessions.join("test-sess.jsonl"), content).unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let parsed = parser.parse("test-sess").unwrap();
        assert_eq!(parsed.session_id, "test-sess");
        assert_eq!(parsed.agent_type, AgentType::CodexCli);
        assert_eq!(parsed.messages.len(), 2);
    }

    // --- list_sessions with worktree filter ---

    #[test]
    fn list_sessions_filters_by_worktree() {
        let dir = tempdir().unwrap();
        let sessions = dir.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();

        // Session with matching worktree
        fs::write(
            sessions.join("match.jsonl"),
            r#"{"payload":{"id":"match","cwd":"/repo/wt/feature-x"}}"#,
        )
        .unwrap();

        // Session with different worktree
        fs::write(
            sessions.join("other.jsonl"),
            r#"{"payload":{"id":"other","cwd":"/other/path"}}"#,
        )
        .unwrap();

        let parser = CodexSessionParser::new(dir.path().to_path_buf());
        let wt_path = std::path::PathBuf::from("/repo/wt/feature-x");
        let filtered = parser.list_sessions(Some(&wt_path));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "match");
    }
}
