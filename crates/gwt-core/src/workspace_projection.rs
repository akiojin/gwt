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
        gwt_project_dir_for_repo_path, gwt_workspace_journal_path_for_repo_path,
        gwt_workspace_projection_path_for_repo_path, gwt_workspace_work_events_path_for_repo_path,
        gwt_workspace_work_items_path_for_repo_path,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAgentAffiliationStatus {
    Unassigned,
    Assigned,
}

fn default_workspace_agent_affiliation_status() -> WorkspaceAgentAffiliationStatus {
    WorkspaceAgentAffiliationStatus::Assigned
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDetails {
    pub branch: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub base_branch: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_state: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_created_at: Option<DateTime<Utc>>,
    pub created_by_start_work: bool,
    pub created_at: DateTime<Utc>,
}

/// SPEC-2359 Phase U-6 (FR-132): coarse Workspace lifecycle stage. Distinct
/// from [`WorkspaceStatusCategory`], which tracks the runtime activity of the
/// linked Agents. `lifecycle_stage` answers "where is this work in its overall
/// progression?" (planning → active → in review → done → archived). It is
/// derived from `events + status_category` via
/// `recompute_lifecycle_stage`, but may also be explicitly set by the user
/// via `gwtd workspace update --status archived`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLifecycleStage {
    #[default]
    Planning,
    Active,
    InReview,
    Done,
    Archived,
}

/// SPEC-2359 Phase U-6 (FR-133): structured reference to a GitHub Issue
/// linked to a Workspace. Workspace Card preview and Detail pane render these
/// as chips (`#Issue-1234`) instead of free-text. The number is required;
/// title / url are populated when known and default to None for legacy data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIssueLink {
    pub number: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// SPEC-2359 Phase U-6 (FR-133): structured reference to a GitHub Pull
/// Request linked to a Workspace. Carries `state` (e.g. open / merged /
/// closed) so UI can render lifecycle hints alongside `lifecycle_stage`
/// without re-querying GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePrLink {
    pub number: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

/// SPEC-2359 Phase U-6 (FR-132, FR-139, FR-143): derive a coarse
/// [`WorkspaceLifecycleStage`] from runtime activity signals. Used by
/// `gwtd workspace update` (FR-139) to keep the lifecycle chip in sync
/// when status / events change, by the retroactive migration (FR-143) to
/// backfill legacy data, and by the default constructor (`Planning` for
/// fresh projections without events).
///
/// Mapping rules (high → low precedence):
/// 1. `status_category = Done` → `Done` (overrides any pending events).
/// 2. `linked_prs` contains an open PR → `InReview`.
/// 3. `status_category = Active` / `Blocked` / `Idle` → `Active`
///    (runtime activity has begun even if no PR is open yet).
/// 4. `status_category = Unknown` → `Planning` (no work signal yet).
pub fn recompute_lifecycle_stage(
    status_category: WorkspaceStatusCategory,
    linked_prs: &[WorkspacePrLink],
) -> WorkspaceLifecycleStage {
    if status_category == WorkspaceStatusCategory::Done {
        return WorkspaceLifecycleStage::Done;
    }
    if linked_prs
        .iter()
        .any(|pr| matches!(pr.state.as_deref(), Some(state) if state.eq_ignore_ascii_case("open")))
    {
        return WorkspaceLifecycleStage::InReview;
    }
    match status_category {
        WorkspaceStatusCategory::Active
        | WorkspaceStatusCategory::Blocked
        | WorkspaceStatusCategory::Idle => WorkspaceLifecycleStage::Active,
        WorkspaceStatusCategory::Done => WorkspaceLifecycleStage::Done,
        WorkspaceStatusCategory::Unknown => WorkspaceLifecycleStage::Planning,
    }
}

/// SPEC-2359 Phase U-6 (FR-131): sentinel default for `created_at` when a
/// legacy `workspace.json` is read without the field present. The retroactive
/// migration in `workspace_projection_migration` detects this value and
/// backfills from the oldest event timestamp or `updated_at`. Using the
/// UNIX_EPOCH sentinel (instead of `Utc::now()`) keeps deserialization
/// deterministic for tests and avoids "this workspace was created at
/// startup" lies for legacy data.
pub fn workspace_projection_default_created_at() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).unwrap_or_else(Utc::now)
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

/// SPEC-2359 US-41 (FR-151): why a saved Workspace projection is treated as
/// stale by [`workspace_projection_stale_reason`]. `WorktreeMissing` and
/// `PrClosed` come from inspecting `git_details` / `linked_prs`;
/// `TimeThreshold` comes from `updated_at` exceeding
/// [`WorkspaceRetentionConfig::archive_after_days`]; `Compound` is returned
/// when two or more of those conditions hold simultaneously, so callers can
/// surface the strongest evidence without losing information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    WorktreeMissing,
    PrClosed,
    TimeThreshold,
    Compound,
}

impl StaleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorktreeMissing => "worktree_missing",
            Self::PrClosed => "pr_closed",
            Self::TimeThreshold => "time_threshold",
            Self::Compound => "compound",
        }
    }
}

/// SPEC-2359 US-41 (FR-152): retention thresholds for the Workspace
/// projection pruner. `archive_after_days` triggers the `Active → Archived`
/// transition; `delete_after_archive_days` triggers the
/// `Archived → physical delete` transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRetentionConfig {
    pub archive_after_days: u32,
    pub delete_after_archive_days: u32,
}

impl Default for WorkspaceRetentionConfig {
    fn default() -> Self {
        Self {
            archive_after_days: 30,
            delete_after_archive_days: 60,
        }
    }
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
    /// SPEC-2359 Phase U-6 (FR-131, FR-135, FR-143): Workspace creation
    /// timestamp, distinct from `updated_at`. Legacy files without the
    /// field deserialize to UNIX_EPOCH via
    /// [`workspace_projection_default_created_at`]; the retroactive
    /// migration backfills the actual value from event timestamps or
    /// `updated_at`.
    #[serde(default = "workspace_projection_default_created_at")]
    pub created_at: DateTime<Utc>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-135): identifier of the Agent or
    /// user that created the Workspace, used for the Detail pane
    /// "Created by @..." line. None for legacy data; the migration
    /// backfills from the first agent's `agent_id` when available.
    #[serde(default)]
    pub creator: Option<String>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-132, FR-139): high-level lifecycle
    /// stage derived from events + status_category via
    /// `recompute_lifecycle_stage`. Defaults to `Planning` for legacy
    /// data; the migration recomputes from actual state on first load.
    #[serde(default)]
    pub lifecycle_stage: WorkspaceLifecycleStage,
    /// SPEC-2359 Phase U-6 (FR-131, FR-141): separate from `status_text`,
    /// `blocked_reason` carries the Board entry body that triggered the
    /// Blocked state so the Detail pane can render a dedicated section
    /// instead of mixing it into the status line.
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-133, FR-146): GitHub Issues linked
    /// to this Workspace, rendered as `#Issue-N` chips in Kanban Card
    /// preview / Detail pane.
    #[serde(default)]
    pub linked_issues: Vec<WorkspaceIssueLink>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-133, FR-146): GitHub PRs linked
    /// to this Workspace, rendered as `#PR-N` chips with optional state
    /// (open / merged / closed) hint.
    #[serde(default)]
    pub linked_prs: Vec<WorkspacePrLink>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-138): freeform tags (e.g.
    /// "bugfix", "onboarding"). Rendered as `#tag` chips in Kanban Card
    /// preview / Detail pane.
    #[serde(default)]
    pub tags: Vec<String>,
    /// SPEC-2359 Phase U-6 (FR-131, FR-138): optional self-reported
    /// progress percentage (0-100). The Detail pane renders a progress
    /// bar when set, hides the section when None.
    #[serde(default)]
    pub progress_pct: Option<u8>,
}

