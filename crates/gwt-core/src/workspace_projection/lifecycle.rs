//! Status taxonomies and lifecycle recompute rules for Workspaces / Works.
//!
//! Three deliberately distinct status taxonomies coexist (FR-007); they share
//! variant names but answer different questions and must not be conflated:
//!
//! - `AgentStatus` (defined in the `gwt-agent` crate): per-agent process
//!   state of one agent session (is the process running, waiting, exited?).
//! - [`WorkspaceStatusCategory`] (this module): activity classification of a
//!   Workspace derived from its assigned agents — `Active` / `Idle` /
//!   `Blocked` / `Done` / `Unknown`.
//! - [`WorkspaceLifecycleStage`] (this module): user-facing workflow stage of
//!   the overall work — `Planning` / `Active` / `InReview` / `Done` /
//!   `Archived`.
//!
//! Additionally [`WorkActiveLifecycleState`] models the agent-session-centric
//! Work lifecycle (`Active` / `Paused` / `Done` / `Discarded`, closed only by
//! an explicit user action).
//!
//! Conversions and recomputations between these taxonomies (the
//! `recompute_*` functions and close-decision helpers) live in this module
//! only; other submodules call into them instead of re-deriving stages.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;

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

/// SPEC-2359 Phase U-6 (FR-132): coarse Workspace lifecycle stage. Distinct
/// from [`WorkspaceStatusCategory`], which tracks the runtime activity of the
/// linked Agents. `lifecycle_stage` answers "where is this work in its overall
/// progression?" (planning → active → in review → done → archived). It is
/// derived from `events + status_category` via
/// `recompute_lifecycle_stage`, but may also be explicitly set by the user
/// via the `workspace.update` operation with `params.status = "archived"`.
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

/// SPEC-2359 Phase U-6 (FR-132, FR-139, FR-143): derive a coarse
/// [`WorkspaceLifecycleStage`] from runtime activity signals. Used by
/// `workspace.update` (FR-139) to keep the lifecycle chip in sync
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

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
}
