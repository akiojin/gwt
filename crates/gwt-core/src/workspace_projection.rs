//! Workspace / Work projection: the repo-local "current state" model that the
//! GUI, CLI, and hooks all read and update.
//!
//! SPEC-2359 Phase W-14 (US-70 / FR-378): every state transition of
//! [`WorkspaceProjection`] (status category changes, agent merge/assign/retain
//! rules, launch/start composition) is owned by the methods on
//! [`WorkspaceProjection`] in this module. Callers in UI/CLI layers must go
//! through these APIs; assigning transition fields (`status_category`,
//! `status_text`, `next_action`, `agents`) directly from outside this module
//! is not allowed in new code, so the transition rules stay single-source.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    coordination::{BoardEntry, BoardEntryKind},
    error::{GwtError, Result},
    paths::{
        gwt_project_dir_for_repo_path, gwt_repo_local_work_events_path,
        gwt_workspace_journal_path_for_repo_path, gwt_workspace_projection_path_for_repo_path,
        gwt_workspace_work_events_closed_path_for_repo_path,
        gwt_workspace_work_events_path_for_repo_path, gwt_workspace_work_items_path_for_repo_path,
        project_scope_hash, resolve_current_worktree_root,
    },
};

/// Runtime activity of a Work / its assigned Agents, updated continuously
/// while agents run ("is somebody working right now, and can they proceed?").
///
/// SPEC-2359 Phase W-14 (US-70 / FR-377): this is one of three deliberately
/// distinct lifecycle enums. [`WorkspaceLifecycleStage`] is the user-facing
/// workflow phase derived from events + this category (planning → active →
/// in review → done → archived), and [`WorkActiveLifecycleState`] is the
/// agent-session-centric Work lifecycle (active / paused / done / discarded,
/// closed only by an explicit user action). They share variant names like
/// `Active` / `Done` but answer different questions and have different
/// transition rules — do not use them interchangeably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatusCategory {
    Active,
    Idle,
    Blocked,
    Done,
    Unknown,
}

/// Whether an agent session is attached to a Workspace. `Unassigned` agents
/// were launched outside Start Work and wait for the user to adopt them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAgentAffiliationStatus {
    Unassigned,
    Assigned,
}

fn default_workspace_agent_affiliation_status() -> WorkspaceAgentAffiliationStatus {
    WorkspaceAgentAffiliationStatus::Assigned
}

pub fn canonical_work_id(
    project_root: &Path,
    branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> Option<String> {
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    let (slug_source, identity_kind, identity_value) = if let Some(branch) = branch {
        let identity = canonical_work_branch_identity(branch);
        (identity.clone(), "branch", identity)
    } else {
        let worktree_path = worktree_path?;
        let identity = canonical_worktree_identity(worktree_path);
        if identity.trim().is_empty() {
            return None;
        }
        let slug_source = worktree_path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("worktree");
        (slug_source.to_string(), "worktree", identity)
    };

    let project_hash = project_scope_hash(project_root);
    let mut hasher = Sha256::new();
    hasher.update(project_hash.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(identity_kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity_value.as_bytes());
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    Some(format!(
        "work-{}-{}",
        canonical_work_slug(&slug_source),
        &hex_full[..8]
    ))
}

/// SPEC-2359 W16-4 (FR-391): derived "Done-equivalent" classification for a
/// merged-and-stale Workspace. PURE display state: callers must never record
/// a close event from this verdict (US-61 — explicit user close only); the
/// flag clears by itself when the Workspace is updated after the merge.
///
/// `merge_reference_time` is the branch tip committer time (proxy for the
/// unknown squash-merge instant — plan decision 8). `None` (unknown) never
/// classifies as Done.
pub fn derive_merged_done_equivalent(
    merged_into_base: bool,
    last_updated_at: DateTime<Utc>,
    merge_reference_time: Option<DateTime<Utc>>,
) -> bool {
    let Some(reference) = merge_reference_time else {
        return false;
    };
    merged_into_base && last_updated_at <= reference
}

/// SPEC-2359 W16-2 (FR-389): the Workspace grouping key for one Work item —
/// derived at view-assembly time, never stored (plan decision 6). Works that
/// share a canonical branch (any spelling: `X`, `origin/X`,
/// `refs/remotes/origin/X`) group under one Workspace row; worktree-only
/// items key on the canonical worktree identity; everything else (legacy
/// `workspace-<millis>` / bare-UUID items without containers) keeps its own
/// `item.id` as the key so old rows never vanish.
pub fn workspace_group_key_for_item(project_root: &Path, item: &WorkItem) -> String {
    let branch = item
        .execution_containers
        .iter()
        .find_map(|container| container.branch.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(key) = canonical_work_id(project_root, branch, None) {
        return key;
    }
    let worktree = item
        .execution_containers
        .iter()
        .find_map(|container| container.worktree_path.as_deref());
    if let Some(key) = canonical_work_id(project_root, None, worktree) {
        return key;
    }
    item.id.clone()
}

fn canonical_work_branch_identity(branch: &str) -> String {
    if let Some(name) = branch.strip_prefix("refs/remotes/") {
        return name.strip_prefix("origin/").unwrap_or(name).to_string();
    }
    branch.strip_prefix("origin/").unwrap_or(branch).to_string()
}

fn canonical_worktree_identity(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn canonical_work_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "work".to_string()
    } else {
        slug
    }
}

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

/// SPEC-2359 Phase W-12 (FR-349): the agent-session-centric Work lifecycle.
///
/// Distinct from [`WorkspaceLifecycleStage`] (the U-6 status-derived chip with
/// Planning/InReview/Archived). This 4-state model treats a Work as one agent
/// session: it is `Active` while the agent runs, `Paused` once the agent stops
/// but the user has not closed it, and `Done` / `Discarded` only on an explicit
/// user close. Agent stop alone never closes a Work (FR-350).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkActiveLifecycleState {
    #[default]
    Active,
    Paused,
    Done,
    Discarded,
}

/// Live runtime state of the agent session that owns a Work, used as the
/// driver for [`recompute_work_active_lifecycle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkAgentRuntime {
    /// The owning agent session has a live window / running process.
    Running,
    /// The owning agent session exists but is stopped / exited.
    Stopped,
    /// No live agent session is associated (e.g. resumed-later Work).
    None,
}

/// Explicit close recorded when the user closes a Work from the Work surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkCloseKind {
    Done,
    Discarded,
}

/// SPEC-2359 Phase W-12 (FR-349/FR-350): derive the agent-session Work
/// lifecycle. An explicit user close wins; otherwise the live agent runtime
/// decides: `Running` → `Active`, `Stopped` / `None` → `Paused`. Agent stop
/// alone must never yield `Done` / `Discarded` (FR-350) — only a user close does.
pub fn recompute_work_active_lifecycle(
    agent_runtime: WorkAgentRuntime,
    closed: Option<WorkCloseKind>,
) -> WorkActiveLifecycleState {
    match closed {
        Some(WorkCloseKind::Done) => WorkActiveLifecycleState::Done,
        Some(WorkCloseKind::Discarded) => WorkActiveLifecycleState::Discarded,
        None => match agent_runtime {
            WorkAgentRuntime::Running => WorkActiveLifecycleState::Active,
            WorkAgentRuntime::Stopped | WorkAgentRuntime::None => WorkActiveLifecycleState::Paused,
        },
    }
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): the outcome of deciding how to handle
/// a user-initiated Work close. Kept as a pure value so the block / cleanup /
/// record-only decision can be unit-tested without touching git or the
/// filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkCloseDecision {
    /// A live agent session still owns this Work — block the close and do not
    /// touch the worktree (FR-352). The owning session must be stopped first.
    BlockedLiveAgent,
    /// No live agent and a worktree path is known — remove the worktree only
    /// (branch / PR are retained) and record the terminal close.
    CleanupWorktree { worktree_path: PathBuf },
    /// No live agent and no resolvable worktree path — record the terminal
    /// close in the work history but perform no filesystem cleanup.
    RecordOnly,
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): decide how to handle a Work close.
///
/// Decision order:
/// 1. If a live agent session owns this Work (`live_agent` is `true`), block the
///    close — the worktree must never be removed while an agent is running.
/// 2. Otherwise, if a worktree path is known, request worktree-only cleanup.
/// 3. Otherwise, record the close without any filesystem side effect.
///
/// Pure: takes only resolved inputs and returns a value, so it is exercised
/// directly by unit tests while the git removal itself is verified separately.
pub fn decide_work_close(live_agent: bool, worktree_path: Option<PathBuf>) -> WorkCloseDecision {
    if live_agent {
        return WorkCloseDecision::BlockedLiveAgent;
    }
    match worktree_path {
        Some(path) if !path.as_os_str().is_empty() => WorkCloseDecision::CleanupWorktree {
            worktree_path: path,
        },
        _ => WorkCloseDecision::RecordOnly,
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

/// Reference from a Work item to one agent session that worked on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAgentRef {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Reference from a Work item to the branch / worktree / PR it executed in.
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

/// Lifecycle event kind in a Work item's history (start, claim, update,
/// pause, done, ...). Each kind maps to one [`WorkEvent`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEventKind {
    Start,
    Claim,
    Update,
    Blocked,
    Handoff,
    Resume,
    Split,
    Merge,
    Pr,
    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): the owning agent session stopped
    /// without an explicit user close. The Work is retained as Paused (not Done)
    /// so it stays on the Work surface until the user closes it.
    Pause,
    Done,
    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): the user explicitly discarded the
    /// Work from the Work surface. This is a terminal close distinct from Done:
    /// the Work leaves the active surface but its provenance is retained as
    /// discarded (not completed). Agent stop alone never yields Discard.
    Discard,
    /// SPEC-2359 Phase W-15 (FR-380): a worktree existed on disk without any
    /// matching Work record, so reconciliation materialized one. The event
    /// must not carry an explicit `status_category`: `apply_event` only
    /// preserves terminal (Done/Discarded) items against implicit-status
    /// events, and a committed Backfill event may be re-ingested on another
    /// machine after the Work was closed there (W-16 intake).
    Backfill,
}

/// One append-only event in a Work item's lifecycle. Events are folded into
/// [`WorkItem`]s by [`WorkItemsProjection`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEvent {
    pub id: String,
    pub work_item_id: String,
    pub kind: WorkEventKind,
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

impl WorkEvent {
    pub fn new(
        kind: WorkEventKind,
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

/// One unit of work on the Work surface: title, status, participating
/// agents, execution containers, and its event history. Built by folding
/// [`WorkEvent`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
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
    pub agents: Vec<WorkAgentRef>,
    #[serde(default)]
    pub execution_containers: Vec<WorkspaceExecutionContainerRef>,
    #[serde(default)]
    pub board_refs: Vec<String>,
    #[serde(default)]
    pub related_work_item_ids: Vec<String>,
    #[serde(default)]
    pub events: Vec<WorkEvent>,
    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): terminal discarded close. A
    /// discarded Work is removed from the active Work surface but kept in the
    /// history with its provenance. Distinct from `status_category == Done`
    /// (which marks completion); `discarded` marks an explicit user discard.
    /// Back-compat default is `false` for projections written before W-12.
    #[serde(default)]
    pub discarded: bool,
}

impl WorkItem {
    /// A Work is incomplete while it is neither completed (Done) nor discarded.
    /// Both Done and Discarded are terminal closes (FR-352).
    pub fn is_incomplete(&self) -> bool {
        self.status_category != WorkspaceStatusCategory::Done && !self.discarded
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): true when the Work has reached a
    /// terminal close — either completed (Done) or explicitly discarded.
    pub fn is_terminal(&self) -> bool {
        self.status_category == WorkspaceStatusCategory::Done || self.discarded
    }

    /// #3065: the most recent non-empty `next_action` across this Work's
    /// events. Used to build a per-work-item resume context instead of
    /// reading the repo-shared current projection.
    pub fn latest_next_action(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|event| {
            event
                .next_action
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    }
}

/// Materialized collection of all Work items for one project, rebuilt by
/// folding the Work event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemsProjection {
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub work_items: Vec<WorkItem>,
}

