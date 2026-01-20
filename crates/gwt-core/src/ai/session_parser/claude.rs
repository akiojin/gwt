//! Claude Code session parser.

use super::{find_session_file, parse_jsonl_session, AgentType, SessionParseError, SessionParser};
use std::path::PathBuf;

pub struct ClaudeSessionParser {
    home_dir: PathBuf,
}

impl ClaudeSessionParser {
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    pub fn with_default_home() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".claude").join("projects")
    }
}

impl SessionParser for ClaudeSessionParser {
    fn parse(&self, session_id: &str) -> Result<super::ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        parse_jsonl_session(&path, session_id, AgentType::ClaudeCode)
    }

    fn agent_type(&self) -> AgentType {
        AgentType::ClaudeCode
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        let root = self.base_dir();
        if let Some(found) = find_session_file(&root, session_id, &["jsonl", "json"]) {
            return found;
        }
        root.join(format!("{}.jsonl", session_id))
    }
}
