//! Sub-agent metadata for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::types::SubAgentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentType {
    ClaudeCode,
    Codex,
    Gemini,
    OpenCode,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentStatus {
    Starting,
    Running,
    WaitingInput,
    Completed,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionSource {
    Hook,
    ProcessExit,
    OutputPattern,
    IdleTimeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    pub id: SubAgentId,
    pub agent_type: SubAgentType,
    pub pane_id: String,
    pub pid: u32,
    pub status: SubAgentStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub completion_source: Option<CompletionSource>,
    /// Auto-mode flag used when launching (e.g., "--dangerously-skip-permissions")
    #[serde(default)]
    pub auto_mode_flag: Option<String>,
}

impl SubAgent {
    pub fn new(id: SubAgentId, agent_type: SubAgentType, pane_id: String, pid: u32) -> Self {
        Self {
            id,
            agent_type,
            pane_id,
            pid,
            status: SubAgentStatus::Starting,
            started_at: Utc::now(),
            completed_at: None,
            completion_source: None,
            auto_mode_flag: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_agent_new() {
        let id = SubAgentId("agent-1".to_string());
        let agent = SubAgent::new(id, SubAgentType::Codex, "%1".to_string(), 123);
        assert_eq!(agent.pid, 123);
        assert_eq!(agent.status, SubAgentStatus::Starting);
    }
}
