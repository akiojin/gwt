//! Session state for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::conversation::Conversation;
use super::task::Task;
use super::types::SessionId;
use super::worktree::WorktreeRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub conversation: Conversation,
    pub tasks: Vec<Task>,
    pub worktrees: Vec<WorktreeRef>,
    pub repository_path: PathBuf,
}

impl AgentSession {
    pub fn new(id: SessionId, repository_path: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
            conversation: Conversation::new(),
            tasks: Vec::new(),
            worktrees: Vec::new(),
            repository_path,
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
    fn test_session_new_sets_active() {
        let session = AgentSession::new(SessionId("sess".to_string()), PathBuf::from("/repo"));
        assert_eq!(session.status, SessionStatus::Active);
    }
}
