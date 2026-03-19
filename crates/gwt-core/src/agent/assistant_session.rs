#![allow(dead_code)]
//! Assistant Mode session state

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{conversation::Conversation, session::SessionStatus, types::SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantSession {
    pub id: SessionId,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repository_path: PathBuf,
    pub base_branch: String,
    pub conversation: Conversation,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
}

impl AssistantSession {
    pub fn new(repository_path: PathBuf, base_branch: String) -> Self {
        let now = Utc::now();
        Self {
            id: SessionId::new(),
            status: SessionStatus::Active,
            created_at: now,
            updated_at: now,
            repository_path,
            base_branch,
            conversation: Conversation::new(),
            llm_call_count: 0,
            estimated_tokens: 0,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assistant_session_new() {
        let session = AssistantSession::new(PathBuf::from("/repo"), "main".to_string());
        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(session.base_branch, "main");
        assert_eq!(session.llm_call_count, 0);
        assert_eq!(session.estimated_tokens, 0);
    }
}