impl WorkItemsProjection {
    pub fn empty(updated_at: DateTime<Utc>) -> Self {
        Self {
            updated_at,
            work_items: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: WorkEvent) {
        let existing_index = self
            .work_items
            .iter()
            .position(|item| item.id == event.work_item_id);
        let index = existing_index.unwrap_or_else(|| {
            self.work_items.push(WorkItem {
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
                discarded: false,
            });
            self.work_items.len() - 1
        });

        let item = &mut self.work_items[index];
        // SPEC-2359 Phase W-16 (FR-403): a Backfill event is a synthetic
        // materialization marker, not activity. Applied to an existing item
        // (a duplicated / replayed backfill line), it must not advance
        // `updated_at`, overwrite the real title, or touch status — otherwise
        // every materialized row collapses onto the replay instant and the
        // recency sort degenerates. Only the execution container may merge.
        if existing_index.is_some() && event.kind == WorkEventKind::Backfill {
            if let Some(container) = event.execution_container.clone() {
                if !item
                    .execution_containers
                    .iter()
                    .any(|existing| workspace_execution_container_same(existing, &container))
                {
                    item.execution_containers.push(container);
                }
            }
            item.events.push(event);
            return;
        }
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
        //
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): Discarded is likewise terminal.
        // Once a Work is discarded, subsequent events (heartbeat updates without
        // an explicit status_category) must not regress its runtime status; the
        // `discarded` flag is monotonic and never reset.
        let new_status = workspace_work_event_status(&event);
        let preserve_terminal = (item.status_category == WorkspaceStatusCategory::Done
            || item.discarded)
            && event.status_category.is_none();
        if !preserve_terminal {
            item.status_category = new_status;
        }
        if event.kind == WorkEventKind::Discard {
            item.discarded = true;
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
                item.agents.push(WorkAgentRef {
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

/// SPEC-2359 Phase W-12 Slice 5b (FR-355): the gitattributes line that joins
/// the append-only Work event log across branch divergence via git's union
/// merge driver. The glob matches the repo-local event log regardless of the
/// directory it is checked out under.
const WORK_EVENTS_GITATTRIBUTES_LINE: &str = "**/.gwt/work/events.jsonl merge=union";

/// SPEC-2359 Phase W-12 Slice 5b (FR-353/FR-358): resolve the repo-local Work
/// event log path (`<repo_root>/.gwt/work/events.jsonl`), running the one-time
/// migration from the home (untracked) sources and ensuring the union-merge
/// gitattribute exists. Returns the repo-local path so every record/read entry
/// point shares one resolution + migration.
///
/// Migration (FR-358) is guarded by existence: when the repo-local event log
/// is absent, the home Project State event log (and its older Workspace path)
/// are copied into it exactly once. Once the repo-local file exists, the home
/// sources are never read again. The copy is idempotent across restarts and
/// across linked worktrees because they all resolve to the same main worktree
/// root.
fn repo_local_work_events_path_with_migration(repo_path: &Path) -> Result<PathBuf> {
    let events_path = gwt_repo_local_work_events_path(repo_path);
    if !events_path.exists() {
        // Primary migration source: the home Project State event log.
        copy_legacy_workspace_file_if_needed(
            &gwt_workspace_work_events_path_for_repo_path(repo_path),
            &events_path,
        )?;
        // Fallback migration source: the older home Workspace event log. Only
        // consulted when neither the repo-local nor the Project State file
        // exists, so the most recent home log always wins.
        copy_legacy_workspace_file_if_needed(
            &legacy_workspace_work_events_path_for_repo_path(repo_path),
            &events_path,
        )?;
    }
    ensure_work_events_gitattributes(repo_path)?;
    Ok(events_path)
}

/// SPEC-2359 Phase W-12 Slice 5b (FR-355): ensure the repo's `.gitattributes`
/// carries the union-merge line for the repo-local Work event log. The entry
/// is appended at most once (idempotent): an existing line — regardless of
/// surrounding whitespace — is left untouched. The file is created when
/// absent. Failures to write are swallowed for non-repository / read-only
/// roots so recording a Work event never fails on the gitattributes side.
fn ensure_work_events_gitattributes(repo_path: &Path) -> Result<()> {
    let root = resolve_current_worktree_root(repo_path);
    // Defensive: a bare repository has no checked-out `.gitattributes`, so it
    // cannot drive the merge driver. `resolve_current_worktree_root` returns
    // the working tree, so this guard is normally inert, but stays as a guard.
    if root.join("HEAD").is_file() && root.join("objects").is_dir() && !root.join(".git").exists() {
        return Ok(());
    }
    let attributes_path = root.join(".gitattributes");
    let existing = fs::read_to_string(&attributes_path).unwrap_or_default();
    if existing
        .lines()
        .any(|line| line.trim() == WORK_EVENTS_GITATTRIBUTES_LINE)
    {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(WORK_EVENTS_GITATTRIBUTES_LINE);
    next.push('\n');
    // Best-effort: a read-only or non-repo root must not fail event recording.
    let _ = fs::write(&attributes_path, next);
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
) -> Result<Option<WorkItemsProjection>> {
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
        Ok(bytes) => {
            let mut projection: WorkspaceProjection = serde_json::from_slice(&bytes)
                .map_err(|error| GwtError::Other(format!("workspace projection json: {error}")))?;
            migrate_workspace_to_work_terminology(&mut projection);
            Ok(Some(projection))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn migrate_workspace_to_work_terminology(projection: &mut WorkspaceProjection) {
    if projection.title == "Workspace" {
        projection.title = "Work".to_string();
    } else if projection.title.starts_with("Workspace ") {
        let suffix = &projection.title["Workspace ".len()..];
        projection.title = format!("Work {suffix}");
    } else if projection.title.ends_with(" workspace") {
        let prefix = &projection.title[..projection.title.len() - " workspace".len()];
        projection.title = format!("{prefix} work");
    }
}

pub fn load_workspace_work_items(repo_path: &Path) -> Result<Option<WorkItemsProjection>> {
    migrate_legacy_workspace_work_items(
        repo_path,
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
    )
}

pub fn load_workspace_work_items_from_path(path: &Path) -> Result<Option<WorkItemsProjection>> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut items: WorkItemsProjection = serde_json::from_slice(&bytes)
                .map_err(|error| GwtError::Other(format!("workspace work items json: {error}")))?;
            for item in &mut items.work_items {
                if item.title == "Workspace" {
                    item.title = "Work".to_string();
                } else if item.title.starts_with("Workspace ") {
                    let suffix = &item.title["Workspace ".len()..];
                    item.title = format!("Work {suffix}");
                } else if item.title.ends_with(" workspace") {
                    let prefix = &item.title[..item.title.len() - " workspace".len()];
                    item.title = format!("{prefix} work");
                }
            }
            Ok(Some(items))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn load_or_synthesize_workspace_work_items(repo_path: &Path) -> Result<WorkItemsProjection> {
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
) -> Result<WorkItemsProjection> {
    if let Some(projection) = load_workspace_work_items_from_path(work_items_path)? {
        return Ok(projection);
    }
    synthesize_workspace_work_items_from_legacy_paths(current_path, journal_path, project_root)
}

/// SPEC-2359 (close-latency root fix, 2026-06-11): mtime+size-keyed cache in
/// front of [`load_or_synthesize_workspace_work_items`]. The home works.json
/// grows to megabytes (hundreds of Work items × thousands of events) and the
/// UI event loop rebuilds the Workspace projection on every broadcast-bearing
/// action; re-parsing the file each time stalls the queue. A cache hit clones
/// the parsed projection instead. Synthesized fallbacks (works.json absent)
/// are never cached — they must observe legacy-file changes.
#[derive(Default)]
pub struct WorkItemsCache {
    entries: HashMap<PathBuf, CachedWorkItemsProjection>,
    /// Lifetime parse counter; tests assert the steady state stops parsing.
    pub parse_count: u64,
}

struct CachedWorkItemsProjection {
    mtime: std::time::SystemTime,
    size: u64,
    projection: WorkItemsProjection,
}

impl WorkItemsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached equivalent of [`load_or_synthesize_workspace_work_items`].
    pub fn load_or_synthesize(&mut self, repo_path: &Path) -> Result<WorkItemsProjection> {
        let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
        if let Some(hit) = self.lookup(&work_items_path) {
            return Ok(hit);
        }
        let projection = load_or_synthesize_workspace_work_items(repo_path)?;
        self.store(&work_items_path, &projection);
        Ok(projection)
    }

    /// Paths-injected variant for tests and path-explicit callers (#3022).
    pub fn load_or_synthesize_from_paths(
        &mut self,
        work_items_path: &Path,
        current_path: &Path,
        journal_path: &Path,
        project_root: &Path,
    ) -> Result<WorkItemsProjection> {
        if let Some(hit) = self.lookup(work_items_path) {
            return Ok(hit);
        }
        let projection = load_or_synthesize_workspace_work_items_from_paths(
            work_items_path,
            current_path,
            journal_path,
            project_root,
        )?;
        self.store(work_items_path, &projection);
        Ok(projection)
    }

    fn lookup(&self, work_items_path: &Path) -> Option<WorkItemsProjection> {
        let meta = fs::metadata(work_items_path).ok()?;
        let mtime = meta.modified().ok()?;
        let hit = self.entries.get(work_items_path)?;
        (hit.mtime == mtime && hit.size == meta.len()).then(|| hit.projection.clone())
    }

    fn store(&mut self, work_items_path: &Path, projection: &WorkItemsProjection) {
        self.parse_count += 1;
        let Ok(meta) = fs::metadata(work_items_path) else {
            // works.json absent: the result was synthesized from legacy
            // sources — do not cache it against a missing file.
            self.entries.remove(work_items_path);
            return;
        };
        let Ok(mtime) = meta.modified() else {
            self.entries.remove(work_items_path);
            return;
        };
        self.entries.insert(
            work_items_path.to_path_buf(),
            CachedWorkItemsProjection {
                mtime,
                size: meta.len(),
                projection: projection.clone(),
            },
        );
    }
}

pub fn save_workspace_work_items_projection_to_path(
    path: &Path,
    projection: &WorkItemsProjection,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| GwtError::Other(format!("workspace work items json: {error}")))?;
    write_atomic(path, &bytes)
}

pub fn record_workspace_work_event(repo_path: &Path, event: WorkEvent) -> Result<()> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(repo_path)?;
    record_workspace_work_event_paths(&work_items_path, &events_path, event)
}

pub fn record_workspace_work_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
) -> Result<()> {
    let mut projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(event.updated_at));
    projection.apply_event(event.clone());
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    append_workspace_work_event_to_path(events_path, &event)?;
    Ok(())
}

/// SPEC-2359 Phase W-15 (FR-379/FR-381): one locally existing worktree as
/// reconcile input. `branch == None` (detached worktree) is never backfilled,
/// which encodes FR-381 — records are only generated for branches that have a
/// real working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeReconcileSource {
    pub branch: Option<String>,
    pub worktree_path: PathBuf,
}

/// SPEC-2359 Phase W-15 (FR-380 idempotency / SC-255): pure decision step of
/// worktree reconciliation. Returns the `(canonical work id, source)` pairs
/// that still need a Backfill record. A source is skipped when any existing
/// item already covers it: matching id, matching execution-container branch
/// (compared via the canonical branch identity, so `origin/x` == `x`), or
/// matching worktree path. Terminal (Done/Discarded) items also match here,
/// which keeps closed Work closed (US-61 — backfill never re-opens).
pub fn worktree_sources_needing_backfill(
    projection: &WorkItemsProjection,
    project_root: &Path,
    sources: &[WorktreeReconcileSource],
) -> Vec<(String, WorktreeReconcileSource)> {
    let mut pending = Vec::new();
    for source in sources {
        let Some(branch) = source
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(work_id) = canonical_work_id(project_root, Some(branch), None) else {
            continue;
        };
        let branch_identity = canonical_work_branch_identity(branch);
        let worktree_identity = canonical_worktree_identity(&source.worktree_path);
        let covered = projection.work_items.iter().any(|item| {
            item.id == work_id
                || item.execution_containers.iter().any(|container| {
                    container.branch.as_deref().is_some_and(|existing| {
                        canonical_work_branch_identity(existing) == branch_identity
                    }) || container.worktree_path.as_deref().is_some_and(|existing| {
                        canonical_worktree_identity(existing) == worktree_identity
                    })
                })
        });
        if covered || pending.iter().any(|(id, _)| *id == work_id) {
            continue;
        }
        pending.push((work_id, source.clone()));
    }
    pending
}

/// #3065: find the Work item that owns a given execution container. The
/// match mirrors the backfill coverage rule: canonical work id, canonical
/// branch identity (`origin/x` == `x`), or canonical worktree path. Used to
/// source the Workspace Resume context from the resumed Work itself instead
/// of the repo-shared current projection (whose identity may belong to a
/// different Work).
pub fn find_work_item_for_container<'a>(
    projection: &'a WorkItemsProjection,
    project_root: &Path,
    branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> Option<&'a WorkItem> {
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    let canonical_id = canonical_work_id(project_root, branch, worktree_path);
    let branch_identity = branch.map(canonical_work_branch_identity);
    let worktree_identity = worktree_path.map(canonical_worktree_identity);
    projection.work_items.iter().find(|item| {
        canonical_id
            .as_deref()
            .is_some_and(|work_id| item.id == work_id)
            || item.execution_containers.iter().any(|container| {
                branch_identity.as_deref().is_some_and(|identity| {
                    container.branch.as_deref().is_some_and(|existing| {
                        canonical_work_branch_identity(existing) == identity
                    })
                }) || worktree_identity.as_deref().is_some_and(|identity| {
                    container
                        .worktree_path
                        .as_deref()
                        .is_some_and(|existing| canonical_worktree_identity(existing) == identity)
                })
            })
    })
}

/// SPEC-2359 Phase W-15 (FR-380): record a single Backfill event for a
/// worktree that has no Work record. `status_category` stays `None` so a
/// re-ingested copy of this event (W-16 intake on another machine) cannot
/// regress a terminal item; the Idle surface state comes from the kind
/// mapping in `workspace_work_event_status`.
pub fn record_workspace_backfill_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_id: &str,
    branch: &str,
    worktree_path: &Path,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let mut event = WorkEvent::new(WorkEventKind::Backfill, work_id, updated_at);
    event.title = Some(branch.to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(worktree_path.to_path_buf()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(work_items_path, events_path, event)
}

/// SPEC-2359 Phase W-15 (FR-379/FR-380): reconcile locally existing worktrees
/// against the Work records and backfill the missing ones. Each Backfill
/// event is appended to the *owning worktree's* repo-local event log
/// (`<worktree>/.gwt/work/events.jsonl`) so it travels with that branch, and
/// applied to the shared home works projection. Returns the number of
/// backfilled worktrees.
pub fn reconcile_worktree_work_items_paths(
    work_items_path: &Path,
    project_root: &Path,
    sources: &[WorktreeReconcileSource],
    now: DateTime<Utc>,
) -> Result<usize> {
    let projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(now));
    let pending = worktree_sources_needing_backfill(&projection, project_root, sources);
    for (work_id, source) in &pending {
        let Some(branch) = source.branch.as_deref() else {
            continue;
        };
        let events_path = gwt_repo_local_work_events_path(&source.worktree_path);
        // FR-403: the baseline timestamp is the worktree's last real activity
        // — the HEAD committer time for git worktrees (directory mtime is
        // polluted by unrelated writes such as the backfill itself creating
        // `.gwt/`), falling back to the directory mtime and finally to `now`.
        // Reconciliation runs at bootstrap/open (not on the projection build
        // hot path), so one git spawn per newly backfilled worktree is fine.
        let baseline = worktree_head_commit_time(&source.worktree_path)
            .or_else(|| {
                fs::metadata(&source.worktree_path)
                    .and_then(|metadata| metadata.modified())
                    .map(DateTime::<Utc>::from)
                    .ok()
            })
            .unwrap_or(now)
            .min(now);
        record_workspace_backfill_event_paths(
            work_items_path,
            &events_path,
            work_id,
            branch,
            &source.worktree_path,
            baseline,
        )?;
    }
    Ok(pending.len())
}

/// #3065: report of one resume-owner-bleed repair pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeOwnerBleedRepairReport {
    /// Resume events whose stamped identity fields were cleared.
    pub sanitized_events: usize,
    /// Whether the shared current projection's identity was cleared.
    pub cleared_current: bool,
    /// Stray agents pruned from the shared current projection.
    pub pruned_current_agents: usize,
}

