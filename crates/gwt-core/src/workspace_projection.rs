use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    coordination::{BoardEntry, BoardEntryKind},
    error::{GwtError, Result},
    paths::gwt_workspace_projection_path_for_repo_path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatusCategory {
    Active,
    Idle,
    Blocked,
    Done,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDetails {
    pub branch: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub base_branch: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_state: Option<String>,
    pub created_by_start_work: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceAgentSummary {
    pub session_id: String,
    #[serde(default)]
    pub window_id: Option<String>,
    pub agent_id: String,
    pub display_name: String,
    pub status_category: WorkspaceStatusCategory,
    pub current_focus: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub last_board_entry_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProjection {
    pub id: String,
    pub project_root: PathBuf,
    pub title: String,
    pub status_category: WorkspaceStatusCategory,
    pub status_text: String,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub agents: Vec<WorkspaceAgentSummary>,
    pub git_details: Option<GitDetails>,
    pub board_refs: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceProjection {
    pub fn default_for_project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            project_root: project_root.into(),
            title: "Workspace".to_string(),
            status_category: WorkspaceStatusCategory::Unknown,
            status_text: "No active work".to_string(),
            owner: None,
            next_action: None,
            agents: Vec::new(),
            git_details: None,
            board_refs: Vec::new(),
            updated_at: Utc::now(),
        }
    }

    pub fn effective_status_category(&self) -> WorkspaceStatusCategory {
        if self
            .agents
            .iter()
            .any(|agent| agent.status_category == WorkspaceStatusCategory::Blocked)
        {
            return WorkspaceStatusCategory::Blocked;
        }
        if self
            .agents
            .iter()
            .any(|agent| agent.status_category == WorkspaceStatusCategory::Active)
        {
            return WorkspaceStatusCategory::Active;
        }
        self.status_category
    }

    pub fn record_board_milestone(&mut self, entry: &BoardEntry) {
        if !self.board_refs.iter().any(|id| id == &entry.id) {
            self.board_refs.push(entry.id.clone());
        }
        if let Some(owner) = entry.related_owners.first() {
            self.owner = Some(owner.clone());
        }

        match entry.kind {
            BoardEntryKind::Blocked => {
                self.status_category = WorkspaceStatusCategory::Blocked;
                self.status_text = entry.body.clone();
                self.next_action = Some("Resolve blocker".to_string());
            }
            BoardEntryKind::Next => {
                self.next_action = Some(entry.body.clone());
                if self.status_category == WorkspaceStatusCategory::Unknown {
                    self.status_category = WorkspaceStatusCategory::Active;
                }
            }
            BoardEntryKind::Status
            | BoardEntryKind::Claim
            | BoardEntryKind::Handoff
            | BoardEntryKind::Decision => {
                self.status_category = WorkspaceStatusCategory::Active;
                self.status_text = entry.body.clone();
            }
            BoardEntryKind::Request | BoardEntryKind::Impact | BoardEntryKind::Question => {}
        }

        if let Some(session_id) = entry.origin_session_id.as_deref() {
            if let Some(agent) = self
                .agents
                .iter_mut()
                .find(|agent| agent.session_id == session_id)
            {
                agent.last_board_entry_id = Some(entry.id.clone());
                agent.current_focus = Some(entry.body.clone());
                agent.updated_at = entry.updated_at;
                match entry.kind {
                    BoardEntryKind::Blocked => {
                        agent.status_category = WorkspaceStatusCategory::Blocked;
                    }
                    BoardEntryKind::Status
                    | BoardEntryKind::Claim
                    | BoardEntryKind::Handoff
                    | BoardEntryKind::Decision => {
                        agent.status_category = WorkspaceStatusCategory::Active;
                    }
                    BoardEntryKind::Next
                    | BoardEntryKind::Request
                    | BoardEntryKind::Impact
                    | BoardEntryKind::Question => {}
                }
            }
        }

        self.updated_at = entry.updated_at;
    }
}

pub fn load_workspace_projection(repo_path: &Path) -> Result<Option<WorkspaceProjection>> {
    load_workspace_projection_from_path(&gwt_workspace_projection_path_for_repo_path(repo_path))
}

pub fn load_or_default_workspace_projection(repo_path: &Path) -> Result<WorkspaceProjection> {
    load_or_default_workspace_projection_from_path(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        repo_path,
    )
}

pub fn save_workspace_projection(repo_path: &Path, projection: &WorkspaceProjection) -> Result<()> {
    save_workspace_projection_to_path(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        projection,
    )
}

pub fn load_workspace_projection_from_path(path: &Path) -> Result<Option<WorkspaceProjection>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| GwtError::Other(format!("workspace projection json: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn load_or_default_workspace_projection_from_path(
    path: &Path,
    project_root: &Path,
) -> Result<WorkspaceProjection> {
    Ok(load_workspace_projection_from_path(path)?
        .unwrap_or_else(|| WorkspaceProjection::default_for_project(project_root)))
}

pub fn save_workspace_projection_to_path(
    path: &Path,
    projection: &WorkspaceProjection,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| GwtError::Other(format!("workspace projection json: {error}")))?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("current.json");
    let tmp = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_status_prioritizes_blocked_agents_over_active_projection() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Still describing the current task".to_string();
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Blocked,
            current_focus: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            updated_at: Utc::now(),
        });

        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Blocked
        );
        assert_eq!(
            projection.status_text, "Still describing the current task",
            "category derivation must not overwrite the display status text"
        );
    }

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
    }

    #[test]
    fn effective_status_uses_active_agent_before_idle_projection() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Idle;
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            updated_at: Utc::now(),
        });

        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Active
        );
    }

    #[test]
    fn board_milestone_records_ref_id_and_copies_current_text() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let mut entry = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Implementation reached verify",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        );
        entry.id = "board-entry-1".to_string();

        projection.record_board_milestone(&entry);
        entry.body = "Edited later".to_string();

        assert_eq!(projection.board_refs, vec!["board-entry-1".to_string()]);
        assert_eq!(projection.status_text, "Implementation reached verify");
        assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
        assert!(!projection
            .board_refs
            .iter()
            .any(|value| value.contains("Implementation reached verify")));
    }

    #[test]
    fn board_milestone_updates_next_and_blocked_state_without_replay() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let mut next = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Next,
            "Run frontend smoke",
            None,
            None,
            Vec::new(),
            Vec::new(),
        );
        next.id = "next-1".to_string();
        projection.record_board_milestone(&next);

        assert_eq!(
            projection.next_action.as_deref(),
            Some("Run frontend smoke")
        );
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);

        let mut blocked = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Blocked,
            "Waiting for release signing",
            None,
            None,
            Vec::new(),
            Vec::new(),
        );
        blocked.id = "blocked-1".to_string();
        projection.record_board_milestone(&blocked);

        assert_eq!(projection.status_category, WorkspaceStatusCategory::Blocked);
        assert_eq!(projection.status_text, "Waiting for release signing");
        assert_eq!(projection.next_action.as_deref(), Some("Resolve blocker"));
        assert_eq!(
            projection.board_refs,
            vec!["next-1".to_string(), "blocked-1".to_string()]
        );
    }

    #[test]
    fn board_milestone_restores_blocked_agent_to_active_on_progress() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            updated_at: Utc::now(),
        });
        let mut blocked = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Blocked,
            "Waiting for credentials",
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .with_origin_session_id("sess-1");
        blocked.id = "blocked-1".to_string();
        projection.record_board_milestone(&blocked);

        let mut status = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Credentials are configured",
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .with_origin_session_id("sess-1");
        status.id = "status-1".to_string();
        projection.record_board_milestone(&status);

        assert_eq!(
            projection.agents[0].status_category,
            WorkspaceStatusCategory::Active
        );
        assert_eq!(
            projection.agents[0].current_focus.as_deref(),
            Some("Credentials are configured")
        );
        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Active
        );
    }

    #[test]
    fn board_milestone_next_keeps_blocked_agent_blocked() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            updated_at: Utc::now(),
        });
        let mut blocked = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Blocked,
            "Waiting for credentials",
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .with_origin_session_id("sess-1");
        blocked.id = "blocked-1".to_string();
        projection.record_board_milestone(&blocked);

        let mut next = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Next,
            "Try a different credential source",
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .with_origin_session_id("sess-1");
        next.id = "next-1".to_string();
        projection.record_board_milestone(&next);

        assert_eq!(
            projection.agents[0].status_category,
            WorkspaceStatusCategory::Blocked
        );
        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Blocked
        );
        assert_eq!(projection.status_text, "Waiting for credentials");
        assert_eq!(
            projection.next_action.as_deref(),
            Some("Try a different credential source")
        );
    }
}
