//! Codex CLI session parser.

use super::{find_session_file, parse_jsonl_session, AgentType, SessionParseError, SessionParser};
use std::path::PathBuf;

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
        root.join(format!("{}.jsonl", session_id))
    }
}