impl WorkspaceProjection {
    pub fn default_for_project(project_root: impl Into<PathBuf>) -> Self {
        let now = Utc::now();
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
            updated_at: now,
            // SPEC-2359 Phase U-6: a freshly created projection has a
            // real `created_at` timestamp (not the legacy sentinel).
            // `lifecycle_stage` starts at `Planning` and is recomputed by
            // `recompute_lifecycle_stage` as events accumulate.
            created_at: now,
            creator: None,
            lifecycle_stage: WorkspaceLifecycleStage::Planning,
            blocked_reason: None,
            linked_issues: Vec::new(),
            linked_prs: Vec::new(),
            tags: Vec::new(),
            progress_pct: None,
        }
    }

    pub fn effective_status_category(&self) -> WorkspaceStatusCategory {
        if self.agents.iter().any(|agent| {
            agent.is_assigned() && agent.status_category == WorkspaceStatusCategory::Blocked
        }) {
            return WorkspaceStatusCategory::Blocked;
        }
        if self.agents.iter().any(|agent| {
            agent.is_assigned() && agent.status_category == WorkspaceStatusCategory::Active
        }) {
            return WorkspaceStatusCategory::Active;
        }
        self.status_category
    }

    pub fn unassigned_agents(&self) -> impl Iterator<Item = &WorkspaceAgentSummary> {
        self.agents.iter().filter(|agent| agent.is_unassigned())
    }

    pub fn assigned_agents(&self) -> impl Iterator<Item = &WorkspaceAgentSummary> {
        self.agents.iter().filter(|agent| agent.is_assigned())
    }

    pub fn register_unassigned_agent(&mut self, mut agent: WorkspaceAgentSummary) {
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
        agent.workspace_id = None;
        if let Some(existing) = self
            .agents
            .iter_mut()
            .find(|existing| existing.session_id == agent.session_id)
        {
            *existing = agent;
        } else {
            self.agents.push(agent);
        }
    }

    pub fn record_board_milestone(&mut self, entry: &BoardEntry) {
        if !self.board_refs.iter().any(|id| id == &entry.id) {
            self.board_refs.push(entry.id.clone());
        }
        let origin_agent_is_unassigned = entry
            .origin_session_id
            .as_deref()
            .and_then(|session_id| {
                self.agents
                    .iter()
                    .find(|agent| agent.session_id == session_id)
            })
            .is_some_and(WorkspaceAgentSummary::is_unassigned);

        if !origin_agent_is_unassigned {
            if let Some(owner) = entry.related_owners.first() {
                self.owner = Some(owner.clone());
            }

            match entry.kind {
                BoardEntryKind::Blocked => {
                    self.status_category = WorkspaceStatusCategory::Blocked;
                    self.status_text = entry.body.clone();
                    self.next_action = Some("Resolve blocker".to_string());
                    // SPEC-2359 Phase U-6 (FR-141): persist the entry body as
                    // a dedicated `blocked_reason` so the Detail pane can
                    // surface it separately from `status_text` (which other
                    // entry kinds overwrite). Trimmed empty bodies are
                    // ignored so manual `gwtd workspace update --status
                    // blocked` paths can still populate via flag instead.
                    let trimmed = entry.body.trim();
                    if !trimmed.is_empty() {
                        self.blocked_reason = Some(trimmed.to_string());
                    }
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
                    // SPEC-2359 Phase U-6 (FR-140): backfill the Workspace
                    // summary from milestone bodies when previously absent
                    // so Workspace Overview Detail pane never renders the
                    // placeholder for in-progress work that has at least one
                    // status / claim / handoff / decision recorded.
                    if self.summary.as_deref().is_none_or(|s| s.trim().is_empty()) {
                        let trimmed = entry.body.trim();
                        if !trimmed.is_empty() {
                            self.summary = Some(trimmed.to_string());
                        }
                    }
                }
                BoardEntryKind::Request | BoardEntryKind::Impact | BoardEntryKind::Question => {}
            }
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
            let focus = update
                .agent_current_focus
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned();
            let title_summary = update
                .agent_title_summary
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned();
            if let Some(agent) = self
                .agents
                .iter_mut()
                .find(|agent| agent.session_id == session_id)
            {
                if let Some(focus) = focus {
                    agent.current_focus = Some(focus);
                    agent.updated_at = updated_at;
                }
                if let Some(title_summary) = title_summary {
                    agent.title_summary = Some(title_summary);
                    agent.updated_at = updated_at;
                }
            } else if focus.is_some() || title_summary.is_some() {
                // SPEC-2359 Phase U-6: upsert a minimal stub for sessions
                // that the launch flow / SessionStart hook has not yet
                // registered. Without this, `gwtd workspace update
                // --title-summary X` would silently drop the update — see
                // the regression tests in this module for the contract.
                self.agents.push(WorkspaceAgentSummary {
                    session_id: session_id.to_string(),
                    window_id: None,
                    agent_id: String::new(),
                    display_name: String::new(),
                    status_category: WorkspaceStatusCategory::Active,
                    current_focus: focus,
                    title_summary,
                    worktree_path: None,
                    branch: None,
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
                    workspace_id: None,
                    updated_at,
                });
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
                agent.is_assigned()
                    && matches!(
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorkAgentRef {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceExecutionContainerRef {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub pr_number: Option<u64>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_state: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceWorkEventKind {
    Start,
    Claim,
    Update,
    Blocked,
    Handoff,
    Resume,
    Split,
    Merge,
    Pr,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorkEvent {
    pub id: String,
    pub work_item_id: String,
    pub kind: WorkspaceWorkEventKind,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub status_category: Option<WorkspaceStatusCategory>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub board_entry_id: Option<String>,
    #[serde(default)]
    pub execution_container: Option<WorkspaceExecutionContainerRef>,
    #[serde(default)]
    pub related_work_item_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceWorkEvent {
    pub fn new(
        kind: WorkspaceWorkEventKind,
        work_item_id: impl Into<String>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            work_item_id: work_item_id.into(),
            kind,
            title: None,
            intent: None,
            summary: None,
            status_category: None,
            owner: None,
            next_action: None,
            agent_session_id: None,
            agent_id: None,
            display_name: None,
            board_entry_id: None,
            execution_container: None,
            related_work_item_id: None,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorkItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    pub status_category: WorkspaceStatusCategory,
    #[serde(default)]
    pub owner: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub agents: Vec<WorkspaceWorkAgentRef>,
    #[serde(default)]
    pub execution_containers: Vec<WorkspaceExecutionContainerRef>,
    #[serde(default)]
    pub board_refs: Vec<String>,
    #[serde(default)]
    pub related_work_item_ids: Vec<String>,
    #[serde(default)]
    pub events: Vec<WorkspaceWorkEvent>,
}

impl WorkspaceWorkItem {
    pub fn is_incomplete(&self) -> bool {
        self.status_category != WorkspaceStatusCategory::Done
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorkItemsProjection {
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub work_items: Vec<WorkspaceWorkItem>,
}

impl WorkspaceWorkItemsProjection {
    fn empty(updated_at: DateTime<Utc>) -> Self {
        Self {
            updated_at,
            work_items: Vec::new(),
        }
    }

    fn apply_event(&mut self, event: WorkspaceWorkEvent) {
        let index = self
            .work_items
            .iter()
            .position(|item| item.id == event.work_item_id)
            .unwrap_or_else(|| {
                self.work_items.push(WorkspaceWorkItem {
                    id: event.work_item_id.clone(),
                    title: event
                        .title
                        .clone()
                        .or_else(|| event.intent.clone())
                        .unwrap_or_else(|| event.work_item_id.clone()),
                    intent: event.intent.clone(),
                    summary: event.summary.clone(),
                    status_category: workspace_work_event_status(&event),
                    owner: event.owner.clone(),
                    created_at: event.updated_at,
                    updated_at: event.updated_at,
                    completed_at: None,
                    agents: Vec::new(),
                    execution_containers: Vec::new(),
                    board_refs: Vec::new(),
                    related_work_item_ids: Vec::new(),
                    events: Vec::new(),
                });
                self.work_items.len() - 1
            });

        let item = &mut self.work_items[index];
        if let Some(title) = non_empty_clone(event.title.as_deref()) {
            item.title = title;
        }
        if let Some(intent) = non_empty_clone(event.intent.as_deref()) {
            item.intent = Some(intent);
        }
        if let Some(summary) = non_empty_clone(event.summary.as_deref()) {
            item.summary = Some(summary);
        }
        if let Some(owner) = non_empty_clone(event.owner.as_deref()) {
            item.owner = Some(owner);
        }
        // SPEC-2359 US-37: Done is a terminal state. Heartbeat update events
        // (kind=Update with status_category=None) emitted after a Done event
        // must not regress the WorkItem to Active/Idle. Only events that
        // carry an explicit `status_category` may transition out of Done.
        let new_status = workspace_work_event_status(&event);
        let preserve_done = item.status_category == WorkspaceStatusCategory::Done
            && event.status_category.is_none();
        if !preserve_done {
            item.status_category = new_status;
        }
        if item.status_category == WorkspaceStatusCategory::Done {
            // Preserve the first Done timestamp so idempotent Done re-applies
            // (e.g. retroactive_auto_done_scan rerun) keep the original
            // completion time.
            item.completed_at = item.completed_at.or(Some(event.updated_at));
        } else {
            item.completed_at = None;
        }
        item.updated_at = event.updated_at;
        if item.created_at > event.updated_at {
            item.created_at = event.updated_at;
        }
        if let Some(session_id) = non_empty_clone(event.agent_session_id.as_deref()) {
            if let Some(agent) = item
                .agents
                .iter_mut()
                .find(|agent| agent.session_id == session_id)
            {
                agent.agent_id = event.agent_id.clone().or(agent.agent_id.clone());
                agent.display_name = event.display_name.clone().or(agent.display_name.clone());
                agent.updated_at = event.updated_at;
            } else {
                item.agents.push(WorkspaceWorkAgentRef {
                    session_id,
                    agent_id: event.agent_id.clone(),
                    display_name: event.display_name.clone(),
                    updated_at: event.updated_at,
                });
            }
        }
        if let Some(container) = event.execution_container.clone() {
            if !item
                .execution_containers
                .iter()
                .any(|existing| workspace_execution_container_same(existing, &container))
            {
                item.execution_containers.push(container);
            }
        }
        if let Some(board_entry_id) = non_empty_clone(event.board_entry_id.as_deref()) {
            push_unique(&mut item.board_refs, board_entry_id);
        }
        if let Some(related_work_item_id) = non_empty_clone(event.related_work_item_id.as_deref()) {
            push_unique(&mut item.related_work_item_ids, related_work_item_id);
        }
        let event_updated_at = event.updated_at;
        item.events.push(event);
        item.events.sort_by_key(|event| event.updated_at);
        if event_updated_at > self.updated_at {
            self.updated_at = event_updated_at;
        }
        self.work_items
            .sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    }
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

fn legacy_workspace_projection_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/current.json")
}

fn legacy_workspace_journal_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/journal.jsonl")
}

fn legacy_workspace_work_items_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/work_items.json")
}

fn legacy_workspace_work_events_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/work_events.jsonl")
}

fn copy_legacy_workspace_file_if_needed(legacy_path: &Path, canonical_path: &Path) -> Result<()> {
    if canonical_path.exists() || legacy_path == canonical_path || !legacy_path.is_file() {
        return Ok(());
    }
    if let Some(parent) = canonical_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(legacy_path, canonical_path)?;
    Ok(())
}

fn migrate_legacy_workspace_projection(
    repo_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkspaceProjection>> {
    if canonical_path.exists() {
        return load_workspace_projection_from_path(canonical_path);
    }

    let legacy_path = legacy_workspace_projection_path_for_repo_path(repo_path);
    if legacy_path == canonical_path {
        return load_workspace_projection_from_path(canonical_path);
    }
    let Some(projection) = load_workspace_projection_from_path(&legacy_path)? else {
        return Ok(None);
    };
    save_workspace_projection_to_path(canonical_path, &projection)?;
    Ok(Some(projection))
}

fn migrate_legacy_workspace_work_items(
    repo_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkspaceWorkItemsProjection>> {
    if let Some(projection) = load_workspace_work_items_from_path(canonical_path)? {
        return Ok(Some(projection));
    }
    let legacy_path = legacy_workspace_work_items_path_for_repo_path(repo_path);
    if legacy_path == canonical_path {
        return load_workspace_work_items_from_path(canonical_path);
    }
    let Some(projection) = load_workspace_work_items_from_path(&legacy_path)? else {
        return Ok(None);
    };
    save_workspace_work_items_projection_to_path(canonical_path, &projection)?;
    Ok(Some(projection))
}

pub fn load_workspace_projection(repo_path: &Path) -> Result<Option<WorkspaceProjection>> {
    let path = gwt_workspace_projection_path_for_repo_path(repo_path);
    if let Some(projection) = load_workspace_projection_from_path(&path)? {
        return Ok(Some(projection));
    }
    migrate_legacy_workspace_projection(repo_path, &path)
}

/// SPEC-2359 FR-094 / FR-097 / FR-098 / FR-099: resolve the currently
/// assigned Workspace id for a given session. Returns `Some(id)` when
/// the agent is assigned to a Workspace and `None` otherwise (Unassigned
/// agent, missing agent, or load failure). Callers use this for Board
/// audience auto-attach, reminder injection scoping, and the
/// duplicate-work coordination gate corpus.
pub fn resolve_workspace_id_for_session(repo_path: &Path, session_id: &str) -> Option<String> {
    let projection = load_workspace_projection(repo_path).ok().flatten()?;
    projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .filter(|agent| !agent.is_unassigned())
        .and_then(|agent| agent.workspace_id.clone())
}

/// SPEC-2359 FR-097: resolve the currently assigned Workspace id for a
/// mention target. `target_kind` is `BoardMentionTargetKind::Agent` or
/// `BoardMentionTargetKind::Session`; the target value is matched
/// against agent display_name / agent_id (agent) or session_id (session).
/// Returns `Some(id)` when the matched agent is assigned, `None`
/// otherwise.
pub fn resolve_workspace_id_for_mention(
    repo_path: &Path,
    target_kind: &str,
    target_value: &str,
) -> Option<String> {
    let projection = load_workspace_projection(repo_path).ok().flatten()?;
    projection
        .agents
        .iter()
        .find(|agent| match target_kind {
            "session" => agent.session_id == target_value,
            "agent" => {
                agent.agent_id == target_value
                    || agent.display_name == target_value
                    || agent.display_name.eq_ignore_ascii_case(target_value)
            }
            _ => false,
        })
        .filter(|agent| !agent.is_unassigned())
        .and_then(|agent| agent.workspace_id.clone())
}

pub fn load_or_default_workspace_projection(repo_path: &Path) -> Result<WorkspaceProjection> {
    Ok(load_workspace_projection(repo_path)?
        .unwrap_or_else(|| WorkspaceProjection::default_for_project(repo_path)))
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
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let journal_path = gwt_workspace_journal_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_journal_path_for_repo_path(repo_path),
        &journal_path,
    )?;
    let entry = update_workspace_projection_with_journal_paths(
        &current_path,
        &journal_path,
        repo_path,
        update,
    )?;
    if let Some(projection) = load_workspace_projection(repo_path)? {
        let event = workspace_work_event_from_journal_entry(&projection, &entry);
        record_workspace_work_event(repo_path, event)?;
    }
    Ok(entry)
}

pub fn mark_workspace_agent_stopped(
    repo_path: &Path,
    session_id: &str,
    window_id: Option<&str>,
) -> Result<bool> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    mark_workspace_agent_stopped_at(&current_path, repo_path, session_id, window_id, Utc::now())
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

pub fn load_workspace_work_items(repo_path: &Path) -> Result<Option<WorkspaceWorkItemsProjection>> {
    migrate_legacy_workspace_work_items(
        repo_path,
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
    )
}

pub fn load_workspace_work_items_from_path(
    path: &Path,
) -> Result<Option<WorkspaceWorkItemsProjection>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| GwtError::Other(format!("workspace work items json: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn load_or_synthesize_workspace_work_items(
    repo_path: &Path,
) -> Result<WorkspaceWorkItemsProjection> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let journal_path = gwt_workspace_journal_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_journal_path_for_repo_path(repo_path),
        &journal_path,
    )?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    load_or_synthesize_workspace_work_items_from_paths(
        &work_items_path,
        &current_path,
        &journal_path,
        repo_path,
    )
}

pub fn load_or_synthesize_workspace_work_items_from_paths(
    work_items_path: &Path,
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
) -> Result<WorkspaceWorkItemsProjection> {
    if let Some(projection) = load_workspace_work_items_from_path(work_items_path)? {
        return Ok(projection);
    }
    synthesize_workspace_work_items_from_legacy_paths(current_path, journal_path, project_root)
}

pub fn save_workspace_work_items_projection_to_path(
    path: &Path,
    projection: &WorkspaceWorkItemsProjection,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| GwtError::Other(format!("workspace work items json: {error}")))?;
    write_atomic(path, &bytes)
}

pub fn record_workspace_work_event(repo_path: &Path, event: WorkspaceWorkEvent) -> Result<()> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let events_path = gwt_workspace_work_events_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_work_events_path_for_repo_path(repo_path),
        &events_path,
    )?;
    record_workspace_work_event_paths(&work_items_path, &events_path, event)
}

pub fn record_workspace_work_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkspaceWorkEvent,
) -> Result<()> {
    let mut projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkspaceWorkItemsProjection::empty(event.updated_at));
    projection.apply_event(event.clone());
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    append_workspace_work_event_to_path(events_path, &event)?;
    Ok(())
}

/// SPEC-2359 US-37 / FR-117..FR-120: Emit a single Done `WorkspaceWorkEvent`
/// for `work_item_id` iff no Done event has been recorded for it yet. This is
/// the canonical write path for auto-done emission from PR merge detection,
/// user-confirmed cleanup, and startup retroactive migration. Returns
/// `Ok(true)` when a new Done event was appended, `Ok(false)` when an
/// existing Done event was found (idempotent noop).
pub fn emit_workspace_done_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    if work_item_has_done_event_in_projection(work_items_path, work_item_id)? {
        return Ok(false);
    }
    let mut event = WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Done, work_item_id, updated_at);
    event.status_category = Some(WorkspaceStatusCategory::Done);
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    Ok(true)
}

/// SPEC-2359 US-37 / FR-117..FR-120: Convenience wrapper resolving the
/// project-scoped work_items and work_events paths from `repo_path` and
/// invoking [`emit_workspace_done_event_if_absent_paths`].
pub fn emit_workspace_done_event_if_absent(
    repo_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_done_event_if_absent_paths(
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_path_for_repo_path(repo_path),
        work_item_id,
        updated_at,
    )
}

fn work_item_has_done_event_in_projection(
    work_items_path: &Path,
    work_item_id: &str,
) -> Result<bool> {
    let Some(projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(false);
    };
    Ok(projection
        .work_items
        .iter()
        .filter(|item| item.id == work_item_id)
        .any(|item| {
            item.events
                .iter()
                .any(|event| event.kind == WorkspaceWorkEventKind::Done)
        }))
}

/// SPEC-2359 US-37 / FR-119: Scan WorkItems and the current Workspace
/// projection and emit a Done event for each eligible target that is still
/// incomplete and not yet recorded as Done.
///
/// Two eligibility sources are consulted:
///
/// 1. Each WorkItem in `work_items.json`: at least one
///    [`WorkspaceExecutionContainerRef`] whose `branch` starts with `"work/"`
///    and whose `pr_state` matches `"merged"` case-insensitively.
/// 2. The current Workspace projection at `current_path`: `git_details.branch`
///    starts with `"work/"`, `pr_state` matches `"merged"` case-insensitively,
///    and `created_by_start_work` is true. This catches the case where an
///    older gwtd version updated `current.json` after a PR merged but never
///    emitted a corresponding Done event, which would otherwise leave the
///    WorkItem stuck outside the Completed column.
///
/// Emission is delegated to [`emit_workspace_done_event_if_absent_paths`] so
/// repeated invocations are idempotent. Returns the number of WorkItems that
/// were newly Done'd. Missing files (`work_items.json`, `current.json`) skip
/// silently without surfacing an error.
pub fn retroactive_auto_done_scan_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    now: DateTime<Utc>,
) -> Result<usize> {
    let mut emitted = 0;

    if let Some(work_items_projection) = load_workspace_work_items_from_path(work_items_path)? {
        let candidates: Vec<String> = work_items_projection
            .work_items
            .iter()
            .filter(|item| item.is_incomplete())
            .filter(|item| work_item_is_eligible_for_auto_done(item))
            .map(|item| item.id.clone())
            .collect();
        for work_item_id in candidates {
            if emit_workspace_done_event_if_absent_paths(
                work_items_path,
                events_path,
                &work_item_id,
                now,
            )? {
                emitted += 1;
            }
        }
    }

    if let Some(current) = load_workspace_projection_from_path(current_path)? {
        if workspace_projection_is_eligible_for_auto_done(&current)
            && emit_workspace_done_event_if_absent_paths(
                work_items_path,
                events_path,
                &current.id,
                now,
            )?
        {
            emitted += 1;
        }
    }

    Ok(emitted)
}

