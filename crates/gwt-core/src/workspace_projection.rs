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
    paths::{
        gwt_workspace_journal_path_for_repo_path, gwt_workspace_projection_path_for_repo_path,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceCleanupReason {
    WorkspaceDone,
    PrMerged,
}

impl WorkspaceCleanupReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceDone => "workspace_done",
            Self::PrMerged => "pr_merged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCleanupCandidate {
    pub branch: String,
    pub worktree_path: Option<PathBuf>,
    pub reason: WorkspaceCleanupReason,
    pub default_delete_remote: bool,
    #[serde(default)]
    pub remote_delete_available: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_summary: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub last_board_entry_id: Option<String>,
    #[serde(default)]
    pub last_board_entry_kind: Option<BoardEntryKind>,
    #[serde(default)]
    pub coordination_scope: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProjection {
    pub id: String,
    pub project_root: PathBuf,
    pub title: String,
    pub status_category: WorkspaceStatusCategory,
    pub status_text: String,
    #[serde(default)]
    pub summary: Option<String>,
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
            summary: None,
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
                agent.last_board_entry_kind = Some(entry.kind.clone());
                agent.coordination_scope = coordination_scope_for_entry(entry);
                agent.current_focus = Some(entry.body.clone());
                if let Some(title_summary) = entry
                    .title_summary
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    agent.title_summary = Some(title_summary.clone());
                }
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

    pub fn apply_update(
        &mut self,
        update: WorkspaceProjectionUpdate,
        updated_at: DateTime<Utc>,
    ) -> WorkspaceJournalEntry {
        if let Some(title) = update
            .title
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            self.title = title.clone();
        }
        if let Some(category) = update.status_category {
            self.status_category = category;
        }
        if let Some(status_text) = update
            .status_text
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            self.status_text = status_text.clone();
        }
        if let Some(summary) = update
            .summary
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            self.summary = Some(summary.clone());
            if update.status_text.is_none() {
                self.status_text = summary.clone();
            }
        }
        if let Some(owner) = update
            .owner
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            self.owner = Some(owner.clone());
        }
        if let Some(next_action) = update.next_action.as_ref() {
            self.next_action = (!next_action.trim().is_empty()).then_some(next_action.clone());
        }
        if let Some(session_id) = update.agent_session_id.as_deref() {
            if let Some(agent) = self
                .agents
                .iter_mut()
                .find(|agent| agent.session_id == session_id)
            {
                if let Some(focus) = update
                    .agent_current_focus
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    agent.current_focus = Some(focus.clone());
                    agent.updated_at = updated_at;
                }
                if let Some(title_summary) = update
                    .agent_title_summary
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    agent.title_summary = Some(title_summary.clone());
                    agent.updated_at = updated_at;
                }
            }
        }
        self.updated_at = updated_at;

        WorkspaceJournalEntry {
            id: Uuid::new_v4().to_string(),
            project_root: self.project_root.clone(),
            title: update.title,
            status_category: update.status_category,
            status_text: update.status_text,
            owner: update.owner,
            next_action: update.next_action,
            summary: update.summary,
            agent_session_id: update.agent_session_id,
            agent_current_focus: update.agent_current_focus,
            agent_title_summary: update.agent_title_summary,
            updated_at,
        }
    }

    pub fn remove_agent_session(
        &mut self,
        session_id: &str,
        window_id: Option<&str>,
        updated_at: DateTime<Utc>,
    ) -> bool {
        let before = self.agents.len();
        self.agents.retain(|agent| {
            if agent.session_id == session_id {
                return false;
            }
            if let (Some(expected), Some(actual)) = (window_id, agent.window_id.as_deref()) {
                return actual != expected;
            }
            true
        });
        let removed = self.agents.len() != before;
        if removed {
            self.updated_at = updated_at;
            if !self.agents.iter().any(|agent| {
                matches!(
                    agent.status_category,
                    WorkspaceStatusCategory::Active | WorkspaceStatusCategory::Blocked
                )
            }) {
                self.status_category = WorkspaceStatusCategory::Idle;
                self.status_text = "No active work".to_string();
                self.next_action = None;
            }
        }
        removed
    }

    pub fn cleanup_candidate(
        &self,
        branch_has_live_agent: bool,
    ) -> Option<WorkspaceCleanupCandidate> {
        if branch_has_live_agent {
            return None;
        }
        let details = self.git_details.as_ref()?;
        if !details.created_by_start_work {
            return None;
        }
        let branch = details.branch.as_ref()?.trim();
        if !branch.starts_with("work/") {
            return None;
        }
        let reason = if details
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"))
        {
            WorkspaceCleanupReason::PrMerged
        } else if self.status_category == WorkspaceStatusCategory::Done {
            WorkspaceCleanupReason::WorkspaceDone
        } else {
            return None;
        };

        Some(WorkspaceCleanupCandidate {
            branch: branch.to_string(),
            worktree_path: details.worktree_path.clone(),
            reason,
            default_delete_remote: false,
            remote_delete_available: false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProjectionUpdate {
    pub title: Option<String>,
    pub status_category: Option<WorkspaceStatusCategory>,
    pub status_text: Option<String>,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub summary: Option<String>,
    pub agent_session_id: Option<String>,
    pub agent_current_focus: Option<String>,
    pub agent_title_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceJournalEntry {
    pub id: String,
    pub project_root: PathBuf,
    pub title: Option<String>,
    pub status_category: Option<WorkspaceStatusCategory>,
    pub status_text: Option<String>,
    pub owner: Option<String>,
    pub next_action: Option<String>,
    pub summary: Option<String>,
    pub agent_session_id: Option<String>,
    pub agent_current_focus: Option<String>,
    pub agent_title_summary: Option<String>,
    pub updated_at: DateTime<Utc>,
}

fn coordination_scope_for_entry(entry: &BoardEntry) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(owner) = entry.related_owners.first() {
        parts.push(owner.clone());
    }
    if let Some(topic) = entry.related_topics.first() {
        if !parts.iter().any(|part| part == topic) {
            parts.push(topic.clone());
        }
    }
    if parts.is_empty() {
        entry.origin_branch.clone()
    } else {
        Some(parts.join(" / "))
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

pub fn update_workspace_projection_with_journal(
    repo_path: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    update_workspace_projection_with_journal_paths(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        &gwt_workspace_journal_path_for_repo_path(repo_path),
        repo_path,
        update,
    )
}

pub fn mark_workspace_agent_stopped(
    repo_path: &Path,
    session_id: &str,
    window_id: Option<&str>,
) -> Result<bool> {
    mark_workspace_agent_stopped_at(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        repo_path,
        session_id,
        window_id,
        Utc::now(),
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

pub fn update_workspace_projection_with_journal_paths(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    update_workspace_projection_with_journal_paths_at(
        current_path,
        journal_path,
        project_root,
        update,
        Utc::now(),
    )
}

pub fn update_workspace_projection_with_journal_paths_at(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
    update: WorkspaceProjectionUpdate,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceJournalEntry> {
    let mut projection =
        load_or_default_workspace_projection_from_path(current_path, project_root)?;
    projection.project_root = project_root.to_path_buf();
    let entry = projection.apply_update(update, updated_at);
    save_workspace_projection_to_path(current_path, &projection)?;
    append_workspace_journal_entry_to_path(journal_path, &entry)?;
    Ok(entry)
}

pub fn mark_workspace_agent_stopped_at(
    current_path: &Path,
    project_root: &Path,
    session_id: &str,
    window_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    let Some(mut projection) = load_workspace_projection_from_path(current_path)? else {
        return Ok(false);
    };
    projection.project_root = project_root.to_path_buf();
    let changed = projection.remove_agent_session(session_id, window_id, updated_at);
    if changed {
        save_workspace_projection_to_path(current_path, &projection)?;
    }
    Ok(changed)
}

pub fn append_workspace_journal_entry_to_path(
    path: &Path,
    entry: &WorkspaceJournalEntry,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, entry)
        .map_err(|error| GwtError::Other(format!("workspace journal json: {error}")))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub fn load_recent_workspace_journal_entries(
    repo_path: &Path,
    limit: usize,
) -> Result<Vec<WorkspaceJournalEntry>> {
    load_recent_workspace_journal_entries_from_path(
        &gwt_workspace_journal_path_for_repo_path(repo_path),
        limit,
    )
}

pub fn load_recent_workspace_journal_entries_from_path(
    path: &Path,
    limit: usize,
) -> Result<Vec<WorkspaceJournalEntry>> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut entries = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<WorkspaceJournalEntry>(line)
                .map_err(|error| GwtError::Other(format!("workspace journal json: {error}")))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
    entries.truncate(limit);
    Ok(entries)
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
    use chrono::TimeZone;

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
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
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
        assert_eq!(summary.title_summary, None);
        assert_eq!(summary.last_board_entry_kind, None);
        assert_eq!(summary.coordination_scope, None);
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
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: Utc::now(),
        });

        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Active
        );
    }

    #[test]
    fn cleanup_candidate_requires_done_or_merged_start_work_workspace_branch() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 7, 2, 0, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Done;
        projection.git_details = Some(GitDetails {
            branch: Some("work/20260507-0200".to_string()),
            worktree_path: Some(PathBuf::from("/repo/work/20260507-0200")),
            base_branch: Some("origin/main".to_string()),
            pr_number: Some(2525),
            pr_state: None,
            created_by_start_work: true,
            created_at,
        });

        let candidate = projection
            .cleanup_candidate(false)
            .expect("done Start Work workspace should be cleanable");

        assert_eq!(candidate.branch, "work/20260507-0200");
        assert_eq!(
            candidate.worktree_path.as_deref(),
            Some(Path::new("/repo/work/20260507-0200"))
        );
        assert_eq!(candidate.reason, WorkspaceCleanupReason::WorkspaceDone);
        assert!(!candidate.default_delete_remote);

        projection.status_category = WorkspaceStatusCategory::Active;
        projection
            .git_details
            .as_mut()
            .expect("git details")
            .pr_state = Some("merged".to_string());
        let merged = projection
            .cleanup_candidate(false)
            .expect("merged PR should trigger cleanup candidate");
        assert_eq!(merged.reason, WorkspaceCleanupReason::PrMerged);
    }

    #[test]
    fn cleanup_candidate_preserves_non_workspace_or_active_branches() {
        let created_at = Utc.with_ymd_and_hms(2026, 5, 7, 2, 0, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Done;
        projection.git_details = Some(GitDetails {
            branch: Some("feature/manual".to_string()),
            worktree_path: Some(PathBuf::from("/repo/feature/manual")),
            base_branch: Some("origin/main".to_string()),
            pr_number: None,
            pr_state: None,
            created_by_start_work: true,
            created_at,
        });
        assert_eq!(projection.cleanup_candidate(false), None);

        projection.git_details.as_mut().expect("git details").branch =
            Some("work/20260507-0200".to_string());
        assert_eq!(
            projection.cleanup_candidate(true),
            None,
            "live Agent sessions must suppress destructive cleanup prompts"
        );

        projection
            .git_details
            .as_mut()
            .expect("git details")
            .created_by_start_work = false;
        assert_eq!(projection.cleanup_candidate(false), None);
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
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
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
    fn board_milestone_keeps_title_summary_separate_from_current_focus() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: Utc::now(),
        });
        let mut entry_value = serde_json::to_value(
            BoardEntry::new(
                crate::coordination::AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Implementing the title-summary contract across Board, Workspace, runtime, and frontend surfaces",
                None,
                None,
                Vec::new(),
                Vec::new(),
            )
            .with_origin_session_id("sess-1"),
        )
        .expect("entry json");
        entry_value["title_summary"] = serde_json::json!("Title summary contract");
        let entry: BoardEntry = serde_json::from_value(entry_value).expect("board entry");

        projection.record_board_milestone(&entry);
        let agent_json = serde_json::to_value(&projection.agents[0]).expect("agent json");

        assert_eq!(
            projection.agents[0].current_focus.as_deref(),
            Some(
                "Implementing the title-summary contract across Board, Workspace, runtime, and frontend surfaces"
            )
        );
        assert_eq!(
            agent_json
                .pointer("/title_summary")
                .and_then(|value| value.as_str()),
            Some("Title summary contract")
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
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
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

    #[test]
    fn board_milestone_records_agent_coordination_kind_and_scope() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: Utc::now(),
        });
        let mut handoff = BoardEntry::new(
            crate::coordination::AuthorKind::Agent,
            "codex",
            BoardEntryKind::Handoff,
            "Implementation complete; reviewer should check visual states",
            None,
            None,
            vec!["workspace-ux".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("sess-1");
        handoff.id = "handoff-1".to_string();

        projection.record_board_milestone(&handoff);

        assert_eq!(
            projection.agents[0].last_board_entry_kind,
            Some(BoardEntryKind::Handoff)
        );
        assert_eq!(
            projection.agents[0].coordination_scope.as_deref(),
            Some("SPEC-2359 / workspace-ux")
        );
        assert_eq!(
            projection.agents[0].current_focus.as_deref(),
            Some("Implementation complete; reviewer should check visual states")
        );
    }

    #[test]
    fn workspace_update_persists_current_summary_and_journal_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let current_path = temp.path().join("workspace/current.json");
        let journal_path = temp.path().join("workspace/journal.jsonl");

        let entry = update_workspace_projection_with_journal_paths(
            &current_path,
            &journal_path,
            &project_root,
            WorkspaceProjectionUpdate {
                title: Some("Fix Active Work lifecycle".to_string()),
                status_category: Some(WorkspaceStatusCategory::Active),
                status_text: Some("Implementing lifecycle cleanup".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Run focused regression tests".to_string()),
                summary: Some("Workspace state is now the source for Active Work.".to_string()),
                agent_session_id: Some("session-1".to_string()),
                agent_current_focus: Some("Writing RED tests".to_string()),
                agent_title_summary: None,
            },
        )
        .expect("update workspace projection");

        let projection = load_workspace_projection_from_path(&current_path)
            .expect("load projection")
            .expect("projection");
        assert_eq!(projection.title, "Fix Active Work lifecycle");
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(projection.status_text, "Implementing lifecycle cleanup");
        assert_eq!(
            projection.summary.as_deref(),
            Some("Workspace state is now the source for Active Work.")
        );
        assert_eq!(
            projection.next_action.as_deref(),
            Some("Run focused regression tests")
        );

        let lines = std::fs::read_to_string(&journal_path).expect("journal");
        let entries = lines.lines().collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let journal: WorkspaceJournalEntry =
            serde_json::from_str(entries[0]).expect("journal entry");
        assert_eq!(journal.id, entry.id);
        assert_eq!(journal.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(
            journal.summary.as_deref(),
            Some("Workspace state is now the source for Active Work.")
        );
        assert_eq!(journal.agent_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            journal.agent_current_focus.as_deref(),
            Some("Writing RED tests")
        );
    }

    #[test]
    fn workspace_update_persists_agent_title_summary_separately_from_focus() {
        let updated_at = Utc.with_ymd_and_hms(2026, 5, 7, 2, 30, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "session-1".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at,
        });

        let journal = projection.apply_update(
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: None,
                agent_session_id: Some("session-1".to_string()),
                agent_current_focus: Some(
                    "Implementing title-summary support across Board and Workspace".to_string(),
                ),
                agent_title_summary: Some("Title summary support".to_string()),
            },
            updated_at,
        );

        assert_eq!(
            projection.agents[0].current_focus.as_deref(),
            Some("Implementing title-summary support across Board and Workspace")
        );
        assert_eq!(
            projection.agents[0].title_summary.as_deref(),
            Some("Title summary support")
        );
        assert_eq!(
            journal.agent_current_focus.as_deref(),
            Some("Implementing title-summary support across Board and Workspace")
        );
        assert_eq!(
            journal.agent_title_summary.as_deref(),
            Some("Title summary support")
        );
    }

    #[test]
    fn stopped_agent_is_removed_from_current_projection_without_losing_summary() {
        let now = Utc::now();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Codex is running".to_string();
        projection.next_action = Some("Review output".to_string());
        projection.summary = Some("Keep this user-facing work summary.".to_string());
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "session-1".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Investigating".to_string()),
            title_summary: None,
            worktree_path: None,
            branch: Some("work/20260506-1652".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            updated_at: now,
        });

        assert!(projection.remove_agent_session("session-1", Some("tab-1::agent-1"), now));

        assert!(projection.agents.is_empty());
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Idle);
        assert_eq!(projection.status_text, "No active work");
        assert_eq!(projection.next_action, None);
        assert_eq!(
            projection.summary.as_deref(),
            Some("Keep this user-facing work summary.")
        );
    }

    #[test]
    fn recent_workspace_journal_entries_load_newest_first_with_limit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let current_path = temp.path().join("workspace/current.json");
        let journal_path = temp.path().join("workspace/journal.jsonl");
        let first_at = Utc.with_ymd_and_hms(2026, 5, 7, 1, 0, 0).unwrap();
        let second_at = Utc.with_ymd_and_hms(2026, 5, 7, 1, 5, 0).unwrap();

        update_workspace_projection_with_journal_paths_at(
            &current_path,
            &journal_path,
            &project_root,
            WorkspaceProjectionUpdate {
                title: Some("Workspace Overview".to_string()),
                status_category: Some(WorkspaceStatusCategory::Active),
                status_text: Some("Drafting overview".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: None,
                summary: Some("First summary".to_string()),
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
            first_at,
        )
        .expect("first update");
        update_workspace_projection_with_journal_paths_at(
            &current_path,
            &journal_path,
            &project_root,
            WorkspaceProjectionUpdate {
                title: None,
                status_category: Some(WorkspaceStatusCategory::Idle),
                status_text: Some("Ready for review".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Review Workspace Overview".to_string()),
                summary: Some("Second summary".to_string()),
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
            second_at,
        )
        .expect("second update");

        let recent = load_recent_workspace_journal_entries_from_path(&journal_path, 1)
            .expect("recent journal entries");

        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].summary.as_deref(), Some("Second summary"));
        assert_eq!(recent[0].updated_at, second_at);
    }
}
