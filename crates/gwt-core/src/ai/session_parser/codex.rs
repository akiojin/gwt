//! Codex CLI session parser.

use super::{find_session_file, parse_jsonl_session, AgentType, SessionParseError, SessionParser};
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