impl ResumeOwnerBleedRepairReport {
    pub fn changed(&self) -> bool {
        self.sanitized_events > 0 || self.cleared_current || self.pruned_current_agents > 0
    }
}

/// #3065: minimum number of distinct Work items sharing one identical resume
/// payload before it is treated as a bleed signature. Two branches may
/// legitimately resume the same SPEC with the same wording; three or more
/// identical (title, owner, next_action) stamps across different Work items
/// only arise from the shared-projection replay bug.
const RESUME_OWNER_BLEED_MIN_ITEMS: usize = 3;

fn resume_bleed_key(event: &WorkEvent) -> Option<(String, String, String)> {
    if event.kind != WorkEventKind::Resume {
        return None;
    }
    let owner = event
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = event.title.as_deref().map(str::trim).unwrap_or("");
    let next_action = event.next_action.as_deref().map(str::trim).unwrap_or("");
    Some((
        title.to_string(),
        owner.to_string(),
        next_action.to_string(),
    ))
}

/// #3065: the contaminated identity pair carried by one bleed event —
/// `(title, owner)`, both trimmed, owner non-empty.
fn bleed_identity_pair(event: &WorkEvent) -> Option<(String, String)> {
    let owner = event
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = event.title.as_deref().map(str::trim).unwrap_or("");
    Some((title.to_string(), owner.to_string()))
}

/// #3065: detection-based, idempotent repair for the resume owner bleed.
/// Detection: an identical (title, owner, next_action) resume payload stamped
/// onto [`RESUME_OWNER_BLEED_MIN_ITEMS`]+ distinct Work items. Sanitization,
/// in two strengths:
///
/// - events carrying a contaminated (title, owner) identity pair — the same
///   shared-projection snapshot also leaked through pause / update stamps —
///   have ALL identity fields cleared;
/// - events carrying a contaminated owner VALUE with a different title (the
///   update/done/pause leak stamped the poisoned owner alongside an
///   agent-authored title) lose only their owner.
///
/// Event ids are kept so the intake dedup still skips re-ingestion, and the
/// Work items are re-folded from their events. The shared current projection
/// is cleared when its (title, owner) pair carries the contamination (full
/// clear) or its owner value alone does (owner-only clear). Stray agents
/// assigned to a different work id are pruned from the current projection in
/// the same pass. Runs after every work-events ingest; converges to a no-op
/// once the data is clean.
pub fn repair_resume_owner_bleed_paths(
    work_items_path: &Path,
    current_projection_path: &Path,
    now: DateTime<Utc>,
) -> Result<ResumeOwnerBleedRepairReport> {
    use std::collections::{HashMap, HashSet};

    let mut report = ResumeOwnerBleedRepairReport::default();
    let Some(mut works) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(report);
    };

    let mut stamped: HashMap<(String, String, String), HashSet<String>> = HashMap::new();
    for item in &works.work_items {
        for event in &item.events {
            if let Some(key) = resume_bleed_key(event) {
                stamped.entry(key).or_default().insert(item.id.clone());
            }
        }
    }
    let contaminated_pairs: HashSet<(String, String)> = stamped
        .into_iter()
        .filter(|(_, items)| items.len() >= RESUME_OWNER_BLEED_MIN_ITEMS)
        .map(|((title, owner, _), _)| (title, owner))
        .collect();
    let contaminated_owners: HashSet<String> = contaminated_pairs
        .iter()
        .map(|(_, owner)| owner.clone())
        .collect();

    if !contaminated_pairs.is_empty() {
        let projection_updated_at = works.updated_at;
        let mut all_events: Vec<WorkEvent> = Vec::new();
        let mut eventless_items: Vec<WorkItem> = Vec::new();
        for item in works.work_items.drain(..) {
            if item.events.is_empty() {
                eventless_items.push(item);
                continue;
            }
            all_events.extend(item.events);
        }
        let mut sanitized = 0usize;
        for event in &mut all_events {
            let Some(pair) = bleed_identity_pair(event) else {
                continue;
            };
            if contaminated_pairs.contains(&pair) {
                event.title = None;
                event.intent = None;
                event.summary = None;
                event.owner = None;
                event.next_action = None;
                sanitized += 1;
            } else if contaminated_owners.contains(&pair.1) {
                event.owner = None;
                sanitized += 1;
            }
        }
        all_events.sort_by_key(|event| event.updated_at);
        let mut rebuilt = WorkItemsProjection::empty(projection_updated_at);
        for event in all_events {
            rebuilt.apply_event(event);
        }
        rebuilt.work_items.extend(eventless_items);
        if projection_updated_at > rebuilt.updated_at {
            rebuilt.updated_at = projection_updated_at;
        }
        save_workspace_work_items_projection_to_path(work_items_path, &rebuilt)?;
        report.sanitized_events = sanitized;
    }

    let Some(mut current) = load_workspace_projection_from_path(current_projection_path)? else {
        return Ok(report);
    };
    let mut current_changed = false;
    let current_pair = (
        current.title.trim().to_string(),
        current
            .owner
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
    );
    if !current_pair.1.is_empty() && contaminated_pairs.contains(&current_pair) {
        current.title = "Work".to_string();
        current.owner = None;
        current.summary = None;
        current.next_action = None;
        current.agents.clear();
        current.status_category = WorkspaceStatusCategory::Idle;
        current.status_text = "No active work".to_string();
        current.updated_at = now;
        report.cleared_current = true;
        current_changed = true;
    } else {
        if !current_pair.1.is_empty() && contaminated_owners.contains(&current_pair.1) {
            // The title drifted (agent-authored) but the owner value is the
            // contaminated one — drop only the owner.
            current.owner = None;
            current.updated_at = now;
            report.cleared_current = true;
            current_changed = true;
        }
        let before = current.agents.len();
        let current_id = current.id.clone();
        current.agents.retain(|agent| {
            agent
                .workspace_id
                .as_deref()
                .is_none_or(|assigned| assigned == current_id)
        });
        let pruned = before - current.agents.len();
        if pruned > 0 {
            report.pruned_current_agents = pruned;
            current.updated_at = now;
            current_changed = true;
        }
    }
    if current_changed {
        save_workspace_projection_to_path(current_projection_path, &current)?;
    }
    Ok(report)
}

/// #3065: repo-path wrapper for [`repair_resume_owner_bleed_paths`] resolving
/// the canonical home-projection paths.
pub fn repair_resume_owner_bleed_for_repo(
    repo_path: &Path,
    now: DateTime<Utc>,
) -> Result<ResumeOwnerBleedRepairReport> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    repair_resume_owner_bleed_paths(&work_items_path, &current_path, now)
}

/// HEAD committer time of a git worktree (`git log -1 --format=%ct`).
/// Returns `None` for non-repositories or unborn branches.
fn worktree_head_commit_time(worktree_path: &Path) -> Option<DateTime<Utc>> {
    let output = crate::process::hidden_command("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let seconds: i64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
}

/// SPEC-2359 Phase W-16 (FR-393): decompose legacy mega-items. The pre-W-12
/// implementation keyed journal/board events to the projection's single UUID
/// (or `workspace-<millis>`), fusing dozens of branches and thousands of
/// sessions into one Work record. Each event still carries its own
/// `execution_container.branch`, so any item whose events span two or more
/// canonical branch identities is re-keyed here: its events are replayed per
/// branch into canonical branch-derived items (`canonical_work_id`), titles
/// and agents follow each branch's events, and the legacy shell (including
/// branchless heartbeat events) is dropped. Canonical/per-session items have
/// a single branch, so a second run finds nothing to decompose (idempotent —
/// no marker file needed). Returns the number of decomposed legacy items.
pub fn decompose_legacy_multi_branch_work_items_paths(
    work_items_path: &Path,
    project_root: &Path,
) -> Result<usize> {
    let Some(mut projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(0);
    };
    let mut decomposed = 0usize;
    let mut replacement: Vec<WorkItem> = Vec::new();
    let mut pending_events: Vec<WorkEvent> = Vec::new();
    for item in projection.work_items.drain(..) {
        let mut branch_identities: Vec<String> = Vec::new();
        for event in &item.events {
            if let Some(branch) = event
                .execution_container
                .as_ref()
                .and_then(|container| container.branch.as_deref())
                .map(canonical_work_branch_identity)
            {
                if !branch_identities.contains(&branch) {
                    branch_identities.push(branch);
                }
            }
        }
        if branch_identities.len() < 2 {
            replacement.push(item);
            continue;
        }
        decomposed += 1;
        for event in &item.events {
            let Some(branch) = event
                .execution_container
                .as_ref()
                .and_then(|container| container.branch.as_deref())
            else {
                // Branchless heartbeat of the legacy shell — dropped with it.
                continue;
            };
            let Some(work_id) = canonical_work_id(project_root, Some(branch), None) else {
                continue;
            };
            let mut event = event.clone();
            event.work_item_id = work_id;
            pending_events.push(event);
        }
    }
    if decomposed == 0 {
        projection.work_items = replacement;
        return Ok(0);
    }
    pending_events.sort_by_key(|event| event.updated_at);
    projection.work_items = replacement;
    for event in pending_events {
        projection.apply_event(event);
    }
    projection.updated_at = chrono::Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    Ok(decomposed)
}

/// Convenience wrapper resolving the home works projection for `repo_path`
/// (with the legacy migration applied), then delegating to
/// [`decompose_legacy_multi_branch_work_items_paths`].
pub fn decompose_legacy_multi_branch_work_items(repo_path: &Path) -> Result<usize> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    decompose_legacy_multi_branch_work_items_paths(&work_items_path, repo_path)
}

/// Convenience wrapper resolving the home works projection for `repo_path`
/// (with the legacy migration applied), then delegating to
/// [`reconcile_worktree_work_items_paths`].
pub fn reconcile_worktree_work_items(
    repo_path: &Path,
    sources: &[WorktreeReconcileSource],
    now: DateTime<Utc>,
) -> Result<usize> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    reconcile_worktree_work_items_paths(&work_items_path, repo_path, sources, now)
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): record a `Pause` work event so a
/// Work whose owning agent session stopped is retained in the work history
/// (and on the Work surface) until the user explicitly closes it. `work_item_id`
/// is the session-derived canonical Work id (`work-session-<session_id>`), which
/// matches the live-agent grouping id so a later resume dedupes to one row.
/// Idempotent for already-closed (Done) Work: the `apply_event` Done-preservation
/// keeps a terminal Work terminal because the Pause event carries no explicit
/// `status_category`.
#[allow(clippy::too_many_arguments)]
pub fn record_workspace_work_paused_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    title: Option<&str>,
    summary: Option<&str>,
    owner: Option<&str>,
    board_refs: &[String],
    execution_container: Option<WorkspaceExecutionContainerRef>,
    agent_session_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let mut event = WorkEvent::new(WorkEventKind::Pause, work_item_id, updated_at);
    event.title = non_empty_clone(title);
    event.summary = non_empty_clone(summary);
    event.owner = non_empty_clone(owner);
    event.agent_session_id = non_empty_clone(agent_session_id);
    event.execution_container = execution_container;
    // Pause must not regress a terminal Work, so leave status_category implicit;
    // record the board refs (if any) so the retained row keeps its provenance.
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    for board_ref in board_refs {
        if let Some(board_ref) = non_empty_clone(Some(board_ref.as_str())) {
            let mut ref_event = WorkEvent::new(WorkEventKind::Update, work_item_id, updated_at);
            ref_event.board_entry_id = Some(board_ref);
            record_workspace_work_event_paths(work_items_path, events_path, ref_event)?;
        }
    }
    Ok(())
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): convenience wrapper resolving the
/// project-scoped work_items and close-event paths from `repo_path` and
/// invoking [`record_workspace_work_paused_event_paths`].
///
/// SPEC-2359 Phase W-15 (FR-384): Pause is a close-kind event, so it is
/// home-persisted only (`work-events-closed.jsonl`) and never enters the
/// git-tracked repo-local log.
#[allow(clippy::too_many_arguments)]
pub fn record_workspace_work_paused_event(
    repo_path: &Path,
    work_item_id: &str,
    title: Option<&str>,
    summary: Option<&str>,
    owner: Option<&str>,
    board_refs: &[String],
    execution_container: Option<WorkspaceExecutionContainerRef>,
    agent_session_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = gwt_workspace_work_events_closed_path_for_repo_path(repo_path);
    record_workspace_work_paused_event_paths(
        &work_items_path,
        &events_path,
        work_item_id,
        title,
        summary,
        owner,
        board_refs,
        execution_container,
        agent_session_id,
        updated_at,
    )
}

