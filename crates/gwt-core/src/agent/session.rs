//! Session state for Project Mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::conversation::Conversation;
use super::developer::AgentType;
use super::issue::ProjectIssue;
use super::lead::LeadState;
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
    /// Base branch from which agent worktrees are created
    #[serde(default)]
    pub base_branch: Option<String>,
    /// Position in the session queue (0 = active)
    #[serde(default)]
    pub queue_position: u32,
    /// Total LLM API calls made in this session
    #[serde(default)]
    pub llm_call_count: u64,
    /// Estimated total tokens consumed
    #[serde(default)]
    pub estimated_tokens: u64,
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
            base_branch: None,
            queue_position: 0,
            llm_call_count: 0,
            estimated_tokens: 0,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Project Mode session (3-layer: Lead / Coordinator / Developer)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectModeSession {
    pub id: SessionId,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub repository_path: PathBuf,
    pub base_branch: String,
    pub lead: LeadState,
    pub issues: Vec<ProjectIssue>,
    pub developer_agent_type: AgentType,
}

impl ProjectModeSession {
    pub fn new(
        id: SessionId,
        repository_path: PathBuf,
        base_branch: impl Into<String>,
        developer_agent_type: AgentType,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            status: SessionStatus::Active,
            created_at: now,
            updated_at: now,
            repository_path,
            base_branch: base_branch.into(),
            lead: LeadState::default(),
            issues: Vec::new(),
            developer_agent_type,
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

    #[test]
    fn test_project_mode_session_new() {
        let session = ProjectModeSession::new(
            SessionId("pt-1".to_string()),
            PathBuf::from("/repo"),
            "feature/project-mode",
            AgentType::Claude,
        );
        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(session.base_branch, "feature/project-mode");
        assert_eq!(session.developer_agent_type, AgentType::Claude);
        assert!(session.issues.is_empty());
        assert_eq!(session.lead.status, super::super::lead::LeadStatus::Idle);
    }

    #[test]
    fn test_project_mode_session_touch() {
        let mut session = ProjectModeSession::new(
            SessionId("pt-2".to_string()),
            PathBuf::from("/repo"),
            "main",
            AgentType::Codex,
        );
        let before = session.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        session.touch();
        assert!(session.updated_at > before);
    }

    #[test]
    fn test_project_mode_session_serde_roundtrip() {
        let session = ProjectModeSession::new(
            SessionId("pt-3".to_string()),
            PathBuf::from("/repo"),
            "develop",
            AgentType::Gemini,
        );
        let json = serde_json::to_string_pretty(&session).unwrap();
        let deserialized: ProjectModeSession = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, session.id);
        assert_eq!(deserialized.status, SessionStatus::Active);
        assert_eq!(deserialized.base_branch, "develop");
        assert_eq!(deserialized.developer_agent_type, AgentType::Gemini);
        assert!(deserialized.issues.is_empty());
    }
}
