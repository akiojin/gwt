//! OpenCode session parser.

use super::{find_session_file, parse_json_session, AgentType, SessionParseError, SessionParser};
use std::path::PathBuf;

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
}