/// SPEC-2359 US-37 / FR-117..FR-120: Emit a single Done `WorkEvent`
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
    let mut event = WorkEvent::new(WorkEventKind::Done, work_item_id, updated_at);
    event.status_category = Some(WorkspaceStatusCategory::Done);
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    Ok(true)
}

/// SPEC-2359 US-37 / FR-117..FR-120: Convenience wrapper resolving the
/// project-scoped work_items and work_events paths from `repo_path` and
/// invoking [`emit_workspace_done_event_if_absent_paths`].
/// SPEC-2359 Phase W-15 (FR-384): Done is a close-kind event — home-persisted
/// only, never written to the git-tracked repo-local log.
pub fn emit_workspace_done_event_if_absent(
    repo_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_done_event_if_absent_paths(
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
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
                .any(|event| event.kind == WorkEventKind::Done)
        }))
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): Emit a single Discard
/// `WorkEvent` for `work_item_id` iff the Work is not already
/// terminal (Done or already Discarded). This is the canonical write path for a
/// user-initiated Discard close from the Work surface. Returns `Ok(true)` when
/// a new Discard event was appended, `Ok(false)` when the Work was already
/// terminal (idempotent noop so a re-close does nothing).
pub fn emit_workspace_discard_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    if work_item_is_terminal_in_projection(work_items_path, work_item_id)? {
        return Ok(false);
    }
    let event = WorkEvent::new(WorkEventKind::Discard, work_item_id, updated_at);
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    Ok(true)
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): Convenience wrapper resolving the
/// project-scoped work_items and work_events paths from `repo_path` and
/// invoking [`emit_workspace_discard_event_if_absent_paths`].
/// SPEC-2359 Phase W-15 (FR-384): Discard is a close-kind event —
/// home-persisted only, never written to the git-tracked repo-local log.
pub fn emit_workspace_discard_event_if_absent(
    repo_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_discard_event_if_absent_paths(
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
        work_item_id,
        updated_at,
    )
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): true when `work_item_id` is already
/// in a terminal close state (Done or discarded) in the saved projection. Used
/// to make Done / Discard close emission idempotent.
fn work_item_is_terminal_in_projection(work_items_path: &Path, work_item_id: &str) -> Result<bool> {
    let Some(projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(false);
    };
    Ok(projection
        .work_items
        .iter()
        .filter(|item| item.id == work_item_id)
        .any(|item| item.is_terminal()))
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
pub const WORK_ITEMS_REBUILD_VERSION: u32 = 1;

/// SPEC-2359 US-37: Outcome of the work_items.json rebuild migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkItemsRebuildOutcome {
    /// `work_events.jsonl` does not exist. Nothing to rebuild.
    Missing,
    /// Marker already records the current rebuild version. Skip silently.
    AlreadyMigrated,
    /// Rebuilt `work_items.json` from the event log and wrote the marker.
    Applied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkItemsRebuildMarker {
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
) -> Result<WorkItemsRebuildOutcome> {
    if rebuild_marker_at_or_above(marker_path, WORK_ITEMS_REBUILD_VERSION)? {
        return Ok(WorkItemsRebuildOutcome::AlreadyMigrated);
    }
    if !events_path.exists() {
        return Ok(WorkItemsRebuildOutcome::Missing);
    }
    let content = fs::read_to_string(events_path)?;
    let mut events: Vec<WorkEvent> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<WorkEvent>(line)
                .map_err(|err| GwtError::Other(format!("workspace work event json: {err}")))
        })
        .collect::<Result<Vec<_>>>()?;
    events.sort_by_key(|event| event.updated_at);
    let initial_updated_at = events
        .first()
        .map(|event| event.updated_at)
        .unwrap_or_else(chrono::Utc::now);
    let mut projection = WorkItemsProjection::empty(initial_updated_at);
    for event in events {
        projection.apply_event(event);
    }
    projection.updated_at = chrono::Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    write_rebuild_marker(marker_path)?;
    Ok(WorkItemsRebuildOutcome::Applied)
}

/// SPEC-2359 US-37 — SUPERSEDED by the W-16 intake consumer
/// (`work_events_intake` + the gwt-side `work_events_ingest` orchestrator).
/// The bootstrap no longer calls this; the permanently-installed idempotent
/// intake covers the same repo-local source plus worktree filesystems and
/// fetched `origin/*` refs. The `work_items.migration.json` marker file is
/// no longer read but is intentionally left on disk. Kept for tests and as
/// a manual recovery tool.
///
/// SPEC-2359 Phase W-15 (FR-384) caveat: close-kind events recorded after
/// W-15 live only in the home close log (`work-events-closed.jsonl`). A
/// replay of solely the repo-local log would resurrect closed Work — any
/// such replay must also merge the home close log.
pub fn rebuild_work_items_from_events_for_repo(
    repo_path: &Path,
) -> Result<WorkItemsRebuildOutcome> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(repo_path)?;
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
    Ok(serde_json::from_str::<WorkItemsRebuildMarker>(&body)
        .map(|marker| marker.version >= required)
        .unwrap_or(false))
}

