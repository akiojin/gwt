//! The materialized [`WorkspaceProjection`] "current state" model: the
//! projection struct, its git/cleanup context types, the update payload
//! structs, and every owned state-transition method (status category
//! changes, agent merge/assign/retain rules, launch/start composition).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::coordination::{BoardEntry, BoardEntryKind};

use super::*;

/// Git execution context of a Workspace: branch, worktree path, base branch,
/// and the linked PR snapshot. Populated by Start Work / Launch
/// materialization and refreshed when PR state is polled.
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

/// Why a branch/worktree pair is offered for cleanup: its Workspace reached
/// Done, or its PR merged.
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

/// One branch/worktree pair proposed to the cleanup UI, with the reason and
/// the default remote-delete decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCleanupCandidate {
    pub branch: String,
    pub worktree_path: Option<PathBuf>,
    pub reason: WorkspaceCleanupReason,
    pub default_delete_remote: bool,
    #[serde(default)]
    pub remote_delete_available: bool,
}

/// Materialized "current state" view of one Workspace (title, status,
/// agents, git details). Persisted per project and consumed by the GUI; the
/// Board stays the coordination/history log while this projection tracks the
/// present.
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
            title: "Work".to_string(),
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
            if !self.has_current_agents() {
                self.transition_to_idle(updated_at);
            }
        }
        removed
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): true when at least one assigned
    /// agent is currently driving this Work (Active or Blocked). Unassigned
    /// or idle agents do not count.
    pub fn has_current_agents(&self) -> bool {
        self.agents.iter().any(|agent| {
            agent.is_assigned()
                && matches!(
                    agent.status_category,
                    WorkspaceStatusCategory::Active | WorkspaceStatusCategory::Blocked
                )
        })
    }

    /// The single Idle transition rule: Idle category, "No active work"
    /// status text, cleared next action.
    fn transition_to_idle(&mut self, updated_at: DateTime<Utc>) {
        self.status_category = WorkspaceStatusCategory::Idle;
        self.status_text = "No active work".to_string();
        self.next_action = None;
        self.updated_at = updated_at;
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): the agent merge rule. Updates
    /// the summary stored for `summary.session_id` (or inserts it). A stored
    /// `Blocked` status is never overwritten by a non-Blocked upsert, `None`
    /// incoming identity fields never clear stored values, and `updated_at`
    /// never rewinds.
    pub fn upsert_agent_summary(&mut self, summary: WorkspaceAgentSummary) {
        if let Some(existing) = self
            .agents
            .iter_mut()
            .find(|agent| agent.session_id == summary.session_id)
        {
            existing.agent_id = summary.agent_id;
            existing.window_id = summary.window_id;
            existing.display_name = summary.display_name;
            existing.worktree_path = summary.worktree_path;
            existing.branch = summary.branch;
            if existing.status_category != WorkspaceStatusCategory::Blocked {
                existing.status_category = summary.status_category;
            }
            if summary.current_focus.is_some() {
                existing.current_focus = summary.current_focus;
            }
            if summary.last_board_entry_id.is_some() {
                existing.last_board_entry_id = summary.last_board_entry_id;
            }
            if summary.last_board_entry_kind.is_some() {
                existing.last_board_entry_kind = summary.last_board_entry_kind;
            }
            if summary.coordination_scope.is_some() {
                existing.coordination_scope = summary.coordination_scope;
            }
            if summary.title_summary.is_some() {
                existing.title_summary = summary.title_summary;
            }
            existing.affiliation_status = summary.affiliation_status;
            existing.workspace_id = summary.workspace_id;
            if summary.updated_at > existing.updated_at {
                existing.updated_at = summary.updated_at;
            }
        } else {
            self.agents.push(summary);
        }
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): drop agents whose session is no
    /// longer live and apply the Idle transition when no assigned Active or
    /// Blocked agent remains. A projection that still has current agents is
    /// left untouched.
    pub fn retain_live_agents<'a>(
        &mut self,
        live_session_ids: impl IntoIterator<Item = &'a str>,
        updated_at: DateTime<Utc>,
    ) {
        let live: std::collections::HashSet<&str> = live_session_ids.into_iter().collect();
        self.agents
            .retain(|agent| live.contains(agent.session_id.as_str()));
        if !self.has_current_agents() {
            self.transition_to_idle(updated_at);
        }
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): reset the projection to the
    /// idle "no current work" identity for a project tab, clearing the
    /// work-specific identity fields alongside the Idle transition.
    pub fn reset_idle_identity(&mut self, tab_title: &str, updated_at: DateTime<Utc>) {
        let title = tab_title.trim();
        self.title = if title.is_empty() {
            "Project Work".to_string()
        } else {
            format!("{title} Work")
        };
        self.summary = None;
        self.owner = None;
        self.git_details = None;
        self.board_refs.clear();
        self.transition_to_idle(updated_at);
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): clear the git execution
    /// details (after a worktree cleanup) and apply the Idle transition.
    pub fn clear_git_details_to_idle(&mut self, updated_at: DateTime<Utc>) {
        self.git_details = None;
        self.transition_to_idle(updated_at);
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): the assign rule. Marks the
    /// agent session as assigned to `workspace_id` and Active, merging the
    /// optional identity fields only when provided. Returns false when the
    /// session is unknown.
    pub fn assign_agent(
        &mut self,
        session_id: &str,
        workspace_id: &str,
        current_focus: Option<String>,
        title_summary: Option<String>,
        updated_at: DateTime<Utc>,
    ) -> bool {
        let Some(agent) = self
            .agents
            .iter_mut()
            .find(|agent| agent.session_id == session_id)
        else {
            return false;
        };
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some(workspace_id.to_string());
        agent.status_category = WorkspaceStatusCategory::Active;
        if current_focus.is_some() {
            agent.current_focus = current_focus;
        }
        if title_summary.is_some() {
            agent.title_summary = title_summary;
        }
        agent.updated_at = updated_at;
        true
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): reflect an agent launch
    /// (Start Work / Resume) into the projection — the Active transition,
    /// agent assignment, running-agents status text, and git details
    /// composition with base-branch fallback.
    pub fn apply_launch(
        &mut self,
        launch: WorkspaceLaunchUpdate,
        mut agent: WorkspaceAgentSummary,
        now: DateTime<Utc>,
    ) {
        if let Some(work_id) = launch.work_id {
            if work_id != self.id {
                // #3065: this projection is shared per repository. A launch
                // that re-points it at a different Work must not inherit the
                // previous Work's identity — otherwise the stale owner/title
                // is replayed into the new Work's event log on every resume.
                self.owner = None;
                self.summary = None;
                self.next_action = None;
                self.agents.retain(|agent| {
                    agent
                        .workspace_id
                        .as_deref()
                        .is_none_or(|assigned| assigned == work_id)
                });
            }
            self.id = work_id;
        }
        self.title = launch.title.unwrap_or_else(|| "Start Work".to_string());
        self.status_category = WorkspaceStatusCategory::Active;
        self.next_action = launch
            .next_action
            .or_else(|| Some("Check Board for latest updates".to_string()));
        if let Some(summary) = launch.summary {
            self.summary = Some(summary);
        }
        if let Some(owner) = launch.owner {
            self.owner = Some(owner);
        }
        let display_name = agent.display_name.clone();
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some(self.id.clone());
        self.upsert_agent_summary(agent);
        let active_agents = self
            .assigned_agents()
            .filter(|agent| agent.status_category == WorkspaceStatusCategory::Active)
            .count();
        self.status_text = if active_agents == 1 {
            format!("{display_name} is running")
        } else {
            format!("{active_agents} active agents")
        };
        let previous_base_branch = self
            .git_details
            .as_ref()
            .and_then(|details| details.base_branch.clone());
        self.git_details = Some(GitDetails {
            branch: Some(launch.branch),
            worktree_path: Some(launch.worktree_path),
            base_branch: launch.base_branch.or(previous_base_branch),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: launch.created_by_start_work,
            created_at: now,
        });
        self.updated_at = now;
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): apply the Active identity of a
    /// newly started Work (the CLI `workspace create` / `ensure` paths).
    pub fn start_work(&mut self, start: WorkspaceStartUpdate, now: DateTime<Utc>) {
        self.id = start.workspace_id;
        self.title = start.title;
        self.status_category = WorkspaceStatusCategory::Active;
        self.status_text = start
            .status_text
            .unwrap_or_else(|| "Workspace created".to_string());
        self.summary = start.summary;
        self.owner = start.owner;
        self.next_action = Some(start.next_action);
        self.updated_at = now;
    }

    /// SPEC-2359 Phase W-14 (US-70 / FR-375): point the projection at an
    /// existing Work item (the CLI `workspace join` / selection paths).
    pub fn apply_work_item(&mut self, item: &WorkItem, updated_at: DateTime<Utc>) {
        self.id = item.id.clone();
        self.title = item.title.clone();
        self.status_category = item.status_category;
        self.status_text = item
            .summary
            .clone()
            .or_else(|| item.intent.clone())
            .unwrap_or_else(|| "Workspace selected".to_string());
        self.summary = item.summary.clone().or_else(|| item.intent.clone());
        self.owner = item.owner.clone();
        self.updated_at = updated_at;
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

/// Partial update applied to a [`WorkspaceProjection`] (e.g. from
/// `gwtd workspace update`); `None` fields keep their current values.
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

/// SPEC-2359 Phase W-14 (US-70 / FR-375): parameters for
/// [`WorkspaceProjection::apply_launch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLaunchUpdate {
    /// Canonical work id; `None` keeps the current projection id.
    pub work_id: Option<String>,
    /// Resume title; `None` falls back to "Start Work".
    pub title: Option<String>,
    /// Resume summary; `None` keeps the current summary.
    pub summary: Option<String>,
    /// Owner label (resume owner or "Issue #N"); `None` keeps the current owner.
    pub owner: Option<String>,
    /// Next action; `None` falls back to "Check Board for latest updates".
    pub next_action: Option<String>,
    pub branch: String,
    pub worktree_path: PathBuf,
    /// Launch base branch; `None` falls back to the previous git details.
    pub base_branch: Option<String>,
    pub created_by_start_work: bool,
}

