//! Per-agent summary slice of a [`WorkspaceProjection`]: session identity,
//! runtime status, current focus, Board linkage, and Workspace affiliation.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::coordination::BoardEntryKind;

use super::*;

fn default_workspace_agent_affiliation_status() -> WorkspaceAgentAffiliationStatus {
    WorkspaceAgentAffiliationStatus::Assigned
}

/// Per-agent slice of a [`WorkspaceProjection`]: session identity, runtime
/// status, current focus, and Board linkage. Updated from agent session
/// events and `gwtd workspace update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceAgentSummary {
    pub session_id: String,
    #[serde(default)]
    pub window_id: Option<String>,
    pub agent_id: String,
    pub display_name: String,
    pub status_category: WorkspaceStatusCategory,
    pub current_focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_summary: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub last_board_entry_id: Option<String>,
    #[serde(default)]
    pub last_board_entry_kind: Option<BoardEntryKind>,
    #[serde(default)]
    pub coordination_scope: Option<String>,
    #[serde(default = "default_workspace_agent_affiliation_status")]
    pub affiliation_status: WorkspaceAgentAffiliationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceAgentSummary {
    pub fn is_unassigned(&self) -> bool {
        self.affiliation_status == WorkspaceAgentAffiliationStatus::Unassigned
    }

    pub fn is_assigned(&self) -> bool {
        self.affiliation_status == WorkspaceAgentAffiliationStatus::Assigned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_summary_deserializes_legacy_payload_without_window_id() {
        let summary: WorkspaceAgentSummary = serde_json::from_value(serde_json::json!({
            "session_id": "sess-1",
            "agent_id": "codex",
            "display_name": "Codex",
            "status_category": "active",
            "current_focus": null,
            "worktree_path": null,
            "branch": "work/20260504-1200",
            "last_board_entry_id": null,
            "updated_at": "2026-05-04T12:00:00Z"
        }))
        .expect("legacy summary");

        assert_eq!(summary.window_id, None);
        assert_eq!(summary.title_summary, None);
        assert_eq!(summary.last_board_entry_kind, None);
        assert_eq!(summary.coordination_scope, None);
    }

    #[test]
    fn legacy_workspace_agent_affiliation_defaults_to_assigned() {
        let payload = serde_json::json!({
            "session_id": "session-legacy",
            "window_id": "tab-1:agent-1",
            "agent_id": "codex",
            "display_name": "Codex",
            "status_category": "active",
            "current_focus": "Implement existing Workspace behavior",
            "title_summary": "Existing Workspace behavior",
            "worktree_path": null,
            "branch": "work/legacy",
            "last_board_entry_id": null,
            "updated_at": "2026-05-11T00:00:00Z"
        });

        let agent: WorkspaceAgentSummary =
            serde_json::from_value(payload).expect("legacy agent summary");

        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), None);
    }
}