/// SPEC-2359 US-37: Schema version recorded in `work_items.migration.json`.
/// Bumping this value forces [`rebuild_work_items_from_events_paths`] and
/// [`rebuild_work_items_from_events_for_repo`] to re-run on existing data.
/// Version 1 corresponds to the terminal-Done apply_event fix; prior
/// projections may show stale non-Done status_category for items whose
/// latest event regressed Done.
pub const WORKSPACE_WORK_ITEMS_REBUILD_VERSION: u32 = 1;

/// SPEC-2359 US-37: Outcome of the work_items.json rebuild migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceWorkItemsRebuildOutcome {
    /// `work_events.jsonl` does not exist. Nothing to rebuild.
    Missing,
    /// Marker already records the current rebuild version. Skip silently.
    AlreadyMigrated,
    /// Rebuilt `work_items.json` from the event log and wrote the marker.
    Applied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceWorkItemsRebuildMarker {
    version: u32,
    #[serde(default)]
    migrated_at: Option<DateTime<Utc>>,
}

/// SPEC-2359 US-37: Rebuild `work_items.json` by replaying every event in
/// `work_events.jsonl` through the (fixed) apply_event semantics. This
/// recovers historical Done state that the legacy apply_event regressed
/// when subsequent heartbeat update events arrived after the Done event.
/// The rebuild is idempotent across daemon restarts: a marker file records
/// the current schema version.
pub fn rebuild_work_items_from_events_paths(
    work_items_path: &Path,
    events_path: &Path,
    marker_path: &Path,
) -> Result<WorkspaceWorkItemsRebuildOutcome> {
    if rebuild_marker_at_or_above(marker_path, WORKSPACE_WORK_ITEMS_REBUILD_VERSION)? {
        return Ok(WorkspaceWorkItemsRebuildOutcome::AlreadyMigrated);
    }
    if !events_path.exists() {
        return Ok(WorkspaceWorkItemsRebuildOutcome::Missing);
    }
    let content = fs::read_to_string(events_path)?;
    let mut events: Vec<WorkspaceWorkEvent> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<WorkspaceWorkEvent>(line)
                .map_err(|err| GwtError::Other(format!("workspace work event json: {err}")))
        })
        .collect::<Result<Vec<_>>>()?;
    events.sort_by_key(|event| event.updated_at);
    let initial_updated_at = events
        .first()
        .map(|event| event.updated_at)
        .unwrap_or_else(chrono::Utc::now);
    let mut projection = WorkspaceWorkItemsProjection::empty(initial_updated_at);
    for event in events {
        projection.apply_event(event);
    }
    projection.updated_at = chrono::Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    write_rebuild_marker(marker_path)?;
    Ok(WorkspaceWorkItemsRebuildOutcome::Applied)
}

/// SPEC-2359 US-37: Convenience wrapper for the daemon bootstrap hook.
/// Resolves the project-scoped paths and invokes
/// [`rebuild_work_items_from_events_paths`].
pub fn rebuild_work_items_from_events_for_repo(
    repo_path: &Path,
) -> Result<WorkspaceWorkItemsRebuildOutcome> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let events_path = gwt_workspace_work_events_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_work_events_path_for_repo_path(repo_path),
        &events_path,
    )?;
    let marker_path = work_items_path
        .parent()
        .map(|dir| dir.join("work_items.migration.json"))
        .unwrap_or_else(|| PathBuf::from("work_items.migration.json"));
    rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
}

fn rebuild_marker_at_or_above(path: &Path, required: u32) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    Ok(
        serde_json::from_str::<WorkspaceWorkItemsRebuildMarker>(&body)
            .map(|marker| marker.version >= required)
            .unwrap_or(false),
    )
}