/// SPEC-2359 Phase W-14 (US-70 / FR-375): parameters for
/// [`WorkspaceProjection::start_work`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStartUpdate {
    pub workspace_id: String,
    pub title: String,
    /// Display status; `None` falls back to "Workspace created".
    pub status_text: Option<String>,
    pub summary: Option<String>,
    pub owner: Option<String>,
    pub next_action: String,
}

/// One append-only journal record of a Workspace update, kept alongside the
/// projection so state changes remain auditable.
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

#[cfg(test)]
mod tests {
    use std::path::Path;

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
    fn board_milestone_never_updates_agent_title_summary_from_board_entry() {
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
            agent_json.pointer("/title_summary"),
            None,
            "Board title_summary is legacy history metadata and must not update live agent purpose"
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

    fn us70_agent(
        session_id: &str,
        status_category: WorkspaceStatusCategory,
        affiliation_status: WorkspaceAgentAffiliationStatus,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status,
            workspace_id: None,
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
        }
    }

    #[test]
    fn upsert_agent_summary_preserves_blocked_status_and_merges_fields() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let mut blocked = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Blocked,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        blocked.current_focus = Some("blocked focus".to_string());
        projection.agents.push(blocked);

        let mut incoming = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        incoming.display_name = "Codex 2".to_string();
        incoming.updated_at = Utc.timestamp_opt(2_000, 0).unwrap();
        projection.upsert_agent_summary(incoming);

