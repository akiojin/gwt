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

/// SPEC-2359 US-80 (FR-426/FR-427): reserved `agent_id` sentinel that marks a
/// projection entry as a plain Start-Work Shell instead of an agent session.
/// Real agents carry their provider command (`claude` / `codex` / ...), which
/// never collides with this value, so a Shell Work is identified by this id
/// alone. Keeping the discriminator inside the existing `agent_id` avoids a new
/// serialized field, so legacy projection JSON deserializes unchanged.
pub const SHELL_WORK_AGENT_ID: &str = "shell";

/// SPEC-2359 US-80 (FR-426): whether a Work projection entry is backed by an
/// agent session or by a plain Start-Work Shell. Shell Works have no agent
/// session, so their identity and status are derived from the shell window /
/// PTY process instead of agent hook telemetry. Defaults to `Agent` so
/// projection entries written before the Shell Work model classify unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    #[default]
    Agent,
    Shell,
}

/// Per-agent slice of a [`WorkspaceProjection`]: session identity, runtime
/// status, current focus, and Board linkage. Updated from agent session
/// events and the `workspace.update` operation.
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

    /// SPEC-2359 US-80 (FR-426): classify this Work entry. A Start-Work Shell
    /// carries the reserved [`SHELL_WORK_AGENT_ID`] sentinel in `agent_id`;
    /// everything else is an agent-session-backed Work.
    pub fn work_kind(&self) -> WorkKind {
        if self.agent_id == SHELL_WORK_AGENT_ID {
            WorkKind::Shell
        } else {
            WorkKind::Agent
        }
    }

    /// SPEC-2359 US-80 (FR-427): true when this entry is a plain Shell Work.
    pub fn is_shell_work(&self) -> bool {
        self.work_kind() == WorkKind::Shell
    }

    /// SPEC-2359 US-80 (FR-427): build a Shell Work summary for a Start-Work
    /// shell window. The window id is the stable identity (Shell Works have no
    /// agent session), and the worktree/branch reuse the same canonical Work id
    /// derivation as agents, so a shell launched on a branch groups into the
    /// same Work row as an agent on that branch.
    pub fn shell_work(
        window_id: impl Into<String>,
        worktree_path: Option<PathBuf>,
        branch: Option<String>,
        status_category: WorkspaceStatusCategory,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let window_id = window_id.into();
        Self {
            session_id: window_id.clone(),
            window_id: Some(window_id),
            agent_id: SHELL_WORK_AGENT_ID.to_string(),
            display_name: "Shell".to_string(),
            status_category,
            current_focus: None,
            title_summary: None,
            worktree_path,
            branch,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at,
        }
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
    fn agent_summary_without_shell_sentinel_classifies_as_agent_work() {
        // SPEC-2359 US-80 / FR-426: projection entries written before the Shell
        // Work model carry a real provider `agent_id`, so they classify as
        // `WorkKind::Agent` with no migration.
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

        assert_eq!(summary.work_kind(), WorkKind::Agent);
        assert!(!summary.is_shell_work());
    }

    #[test]
    fn agent_summary_with_shell_sentinel_classifies_as_shell_work() {
        // SPEC-2359 US-80 / FR-426/FR-427: the reserved `agent_id` sentinel
        // marks a Start-Work Shell, identified by its stable window id.
        let summary: WorkspaceAgentSummary = serde_json::from_value(serde_json::json!({
            "session_id": "tab-1:shell-3",
            "agent_id": SHELL_WORK_AGENT_ID,
            "display_name": "Shell",
            "status_category": "active",
            "current_focus": null,
            "worktree_path": null,
            "branch": "work/20260621-0333",
            "last_board_entry_id": null,
            "updated_at": "2026-06-21T03:00:00Z"
        }))
        .expect("shell summary");

        assert_eq!(summary.work_kind(), WorkKind::Shell);
        assert!(summary.is_shell_work());
        assert_eq!(summary.session_id, "tab-1:shell-3");
    }

    #[test]
    fn shell_work_summary_groups_with_agent_on_same_branch() {
        // SPEC-2359 US-80 / FR-427: a Shell Work reuses the agent canonical Work
        // id derivation, so a shell and an agent on the same branch land in one
        // Work row.
        let updated_at: chrono::DateTime<chrono::Utc> = "2026-06-21T03:00:00Z".parse().expect("ts");
        let shell = WorkspaceAgentSummary::shell_work(
            "tab-1:shell-3",
            Some(std::path::PathBuf::from("/repo/work/x")),
            Some("work/x".to_string()),
            WorkspaceStatusCategory::Active,
            updated_at,
        );
        assert!(shell.is_shell_work());
        assert_eq!(shell.session_id, "tab-1:shell-3");
        assert_eq!(shell.window_id.as_deref(), Some("tab-1:shell-3"));
        assert!(shell.is_assigned());

        let project_root = std::path::Path::new("/repo");
        let shell_work_id = canonical_work_id(
            project_root,
            shell.branch.as_deref(),
            shell.worktree_path.as_deref(),
        );
        let agent_work_id = canonical_work_id(project_root, Some("work/x"), None);
        assert_eq!(shell_work_id, agent_work_id);
        assert!(shell_work_id.is_some());
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