fn write_rebuild_marker(path: &Path) -> Result<()> {
    let marker = WorkspaceWorkItemsRebuildMarker {
        version: WORKSPACE_WORK_ITEMS_REBUILD_VERSION,
        migrated_at: Some(chrono::Utc::now()),
    };
    let body = serde_json::to_vec_pretty(&marker)
        .map_err(|error| GwtError::Other(format!("work items migration marker: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, &body)
}

/// SPEC-2359 US-37 / FR-119: Convenience wrapper resolving the project-scoped
/// current, work_items, and work_events paths from `repo_path` and invoking
/// [`retroactive_auto_done_scan_paths`].
pub fn retroactive_auto_done_scan(repo_path: &Path, now: DateTime<Utc>) -> Result<usize> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let events_path = gwt_workspace_work_events_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_work_events_path_for_repo_path(repo_path),
        &events_path,
    )?;
    retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
}

fn work_item_is_eligible_for_auto_done(item: &WorkspaceWorkItem) -> bool {
    item.execution_containers.iter().any(|container| {
        let branch_starts_with_work = container
            .branch
            .as_deref()
            .is_some_and(|branch| branch.starts_with("work/"));
        let pr_state_merged = container
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
        branch_starts_with_work && pr_state_merged
    })
}

fn workspace_projection_is_eligible_for_auto_done(projection: &WorkspaceProjection) -> bool {
    let Some(details) = projection.git_details.as_ref() else {
        return false;
    };
    let branch_starts_with_work = details
        .branch
        .as_deref()
        .is_some_and(|branch| branch.starts_with("work/"));
    let pr_state_merged = details
        .pr_state
        .as_deref()
        .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
    branch_starts_with_work && pr_state_merged && details.created_by_start_work
}

/// SPEC-2359 US-37 / FR-118: Emit a Done WorkspaceWorkEvent for the Workspace
/// WorkItem currently associated with `branch`. The function loads the current
/// projection at `current_path` and emits Done iff
/// `projection.git_details.branch` matches `branch`. Used by user-confirmed
/// cleanup to mark the matching Workspace as completed after worktree/branch
/// deletion succeeds. Idempotent per `work_item_id` via
/// [`emit_workspace_done_event_if_absent_paths`].
pub fn emit_workspace_done_event_for_branch_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    branch: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let Some(projection) = load_workspace_projection_from_path(current_path)? else {
        return Ok(false);
    };
    let matches = projection
        .git_details
        .as_ref()
        .and_then(|details| details.branch.as_deref())
        .is_some_and(|stored_branch| stored_branch == branch);
    if !matches {
        return Ok(false);
    }
    emit_workspace_done_event_if_absent_paths(work_items_path, events_path, &projection.id, now)
}

/// SPEC-2359 US-37 / FR-118: Convenience wrapper resolving project-scoped
/// paths from `repo_path` and invoking
/// [`emit_workspace_done_event_for_branch_paths`].
pub fn emit_workspace_done_event_for_branch(
    repo_path: &Path,
    branch: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_done_event_for_branch_paths(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_path_for_repo_path(repo_path),
        branch,
        now,
    )
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

pub fn append_workspace_work_event_to_path(path: &Path, event: &WorkspaceWorkEvent) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, event)
        .map_err(|error| GwtError::Other(format!("workspace work event json: {error}")))?;
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

fn synthesize_workspace_work_items_from_legacy_paths(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
) -> Result<WorkspaceWorkItemsProjection> {
    let projection = load_workspace_projection_from_path(current_path)?;
    let mut journal_entries =
        load_recent_workspace_journal_entries_from_path(journal_path, usize::MAX)?;
    journal_entries.sort_by_key(|entry| entry.updated_at);
    let updated_at = projection
        .as_ref()
        .map(|projection| projection.updated_at)
        .or_else(|| journal_entries.last().map(|entry| entry.updated_at))
        .unwrap_or_else(Utc::now);
    let Some(item) = synthesize_workspace_work_item_from_legacy(
        projection.as_ref(),
        &journal_entries,
        project_root,
    ) else {
        return Ok(WorkspaceWorkItemsProjection::empty(updated_at));
    };
    Ok(WorkspaceWorkItemsProjection {
        updated_at: item.updated_at,
        work_items: vec![item],
    })
}

fn synthesize_workspace_work_item_from_legacy(
    projection: Option<&WorkspaceProjection>,
    journal_entries: &[WorkspaceJournalEntry],
    _project_root: &Path,
) -> Option<WorkspaceWorkItem> {
    if projection.is_none() && journal_entries.is_empty() {
        return None;
    }
    if let Some(projection) = projection {
        let has_workspace_identity = projection.assigned_agents().next().is_some()
            || projection.git_details.is_some()
            || !projection.board_refs.is_empty()
            || projection.status_category != WorkspaceStatusCategory::Unknown;
        if journal_entries.is_empty() && !has_workspace_identity {
            return None;
        }
    }
    let first_entry = journal_entries.first();
    let last_entry = journal_entries.last();
    let id = projection
        .map(|projection| projection.id.clone())
        .or_else(|| first_entry.map(|entry| format!("legacy-{}", entry.id)))?;
    let title = projection
        .map(|projection| projection.title.clone())
        .or_else(|| {
            first_entry.and_then(|entry| {
                non_empty_clone(entry.title.as_deref())
                    .or_else(|| non_empty_clone(entry.agent_title_summary.as_deref()))
                    .or_else(|| non_empty_clone(entry.summary.as_deref()))
            })
        })
        .unwrap_or_else(|| "Workspace history".to_string());
    let status_category = projection
        .map(WorkspaceProjection::effective_status_category)
        .or_else(|| last_entry.and_then(|entry| entry.status_category))
        .unwrap_or(WorkspaceStatusCategory::Unknown);
    let summary = projection
        .and_then(|projection| projection.summary.clone())
        .or_else(|| last_entry.and_then(|entry| entry.summary.clone()))
        .or_else(|| last_entry.and_then(|entry| entry.status_text.clone()));
    let owner = projection
        .and_then(|projection| projection.owner.clone())
        .or_else(|| last_entry.and_then(|entry| entry.owner.clone()));
    let created_at = first_entry
        .map(|entry| entry.updated_at)
        .or_else(|| projection.map(|projection| projection.updated_at))
        .unwrap_or_else(Utc::now);
    let updated_at = projection
        .map(|projection| projection.updated_at)
        .or_else(|| last_entry.map(|entry| entry.updated_at))
        .unwrap_or(created_at);
    let completed_at = (status_category == WorkspaceStatusCategory::Done).then_some(updated_at);
    let mut item = WorkspaceWorkItem {
        id: id.clone(),
        title,
        intent: summary.clone(),
        summary,
        status_category,
        owner,
        created_at,
        updated_at,
        completed_at,
        agents: Vec::new(),
        execution_containers: Vec::new(),
        board_refs: projection
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
    };
    if let Some(projection) = projection {
        item.agents.extend(
            projection
                .assigned_agents()
                .map(|agent| WorkspaceWorkAgentRef {
                    session_id: agent.session_id.clone(),
                    agent_id: Some(agent.agent_id.clone()),
                    display_name: Some(agent.display_name.clone()),
                    updated_at: agent.updated_at,
                }),
        );
        if let Some(details) = projection.git_details.as_ref() {
            item.execution_containers
                .push(WorkspaceExecutionContainerRef {
                    branch: details.branch.clone(),
                    worktree_path: details.worktree_path.clone(),
                    pr_number: details.pr_number,
                    pr_url: details.pr_url.clone(),
                    pr_state: details.pr_state.clone(),
                });
        }
    }
    for (index, entry) in journal_entries.iter().enumerate() {
        let mut event = WorkspaceWorkEvent::new(
            workspace_work_event_kind_from_journal(index, entry),
            id.clone(),
            entry.updated_at,
        );
        event.id = format!("legacy-journal-{}", entry.id);
        event.title = entry
            .title
            .clone()
            .or_else(|| entry.agent_title_summary.clone());
        event.intent = entry
            .agent_current_focus
            .clone()
            .or_else(|| entry.summary.clone());
        event.summary = entry.summary.clone().or_else(|| entry.status_text.clone());
        event.status_category = entry.status_category;
        event.owner = entry.owner.clone();
        event.next_action = entry.next_action.clone();
        event.agent_session_id = entry.agent_session_id.clone();
        if let Some(session_id) = non_empty_clone(entry.agent_session_id.as_deref()) {
            if !item
                .agents
                .iter()
                .any(|agent| agent.session_id == session_id)
            {
                item.agents.push(WorkspaceWorkAgentRef {
                    session_id,
                    agent_id: None,
                    display_name: entry.agent_title_summary.clone(),
                    updated_at: entry.updated_at,
                });
            }
        }
        item.events.push(event);
    }
    if item.events.is_empty() {
        let mut event = WorkspaceWorkEvent::new(
            workspace_work_event_kind_from_status(status_category, 0),
            id,
            updated_at,
        );
        event.title = Some(item.title.clone());
        event.summary = item.summary.clone();
        event.status_category = Some(status_category);
        event.owner = item.owner.clone();
        item.events.push(event);
    }
    item.events.sort_by_key(|event| event.updated_at);
    Some(item)
}

fn workspace_work_event_from_journal_entry(
    projection: &WorkspaceProjection,
    entry: &WorkspaceJournalEntry,
) -> WorkspaceWorkEvent {
    let mut event = WorkspaceWorkEvent::new(
        workspace_work_event_kind_from_status(
            entry.status_category.unwrap_or(projection.status_category),
            1,
        ),
        projection.id.clone(),
        entry.updated_at,
    );
    event.title = entry
        .title
        .clone()
        .or_else(|| entry.agent_title_summary.clone())
        .or_else(|| Some(projection.title.clone()));
    event.intent = entry
        .agent_current_focus
        .clone()
        .or_else(|| entry.summary.clone())
        .or_else(|| projection.summary.clone());
    event.summary = entry.summary.clone().or_else(|| entry.status_text.clone());
    event.status_category = Some(entry.status_category.unwrap_or(projection.status_category));
    event.owner = entry.owner.clone().or_else(|| projection.owner.clone());
    event.next_action = entry.next_action.clone();
    event.agent_session_id = entry.agent_session_id.clone();
    if let Some(session_id) = entry.agent_session_id.as_deref() {
        if let Some(agent) = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session_id)
        {
            event.agent_id = Some(agent.agent_id.clone());
            event.display_name = Some(agent.display_name.clone());
        }
    }
    event.execution_container = workspace_execution_container_from_projection(projection);
    event
}

pub fn workspace_work_event_from_board_entry(
    projection: &WorkspaceProjection,
    entry: &BoardEntry,
) -> WorkspaceWorkEvent {
    let mut event = WorkspaceWorkEvent::new(
        workspace_work_event_kind_from_board_entry(entry),
        projection.id.clone(),
        entry.updated_at,
    );
    event.title = entry
        .title_summary
        .clone()
        .or_else(|| first_nonempty_line(&entry.body))
        .or_else(|| Some(projection.title.clone()));
    event.intent = entry.title_summary.clone();
    event.summary = Some(entry.body.clone());
    event.status_category = Some(match entry.kind {
        BoardEntryKind::Blocked => WorkspaceStatusCategory::Blocked,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Claim
        | BoardEntryKind::Handoff
        | BoardEntryKind::Decision => WorkspaceStatusCategory::Active,
        BoardEntryKind::Request | BoardEntryKind::Impact | BoardEntryKind::Question => {
            projection.status_category
        }
    });
    event.owner = entry
        .related_owners
        .first()
        .cloned()
        .or_else(|| projection.owner.clone());
    event.agent_session_id = entry.origin_session_id.clone();
    event.agent_id = entry.origin_agent_id.clone();
    event.board_entry_id = Some(entry.id.clone());
    event.execution_container = workspace_execution_container_from_projection(projection);
    event
}