fn write_rebuild_marker(path: &Path) -> Result<()> {
    let marker = WorkItemsRebuildMarker {
        version: WORK_ITEMS_REBUILD_VERSION,
        migrated_at: Some(chrono::Utc::now()),
    };
    let body = serde_json::to_vec_pretty(&marker)
        .map_err(|error| GwtError::Other(format!("work items migration marker: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, &body)
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): schema version for the one-time
/// agent identity reset. Bumping this re-runs [`reset_legacy_agent_identity_at`]
/// on existing data. Version 1 clears `title_summary` / `current_focus`
/// written by the legacy prompt-derivation hook so the display fallback and
/// agent re-authoring take over.
pub const WORKSPACE_AGENT_IDENTITY_RESET_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AgentIdentityResetMarker {
    version: u32,
    #[serde(default)]
    migrated_at: Option<DateTime<Utc>>,
}

fn agent_identity_reset_marker_path(current_path: &Path) -> PathBuf {
    current_path
        .parent()
        .map(|dir| dir.join("agent_identity.migration.json"))
        .unwrap_or_else(|| PathBuf::from("agent_identity.migration.json"))
}

fn agent_identity_reset_marker_at_or_above(path: &Path, required: u32) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<AgentIdentityResetMarker>(&body)
        .map(|marker| marker.version >= required)
        .unwrap_or(false))
}

fn write_agent_identity_reset_marker(path: &Path) -> Result<()> {
    let marker = AgentIdentityResetMarker {
        version: WORKSPACE_AGENT_IDENTITY_RESET_VERSION,
        migrated_at: Some(chrono::Utc::now()),
    };
    let body = serde_json::to_vec_pretty(&marker)
        .map_err(|error| GwtError::Other(format!("agent identity reset marker: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, &body)
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): clear legacy `title_summary` /
/// `current_focus` from the canonical projection at `current_path` exactly
/// once, guarded by a version marker. After this reset those fields are
/// authored only by the agent (`gwtd workspace update` / `gwtd board post`),
/// and empty values resolve through the display fallback chain. Returns
/// `true` when the reset marker was newly written, `false` when the marker
/// already records the current version (so agent-authored values written
/// after the reset are never cleared again).
pub fn reset_legacy_agent_identity_at(current_path: &Path) -> Result<bool> {
    let marker_path = agent_identity_reset_marker_path(current_path);
    if agent_identity_reset_marker_at_or_above(
        &marker_path,
        WORKSPACE_AGENT_IDENTITY_RESET_VERSION,
    )? {
        return Ok(false);
    }
    if let Some(mut projection) = load_workspace_projection_from_path(current_path)? {
        let mut changed = false;
        for agent in &mut projection.agents {
            if agent.title_summary.take().is_some() {
                changed = true;
            }
            if agent.current_focus.take().is_some() {
                changed = true;
            }
        }
        if changed {
            save_workspace_projection_to_path(current_path, &projection)?;
        }
    }
    write_agent_identity_reset_marker(&marker_path)?;
    Ok(true)
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): repo-scoped convenience wrapper for
/// the startup bootstrap. Resolves the canonical projection path and runs the
/// version-guarded one-time legacy identity reset. Call this once at startup
/// (alongside the work-items rebuild), not on every projection load, so a
/// freshly agent-authored title is never cleared.
pub fn reset_legacy_agent_identity_for_repo(repo_path: &Path) -> Result<bool> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    reset_legacy_agent_identity_at(&current_path)
}

/// SPEC-2359 US-37 / FR-119: Convenience wrapper resolving the project-scoped
/// current, work_items, and work_events paths from `repo_path` and invoking
/// [`retroactive_auto_done_scan_paths`].
/// SPEC-2359 Phase W-15 (FR-384): the emitted Done events are close-kind, so
/// they go to the home close log, never to the git-tracked repo-local log.
pub fn retroactive_auto_done_scan(repo_path: &Path, now: DateTime<Utc>) -> Result<usize> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = gwt_workspace_work_events_closed_path_for_repo_path(repo_path);
    retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
}

fn work_item_is_eligible_for_auto_done(item: &WorkItem) -> bool {
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

/// SPEC-2359 US-37 / FR-118: Emit a Done WorkEvent for the Workspace
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
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
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

pub fn append_workspace_work_event_to_path(path: &Path, event: &WorkEvent) -> Result<()> {
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
) -> Result<WorkItemsProjection> {
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
        return Ok(WorkItemsProjection::empty(updated_at));
    };
    Ok(WorkItemsProjection {
        updated_at: item.updated_at,
        work_items: vec![item],
    })
}

fn synthesize_workspace_work_item_from_legacy(
    projection: Option<&WorkspaceProjection>,
    journal_entries: &[WorkspaceJournalEntry],
    _project_root: &Path,
) -> Option<WorkItem> {
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
        .unwrap_or_else(|| "Work history".to_string());
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
    let mut item = WorkItem {
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
        discarded: false,
    };
    if let Some(projection) = projection {
        item.agents
            .extend(projection.assigned_agents().map(|agent| WorkAgentRef {
                session_id: agent.session_id.clone(),
                agent_id: Some(agent.agent_id.clone()),
                display_name: Some(agent.display_name.clone()),
                updated_at: agent.updated_at,
            }));
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
        let mut event = WorkEvent::new(
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
                item.agents.push(WorkAgentRef {
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
        let mut event = WorkEvent::new(
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
) -> WorkEvent {
    let mut event = WorkEvent::new(
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
) -> WorkEvent {
    let mut event = WorkEvent::new(
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

fn workspace_work_event_status(event: &WorkEvent) -> WorkspaceStatusCategory {
    event.status_category.unwrap_or(match event.kind {
        WorkEventKind::Done => WorkspaceStatusCategory::Done,
        WorkEventKind::Blocked => WorkspaceStatusCategory::Blocked,
        // Pause keeps the Work incomplete (non-Done) while the agent is stopped;
        // the Idle status preserves the retained-but-not-running semantics.
        WorkEventKind::Pause => WorkspaceStatusCategory::Idle,
        // Discard does not complete the Work (status stays non-Done); the
        // terminal close is carried by the `discarded` flag (FR-352). Idle
        // mirrors the retained-but-not-running runtime status.
        WorkEventKind::Discard => WorkspaceStatusCategory::Idle,
        // Backfill materializes a Work for an existing worktree with no live
        // agent, so it surfaces as retained-but-not-running (rendered Paused).
        WorkEventKind::Backfill => WorkspaceStatusCategory::Idle,
        WorkEventKind::Start
        | WorkEventKind::Claim
        | WorkEventKind::Update
        | WorkEventKind::Handoff
        | WorkEventKind::Resume
        | WorkEventKind::Split
        | WorkEventKind::Merge
        | WorkEventKind::Pr => WorkspaceStatusCategory::Active,
    })
}

fn workspace_work_event_kind_from_board_entry(entry: &BoardEntry) -> WorkEventKind {
    match entry.kind {
        BoardEntryKind::Claim => WorkEventKind::Claim,
        BoardEntryKind::Blocked => WorkEventKind::Blocked,
        BoardEntryKind::Handoff => WorkEventKind::Handoff,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Decision
        | BoardEntryKind::Request
        | BoardEntryKind::Impact
        | BoardEntryKind::Question => WorkEventKind::Update,
    }
}

fn workspace_work_event_kind_from_journal(
    index: usize,
    entry: &WorkspaceJournalEntry,
) -> WorkEventKind {
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
) -> WorkEventKind {
    match status_category {
        WorkspaceStatusCategory::Done => WorkEventKind::Done,
        WorkspaceStatusCategory::Blocked => WorkEventKind::Blocked,
        _ if index == 0 => WorkEventKind::Start,
        _ => WorkEventKind::Update,
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

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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

/// SPEC-2359 US-66 (T-529): legacy adapter aliases for the renamed Work
/// entity family. In-repo call sites are fully migrated; these exist only
/// so external consumers keep compiling. New code must use the Work names.
pub type WorkspaceWorkItem = WorkItem;
pub type WorkspaceWorkEvent = WorkEvent;
pub type WorkspaceWorkEventKind = WorkEventKind;
pub type WorkspaceWorkItemsProjection = WorkItemsProjection;
pub type WorkspaceWorkAgentRef = WorkAgentRef;
pub type WorkspaceWorkItemsCache = WorkItemsCache;
pub type WorkspaceWorkItemsRebuildOutcome = WorkItemsRebuildOutcome;

#[cfg(test)]
mod tests {
    // SPEC-2359 close-latency root fix: the works.json cache must stop
    // re-parsing unchanged files, observe content changes, and never cache a
    // synthesized fallback against a missing works.json.
    #[test]
    fn work_items_cache_reuses_unchanged_file_and_reparses_on_change() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let work_items_path = tmp.path().join("works.json");
        let current_path = tmp.path().join("current.json");
        let journal_path = tmp.path().join("journal.jsonl");
        let project_root = tmp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("repo dir");

        let now = chrono::Utc::now();
        let mut projection = super::WorkItemsProjection::empty(now);
        projection.apply_event(sample_work_event("work-1", now));
        super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("save works.json");

        let mut cache = super::WorkItemsCache::new();
        let first = cache
            .load_or_synthesize_from_paths(
                &work_items_path,
                &current_path,
                &journal_path,
                &project_root,
            )
            .expect("first load");
        assert_eq!(first.work_items.len(), 1);
        assert_eq!(cache.parse_count, 1);

        let second = cache
            .load_or_synthesize_from_paths(
                &work_items_path,
                &current_path,
                &journal_path,
                &project_root,
            )
            .expect("second load");
        assert_eq!(second.work_items.len(), 1);
        assert_eq!(
            cache.parse_count, 1,
            "unchanged works.json must not re-parse"
        );

        // Grow the file (extra item) so mtime granularity cannot mask the change.
        projection.apply_event(sample_work_event(
            "work-2",
            now + chrono::Duration::seconds(1),
        ));
        super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("resave works.json");
        let third = cache
            .load_or_synthesize_from_paths(
                &work_items_path,
                &current_path,
                &journal_path,
                &project_root,
            )
            .expect("third load");
        assert_eq!(third.work_items.len(), 2, "changed works.json must reload");
        assert_eq!(cache.parse_count, 2);
    }

    #[test]
    fn work_items_cache_never_caches_synthesized_fallback() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let work_items_path = tmp.path().join("works.json");
        let current_path = tmp.path().join("current.json");
        let journal_path = tmp.path().join("journal.jsonl");
        let project_root = tmp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("repo dir");

        let mut cache = super::WorkItemsCache::new();
        let synthesized = cache
            .load_or_synthesize_from_paths(
                &work_items_path,
                &current_path,
                &journal_path,
                &project_root,
            )
            .expect("synthesized load");
        assert!(synthesized.work_items.is_empty());

        // works.json appears afterwards: the next load must see it.
        let now = chrono::Utc::now();
        let mut projection = super::WorkItemsProjection::empty(now);
        projection.apply_event(sample_work_event("work-1", now));
        super::save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("save works.json");
        let loaded = cache
            .load_or_synthesize_from_paths(
                &work_items_path,
                &current_path,
                &journal_path,
                &project_root,
            )
            .expect("post-create load");
        assert_eq!(loaded.work_items.len(), 1);
    }

    fn sample_work_event(
        work_id: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> super::WorkEvent {
        let mut event = super::WorkEvent::new(super::WorkEventKind::Start, work_id, updated_at);
        event.status_category = Some(super::WorkspaceStatusCategory::Active);
        event.title = Some(format!("title {work_id}"));
        event
    }

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

    /// SPEC-2359 Phase W-11 (US-58 / SC-228): the one-time reset clears
    /// legacy title_summary / current_focus exactly once (version-guarded),
    /// later runs are a no-op, and agent-authored values written after the
    /// reset are preserved.
    #[test]
    fn reset_legacy_agent_identity_clears_once_and_preserves_later_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let current_path = temp.path().join("current.json");

        let mut projection = WorkspaceProjection::default_for_project(temp.path());
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "sess-legacy".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("/gwt-discussion 生プロンプト focus".to_string()),
            title_summary: Some("あなたの目的は何ですか".to_string()),
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: Utc::now(),
        });
        save_workspace_projection_to_path(&current_path, &projection).expect("save");

        // First reset clears the legacy values and writes the marker.
        let applied = reset_legacy_agent_identity_at(&current_path).expect("reset");
        assert!(applied, "first reset should run and write the marker");
        let after = load_workspace_projection_from_path(&current_path)
            .expect("load")
            .expect("present");
        assert_eq!(after.agents[0].title_summary, None);
        assert_eq!(after.agents[0].current_focus, None);

        // The agent authors a real purpose after the migration.
        let mut authored = after;
        authored.agents[0].title_summary = Some("Agent タイトル目的化".to_string());
        save_workspace_projection_to_path(&current_path, &authored).expect("save authored");

        // Second reset is a no-op (marker guard) and preserves the agent value.
        let applied_again = reset_legacy_agent_identity_at(&current_path).expect("reset again");
        assert!(!applied_again, "marker must prevent a second clear");
        let preserved = load_workspace_projection_from_path(&current_path)
            .expect("load")
            .expect("present");
        assert_eq!(
            preserved.agents[0].title_summary.as_deref(),
            Some("Agent タイトル目的化"),
            "agent-authored title must survive later loads"
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

        let mut start = WorkEvent::new(
            WorkEventKind::Start,
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

        let mut done = WorkEvent::new(WorkEventKind::Done, "workitem-workspace-history", done_at);
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
        assert_eq!(item.title, "Work WorkItem history");
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
        assert_eq!(item.events[0].kind, WorkEventKind::Start);
        assert_eq!(item.events[1].kind, WorkEventKind::Done);

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
        assert_eq!(item.title, "Work WorkItem history");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Active);
        assert_eq!(item.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(item.board_refs, vec!["board-legacy-1".to_string()]);
        assert_eq!(item.events.len(), 2);
        assert_eq!(
            item.events[0].summary.as_deref(),
            Some("Started from legacy journal.")
        );
        assert_eq!(item.events[1].kind, WorkEventKind::Blocked);
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

    fn lock_test_env() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn resolve_workspace_id_for_session_returns_assigned_workspace_id() {
        let _guard = lock_test_env();
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
        let _guard = lock_test_env();
        let dir = tempfile::tempdir().unwrap();
        let mut projection = WorkspaceProjection::default_for_project(dir.path());
        projection.agents.push(unassigned_agent("sess-B", "codex"));
        save_workspace_projection(dir.path(), &projection).unwrap();

        assert_eq!(resolve_workspace_id_for_session(dir.path(), "sess-B"), None);
    }

    #[test]
    fn resolve_workspace_id_for_session_returns_none_when_session_missing() {
        let _guard = lock_test_env();
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
        let _guard = lock_test_env();
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
        let _guard = lock_test_env();
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
        let _guard = lock_test_env();
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
        let _guard = lock_test_env();
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

        let mut start = WorkEvent::new(WorkEventKind::Start, "wi-auto-done", started_at);
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

        let mut start = WorkEvent::new(WorkEventKind::Start, "wi-idempotent", started_at);
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

        let mut eligible = WorkEvent::new(WorkEventKind::Start, "wi-eligible", started_at);
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

        let mut non_work = WorkEvent::new(WorkEventKind::Start, "wi-non-work-branch", started_at);
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

        let mut not_merged = WorkEvent::new(WorkEventKind::Start, "wi-not-merged", started_at);
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

        let mut eligible = WorkEvent::new(WorkEventKind::Start, "wi-twice", started_at);
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

        let mut start = WorkEvent::new(
            WorkEventKind::Start,
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

        let mut start = WorkEvent::new(
            WorkEventKind::Start,
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

        let mut projection = WorkItemsProjection::empty(t1);

        let mut done_event = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
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

        let update_event = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
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
        let mut done_event = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
        done_event.status_category = Some(WorkspaceStatusCategory::Done);
        done_event.title = Some("Recovered work".to_string());
        append_workspace_work_event_to_path(&events_path, &done_event).expect("append done");
        let update_event = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
        append_workspace_work_event_to_path(&events_path, &update_event).expect("append update");

        let outcome =
            rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
                .expect("rebuild");
        assert_eq!(outcome, WorkItemsRebuildOutcome::Applied);

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
        assert_eq!(outcome_again, WorkItemsRebuildOutcome::AlreadyMigrated);
    }

    #[test]
    fn apply_event_discard_marks_work_terminal_discarded() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a Discard event makes the Work
        // terminal-discarded (not Done) and removes it from the incomplete set.
        let work_item_id = "test-item-discard";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);
        let mut start = WorkEvent::new(WorkEventKind::Start, work_item_id, t1);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.title = Some("Discardable work".to_string());
        projection.apply_event(start);

        let discard = WorkEvent::new(WorkEventKind::Discard, work_item_id, t2);
        projection.apply_event(discard);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists");
        assert!(item.discarded, "Discard event must mark the Work discarded");
        assert!(item.is_terminal(), "discarded Work is terminal");
        assert!(!item.is_incomplete(), "discarded Work is not incomplete");
        assert_ne!(
            item.status_category,
            WorkspaceStatusCategory::Done,
            "Discard is distinct from Done"
        );
        assert_eq!(
            item.completed_at, None,
            "discarded Work is not completed, so completed_at stays None"
        );
    }

    #[test]
    fn apply_event_preserves_discarded_against_subsequent_heartbeat() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): Discarded is terminal — a later
        // heartbeat update (no explicit status_category) must not un-discard.
        let work_item_id = "test-item-discard-preserve";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);
        let discard = WorkEvent::new(WorkEventKind::Discard, work_item_id, t1);
        projection.apply_event(discard);
        let update = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
        projection.apply_event(update);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists");
        assert!(
            item.discarded,
            "heartbeat update must not clear the discarded terminal flag"
        );
        assert!(!item.is_incomplete());
    }

    #[test]
    fn emit_workspace_discard_event_if_absent_is_idempotent_for_terminal_work() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a re-close of an already
        // discarded (or already Done) Work is a noop.
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("work_items.json");
        let events_path = temp.path().join("work_events.jsonl");
        let work_item_id = "wi-discard-idem";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

        let mut start = WorkEvent::new(WorkEventKind::Start, work_item_id, t1);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("record start");

        assert!(
            emit_workspace_discard_event_if_absent_paths(
                &work_items_path,
                &events_path,
                work_item_id,
                t2
            )
            .expect("first discard"),
            "first discard appends a new event"
        );
        assert!(
            !emit_workspace_discard_event_if_absent_paths(
                &work_items_path,
                &events_path,
                work_item_id,
                t2
            )
            .expect("second discard"),
            "re-discarding a terminal Work is a noop"
        );

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load")
            .expect("present");
        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists");
        assert!(item.discarded);
        let discard_events = item
            .events
            .iter()
            .filter(|e| e.kind == WorkEventKind::Discard)
            .count();
        assert_eq!(discard_events, 1, "only one Discard event is recorded");
    }

    #[test]
    fn decide_work_close_blocks_when_live_agent_present() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a live agent blocks the close
        // and never requests worktree cleanup.
        assert_eq!(
            decide_work_close(true, Some(PathBuf::from("/repo/work/live"))),
            WorkCloseDecision::BlockedLiveAgent
        );
        assert_eq!(
            decide_work_close(true, None),
            WorkCloseDecision::BlockedLiveAgent
        );
    }

    #[test]
    fn decide_work_close_cleans_worktree_when_paused_with_path() {
        assert_eq!(
            decide_work_close(false, Some(PathBuf::from("/repo/work/paused"))),
            WorkCloseDecision::CleanupWorktree {
                worktree_path: PathBuf::from("/repo/work/paused")
            }
        );
    }

    #[test]
    fn decide_work_close_records_only_without_worktree_path() {
        assert_eq!(
            decide_work_close(false, None),
            WorkCloseDecision::RecordOnly
        );
        assert_eq!(
            decide_work_close(false, Some(PathBuf::new())),
            WorkCloseDecision::RecordOnly,
            "an empty worktree path is treated as unresolved"
        );
    }

    #[test]
    fn apply_event_idempotent_done_keeps_first_timestamp() {
        let work_item_id = "test-item-idempotent";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);

        let mut first_done = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
        first_done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(first_done);

        let mut second_done = WorkEvent::new(WorkEventKind::Done, work_item_id, t2);
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

    #[test]
    fn derive_merged_done_equivalent_classifies_only_merged_and_stale() {
        let merged_at = chrono::Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
        let before = merged_at - chrono::Duration::hours(1);
        let after = merged_at + chrono::Duration::hours(1);

        // merged ∧ stale (no update after the merge) → Done-equivalent.
        assert!(derive_merged_done_equivalent(true, before, Some(merged_at)));
        assert!(derive_merged_done_equivalent(
            true,
            merged_at,
            Some(merged_at)
        ));
        // updated after the merge → back to Active/Paused (FR-391).
        assert!(!derive_merged_done_equivalent(true, after, Some(merged_at)));
        // unmerged → never.
        assert!(!derive_merged_done_equivalent(
            false,
            before,
            Some(merged_at)
        ));
        // unknown merge reference → never.
        assert!(!derive_merged_done_equivalent(true, before, None));
    }

    #[test]
    fn workspace_group_key_groups_same_branch_across_spellings_and_ids() {
        let project_root = Path::new("/tmp/repo");
        let now = chrono::Utc::now();
        let mut item_a = WorkItem {
            id: "work-session-aaaa".to_string(),
            title: "a".to_string(),
            intent: None,
            summary: None,
            status_category: WorkspaceStatusCategory::Active,
            owner: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            agents: Vec::new(),
            execution_containers: vec![WorkspaceExecutionContainerRef {
                branch: Some("work/x".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            }],
            board_refs: Vec::new(),
            related_work_item_ids: Vec::new(),
            events: Vec::new(),
            discarded: false,
        };
        let mut item_b = item_a.clone();
        item_b.id = "work-session-bbbb".to_string();
        item_b.execution_containers[0].branch = Some("origin/work/x".to_string());
        let mut item_c = item_a.clone();
        item_c.id = "work-x-12345678".to_string();
        item_c.execution_containers[0].branch = Some("refs/remotes/origin/work/x".to_string());

        let key_a = workspace_group_key_for_item(project_root, &item_a);
        let key_b = workspace_group_key_for_item(project_root, &item_b);
        let key_c = workspace_group_key_for_item(project_root, &item_c);
        assert_eq!(key_a, key_b, "origin/X spelling groups with X");
        assert_eq!(key_a, key_c, "refs/remotes/origin/X spelling groups with X");

        // Branchless legacy items keep their own id (adapter: old rows never
        // vanish and never merge into each other).
        item_a.execution_containers.clear();
        item_a.id = "workspace-1748822400000".to_string();
        assert_eq!(
            workspace_group_key_for_item(project_root, &item_a),
            "workspace-1748822400000"
        );
        item_a.id = "0f5e2c1a-aaaa-bbbb-cccc-1234567890ab".to_string();
        assert_eq!(
            workspace_group_key_for_item(project_root, &item_a),
            "0f5e2c1a-aaaa-bbbb-cccc-1234567890ab"
        );
    }

    #[test]
    fn canonical_work_id_is_stable_for_branch_and_uses_readable_slug() {
        let repo = Path::new("/tmp/gwt/repo");

        let first = super::canonical_work_id(repo, Some("work/20260526-0043"), None)
            .expect("branch-derived work id");
        let second = super::canonical_work_id(repo, Some("work/20260526-0043"), None)
            .expect("branch-derived work id");

        assert_eq!(first, second);
        assert!(first.starts_with("work-work-20260526-0043-"));
        assert_eq!(first.rsplit('-').next().expect("hash").len(), 8);
    }

    #[test]
    fn canonical_work_id_changes_when_branch_or_project_changes() {
        let repo_a = Path::new("/tmp/gwt/repo-a");
        let repo_b = Path::new("/tmp/gwt/repo-b");

        let work_a = super::canonical_work_id(repo_a, Some("work/a"), None).expect("work a");
        let work_b = super::canonical_work_id(repo_a, Some("work/b"), None).expect("work b");
        let same_branch_other_project =
            super::canonical_work_id(repo_b, Some("work/a"), None).expect("work a in repo b");

        assert_ne!(work_a, work_b);
        assert_ne!(work_a, same_branch_other_project);
    }

    #[test]
    fn canonical_work_id_normalizes_remote_branch_names() {
        let repo = Path::new("/tmp/gwt/repo");

        let local = super::canonical_work_id(repo, Some("feature/gui"), None).expect("local id");
        let remote =
            super::canonical_work_id(repo, Some("origin/feature/gui"), None).expect("remote id");
        let ref_remote =
            super::canonical_work_id(repo, Some("refs/remotes/origin/feature/gui"), None)
                .expect("remote ref id");

        assert_eq!(local, remote);
        assert_eq!(local, ref_remote);
    }

    #[test]
    fn work_active_lifecycle_runs_active_when_agent_running() {
        assert_eq!(
            recompute_work_active_lifecycle(WorkAgentRuntime::Running, None),
            WorkActiveLifecycleState::Active
        );
    }

    #[test]
    fn work_active_lifecycle_pauses_when_agent_stopped_or_absent_and_not_closed() {
        // FR-350: agent stop alone never closes a Work; it becomes Paused.
        assert_eq!(
            recompute_work_active_lifecycle(WorkAgentRuntime::Stopped, None),
            WorkActiveLifecycleState::Paused
        );
        assert_eq!(
            recompute_work_active_lifecycle(WorkAgentRuntime::None, None),
            WorkActiveLifecycleState::Paused
        );
    }

    #[test]
    fn work_active_lifecycle_respects_explicit_user_close() {
        // Explicit user close wins over runtime, even while the agent is running.
        assert_eq!(
            recompute_work_active_lifecycle(WorkAgentRuntime::Running, Some(WorkCloseKind::Done)),
            WorkActiveLifecycleState::Done
        );
        assert_eq!(
            recompute_work_active_lifecycle(
                WorkAgentRuntime::Stopped,
                Some(WorkCloseKind::Discarded)
            ),
            WorkActiveLifecycleState::Discarded
        );
    }

    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): recording a Pause event persists
    /// the Work in the history as a non-Done (incomplete) item keyed by the
    /// session-derived id, carrying the branch / worktree execution container so
    /// the Work surface can render the retained Paused row.
    #[test]
    fn record_workspace_work_paused_event_retains_incomplete_history_item() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("works.json");
        let events_path = temp.path().join("work-events.jsonl");
        let container = WorkspaceExecutionContainerRef {
            branch: Some("work/paused".to_string()),
            worktree_path: Some(temp.path().join("work/paused")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };

        super::record_workspace_work_paused_event_paths(
            &work_items_path,
            &events_path,
            "work-session-session-paused",
            Some("Paused persistence"),
            Some("agent stopped"),
            Some("SPEC-2359"),
            &["board-1".to_string()],
            Some(container),
            Some("session-paused"),
            Utc::now(),
        )
        .expect("record paused event");

        let projection = super::load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items present");
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-session-session-paused")
            .expect("paused work item");
        assert!(item.is_incomplete(), "paused Work must stay non-Done");
        assert_ne!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, None);
        assert_eq!(item.title, "Paused persistence");
        assert_eq!(item.execution_containers.len(), 1);
        assert_eq!(
            item.execution_containers[0].branch.as_deref(),
            Some("work/paused")
        );
        assert!(item.board_refs.iter().any(|value| value == "board-1"));
        assert!(item
            .events
            .iter()
            .any(|event| event.kind == WorkEventKind::Pause));
    }

    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): a Pause event carries no explicit
    /// status, so the Done-preservation in `apply_event` keeps an already-closed
    /// (Done) Work terminal — agent stop must never reopen a closed Work.
    #[test]
    fn record_workspace_work_paused_event_does_not_reopen_done_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("works.json");
        let events_path = temp.path().join("work-events.jsonl");
        let now = Utc::now();
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-session-x", now);
        done.status_category = Some(WorkspaceStatusCategory::Done);
        super::record_workspace_work_event_paths(&work_items_path, &events_path, done)
            .expect("record done");

        super::record_workspace_work_paused_event_paths(
            &work_items_path,
            &events_path,
            "work-session-x",
            Some("Closed Work"),
            None,
            None,
            &[],
            None,
            Some("session-x"),
            now + chrono::Duration::seconds(1),
        )
        .expect("record paused event");

        let projection = super::load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items present");
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-session-x")
            .expect("work item");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert!(item.completed_at.is_some());
    }

    // ---------------------------------------------------------------------
    // SPEC-2359 Phase W-12 Slice 5b (FR-353 / FR-355 / FR-358): the Work
    // event log persistent core is repo-local and git-tracked.
    // ---------------------------------------------------------------------

    /// Override `HOME` for the duration of a test so the home-side projection
    /// writes (works.json, project-state) and the legacy migration sources
    /// resolve under an isolated temp directory. Restores the previous value
    /// on drop.
    struct ScopedHome {
        previous_home: Option<std::ffi::OsString>,
    }

    impl ScopedHome {
        fn set(path: &Path) -> Self {
            let previous_home = std::env::var_os("HOME");
            std::env::set_var("HOME", path);
            Self { previous_home }
        }
    }

    impl Drop for ScopedHome {
        fn drop(&mut self) {
            match self.previous_home.as_ref() {
                Some(previous) => std::env::set_var("HOME", previous),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn init_test_git_repo(path: &Path) {
        std::fs::create_dir_all(path).expect("create repo dir");
        let output = crate::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");
        for args in [
            ["config", "user.email", "test@example.com"],
            ["config", "user.name", "Test User"],
        ] {
            let mut cmd = crate::process::hidden_command("git");
            cmd.args(args).current_dir(path);
            crate::process::scrub_git_env(&mut cmd);
            assert!(cmd.output().expect("git config").status.success());
        }
    }

    fn start_event(work_item_id: &str, at: DateTime<Utc>) -> WorkEvent {
        let mut event = WorkEvent::new(WorkEventKind::Start, work_item_id, at);
        event.status_category = Some(WorkspaceStatusCategory::Active);
        event.title = Some("Repo-local work".to_string());
        event
    }

    #[test]
    fn record_workspace_work_event_writes_to_repo_local_events_log() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = workspace.path().join("repo");
        init_test_git_repo(&repo);

        let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
        record_workspace_work_event(&repo, start_event("wi-repo-local", t1)).expect("record event");

        // The event must land in the repo-local, git-tracked event log.
        let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
        assert!(
            repo_local.is_file(),
            "event must be written to repo-local .gwt/work/events.jsonl"
        );
        let body = std::fs::read_to_string(&repo_local).expect("read events");
        assert!(body.contains("wi-repo-local"), "event payload present");

        // The home Project State event log must NOT be written for new events.
        let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
        assert!(
            !home_events.exists(),
            "home project-state event log must not receive new events"
        );
    }

    #[test]
    fn record_workspace_work_event_adds_union_merge_gitattribute_idempotently() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = workspace.path().join("repo");
        init_test_git_repo(&repo);
        // Seed a pre-existing .gitattributes to confirm we append, not clobber.
        std::fs::write(repo.join(".gitattributes"), "*.sh text eol=lf\n")
            .expect("seed gitattributes");

        let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
        record_workspace_work_event(&repo, start_event("wi-attr", t1)).expect("record 1");
        record_workspace_work_event(
            &repo,
            start_event("wi-attr-2", t1 + chrono::Duration::seconds(1)),
        )
        .expect("record 2");

        let attributes =
            std::fs::read_to_string(repo.join(".gitattributes")).expect("read gitattributes");
        let union_lines = attributes
            .lines()
            .filter(|line| line.trim() == "**/.gwt/work/events.jsonl merge=union")
            .count();
        assert_eq!(
            union_lines, 1,
            "union-merge entry must be added exactly once"
        );
        assert!(
            attributes.contains("*.sh text eol=lf"),
            "pre-existing gitattributes content must be preserved"
        );
    }

    #[test]
    fn migrates_home_events_into_repo_local_once_then_skips() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = workspace.path().join("repo");
        init_test_git_repo(&repo);

        // Seed the home Project State event log with a historical event so the
        // one-time migration has something to copy.
        let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
        let t0 = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
        append_workspace_work_event_to_path(&home_events, &start_event("wi-historical", t0))
            .expect("seed home events");

        let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
        assert!(!repo_local.exists(), "precondition: repo-local absent");

        // First record triggers migration: the historical event is copied in,
        // then the new event is appended.
        let t1 = Utc.with_ymd_and_hms(2026, 6, 5, 10, 0, 0).unwrap();
        record_workspace_work_event(&repo, start_event("wi-new", t1)).expect("record new");

        let body = std::fs::read_to_string(&repo_local).expect("read repo-local");
        assert!(
            body.contains("wi-historical"),
            "migration must copy the home historical event into the repo-local log"
        );
        assert!(
            body.contains("wi-new"),
            "the new event is appended after migration"
        );

        // Mutate the home log AFTER migration. Because the repo-local file now
        // exists, the home source must never be read again (idempotent skip).
        append_workspace_work_event_to_path(
            &home_events,
            &start_event("wi-home-after-migration", t1 + chrono::Duration::seconds(5)),
        )
        .expect("append post-migration home event");

        record_workspace_work_event(
            &repo,
            start_event("wi-second", t1 + chrono::Duration::seconds(10)),
        )
        .expect("record second");

        let body2 = std::fs::read_to_string(&repo_local).expect("read repo-local again");
        assert!(
            !body2.contains("wi-home-after-migration"),
            "once repo-local exists the home source must not be migrated again"
        );
        assert!(body2.contains("wi-second"), "second new event appended");
    }

    #[test]
    fn rebuild_work_items_uses_repo_local_events_after_migration() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let workspace = tempfile::tempdir().expect("workspace");
        let repo = workspace.path().join("repo");
        init_test_git_repo(&repo);

        // A Done then later Update in the home log; rebuild must replay through
        // the repo-local log and recover the terminal Done state (regression
        // coverage that the repo-local path drives the existing rebuild).
        let home_events = gwt_workspace_work_events_path_for_repo_path(&repo);
        let t1 = Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 1, 11, 0, 0).unwrap();
        let mut done = WorkEvent::new(WorkEventKind::Done, "wi-rebuild", t1);
        done.status_category = Some(WorkspaceStatusCategory::Done);
        append_workspace_work_event_to_path(&home_events, &done).expect("seed done");
        let update = WorkEvent::new(WorkEventKind::Update, "wi-rebuild", t2);
        append_workspace_work_event_to_path(&home_events, &update).expect("seed update");

        let outcome = rebuild_work_items_from_events_for_repo(&repo).expect("rebuild");
        assert_eq!(outcome, WorkItemsRebuildOutcome::Applied);

        // The rebuild must have migrated and replayed the repo-local log.
        let repo_local = repo.join(".gwt").join("work").join("events.jsonl");
        assert!(
            repo_local.is_file(),
            "rebuild migrates events into repo-local log"
        );

        let projection = load_workspace_work_items_from_path(
            &gwt_workspace_work_items_path_for_repo_path(&repo),
        )
        .expect("load")
        .expect("present");
        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == "wi-rebuild")
            .expect("rebuilt item");
        assert_eq!(
            item.status_category,
            WorkspaceStatusCategory::Done,
            "Done terminal state recovered via repo-local replay"
        );
    }

    fn backfill_source(branch: Option<&str>, worktree_path: &Path) -> WorktreeReconcileSource {
        WorktreeReconcileSource {
            branch: branch.map(str::to_string),
            worktree_path: worktree_path.to_path_buf(),
        }
    }

    fn seeded_work_item(
        id: &str,
        branch: Option<&str>,
        status: WorkspaceStatusCategory,
        discarded: bool,
        at: DateTime<Utc>,
    ) -> WorkItem {
        WorkItem {
            id: id.to_string(),
            title: id.to_string(),
            intent: None,
            summary: None,
            status_category: status,
            owner: None,
            created_at: at,
            updated_at: at,
            completed_at: None,
            agents: Vec::new(),
            execution_containers: branch
                .map(|branch| {
                    vec![WorkspaceExecutionContainerRef {
                        branch: Some(branch.to_string()),
                        worktree_path: None,
                        pr_number: None,
                        pr_url: None,
                        pr_state: None,
                    }]
                })
                .unwrap_or_default(),
            board_refs: Vec::new(),
            related_work_item_ids: Vec::new(),
            events: Vec::new(),
            discarded,
        }
    }

    /// SPEC-2359 Phase W-15 (FR-379/FR-380): a real worktree without any
    /// matching record gets a Backfill event recorded into the worktree's own
    /// repo-local event log and surfaces as an Idle (-> Paused) work item with
    /// title = branch name and the canonical branch-derived work id.
    #[test]
    fn backfill_records_work_item_for_worktree_without_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-wt");
        fs::create_dir_all(&worktree).expect("worktree dir");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let backfilled = reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[backfill_source(Some("work/foo"), &worktree)],
            now,
        )
        .expect("reconcile");
        assert_eq!(backfilled, 1);

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert_eq!(projection.work_items.len(), 1);
        let item = &projection.work_items[0];
        let expected_id = canonical_work_id(&project_root, Some("work/foo"), None).unwrap();
        assert_eq!(item.id, expected_id);
        assert_eq!(item.title, "work/foo");
        assert_eq!(
            item.status_category,
            WorkspaceStatusCategory::Idle,
            "backfill surfaces as Idle (rendered Paused without live agent)"
        );
        assert!(item
            .execution_containers
            .iter()
            .any(|container| container.branch.as_deref() == Some("work/foo")
                && container.worktree_path.as_deref() == Some(worktree.as_path())));

        let events_path = gwt_repo_local_work_events_path(&worktree);
        let events_text = fs::read_to_string(&events_path).expect("worktree events log");
        let lines: Vec<&str> = events_text.lines().collect();
        assert_eq!(lines.len(), 1, "exactly one backfill event line");
        let event: WorkEvent = serde_json::from_str(lines[0]).expect("event json");
        assert_eq!(event.kind, WorkEventKind::Backfill);
        assert_eq!(
            event.status_category, None,
            "backfill must not carry an explicit status so apply_event terminal \
             preservation keeps closed items closed when the event is re-ingested"
        );
    }

    /// SPEC-2359 Phase W-15 (SC-255): repeated reconcile over the same sources
    /// is idempotent — no duplicate work items and no duplicate event lines.
    #[test]
    fn backfill_is_idempotent_across_repeated_reconcile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-wt");
        fs::create_dir_all(&worktree).expect("worktree dir");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
        let sources = [backfill_source(Some("work/foo"), &worktree)];

        let first =
            reconcile_worktree_work_items_paths(&work_items_path, &project_root, &sources, now)
                .expect("first reconcile");
        let second =
            reconcile_worktree_work_items_paths(&work_items_path, &project_root, &sources, now)
                .expect("second reconcile");
        assert_eq!((first, second), (1, 0));

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert_eq!(projection.work_items.len(), 1);
        let events_text =
            fs::read_to_string(gwt_repo_local_work_events_path(&worktree)).expect("events log");
        assert_eq!(events_text.lines().count(), 1);
    }

    /// SPEC-2359 Phase W-15 (FR-380 idempotency): a worktree whose branch is
    /// already covered by a session-keyed record (work-session-<uuid>) must not
    /// be backfilled, including when the recorded branch carries an origin/
    /// prefix (canonical branch identity comparison).
    #[test]
    fn backfill_skips_worktree_already_covered_by_session_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-wt");
        fs::create_dir_all(&worktree).expect("worktree dir");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let projection = WorkItemsProjection {
            updated_at: now,
            work_items: vec![seeded_work_item(
                "work-session-abc",
                Some("origin/work/foo"),
                WorkspaceStatusCategory::Active,
                false,
                now,
            )],
        };
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("seed works");

        let backfilled = reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[backfill_source(Some("work/foo"), &worktree)],
            now,
        )
        .expect("reconcile");
        assert_eq!(backfilled, 0);
        assert!(
            !gwt_repo_local_work_events_path(&worktree).exists(),
            "no backfill event log should be created for a covered branch"
        );
    }

    /// SPEC-2359 Phase W-15 (FR-381): a detached worktree (no branch) is never
    /// backfilled.
    #[test]
    fn backfill_skips_detached_worktree_without_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-wt");
        fs::create_dir_all(&worktree).expect("worktree dir");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let backfilled = reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[backfill_source(None, &worktree)],
            now,
        )
        .expect("reconcile");
        assert_eq!(backfilled, 0);
        assert!(load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .is_none());
    }

    /// SPEC-2359 Phase W-15 (US-61 preservation): a terminal record (Done or
    /// discarded) matching the worktree's branch is skipped entirely — no
    /// event is appended and the terminal status never regresses.
    #[test]
    fn backfill_does_not_reopen_terminal_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let done_worktree = temp.path().join("repo-done");
        let discarded_worktree = temp.path().join("repo-discarded");
        fs::create_dir_all(&done_worktree).expect("worktree dir");
        fs::create_dir_all(&discarded_worktree).expect("worktree dir");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let projection = WorkItemsProjection {
            updated_at: now,
            work_items: vec![
                seeded_work_item(
                    "work-session-done",
                    Some("work/done"),
                    WorkspaceStatusCategory::Done,
                    false,
                    now,
                ),
                seeded_work_item(
                    "work-session-discarded",
                    Some("work/discarded"),
                    WorkspaceStatusCategory::Idle,
                    true,
                    now,
                ),
            ],
        };
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("seed works");

        let backfilled = reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[
                backfill_source(Some("work/done"), &done_worktree),
                backfill_source(Some("work/discarded"), &discarded_worktree),
            ],
            now,
        )
        .expect("reconcile");
        assert_eq!(backfilled, 0);

        let reloaded = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert_eq!(reloaded.work_items.len(), 2);
        assert_eq!(
            reloaded.work_items[0].status_category,
            WorkspaceStatusCategory::Done
        );
        assert!(reloaded.work_items[1].discarded);
        assert!(!gwt_repo_local_work_events_path(&done_worktree).exists());
        assert!(!gwt_repo_local_work_events_path(&discarded_worktree).exists());
    }

    /// SPEC-2359 Phase W-15 (FR-380): the Backfill kind serializes as
    /// snake_case "backfill" on the wire and round-trips.
    #[test]
    fn backfill_event_kind_serializes_as_snake_case() {
        let json = serde_json::to_string(&WorkEventKind::Backfill).expect("serialize");
        assert_eq!(json, "\"backfill\"");
        let parsed: WorkEventKind = serde_json::from_str("\"backfill\"").expect("parse");
        assert_eq!(parsed, WorkEventKind::Backfill);
    }

    // --- SPEC-2359 Phase W-14 (US-70 / FR-375, SC-251): transition service ---

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

    fn legacy_event(
        work_item_id: &str,
        branch: Option<&str>,
        title: Option<&str>,
        session: Option<&str>,
        at: DateTime<Utc>,
    ) -> WorkEvent {
        let mut event = WorkEvent::new(WorkEventKind::Update, work_item_id, at);
        event.title = title.map(str::to_string);
        event.agent_session_id = session.map(str::to_string);
        event.execution_container = branch.map(|branch| WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        event
    }

    /// SPEC-2359 Phase W-16 (FR-393): a legacy mega-item whose events span
    /// multiple branches is decomposed into canonical branch-keyed items.
    /// Titles/agents follow each branch's events; the legacy item disappears;
    /// a second run is a no-op (idempotent).
    #[test]
    fn legacy_multi_branch_work_item_is_decomposed_per_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let work_items_path = temp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 10, 11, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let mega_id = "0c14f2ab-9f9a-4e79-94ab-db590cf88343";
        let mut projection = WorkItemsProjection::empty(t0);
        projection.apply_event(legacy_event(
            mega_id,
            Some("develop"),
            Some("develop での調査"),
            Some("sess-dev-1"),
            t0,
        ));
        projection.apply_event(legacy_event(
            mega_id,
            Some("work/foo"),
            Some("foo の実装"),
            Some("sess-foo-1"),
            t1,
        ));
        projection.apply_event(legacy_event(
            mega_id,
            Some("origin/develop"),
            Some("develop PR 監視"),
            Some("sess-dev-2"),
            t2,
        ));
        // Branchless heartbeat: dropped with the legacy shell on decomposition.
        projection.apply_event(legacy_event(mega_id, None, None, None, t2));
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("seed works");

        let decomposed =
            decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
                .expect("decompose");
        assert_eq!(decomposed, 1, "one legacy mega-item decomposed");

        let reloaded = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        let develop_id = canonical_work_id(&project_root, Some("develop"), None).unwrap();
        let foo_id = canonical_work_id(&project_root, Some("work/foo"), None).unwrap();
        assert!(
            reloaded.work_items.iter().all(|item| item.id != mega_id),
            "legacy mega-item must be removed"
        );
        let develop = reloaded
            .work_items
            .iter()
            .find(|item| item.id == develop_id)
            .expect("develop item");
        assert_eq!(
            develop.title, "develop PR 監視",
            "last develop event title wins (origin/develop normalizes to develop)"
        );
        assert_eq!(develop.events.len(), 2);
        let develop_sessions: Vec<_> = develop
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect();
        assert!(develop_sessions.contains(&"sess-dev-1"));
        assert!(develop_sessions.contains(&"sess-dev-2"));
        let foo = reloaded
            .work_items
            .iter()
            .find(|item| item.id == foo_id)
            .expect("work/foo item");
        assert_eq!(foo.title, "foo の実装");
        assert_eq!(foo.agents.len(), 1);

        let second =
            decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
                .expect("second run");
        assert_eq!(
            second, 0,
            "idempotent: canonical items are not re-decomposed"
        );
    }

    /// SPEC-2359 Phase W-16 (FR-393): single-branch items (the normal
    /// work-session shape) are left untouched by the decomposition.
    #[test]
    fn single_branch_work_items_are_not_decomposed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let work_items_path = temp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t0);
        projection.apply_event(legacy_event(
            "work-session-abc",
            Some("work/bar"),
            Some("bar の作業"),
            Some("sess-bar"),
            t0,
        ));
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("seed works");

        let decomposed =
            decompose_legacy_multi_branch_work_items_paths(&work_items_path, &project_root)
                .expect("decompose");
        assert_eq!(decomposed, 0);
        let reloaded = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert_eq!(reloaded.work_items.len(), 1);
        assert_eq!(reloaded.work_items[0].id, "work-session-abc");
    }

    /// SPEC-2359 Phase W-16 (FR-403 follow-up): a Backfill event is a
    /// synthetic materialization marker, not activity. Re-applying one (e.g.
    /// replaying a duplicated backfill line) must not advance an existing
    /// item's `updated_at` — otherwise hundreds of rows collapse onto the
    /// replay instant and the recency sort degenerates.
    #[test]
    fn backfill_event_does_not_bump_updated_at_of_existing_item() {
        let t_old = Utc.with_ymd_and_hms(2026, 5, 18, 9, 15, 0).unwrap();
        let t_backfill = Utc.with_ymd_and_hms(2026, 6, 10, 6, 19, 47).unwrap();
        let mut projection = WorkItemsProjection::empty(t_old);
        let mut start = WorkEvent::new(WorkEventKind::Update, "work-x", t_old);
        start.title = Some("作業中".to_string());
        projection.apply_event(start);
        assert_eq!(projection.work_items[0].updated_at, t_old);

        let mut backfill = WorkEvent::new(WorkEventKind::Backfill, "work-x", t_backfill);
        backfill.title = Some("work/x".to_string());
        projection.apply_event(backfill);

        let item = &projection.work_items[0];
        assert_eq!(
            item.updated_at, t_old,
            "backfill must not advance an existing item's updated_at"
        );
        assert_eq!(
            item.title, "作業中",
            "backfill must not overwrite a real title"
        );

        // A brand-new item still gets the backfill time as its baseline.
        let mut fresh = WorkEvent::new(WorkEventKind::Backfill, "work-new", t_backfill);
        fresh.title = Some("work/new".to_string());
        projection.apply_event(fresh);
        let fresh_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-new")
            .expect("new item");
        assert_eq!(fresh_item.updated_at, t_backfill);
    }

    /// SPEC-2359 Phase W-16 (FR-403 follow-up): a backfilled worktree's
    /// baseline timestamp is the worktree directory's mtime (its last real
    /// activity), not "now" — otherwise every freshly materialized old
    /// worktree floods the top of the recency-sorted list.
    #[test]
    fn backfill_uses_worktree_mtime_as_baseline_timestamp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-old");
        fs::create_dir_all(&worktree).expect("worktree dir");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_750_000_000); // 2025-06-15ish
        let dir = fs::File::open(&worktree).expect("open dir");
        dir.set_times(fs::FileTimes::new().set_modified(old))
            .expect("set mtime");
        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[WorktreeReconcileSource {
                branch: Some("work/old".to_string()),
                worktree_path: worktree.clone(),
            }],
            now,
        )
        .expect("reconcile");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        let item = &projection.work_items[0];
        assert!(
            item.updated_at < Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            "baseline must be the worktree mtime (2025), not now: {}",
            item.updated_at
        );
    }

    /// SPEC-2359 Phase W-16 (FR-403 follow-up): for a git worktree the
    /// backfill baseline is the HEAD committer time — directory mtime is
    /// polluted by unrelated writes (e.g. the backfill itself creating
    /// `.gwt/`), which collapsed mid-list ordering onto one instant.
    #[test]
    fn backfill_uses_head_commit_time_for_git_worktrees() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let worktree = temp.path().join("repo-wt");
        fs::create_dir_all(&worktree).expect("worktree dir");
        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "t@example.com"].as_slice(),
            ["config", "user.name", "T"].as_slice(),
        ] {
            let output = crate::process::hidden_command("git")
                .args(args)
                .current_dir(&worktree)
                .output()
                .expect("git");
            assert!(output.status.success());
        }
        let mut commit = crate::process::hidden_command("git");
        commit
            .args(["commit", "--allow-empty", "-m", "old"])
            .env("GIT_COMMITTER_DATE", "2025-06-15T00:00:00Z")
            .env("GIT_AUTHOR_DATE", "2025-06-15T00:00:00Z")
            .current_dir(&worktree);
        assert!(commit.output().expect("commit").status.success());

        let work_items_path = temp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 6, 11, 12, 0, 0).unwrap();
        reconcile_worktree_work_items_paths(
            &work_items_path,
            &project_root,
            &[WorktreeReconcileSource {
                branch: Some("work/old".to_string()),
                worktree_path: worktree.clone(),
            }],
            now,
        )
        .expect("reconcile");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert_eq!(
            projection.work_items[0].updated_at,
            Utc.with_ymd_and_hms(2025, 6, 15, 0, 0, 0).unwrap(),
            "git worktree baseline is the HEAD committer time"
        );
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

    // #3065: the resume context source lookup — a work item is found by
    // canonical branch identity (local or origin/ prefixed), by worktree
    // path, and misses cleanly for unknown containers.
    #[test]
    fn find_work_item_for_container_matches_branch_worktree_and_id() {
        let project_root = Path::new("/repo");
        let now = Utc.timestamp_opt(9_000, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(now);
        let work_id =
            canonical_work_id(project_root, Some("work/foo"), None).expect("canonical id");
        let mut event = WorkEvent::new(WorkEventKind::Backfill, work_id.clone(), now);
        event.title = Some("work/foo".to_string());
        event.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/foo".to_string()),
            worktree_path: Some(PathBuf::from("/wt/foo")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(event);

        let by_branch =
            find_work_item_for_container(&projection, project_root, Some("work/foo"), None)
                .expect("matched by branch");
        assert_eq!(by_branch.id, work_id);
        let by_remote =
            find_work_item_for_container(&projection, project_root, Some("origin/work/foo"), None)
                .expect("matched by remote-prefixed branch");
        assert_eq!(by_remote.id, work_id);
        let by_worktree = find_work_item_for_container(
            &projection,
            project_root,
            None,
            Some(Path::new("/wt/foo")),
        )
        .expect("matched by worktree path");
        assert_eq!(by_worktree.id, work_id);
        assert!(
            find_work_item_for_container(&projection, project_root, Some("work/other"), None)
                .is_none()
        );
    }

    #[test]
    fn work_item_latest_next_action_reads_most_recent_event() {
        let now = Utc.timestamp_opt(9_100, 0).unwrap();
        let later = Utc.timestamp_opt(9_200, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(now);
        let mut first = WorkEvent::new(WorkEventKind::Start, "work-x", now);
        first.next_action = Some("older action".to_string());
        projection.apply_event(first);
        let mut second = WorkEvent::new(WorkEventKind::Update, "work-x", later);
        second.next_action = Some("newer action".to_string());
        projection.apply_event(second);

        assert_eq!(
            projection.work_items[0].latest_next_action(),
            Some("newer action")
        );
    }

    fn bleed_resume_event(work_id: &str, seq: i64) -> WorkEvent {
        let mut event = WorkEvent::new(
            WorkEventKind::Resume,
            work_id,
            Utc.timestamp_opt(20_000 + seq, 0).unwrap(),
        );
        event.title = Some("gwt-manage-pr".to_string());
        event.owner = Some("SPEC-2359".to_string());
        event.summary = Some("765 active agents".to_string());
        event.next_action = Some("merged build re-check".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        event.agent_session_id = Some(format!("sess-{seq}"));
        event
    }

    fn backfill_event(work_id: &str, branch: &str, seq: i64) -> WorkEvent {
        let mut event = WorkEvent::new(
            WorkEventKind::Backfill,
            work_id,
            Utc.timestamp_opt(10_000 + seq, 0).unwrap(),
        );
        event.title = Some(branch.to_string());
        event.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(PathBuf::from(format!("/wt/{branch}"))),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        event
    }

    // #3065: the repair detects the bleed signature — an identical
    // (title, owner, next_action) resume payload stamped onto 3+ distinct
    // work items — sanitizes every event carrying the contaminated
    // (title, owner) identity (resume AND pause/update stamps), re-folds the
    // items, and clears the contaminated shared current projection even when
    // its next_action has drifted. Idempotent: a second run is a no-op.
    #[test]
    fn repair_resume_owner_bleed_sanitizes_cross_item_stamp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("works.json");
        let current_path = temp.path().join("current.json");
        let now = Utc.timestamp_opt(30_000, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(now);
        for (index, branch) in ["work/a", "work/b", "work/c"].iter().enumerate() {
            let work_id = format!("work-{}-0000000{index}", branch.replace('/', "-"));
            projection.apply_event(backfill_event(&work_id, branch, index as i64));
            projection.apply_event(bleed_resume_event(&work_id, index as i64));
        }
        // A pause stamp carrying the same contaminated identity on a fourth
        // item (the work-session-* leak path) is sanitized by the pair rule.
        let mut pause = WorkEvent::new(
            WorkEventKind::Pause,
            "work-session-sess-dead",
            Utc.timestamp_opt(20_900, 0).unwrap(),
        );
        pause.title = Some("gwt-manage-pr".to_string());
        pause.owner = Some("SPEC-2359".to_string());
        projection.apply_event(pause);
        // An update stamp with an agent-authored title but the contaminated
        // owner (the update/done leak) loses only its owner; the title and
        // the rest of the payload survive.
        let mut update = WorkEvent::new(
            WorkEventKind::Update,
            "work-work-d-00000003",
            Utc.timestamp_opt(20_950, 0).unwrap(),
        );
        update.title = Some("agent authored title".to_string());
        update.owner = Some("SPEC-2359".to_string());
        update.status_category = Some(WorkspaceStatusCategory::Active);
        projection.apply_event(update);
        let event_ids_before: std::collections::BTreeSet<String> = projection
            .work_items
            .iter()
            .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
            .collect();
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("save works");

        let mut current = WorkspaceProjection::default_for_project("/repo");
        current.id = "work-work-a-00000000".to_string();
        current.title = "gwt-manage-pr".to_string();
        current.owner = Some("SPEC-2359".to_string());
        // next_action drifted after the stamps were written; the pair rule
        // must still clear the identity.
        current.next_action = Some("Check Board for latest updates".to_string());
        current.status_text = "883 active agents".to_string();
        for seq in 0..5 {
            let mut agent = us70_agent(
                &format!("dead-{seq}"),
                WorkspaceStatusCategory::Active,
                WorkspaceAgentAffiliationStatus::Assigned,
            );
            agent.workspace_id = Some(format!("work-other-{seq}"));
            current.agents.push(agent);
        }
        save_workspace_projection_to_path(&current_path, &current).expect("save current");

        let report =
            repair_resume_owner_bleed_paths(&work_items_path, &current_path, now).expect("repair");
        assert_eq!(
            report.sanitized_events, 5,
            "3 resume stamps + 1 pause stamp + 1 owner-only update stamp"
        );
        assert!(report.cleared_current, "current.json identity cleared");

        let repaired = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        for item in &repaired.work_items {
            assert_eq!(item.owner, None, "owner cleared for {}", item.id);
            if item.id == "work-work-d-00000003" {
                assert_eq!(
                    item.title, "agent authored title",
                    "owner-only sanitize keeps the agent-authored title"
                );
            } else if item.id != "work-session-sess-dead" {
                assert!(
                    item.title.starts_with("work/"),
                    "title restored to branch name, got {}",
                    item.title
                );
            }
        }
        let event_ids_after: std::collections::BTreeSet<String> = repaired
            .work_items
            .iter()
            .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
            .collect();
        assert_eq!(
            event_ids_before, event_ids_after,
            "sanitized events keep their ids so the intake dedup still skips them"
        );

        let repaired_current = load_workspace_projection_from_path(&current_path)
            .expect("load current")
            .expect("current exists");
        assert_eq!(repaired_current.owner, None);
        assert_eq!(repaired_current.next_action, None);
        assert!(repaired_current.agents.is_empty(), "dead agents purged");

        let second = repair_resume_owner_bleed_paths(&work_items_path, &current_path, now)
            .expect("repair rerun");
        assert_eq!(second.sanitized_events, 0, "second run is a no-op");
        assert!(!second.cleared_current);
    }

    // #3065: two work items legitimately sharing the same owner/title (e.g.
    // two branches working one SPEC) stay untouched — the signature requires
    // 3+ distinct work items.
    #[test]
    fn repair_resume_owner_bleed_keeps_legitimate_duplicates_below_threshold() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("works.json");
        let current_path = temp.path().join("current.json");
        let now = Utc.timestamp_opt(31_000, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(now);
        for (index, branch) in ["work/a", "work/b"].iter().enumerate() {
            let work_id = format!("work-{}-0000000{index}", branch.replace('/', "-"));
            projection.apply_event(backfill_event(&work_id, branch, index as i64));
            projection.apply_event(bleed_resume_event(&work_id, index as i64));
        }
        save_workspace_work_items_projection_to_path(&work_items_path, &projection)
            .expect("save works");

        let report =
            repair_resume_owner_bleed_paths(&work_items_path, &current_path, now).expect("repair");
        assert_eq!(report.sanitized_events, 0);

        let untouched = load_workspace_work_items_from_path(&work_items_path)
            .expect("load works")
            .expect("projection exists");
        assert!(
            untouched
                .work_items
                .iter()
                .all(|item| item.owner.as_deref() == Some("SPEC-2359")),
            "below-threshold duplicates keep their owner"
        );
    }
}
