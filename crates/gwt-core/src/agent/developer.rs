//! Developer state for agent mode (Project Team model)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::sub_agent::CompletionSource;
use super::types::SubAgentId;
use super::worktree::WorktreeRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Claude,
    Codex,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperStatus {
    Starting,
    Running,
    WaitingInput,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperState {
    pub id: SubAgentId,
    pub agent_type: AgentType,
    pub pane_id: String,
    pub pid: Option<u32>,
    pub status: DeveloperStatus,
    pub worktree: WorktreeRef,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub completion_source: Option<CompletionSource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_developer_status_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&DeveloperStatus::Starting).unwrap(),
            "\"starting\""
        );
        assert_eq!(
            serde_json::to_string(&DeveloperStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&DeveloperStatus::WaitingInput).unwrap(),
            "\"waiting_input\""
        );
        assert_eq!(
            serde_json::to_string(&DeveloperStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&DeveloperStatus::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn test_developer_status_deserialize_snake_case() {
        assert_eq!(
            serde_json::from_str::<DeveloperStatus>("\"starting\"").unwrap(),
            DeveloperStatus::Starting
        );
        assert_eq!(
            serde_json::from_str::<DeveloperStatus>("\"waiting_input\"").unwrap(),
            DeveloperStatus::WaitingInput
        );
        assert_eq!(
            serde_json::from_str::<DeveloperStatus>("\"completed\"").unwrap(),
            DeveloperStatus::Completed
        );
    }

    #[test]
    fn test_agent_type_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&AgentType::Claude).unwrap(),
            "\"claude\""
        );
        assert_eq!(
            serde_json::to_string(&AgentType::Codex).unwrap(),
            "\"codex\""
        );
        assert_eq!(
            serde_json::to_string(&AgentType::Gemini).unwrap(),
            "\"gemini\""
        );
    }

    #[test]
    fn test_agent_type_deserialize_snake_case() {
        assert_eq!(
            serde_json::from_str::<AgentType>("\"claude\"").unwrap(),
            AgentType::Claude
        );
        assert_eq!(
            serde_json::from_str::<AgentType>("\"codex\"").unwrap(),
            AgentType::Codex
        );
        assert_eq!(
            serde_json::from_str::<AgentType>("\"gemini\"").unwrap(),
            AgentType::Gemini
        );
    }

    #[test]
    fn test_developer_state_serde_roundtrip() {
        let worktree = WorktreeRef::new(
            "agent/login-form",
            PathBuf::from(".worktrees/agent-login-form"),
            vec![],
        );
        let state = DeveloperState {
            id: SubAgentId("dev-1".to_string()),
            agent_type: AgentType::Claude,
            pane_id: "pane-1".to_string(),
            pid: Some(12345),
            status: DeveloperStatus::Running,
            worktree,
            started_at: Utc::now(),
            completed_at: None,
            completion_source: None,
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: DeveloperState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, state.id);
        assert_eq!(deserialized.agent_type, AgentType::Claude);
        assert_eq!(deserialized.pane_id, "pane-1");
        assert_eq!(deserialized.pid, Some(12345));
        assert_eq!(deserialized.status, DeveloperStatus::Running);
        assert_eq!(deserialized.worktree.branch_name, "agent/login-form");
        assert!(deserialized.completed_at.is_none());
        assert!(deserialized.completion_source.is_none());
    }

    #[test]
    fn test_developer_state_with_completion() {
        let worktree = WorktreeRef::new(
            "agent/task-1",
            PathBuf::from(".worktrees/agent-task-1"),
            vec![],
        );
        let state = DeveloperState {
            id: SubAgentId("dev-2".to_string()),
            agent_type: AgentType::Codex,
            pane_id: "pane-2".to_string(),
            pid: Some(9999),
            status: DeveloperStatus::Completed,
            worktree,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            completion_source: Some(CompletionSource::ProcessExit),
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: DeveloperState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.status, DeveloperStatus::Completed);
        assert!(deserialized.completed_at.is_some());
        assert_eq!(
            deserialized.completion_source,
            Some(CompletionSource::ProcessExit)
        );
    }
}