fn workspace_work_event_status(event: &WorkspaceWorkEvent) -> WorkspaceStatusCategory {
    event.status_category.unwrap_or(match event.kind {
        WorkspaceWorkEventKind::Done => WorkspaceStatusCategory::Done,
        WorkspaceWorkEventKind::Blocked => WorkspaceStatusCategory::Blocked,
        WorkspaceWorkEventKind::Start
        | WorkspaceWorkEventKind::Claim
        | WorkspaceWorkEventKind::Update
        | WorkspaceWorkEventKind::Handoff
        | WorkspaceWorkEventKind::Resume
        | WorkspaceWorkEventKind::Split
        | WorkspaceWorkEventKind::Merge
        | WorkspaceWorkEventKind::Pr => WorkspaceStatusCategory::Active,
    })
}

fn workspace_work_event_kind_from_board_entry(entry: &BoardEntry) -> WorkspaceWorkEventKind {
    match entry.kind {
        BoardEntryKind::Claim => WorkspaceWorkEventKind::Claim,
        BoardEntryKind::Blocked => WorkspaceWorkEventKind::Blocked,
        BoardEntryKind::Handoff => WorkspaceWorkEventKind::Handoff,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Decision
        | BoardEntryKind::Request
        | BoardEntryKind::Impact
        | BoardEntryKind::Question => WorkspaceWorkEventKind::Update,
    }
}

fn workspace_work_event_kind_from_journal(
    index: usize,
    entry: &WorkspaceJournalEntry,
) -> WorkspaceWorkEventKind {
    workspace_work_event_kind_from_status(
        entry
            .status_category
            .unwrap_or(WorkspaceStatusCategory::Unknown),
        index,
    )
}

fn workspace_work_event_kind_from_status(
    status_category: WorkspaceStatusCategory,
    index: usize,
) -> WorkspaceWorkEventKind {
    match status_category {
        WorkspaceStatusCategory::Done => WorkspaceWorkEventKind::Done,
        WorkspaceStatusCategory::Blocked => WorkspaceWorkEventKind::Blocked,
        _ if index == 0 => WorkspaceWorkEventKind::Start,
        _ => WorkspaceWorkEventKind::Update,
    }
}

fn workspace_execution_container_from_projection(
    projection: &WorkspaceProjection,
) -> Option<WorkspaceExecutionContainerRef> {
    projection
        .git_details
        .as_ref()
        .map(|details| WorkspaceExecutionContainerRef {
            branch: details.branch.clone(),
            worktree_path: details.worktree_path.clone(),
            pr_number: details.pr_number,
            pr_url: details.pr_url.clone(),
            pr_state: details.pr_state.clone(),
        })
}

fn workspace_execution_container_same(
    left: &WorkspaceExecutionContainerRef,
    right: &WorkspaceExecutionContainerRef,
) -> bool {
    (left.branch.is_some() && left.branch == right.branch)
        || (left.worktree_path.is_some() && left.worktree_path == right.worktree_path)
        || (left.pr_number.is_some() && left.pr_number == right.pr_number)
        || (left.pr_url.is_some() && left.pr_url == right.pr_url)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn non_empty_clone(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn first_nonempty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
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

/// SPEC-2359 US-41 (FR-151): classify a Workspace projection as stale based
/// on Git side-effect signals (worktree existence, linked PR state) and a
/// time threshold (`now - updated_at` > `config.archive_after_days`).
///
/// Returns `None` when no stale signal applies; returns the single matching
/// [`StaleReason`] when exactly one applies; returns [`StaleReason::Compound`]
/// when more than one applies so callers can prioritize compound evidence.
///
/// Network-free: PR state is read from `projection.linked_prs[].state` and
/// `git_details.pr_state`, which are populated by the existing GitHub Issue
/// cache via `gh` API (the caller keeps that cache fresh).
pub fn workspace_projection_stale_reason(
    projection: &WorkspaceProjection,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
) -> Option<StaleReason> {
    let mut reasons: Vec<StaleReason> = Vec::with_capacity(3);

    if let Some(git) = &projection.git_details {
        if let Some(worktree_path) = &git.worktree_path {
            if !worktree_path.exists() {
                reasons.push(StaleReason::WorktreeMissing);
            }
        }
    }

    let mut pr_closed = projection
        .git_details
        .as_ref()
        .and_then(|git| git.pr_state.as_deref())
        .map(pr_state_is_closed)
        .unwrap_or(false);
    if !pr_closed {
        pr_closed = projection
            .linked_prs
            .iter()
            .any(|pr| pr.state.as_deref().map(pr_state_is_closed).unwrap_or(false));
    }
    if pr_closed {
        reasons.push(StaleReason::PrClosed);
    }

    let threshold = chrono::Duration::days(config.archive_after_days as i64);
    if now.signed_duration_since(projection.updated_at) > threshold {
        reasons.push(StaleReason::TimeThreshold);
    }

    match reasons.len() {
        0 => None,
        1 => Some(reasons[0]),
        _ => Some(StaleReason::Compound),
    }
}

fn pr_state_is_closed(state: &str) -> bool {
    matches!(state.to_ascii_lowercase().as_str(), "merged" | "closed")
}

/// SPEC-2359 US-41 (FR-155): why a Workspace projection is left untouched by
/// the pruner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneSkipReason {
    /// `workspace_projection_stale_reason()` returned `None`.
    NotStale,
    /// One or more agents on this Workspace are still affiliated or have a
    /// live window (FR-155).
    ActiveAgent,
    /// `lifecycle_stage = Archived` but `delete_after_archive_days` has not
    /// elapsed yet.
    ArchivedTooSoon,
}

/// SPEC-2359 US-41 (FR-154): action the pruner intends to take on a single
/// Workspace projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PruneAction {
    /// Leave the projection untouched.
    Skip { reason: PruneSkipReason },
    /// Transition `lifecycle_stage` from `Active` (or any non-Archived stage)
    /// to `Archived` and persist the change. The directory is preserved so
    /// users can recover it manually.
    Archive,
    /// Physically remove `~/.gwt/projects/<repo-hash>/workspace/` after the
    /// archive grace period.
    Delete,
}

/// SPEC-2359 US-41: a single Workspace projection classified by
/// [`classify_workspace_projections`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedProjection {
    pub workspace_id: String,
    pub project_root: PathBuf,
    pub workspace_dir: PathBuf,
    pub lifecycle_stage: WorkspaceLifecycleStage,
    pub updated_at: DateTime<Utc>,
    pub stale_reason: Option<StaleReason>,
    pub action: PruneAction,
}

/// SPEC-2359 US-41 (FR-153): aggregate counts returned by [`apply_prune_plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneSummary {
    pub skipped: usize,
    pub archived: usize,
    pub deleted: usize,
}

/// SPEC-2359 US-41 (FR-153, FR-154, FR-155): walk `scan_root` (typically
/// `~/.gwt/projects/`), load each Workspace projection, and classify it as
/// `Skip` / `Archive` / `Delete`.
///
/// `is_active_session` lets the caller bridge in the live-window registry or
/// agent affiliation state held outside this crate, so the pruner stays
/// network-free and registry-free at the core layer.
pub fn classify_workspace_projections<F>(
    scan_root: &Path,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
    is_active_session: F,
) -> Vec<ClassifiedProjection>
where
    F: Fn(&WorkspaceProjection) -> bool,
{
    let mut results = Vec::new();

    let entries = match fs::read_dir(scan_root) {
        Ok(entries) => entries,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }
        let state_dir = project_dir.join("project-state");
        let legacy_dir = project_dir.join("workspace");
        let workspace_dir = if state_dir.join("current.json").is_file() {
            state_dir
        } else if legacy_dir.join("current.json").is_file() {
            legacy_dir
        } else {
            continue;
        };
        let current_json = workspace_dir.join("current.json");
        let projection = match load_workspace_projection_from_path(&current_json) {
            Ok(Some(p)) => p,
            _ => continue,
        };

        let stale_reason = workspace_projection_stale_reason(&projection, config, now);

        let action = if is_active_session(&projection) {
            PruneAction::Skip {
                reason: PruneSkipReason::ActiveAgent,
            }
        } else {
            match projection.lifecycle_stage {
                WorkspaceLifecycleStage::Archived => {
                    let elapsed = now.signed_duration_since(projection.updated_at);
                    let threshold = chrono::Duration::days(config.delete_after_archive_days as i64);
                    if elapsed > threshold {
                        PruneAction::Delete
                    } else {
                        PruneAction::Skip {
                            reason: PruneSkipReason::ArchivedTooSoon,
                        }
                    }
                }
                _ => match stale_reason {
                    Some(_) => PruneAction::Archive,
                    None => PruneAction::Skip {
                        reason: PruneSkipReason::NotStale,
                    },
                },
            }
        };

        results.push(ClassifiedProjection {
            workspace_id: projection.id.clone(),
            project_root: projection.project_root.clone(),
            workspace_dir,
            lifecycle_stage: projection.lifecycle_stage,
            updated_at: projection.updated_at,
            stale_reason,
            action,
        });
    }

    results
}