        let agent = &projection.agents[0];
        assert_eq!(
            agent.status_category,
            WorkspaceStatusCategory::Blocked,
            "Blocked must not be overwritten by a non-Blocked upsert"
        );
        assert_eq!(agent.display_name, "Codex 2");
        assert_eq!(
            agent.current_focus.as_deref(),
            Some("blocked focus"),
            "a None incoming focus must not clear the stored focus"
        );
        assert_eq!(agent.updated_at, Utc.timestamp_opt(2_000, 0).unwrap());
    }

    #[test]
    fn upsert_agent_summary_inserts_new_sessions_and_never_rewinds_updated_at() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let mut first = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        first.updated_at = Utc.timestamp_opt(2_000, 0).unwrap();
        projection.upsert_agent_summary(first);
        assert_eq!(projection.agents.len(), 1);

        let mut stale = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        stale.updated_at = Utc.timestamp_opt(500, 0).unwrap();
        projection.upsert_agent_summary(stale);
        assert_eq!(
            projection.agents[0].updated_at,
            Utc.timestamp_opt(2_000, 0).unwrap(),
            "an older upsert must not rewind updated_at"
        );
    }

    #[test]
    fn retain_live_agents_transitions_to_idle_when_no_assigned_agent_remains() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Codex is running".to_string();
        projection.next_action = Some("Keep going".to_string());
        projection.agents.push(us70_agent(
            "dead",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        ));

        let now = Utc.timestamp_opt(3_000, 0).unwrap();
        projection.retain_live_agents(["live-other"], now);

        assert!(projection.agents.is_empty());
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Idle);
        assert_eq!(projection.status_text, "No active work");
        assert_eq!(projection.next_action, None);
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn retain_live_agents_keeps_active_state_while_assigned_agent_lives() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let before = projection.updated_at;
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Codex is running".to_string();
        projection.agents.push(us70_agent(
            "live",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        ));

        projection.retain_live_agents(["live"], Utc.timestamp_opt(3_000, 0).unwrap());

        assert_eq!(projection.agents.len(), 1);
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(projection.status_text, "Codex is running");
        assert_eq!(
            projection.updated_at, before,
            "a live projection must not be touched by retain"
        );
    }

    #[test]
    fn assign_agent_marks_assigned_active_and_merges_identity() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Idle,
            WorkspaceAgentAffiliationStatus::Unassigned,
        ));

        let now = Utc.timestamp_opt(4_000, 0).unwrap();
        assert!(projection.assign_agent(
            "sess-1",
            "work-abc",
            Some("focus".to_string()),
            None,
            now
        ));
        let agent = &projection.agents[0];
        assert!(agent.is_assigned());
        assert_eq!(agent.workspace_id.as_deref(), Some("work-abc"));
        assert_eq!(agent.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(agent.current_focus.as_deref(), Some("focus"));
        assert_eq!(
            agent.title_summary, None,
            "a None title_summary must not overwrite"
        );
        assert_eq!(agent.updated_at, now);

        assert!(
            !projection.assign_agent("unknown", "work-abc", None, None, now),
            "assigning an unknown session must report false"
        );
    }

    #[test]
    fn apply_launch_composes_active_projection_with_defaults() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.git_details = Some(GitDetails {
            branch: Some("work/old".to_string()),
            worktree_path: None,
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: false,
            created_at: Utc.timestamp_opt(10, 0).unwrap(),
        });

        let agent = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Unassigned,
        );
        let now = Utc.timestamp_opt(5_000, 0).unwrap();
        projection.apply_launch(
            WorkspaceLaunchUpdate {
                work_id: Some("work-foo-12345678".to_string()),
                title: None,
                summary: None,
                owner: Some("Issue #42".to_string()),
                next_action: None,
                branch: "work/foo".to_string(),
                worktree_path: PathBuf::from("/wt"),
                base_branch: None,
                created_by_start_work: true,
            },
            agent,
            now,
        );

        assert_eq!(projection.id, "work-foo-12345678");
        assert_eq!(projection.title, "Start Work");
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(
            projection.next_action.as_deref(),
            Some("Check Board for latest updates")
        );
        assert_eq!(projection.owner.as_deref(), Some("Issue #42"));
        assert_eq!(projection.status_text, "Codex is running");
        let details = projection.git_details.as_ref().expect("git details");
        assert_eq!(details.branch.as_deref(), Some("work/foo"));
        assert_eq!(details.worktree_path.as_deref(), Some(Path::new("/wt")));
        assert_eq!(
            details.base_branch.as_deref(),
            Some("origin/develop"),
            "base branch must fall back to the previous git details"
        );
        assert!(details.created_by_start_work);
        let stored = &projection.agents[0];
        assert!(stored.is_assigned());
        assert_eq!(stored.workspace_id.as_deref(), Some("work-foo-12345678"));
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn apply_launch_reports_multiple_active_agents() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let mut resident = us70_agent(
            "sess-0",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        resident.workspace_id = Some("work-foo-12345678".to_string());
        projection.agents.push(resident);

        let agent = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Unassigned,
        );
        projection.apply_launch(
            WorkspaceLaunchUpdate {
                work_id: Some("work-foo-12345678".to_string()),
                title: Some("Resume Work".to_string()),
                summary: Some("resume summary".to_string()),
                owner: None,
                next_action: Some("Pick up review".to_string()),
                branch: "work/foo".to_string(),
                worktree_path: PathBuf::from("/wt"),
                base_branch: Some("origin/main".to_string()),
                created_by_start_work: false,
            },
            agent,
            Utc.timestamp_opt(6_000, 0).unwrap(),
        );

        assert_eq!(projection.title, "Resume Work");
        assert_eq!(projection.summary.as_deref(), Some("resume summary"));
        assert_eq!(projection.next_action.as_deref(), Some("Pick up review"));
        assert_eq!(projection.status_text, "2 active agents");
        assert_eq!(
            projection
                .git_details
                .as_ref()
                .and_then(|details| details.base_branch.as_deref()),
            Some("origin/main")
        );
    }

    #[test]
    fn start_work_applies_active_identity_with_defaults() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let now = Utc.timestamp_opt(7_000, 0).unwrap();
        projection.start_work(
            WorkspaceStartUpdate {
                workspace_id: "workspace-1".to_string(),
                title: "New Work".to_string(),
                status_text: None,
                summary: Some("focus text".to_string()),
                owner: Some("SPEC-1".to_string()),
                next_action: "Coordinate on Board before implementation".to_string(),
            },
            now,
        );

        assert_eq!(projection.id, "workspace-1");
        assert_eq!(projection.title, "New Work");
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(projection.status_text, "Workspace created");
        assert_eq!(projection.summary.as_deref(), Some("focus text"));
        assert_eq!(projection.owner.as_deref(), Some("SPEC-1"));
        assert_eq!(
            projection.next_action.as_deref(),
            Some("Coordinate on Board before implementation")
        );
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn apply_work_item_copies_status_fields() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let now = Utc.timestamp_opt(8_000, 0).unwrap();
        let item = WorkItem {
            id: "item-1".to_string(),
            title: "Item Title".to_string(),
            intent: Some("intent text".to_string()),
            summary: None,
            status_category: WorkspaceStatusCategory::Blocked,
            owner: Some("Issue #7".to_string()),
            created_at: now,
            updated_at: now,
            completed_at: None,
            agents: Vec::new(),
            execution_containers: Vec::new(),
            board_refs: Vec::new(),
            related_work_item_ids: Vec::new(),
            events: Vec::new(),
            discarded: false,
        };

        projection.apply_work_item(&item, now);

        assert_eq!(projection.id, "item-1");
        assert_eq!(projection.title, "Item Title");
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Blocked);
        assert_eq!(projection.status_text, "intent text");
        assert_eq!(projection.summary.as_deref(), Some("intent text"));
        assert_eq!(projection.owner.as_deref(), Some("Issue #7"));
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn reset_idle_identity_clears_identity_and_transitions_to_idle() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Codex is running".to_string();
        projection.summary = Some("summary".to_string());
        projection.owner = Some("owner".to_string());
        projection.next_action = Some("next".to_string());
        projection.board_refs.push("board-1".to_string());

        let now = Utc.timestamp_opt(9_000, 0).unwrap();
        projection.reset_idle_identity("My Repo", now);

        assert_eq!(projection.title, "My Repo Work");
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Idle);
        assert_eq!(projection.status_text, "No active work");
        assert_eq!(projection.summary, None);
        assert_eq!(projection.owner, None);
        assert_eq!(projection.next_action, None);
        assert_eq!(projection.git_details, None);
        assert!(projection.board_refs.is_empty());
        assert_eq!(projection.updated_at, now);

        projection.reset_idle_identity("  ", now);
        assert_eq!(
            projection.title, "Project Work",
            "a blank tab title must fall back to Project Work"
        );
    }

    #[test]
    fn clear_git_details_to_idle_clears_details_and_transitions() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.status_category = WorkspaceStatusCategory::Active;
        projection.status_text = "Codex is running".to_string();
        projection.next_action = Some("next".to_string());
        projection.git_details = Some(GitDetails {
            branch: Some("work/foo".to_string()),
            worktree_path: None,
            base_branch: None,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: false,
            created_at: Utc.timestamp_opt(10, 0).unwrap(),
        });

        let now = Utc.timestamp_opt(10_000, 0).unwrap();
        projection.clear_git_details_to_idle(now);

        assert_eq!(projection.git_details, None);
        assert_eq!(projection.status_category, WorkspaceStatusCategory::Idle);
        assert_eq!(projection.status_text, "No active work");
        assert_eq!(projection.next_action, None);
        assert_eq!(projection.updated_at, now);
    }

    #[test]
    fn has_current_agents_requires_assigned_active_or_blocked() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        assert!(!projection.has_current_agents());

        projection.agents.push(us70_agent(
            "unassigned",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Unassigned,
        ));
        assert!(
            !projection.has_current_agents(),
            "unassigned agents must not count"
        );

        projection.agents.push(us70_agent(
            "idle",
            WorkspaceStatusCategory::Idle,
            WorkspaceAgentAffiliationStatus::Assigned,
        ));
        assert!(
            !projection.has_current_agents(),
            "assigned idle agents must not count"
        );

        projection.agents.push(us70_agent(
            "blocked",
            WorkspaceStatusCategory::Blocked,
            WorkspaceAgentAffiliationStatus::Assigned,
        ));
        assert!(projection.has_current_agents());
    }

    // #3065: a launch that re-points the shared projection at a DIFFERENT
    // work item must not inherit the previous work's identity (owner /
    // summary / next_action) or keep agents assigned to the previous work.
    #[test]
    fn apply_launch_does_not_inherit_identity_across_work_items() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.id = "work-old-11111111".to_string();
        projection.owner = Some("SPEC-2359".to_string());
        projection.summary = Some("old summary".to_string());
        projection.next_action = Some("old next action".to_string());
        let mut resident = us70_agent(
            "sess-old",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        resident.workspace_id = Some("work-old-11111111".to_string());
        projection.agents.push(resident);

        let agent = us70_agent(
            "sess-new",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Unassigned,
        );
        projection.apply_launch(
            WorkspaceLaunchUpdate {
                work_id: Some("work-new-22222222".to_string()),
                title: None,
                summary: None,
                owner: None,
                next_action: None,
                branch: "work/new".to_string(),
                worktree_path: PathBuf::from("/wt-new"),
                base_branch: None,
                created_by_start_work: false,
            },
            agent,
            Utc.timestamp_opt(8_000, 0).unwrap(),
        );

        assert_eq!(projection.id, "work-new-22222222");
        assert_eq!(
            projection.owner, None,
            "owner must not leak across work items"
        );
        assert_eq!(
            projection.summary, None,
            "summary must not leak across work items"
        );
        assert_eq!(
            projection.next_action.as_deref(),
            Some("Check Board for latest updates"),
            "next action falls back to the default, not the previous work's"
        );
        assert!(
            projection
                .agents
                .iter()
                .all(|agent| agent.workspace_id.as_deref() != Some("work-old-11111111")),
            "agents assigned to the previous work item are dropped"
        );
        assert_eq!(projection.status_text, "Codex is running");
    }

    // #3065: resuming the SAME work item keeps the inherited identity —
    // the boundary only applies across different work ids.
    #[test]
    fn apply_launch_keeps_identity_for_same_work_item() {
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.id = "work-foo-12345678".to_string();
        projection.owner = Some("Issue #42".to_string());
        projection.summary = Some("kept summary".to_string());

        let agent = us70_agent(
            "sess-1",
            WorkspaceStatusCategory::Active,
            WorkspaceAgentAffiliationStatus::Unassigned,
        );
        projection.apply_launch(
            WorkspaceLaunchUpdate {
                work_id: Some("work-foo-12345678".to_string()),
                title: None,
                summary: None,
                owner: None,
                next_action: None,
                branch: "work/foo".to_string(),
                worktree_path: PathBuf::from("/wt"),
                base_branch: None,
                created_by_start_work: false,
            },
            agent,
            Utc.timestamp_opt(8_100, 0).unwrap(),
        );

        assert_eq!(projection.owner.as_deref(), Some("Issue #42"));
        assert_eq!(projection.summary.as_deref(), Some("kept summary"));
    }
}
