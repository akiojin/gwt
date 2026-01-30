//! Codex CLI session parser.

use super::{
    find_session_file, parse_jsonl_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};
use chrono::{DateTime, Utc};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

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
    use super::*;
    use tempfile::tempdir;

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
}