/// SPEC-2359 US-41 (FR-153, FR-154): apply a previously-classified plan.
///
/// When `dry_run` is `true`, the function counts the actions without touching
/// the filesystem so callers can preview the outcome. When `dry_run` is
/// `false`, `Archive` entries are persisted via `save_workspace_projection_to_path`
/// and `Delete` entries are removed via `fs::remove_dir_all` on the workspace
/// directory.
pub fn apply_prune_plan(plan: &[ClassifiedProjection], dry_run: bool) -> Result<PruneSummary> {
    let mut summary = PruneSummary::default();
    for item in plan {
        match &item.action {
            PruneAction::Skip { .. } => {
                summary.skipped += 1;
            }
            PruneAction::Archive => {
                if !dry_run {
                    let current_json = item.workspace_dir.join("current.json");
                    if let Ok(Some(mut projection)) =
                        load_workspace_projection_from_path(&current_json)
                    {
                        projection.lifecycle_stage = WorkspaceLifecycleStage::Archived;
                        projection.updated_at = Utc::now();
                        save_workspace_projection_to_path(&current_json, &projection)?;
                    }
                }
                summary.archived += 1;
            }
            PruneAction::Delete => {
                if !dry_run {
                    fs::remove_dir_all(&item.workspace_dir).map_err(|err| {
                        GwtError::Other(format!(
                            "failed to remove workspace dir {}: {}",
                            item.workspace_dir.display(),
                            err
                        ))
                    })?;
                }
                summary.deleted += 1;
            }
        }
    }
    Ok(summary)
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
            pr_url: None,
            pr_created_at: None,
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
            pr_url: None,
            pr_created_at: None,
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
    fn workspace_work_events_build_hot_projection_with_lifecycle_refs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 11, 1, 0, 0).unwrap();
        let done_at = Utc.with_ymd_and_hms(2026, 5, 11, 1, 30, 0).unwrap();

        let mut start = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "workitem-workspace-history",
            started_at,
        );
        start.title = Some("Workspace WorkItem history".to_string());
        start.intent = Some("Group duplicate Workspace work under one WorkItem".to_string());
        start.summary = Some("Start the WorkItem lifecycle implementation.".to_string());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.owner = Some("SPEC-2359".to_string());
        start.agent_session_id = Some("session-1".to_string());
        start.agent_id = Some("codex".to_string());
        start.display_name = Some("Codex".to_string());
        start.board_entry_id = Some("board-claim-1".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260510-2353".to_string()),
            worktree_path: Some(PathBuf::from("/repo/work/20260510-2353")),
            pr_number: Some(2638),
            pr_url: Some("https://github.com/akiojin/gwt/pull/2638".to_string()),
            pr_state: Some("open".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("record start event");

        let mut done = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Done,
            "workitem-workspace-history",
            done_at,
        );
        done.summary = Some("WorkItem lifecycle history is implemented.".to_string());
        done.status_category = Some(WorkspaceStatusCategory::Done);
        done.agent_session_id = Some("session-1".to_string());
        done.board_entry_id = Some("board-done-1".to_string());
        record_workspace_work_event_paths(&work_items_path, &events_path, done)
            .expect("record done event");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items");
        assert_eq!(projection.work_items.len(), 1);
        let item = &projection.work_items[0];
        assert_eq!(item.id, "workitem-workspace-history");
        assert_eq!(item.title, "Workspace WorkItem history");
        assert_eq!(
            item.intent.as_deref(),
            Some("Group duplicate Workspace work under one WorkItem")
        );
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(item.completed_at, Some(done_at));
        assert_eq!(
            item.board_refs,
            vec!["board-claim-1".to_string(), "board-done-1".to_string()]
        );
        assert_eq!(item.agents.len(), 1);
        assert_eq!(item.agents[0].session_id, "session-1");
        assert_eq!(item.execution_containers.len(), 1);
        assert_eq!(
            item.execution_containers[0].branch.as_deref(),
            Some("work/20260510-2353")
        );
        assert_eq!(item.events.len(), 2);
        assert_eq!(item.events[0].kind, WorkspaceWorkEventKind::Start);
        assert_eq!(item.events[1].kind, WorkspaceWorkEventKind::Done);

        let event_lines = std::fs::read_to_string(&events_path).expect("event log");
        assert_eq!(event_lines.lines().count(), 2);
    }

    #[test]
    fn workspace_work_items_synthesize_from_legacy_current_and_journal_without_rewrite() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let current_path = temp.path().join("workspace/current.json");
        let journal_path = temp.path().join("workspace/journal.jsonl");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let first_at = Utc.with_ymd_and_hms(2026, 5, 11, 2, 0, 0).unwrap();
        let second_at = Utc.with_ymd_and_hms(2026, 5, 11, 2, 5, 0).unwrap();

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        projection.id = "workspace-current".to_string();
        projection.title = "Workspace WorkItem history".to_string();
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Implementing WorkItem projection".to_string();
        projection.summary = Some("Legacy current state remains readable.".to_string());
        projection.owner = Some("SPEC-2359".to_string());
        projection.board_refs.push("board-legacy-1".to_string());
        save_workspace_projection_to_path(&current_path, &projection)
            .expect("save legacy projection");

        append_workspace_journal_entry_to_path(
            &journal_path,
            &WorkspaceJournalEntry {
                id: "journal-start".to_string(),
                project_root: project_root.clone(),
                title: Some("Workspace WorkItem history".to_string()),
                status_category: Some(WorkspaceStatusCategory::Active),
                status_text: Some("Started".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: None,
                summary: Some("Started from legacy journal.".to_string()),
                agent_session_id: Some("session-legacy".to_string()),
                agent_current_focus: Some("Implement lifecycle events".to_string()),
                agent_title_summary: Some("WorkItem history".to_string()),
                updated_at: first_at,
            },
        )
        .expect("append first journal");
        append_workspace_journal_entry_to_path(
            &journal_path,
            &WorkspaceJournalEntry {
                id: "journal-update".to_string(),
                project_root: project_root.clone(),
                title: None,
                status_category: Some(WorkspaceStatusCategory::Blocked),
                status_text: Some("Waiting for coordination decision".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Post Board handoff".to_string()),
                summary: Some("Blocked state from legacy journal.".to_string()),
                agent_session_id: Some("session-legacy".to_string()),
                agent_current_focus: None,
                agent_title_summary: Some("WorkItem history".to_string()),
                updated_at: second_at,
            },
        )
        .expect("append second journal");

        let synthesized = load_or_synthesize_workspace_work_items_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            &project_root,
        )
        .expect("synthesize work items");

        assert_eq!(synthesized.work_items.len(), 1);
        let item = &synthesized.work_items[0];
        assert_eq!(item.id, "workspace-current");
        assert_eq!(item.title, "Workspace WorkItem history");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(item.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(item.board_refs, vec!["board-legacy-1".to_string()]);
        assert_eq!(item.events.len(), 2);
        assert_eq!(
            item.events[0].summary.as_deref(),
            Some("Started from legacy journal.")
        );
        assert_eq!(item.events[1].kind, WorkspaceWorkEventKind::Blocked);
        assert!(
            !work_items_path.exists(),
            "legacy migration must be read-only until a real WorkItem event is recorded"
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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
    fn apply_update_upserts_minimal_agent_when_session_id_not_present() {
        // SPEC-2359 Phase U-6 (real root cause fix): `gwtd workspace update
        // --agent-session ... --title-summary X` must not silently drop the
        // update when the session is not yet in `projection.agents[]`. The
        // OLD installed gwtd lacks the SessionStart hook registration path
        // (Phase U-3) so `apply_update` is the only point where this CLI
        // path can guarantee the agent is registered.
        let updated_at = Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        assert!(projection.agents.is_empty());

        let journal = projection.apply_update(
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: None,
                agent_session_id: Some("session-new".to_string()),
                agent_current_focus: Some("focus".to_string()),
                agent_title_summary: Some("title from upsert".to_string()),
            },
            updated_at,
        );

        assert_eq!(projection.agents.len(), 1, "upsert must add the agent");
        let agent = &projection.agents[0];
        assert_eq!(agent.session_id, "session-new");
        assert_eq!(agent.title_summary.as_deref(), Some("title from upsert"));
        assert_eq!(agent.current_focus.as_deref(), Some("focus"));
        assert!(agent.is_unassigned());
        assert_eq!(agent.updated_at, updated_at);
        // Journal entry should still carry the requested fields.
        assert_eq!(journal.agent_session_id.as_deref(), Some("session-new"));
        assert_eq!(
            journal.agent_title_summary.as_deref(),
            Some("title from upsert"),
        );
        assert_eq!(journal.agent_current_focus.as_deref(), Some("focus"));
    }

    #[test]
    fn apply_update_preserves_existing_agent_when_session_id_matches() {
        // Upsert MUST NOT clobber pre-existing agent fields when the
        // session already exists in projection.agents (launch flow already
        // registered it with richer state). Only title_summary /
        // current_focus from the update should change.
        let updated_at = Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "session-launched".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("old focus".to_string()),
            title_summary: Some("old title".to_string()),
            worktree_path: Some(std::path::PathBuf::from("/repo")),
            branch: Some("work/x".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some("ws-1".to_string()),
            updated_at,
        });

        projection.apply_update(
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: None,
                agent_session_id: Some("session-launched".to_string()),
                agent_current_focus: None,
                agent_title_summary: Some("new title".to_string()),
            },
            updated_at,
        );

        assert_eq!(projection.agents.len(), 1, "must not duplicate");
        let agent = &projection.agents[0];
        // title_summary updated
        assert_eq!(agent.title_summary.as_deref(), Some("new title"));
        // everything else preserved
        assert_eq!(agent.window_id.as_deref(), Some("tab-1::agent-1"));
        assert_eq!(agent.agent_id, "codex");
        assert_eq!(agent.display_name, "Codex");
        assert_eq!(agent.current_focus.as_deref(), Some("old focus"));
        assert_eq!(
            agent.worktree_path.as_deref(),
            Some(std::path::Path::new("/repo"))
        );
        assert_eq!(agent.branch.as_deref(), Some("work/x"));
        assert_eq!(agent.workspace_id.as_deref(), Some("ws-1"));
        assert!(agent.is_assigned());
    }

    #[test]
    fn apply_update_upsert_records_journal_entry_with_same_shape() {
        // Journal entries from the upsert path must have the same shape as
        // entries from the pre-existing-agent path so downstream consumers
        // (UI, broadcasts) cannot tell them apart.
        let updated_at = Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, 0).unwrap();
        let mut projection = WorkspaceProjection::default_for_project("/repo");

        let journal = projection.apply_update(
            WorkspaceProjectionUpdate {
                title: None,
                status_category: None,
                status_text: None,
                owner: None,
                next_action: None,
                summary: None,
                agent_session_id: Some("session-stub".to_string()),
                agent_current_focus: None,
                agent_title_summary: Some("stub title".to_string()),
            },
            updated_at,
        );

        assert!(!journal.id.is_empty());
        assert_eq!(journal.project_root, projection.project_root);
        assert_eq!(journal.agent_session_id.as_deref(), Some("session-stub"));
        assert_eq!(journal.agent_title_summary.as_deref(), Some("stub title"));
        assert_eq!(journal.agent_current_focus, None);
        assert_eq!(journal.updated_at, updated_at);
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

    #[test]
    fn unassigned_agent_is_not_effective_active_workspace_status() {
        let mut projection = WorkspaceProjection::default_for_project("/tmp/repo");
        projection.register_unassigned_agent(WorkspaceAgentSummary {
            session_id: "session-unassigned".to_string(),
            window_id: Some("tab-1:agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: Some("work/20260511-0100".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: Utc::now(),
        });

        assert_eq!(projection.unassigned_agents().count(), 1);
        assert_eq!(
            projection.effective_status_category(),
            WorkspaceStatusCategory::Unknown,
            "Unassigned Agents must not make a Workspace active"
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
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
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

    fn assigned_agent(
        session_id: &str,
        agent_id: &str,
        workspace_id: &str,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.into(),
            window_id: None,
            agent_id: agent_id.into(),
            display_name: agent_id.into(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some(workspace_id.into()),
            updated_at: Utc::now(),
        }
    }

    fn unassigned_agent(session_id: &str, agent_id: &str) -> WorkspaceAgentSummary {
        let mut a = assigned_agent(session_id, agent_id, "_unused");
        a.affiliation_status = WorkspaceAgentAffiliationStatus::Unassigned;
        a.workspace_id = None;
        a
    }

    #[test]
    fn resolve_workspace_id_for_session_returns_assigned_workspace_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection
            .agents
            .push(assigned_agent("sess-A", "codex", "ws-1"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_session(dir.path(), "sess-A"),
            Some("ws-1".into())
        );
    }

    #[test]
    fn resolve_workspace_id_for_session_returns_none_for_unassigned_agent() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection.agents.push(unassigned_agent("sess-B", "codex"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(resolve_workspace_id_for_session(dir.path(), "sess-B"), None);
    }

    #[test]
    fn resolve_workspace_id_for_session_returns_none_when_session_missing() {
        let dir = tempfile::tempdir().unwrap();
        let projection = WorkspaceProjection::default_for_project(dir.path());
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_session(dir.path(), "sess-missing"),
            None
        );
    }

    #[test]
    fn resolve_workspace_id_for_mention_session_matches_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection
            .agents
            .push(assigned_agent("sess-C", "codex", "ws-2"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "session", "sess-C"),
            Some("ws-2".into())
        );
    }

    #[test]
    fn resolve_workspace_id_for_mention_agent_matches_display_or_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection
            .agents
            .push(assigned_agent("sess-D", "codex", "ws-3"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
            Some("ws-3".into())
        );
        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "agent", "Codex"),
            Some("ws-3".into()),
            "case-insensitive display-name match"
        );
    }

    #[test]
    fn resolve_workspace_id_for_mention_returns_none_for_unassigned_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection.agents.push(unassigned_agent("sess-E", "codex"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "session", "sess-E"),
            None
        );
        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "agent", "codex"),
            None
        );
    }

    #[test]
    fn resolve_workspace_id_for_mention_user_or_branch_kind_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection
            .agents
            .push(assigned_agent("sess-F", "codex", "ws-4"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "user", "akiojin"),
            None
        );
        assert_eq!(
            resolve_workspace_id_for_mention(dir.path(), "branch", "feature/x"),
            None
        );
    }

    // SPEC-2359 US-37 / T-236..T-239: auto-done emit helper and retroactive migration scanner.

    #[test]
    fn auto_done_emit_helper_appends_single_done_event_and_marks_work_item_done() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
        let done_at = Utc.with_ymd_and_hms(2026, 5, 13, 2, 0, 0).unwrap();

        let mut start =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, "wi-auto-done", started_at);
        start.title = Some("Auto-done test work".to_string());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("record start event");

        let emitted = emit_workspace_done_event_if_absent_paths(
            &work_items_path,
            &events_path,
            "wi-auto-done",
            done_at,
        )
        .expect("emit done");
        assert!(emitted, "first call must append a Done event");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items");
        assert_eq!(projection.work_items.len(), 1);
        let item = &projection.work_items[0];
        assert_eq!(item.id, "wi-auto-done");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(done_at));

        let events_text = std::fs::read_to_string(&events_path).expect("read events");
        let done_lines = events_text
            .lines()
            .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-auto-done"))
            .count();
        assert_eq!(done_lines, 1, "exactly one Done event must be persisted");
    }

    #[test]
    fn auto_done_emit_helper_is_idempotent_per_work_item_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
        let first_done_at = Utc.with_ymd_and_hms(2026, 5, 13, 2, 0, 0).unwrap();
        let second_done_at = Utc.with_ymd_and_hms(2026, 5, 13, 3, 0, 0).unwrap();

        let mut start =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, "wi-idempotent", started_at);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("record start event");

        let first = emit_workspace_done_event_if_absent_paths(
            &work_items_path,
            &events_path,
            "wi-idempotent",
            first_done_at,
        )
        .expect("first emit");
        let second = emit_workspace_done_event_if_absent_paths(
            &work_items_path,
            &events_path,
            "wi-idempotent",
            second_done_at,
        )
        .expect("second emit");

        assert!(first, "first call must append Done");
        assert!(!second, "second call must be a noop");

        let events_text = std::fs::read_to_string(&events_path).expect("read events");
        let done_lines = events_text
            .lines()
            .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-idempotent"))
            .count();
        assert_eq!(done_lines, 1, "Done event must not be duplicated");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items");
        let item = &projection.work_items[0];
        assert_eq!(item.completed_at, Some(first_done_at));
    }

    #[test]
    fn retroactive_auto_done_scan_marks_eligible_merged_work_branch_workitems() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 9, 0, 0).unwrap();

        let mut eligible =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, "wi-eligible", started_at);
        eligible.status_category = Some(WorkspaceStatusCategory::Active);
        eligible.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260513-0100".to_string()),
            worktree_path: None,
            pr_number: Some(1),
            pr_url: None,
            pr_state: Some("merged".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, eligible)
            .expect("record eligible start");

        let mut non_work = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "wi-non-work-branch",
            started_at,
        );
        non_work.status_category = Some(WorkspaceStatusCategory::Active);
        non_work.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/manual".to_string()),
            worktree_path: None,
            pr_number: Some(2),
            pr_url: None,
            pr_state: Some("merged".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, non_work)
            .expect("record non-work start");

        let mut not_merged =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, "wi-not-merged", started_at);
        not_merged.status_category = Some(WorkspaceStatusCategory::Active);
        not_merged.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260513-0200".to_string()),
            worktree_path: None,
            pr_number: Some(3),
            pr_url: None,
            pr_state: Some("open".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, not_merged)
            .expect("record not-merged start");

        let count =
            retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
                .expect("retroactive scan");
        assert_eq!(count, 1, "only the eligible WorkItem must be auto-Done'd");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items");
        let eligible_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "wi-eligible")
            .expect("eligible item");
        assert_eq!(eligible_item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(eligible_item.completed_at, Some(now));

        let non_work_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "wi-non-work-branch")
            .expect("non-work item");
        assert_eq!(
            non_work_item.status_category,
            WorkspaceStatusCategory::Active,
            "non-work/ branch must not be auto-Done'd",
        );

        let not_merged_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "wi-not-merged")
            .expect("not-merged item");
        assert_eq!(
            not_merged_item.status_category,
            WorkspaceStatusCategory::Active,
            "WorkItem without merged PR must not be auto-Done'd",
        );
    }

    #[test]
    fn retroactive_auto_done_scan_is_idempotent_across_invocations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap();
        let first_run = Utc.with_ymd_and_hms(2026, 5, 13, 9, 0, 0).unwrap();
        let second_run = Utc.with_ymd_and_hms(2026, 5, 13, 10, 0, 0).unwrap();

        let mut eligible =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, "wi-twice", started_at);
        eligible.status_category = Some(WorkspaceStatusCategory::Active);
        eligible.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260513-0100".to_string()),
            worktree_path: None,
            pr_number: Some(7),
            pr_url: None,
            pr_state: Some("merged".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, eligible)
            .expect("record start");

        let first = retroactive_auto_done_scan_paths(
            &current_path,
            &work_items_path,
            &events_path,
            first_run,
        )
        .expect("first scan");
        let second = retroactive_auto_done_scan_paths(
            &current_path,
            &work_items_path,
            &events_path,
            second_run,
        )
        .expect("second scan");

        assert_eq!(first, 1, "first scan must emit Done");
        assert_eq!(second, 0, "second scan must be noop");

        let events_text = std::fs::read_to_string(&events_path).expect("read events");
        let done_lines = events_text
            .lines()
            .filter(|line| line.contains("\"kind\":\"done\"") && line.contains("wi-twice"))
            .count();
        assert_eq!(done_lines, 1);
    }

    // SPEC-2359 US-37 / T-241: cleanup hook auto-done by branch match.

    #[test]
    fn emit_workspace_done_event_for_branch_emits_done_when_branch_matches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create repo");

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        projection.id = "wi-cleanup-target".to_string();
        projection.git_details = Some(GitDetails {
            branch: Some("work/auto-done-branch".to_string()),
            worktree_path: None,
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

        let mut start = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "wi-cleanup-target",
            Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap(),
        );
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("seed start");

        let emitted = emit_workspace_done_event_for_branch_paths(
            &current_path,
            &work_items_path,
            &events_path,
            "work/auto-done-branch",
            now,
        )
        .expect("emit");
        assert!(emitted, "branch match must trigger Done emit");

        let work_items = load_workspace_work_items_from_path(&work_items_path)
            .expect("load")
            .expect("work items");
        let item = work_items
            .work_items
            .iter()
            .find(|item| item.id == "wi-cleanup-target")
            .expect("item");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(now));
    }

    #[test]
    fn emit_workspace_done_event_for_branch_is_noop_when_branch_does_not_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create repo");

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        projection.id = "wi-different-branch".to_string();
        projection.git_details = Some(GitDetails {
            branch: Some("work/current-branch".to_string()),
            worktree_path: None,
            base_branch: None,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

        let mut start = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "wi-different-branch",
            Utc.with_ymd_and_hms(2026, 5, 13, 1, 0, 0).unwrap(),
        );
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("seed start");

        let emitted = emit_workspace_done_event_for_branch_paths(
            &current_path,
            &work_items_path,
            &events_path,
            "work/different-branch",
            now,
        )
        .expect("emit");
        assert!(!emitted, "non-matching branch must not trigger Done");

        let work_items = load_workspace_work_items_from_path(&work_items_path)
            .expect("load")
            .expect("work items");
        let item = work_items
            .work_items
            .iter()
            .find(|item| item.id == "wi-different-branch")
            .expect("item");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
    }

    // SPEC-2359 US-37 / T-242: retroactive migration startup robustness.

    #[test]
    fn retroactive_auto_done_scan_returns_zero_when_work_items_file_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();

        assert!(!work_items_path.exists());
        assert!(!current_path.exists());
        let count =
            retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
                .expect("scan with missing files must not error");
        assert_eq!(count, 0);
        assert!(
            !events_path.exists(),
            "missing inputs must skip without writing events"
        );
    }

    // SPEC-2359 US-37 / FR-119 current.json fallback (upgrade path).

    #[test]
    fn retroactive_auto_done_scan_emits_done_from_current_projection_when_work_items_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create repo");

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        projection.id = "wi-current-merged".to_string();
        projection.git_details = Some(GitDetails {
            branch: Some("work/20260513-0100".to_string()),
            worktree_path: None,
            base_branch: Some("origin/develop".to_string()),
            pr_number: Some(42),
            pr_state: Some("MERGED".to_string()),
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

        let count =
            retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
                .expect("scan");
        assert_eq!(
            count, 1,
            "current.json with merged work/* + start_work must trigger one Done emit",
        );

        let work_items = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items projection created via emit");
        let item = work_items
            .work_items
            .iter()
            .find(|item| item.id == "wi-current-merged")
            .expect("WorkItem created from emit");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(now));

        let second =
            retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
                .expect("second scan");
        assert_eq!(
            second, 0,
            "second scan must be noop after Done event exists"
        );
    }

    #[test]
    fn retroactive_auto_done_scan_skips_current_projection_without_start_work_flag() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("workspace/current.json");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let now = Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap();
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create repo");

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        projection.id = "wi-manual-branch".to_string();
        projection.git_details = Some(GitDetails {
            branch: Some("work/20260513-0200".to_string()),
            worktree_path: None,
            base_branch: None,
            pr_number: Some(43),
            pr_state: Some("merged".to_string()),
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: false,
            created_at: now,
        });
        save_workspace_projection_to_path(&current_path, &projection).expect("save projection");

        let count =
            retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
                .expect("scan");
        assert_eq!(
            count, 0,
            "non-start_work workspaces must be excluded from current.json fallback",
        );
        assert!(
            !events_path.exists(),
            "no work_events should be written for ineligible current projection",
        );
    }

    fn make_stale_projection(updated_at: DateTime<Utc>) -> WorkspaceProjection {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.updated_at = updated_at;
        projection
    }

    #[test]
    fn stale_reason_returns_none_for_fresh_active_workspace() {
        let now = Utc::now();
        let projection = make_stale_projection(now);
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            None,
        );
    }

    #[test]
    fn stale_reason_detects_missing_worktree() {
        let now = Utc::now();
        let mut projection = make_stale_projection(now);
        projection.git_details = Some(GitDetails {
            branch: Some("work/test".to_string()),
            worktree_path: Some(PathBuf::from(
                "/nonexistent/path/__stale_reason_should_not_exist_xyz__",
            )),
            base_branch: None,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            Some(StaleReason::WorktreeMissing),
        );
    }

    #[test]
    fn stale_reason_detects_pr_merged_via_git_details() {
        let now = Utc::now();
        let mut projection = make_stale_projection(now);
        projection.git_details = Some(GitDetails {
            branch: Some("work/test".to_string()),
            worktree_path: None,
            base_branch: None,
            pr_number: Some(123),
            pr_state: Some("merged".to_string()),
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            Some(StaleReason::PrClosed),
        );
    }

    #[test]
    fn stale_reason_detects_pr_closed_via_linked_prs() {
        let now = Utc::now();
        let mut projection = make_stale_projection(now);
        projection.linked_prs.push(WorkspacePrLink {
            number: 456,
            title: None,
            url: None,
            state: Some("Closed".to_string()),
        });
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            Some(StaleReason::PrClosed),
        );
    }

    #[test]
    fn stale_reason_detects_time_threshold() {
        let now = Utc::now();
        let projection = make_stale_projection(now - chrono::Duration::days(40));
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            Some(StaleReason::TimeThreshold),
        );
    }

    #[test]
    fn stale_reason_returns_compound_when_multiple_conditions_hold() {
        let now = Utc::now();
        let mut projection = make_stale_projection(now - chrono::Duration::days(40));
        projection.git_details = Some(GitDetails {
            branch: Some("work/test".to_string()),
            worktree_path: None,
            base_branch: None,
            pr_number: Some(789),
            pr_state: Some("merged".to_string()),
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: now,
        });
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(
            workspace_projection_stale_reason(&projection, &config, now),
            Some(StaleReason::Compound),
        );
    }

    #[test]
    fn workspace_retention_config_default_uses_30_60_days() {
        let config = WorkspaceRetentionConfig::default();
        assert_eq!(config.archive_after_days, 30);
        assert_eq!(config.delete_after_archive_days, 60);
    }

    #[test]
    fn stale_reason_as_str_matches_snake_case_serde() {
        assert_eq!(StaleReason::WorktreeMissing.as_str(), "worktree_missing");
        assert_eq!(StaleReason::PrClosed.as_str(), "pr_closed");
        assert_eq!(StaleReason::TimeThreshold.as_str(), "time_threshold");
        assert_eq!(StaleReason::Compound.as_str(), "compound");
    }

    fn write_projection_at(workspace_dir: &Path, projection: &WorkspaceProjection) {
        std::fs::create_dir_all(workspace_dir).expect("create workspace dir");
        let current = workspace_dir.join("current.json");
        save_workspace_projection_to_path(&current, projection).expect("save projection");
    }

    fn make_classify_projection(
        id: &str,
        project_root: &Path,
        updated_at: DateTime<Utc>,
        lifecycle: WorkspaceLifecycleStage,
    ) -> WorkspaceProjection {
        let mut projection = WorkspaceProjection::default_for_project(project_root);
        projection.id = id.to_string();
        projection.updated_at = updated_at;
        projection.lifecycle_stage = lifecycle;
        projection
    }

    #[test]
    fn classify_workspace_projections_returns_empty_for_missing_scan_root() {
        let scan_root = PathBuf::from("/nonexistent/projects/scan-root-xyz");
        let now = Utc::now();
        let result = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn classify_workspace_projections_classifies_stale_active_as_archive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();
        let project_dir = scan_root.join("abc123");
        let workspace_dir = project_dir.join("workspace");
        let now = Utc::now();
        let projection = make_classify_projection(
            "ws-archive-me",
            &project_dir,
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        write_projection_at(&workspace_dir, &projection);

        let result = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].workspace_id, "ws-archive-me");
        assert_eq!(result[0].action, PruneAction::Archive);
        assert_eq!(result[0].stale_reason, Some(StaleReason::TimeThreshold));
    }

    #[test]
    fn classify_workspace_projections_classifies_archived_beyond_threshold_as_delete() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();
        let project_dir = scan_root.join("def456");
        let workspace_dir = project_dir.join("workspace");
        let now = Utc::now();
        let projection = make_classify_projection(
            "ws-delete-me",
            &project_dir,
            now - chrono::Duration::days(90),
            WorkspaceLifecycleStage::Archived,
        );
        write_projection_at(&workspace_dir, &projection);

        let result = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].action, PruneAction::Delete);
    }

    #[test]
    fn classify_workspace_projections_skips_archived_too_soon() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();
        let project_dir = scan_root.join("ghi789");
        let workspace_dir = project_dir.join("workspace");
        let now = Utc::now();
        let projection = make_classify_projection(
            "ws-keep-archived",
            &project_dir,
            now - chrono::Duration::days(10),
            WorkspaceLifecycleStage::Archived,
        );
        write_projection_at(&workspace_dir, &projection);

        let result = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].action,
            PruneAction::Skip {
                reason: PruneSkipReason::ArchivedTooSoon,
            }
        );
    }

    #[test]
    fn classify_workspace_projections_skips_active_session() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();
        let project_dir = scan_root.join("jkl012");
        let workspace_dir = project_dir.join("workspace");
        let now = Utc::now();
        let projection = make_classify_projection(
            "ws-active",
            &project_dir,
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        write_projection_at(&workspace_dir, &projection);

        let result = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| true, // every workspace has an active session
        );
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].action,
            PruneAction::Skip {
                reason: PruneSkipReason::ActiveAgent,
            }
        );
    }

    #[test]
    fn apply_prune_plan_dry_run_counts_without_filesystem_change() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();
        let project_dir = scan_root.join("dry-run-test");
        let workspace_dir = project_dir.join("workspace");
        let now = Utc::now();
        let projection = make_classify_projection(
            "ws-dry",
            &project_dir,
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        write_projection_at(&workspace_dir, &projection);

        let plan = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        let summary = apply_prune_plan(&plan, true).expect("dry run summary");
        assert_eq!(summary.archived, 1);
        assert_eq!(summary.deleted, 0);
        assert_eq!(summary.skipped, 0);

        let loaded = load_workspace_projection_from_path(&workspace_dir.join("current.json"))
            .expect("load")
            .expect("present");
        assert_eq!(
            loaded.lifecycle_stage,
            WorkspaceLifecycleStage::Active,
            "dry-run must not mutate lifecycle_stage",
        );
    }

    #[test]
    fn apply_prune_plan_archives_then_deletes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let scan_root = tmp.path().to_path_buf();

        let now = Utc::now();
        let archive_dir = scan_root.join("archive-target").join("workspace");
        let archive_projection = make_classify_projection(
            "ws-arch",
            &scan_root.join("archive-target"),
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        write_projection_at(&archive_dir, &archive_projection);

        let delete_dir = scan_root.join("delete-target").join("workspace");
        let delete_projection = make_classify_projection(
            "ws-del",
            &scan_root.join("delete-target"),
            now - chrono::Duration::days(90),
            WorkspaceLifecycleStage::Archived,
        );
        write_projection_at(&delete_dir, &delete_projection);

        let plan = classify_workspace_projections(
            &scan_root,
            &WorkspaceRetentionConfig::default(),
            now,
            |_| false,
        );
        let summary = apply_prune_plan(&plan, false).expect("apply prune");
        assert_eq!(summary.archived, 1);
        assert_eq!(summary.deleted, 1);

        let loaded = load_workspace_projection_from_path(&archive_dir.join("current.json"))
            .expect("load")
            .expect("present");
        assert_eq!(loaded.lifecycle_stage, WorkspaceLifecycleStage::Archived);

        assert!(
            !delete_dir.exists(),
            "delete target workspace dir should have been removed",
        );
    }

    #[test]
    fn apply_event_preserves_done_against_subsequent_heartbeat() {
        let work_item_id = "test-item-preserve";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 14, 11, 0, 0).unwrap();

        let mut projection = WorkspaceWorkItemsProjection::empty(t1);

        let mut done_event =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Done, work_item_id, t1);
        done_event.status_category = Some(WorkspaceStatusCategory::Done);
        done_event.title = Some("Test work item".to_string());
        projection.apply_event(done_event);

        let item_after_done = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("done event must create work item");
        assert_eq!(
            item_after_done.status_category,
            WorkspaceStatusCategory::Done
        );
        assert_eq!(item_after_done.completed_at, Some(t1));

        let update_event =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Update, work_item_id, t2);
        assert!(
            update_event.status_category.is_none(),
            "heartbeat update event has no explicit status_category"
        );
        projection.apply_event(update_event);

        let item_after_update = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item still exists");
        assert_eq!(
            item_after_update.status_category,
            WorkspaceStatusCategory::Done,
            "SPEC-2359 US-37: Done is a terminal state; heartbeat update with status_category=None must not regress it"
        );
        assert_eq!(
            item_after_update.completed_at,
            Some(t1),
            "initial Done timestamp must be preserved across subsequent update events"
        );
    }

    #[test]
    fn rebuild_work_items_from_events_recovers_done_after_subsequent_update() {
        // SPEC-2359 US-37: Existing work_items.json files written with the
        // legacy apply_event semantics may show status=active even though
        // work_events.jsonl contains a Done event. Replaying events through
        // the fixed apply_event must restore the Done terminal state.
        let temp = tempfile::tempdir().expect("tempdir");
        let events_path = temp.path().join("work_events.jsonl");
        let work_items_path = temp.path().join("work_items.json");
        let marker_path = temp.path().join("work_items.migration.json");

        let work_item_id = "wi-recovered";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 10, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 10, 11, 0, 0).unwrap();
        let mut done_event =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Done, work_item_id, t1);
        done_event.status_category = Some(WorkspaceStatusCategory::Done);
        done_event.title = Some("Recovered work".to_string());
        append_workspace_work_event_to_path(&events_path, &done_event).expect("append done");
        let update_event =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Update, work_item_id, t2);
        append_workspace_work_event_to_path(&events_path, &update_event).expect("append update");

        let outcome =
            rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
                .expect("rebuild");
        assert_eq!(outcome, WorkspaceWorkItemsRebuildOutcome::Applied);

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load")
            .expect("present");
        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("recovered item exists");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(t1));

        // Re-running is idempotent (marker prevents rebuild).
        let outcome_again =
            rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
                .expect("rebuild idempotent");
        assert_eq!(
            outcome_again,
            WorkspaceWorkItemsRebuildOutcome::AlreadyMigrated
        );
    }

    #[test]
    fn apply_event_idempotent_done_keeps_first_timestamp() {
        let work_item_id = "test-item-idempotent";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();

        let mut projection = WorkspaceWorkItemsProjection::empty(t1);

        let mut first_done =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Done, work_item_id, t1);
        first_done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(first_done);

        let mut second_done =
            WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Done, work_item_id, t2);
        second_done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(second_done);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists after idempotent done");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(
            item.completed_at,
            Some(t1),
            "first Done timestamp must be preserved on idempotent Done re-apply"
        );
        assert_eq!(item.updated_at, t2, "updated_at should still advance");
    }
}
