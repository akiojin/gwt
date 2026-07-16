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
//!
//! The module is split into responsibility-focused submodules; every public
//! item is re-exported from this module root so `workspace_projection::X`
//! paths stay stable: `identity` (canonical Work IDs and grouping keys),
//! `lifecycle` (status taxonomies and recompute rules), `work_items` (Work
//! item / Work event model), `agents` (per-agent summaries), `projection`
//! (the [`WorkspaceProjection`] state model and its transitions), and
//! `persistence` (load/save, migration, event recording, rebuild, prune).

mod agents;
mod identity;
mod lifecycle;
mod persistence;
mod projection;
mod work_items;

pub use agents::{WorkKind, WorkspaceAgentSummary, SHELL_WORK_AGENT_ID};
pub use identity::{canonical_work_id, workspace_group_key_for_item};
pub use lifecycle::{
    decide_work_close, derive_merged_done_equivalent, recompute_lifecycle_stage,
    recompute_work_active_lifecycle, WorkActiveLifecycleState, WorkAgentRuntime, WorkCloseDecision,
    WorkCloseKind, WorkspaceAgentAffiliationStatus, WorkspaceLifecycleStage,
    WorkspaceStatusCategory,
};
pub use persistence::{
    append_workspace_journal_entry_to_path, append_workspace_work_event_to_path, apply_prune_plan,
    classify_workspace_projections, decompose_legacy_multi_branch_work_items,
    decompose_legacy_multi_branch_work_items_paths, emit_workspace_discard_event_for_session,
    emit_workspace_discard_event_for_session_paths, emit_workspace_discard_event_if_absent,
    emit_workspace_discard_event_if_absent_paths, emit_workspace_done_event_for_branch,
    emit_workspace_done_event_for_branch_paths, emit_workspace_done_event_for_session,
    emit_workspace_done_event_for_session_paths, emit_workspace_done_event_if_absent,
    emit_workspace_done_event_if_absent_paths, find_work_item_for_container,
    load_or_default_workspace_projection, load_or_default_workspace_projection_from_path,
    load_or_synthesize_workspace_work_items, load_or_synthesize_workspace_work_items_from_paths,
    load_recent_workspace_journal_entries, load_recent_workspace_journal_entries_from_path,
    load_workspace_projection, load_workspace_projection_from_path, load_workspace_work_items,
    load_workspace_work_items_from_path, mark_workspace_agent_stopped,
    mark_workspace_agent_stopped_at, mutate_existing_workspace_projection,
    mutate_workspace_projection, mutate_workspace_projection_at,
    rebuild_work_items_from_events_for_repo, rebuild_work_items_from_events_paths,
    reconcile_worktree_work_items, reconcile_worktree_work_items_paths,
    record_workspace_backfill_event_paths, record_workspace_work_event,
    record_workspace_work_event_paths, record_workspace_work_events_paths,
    record_workspace_work_paused_event, record_workspace_work_paused_event_paths,
    repair_resume_owner_bleed_for_repo, repair_resume_owner_bleed_paths,
    reset_legacy_agent_identity_at, reset_legacy_agent_identity_for_repo,
    resolve_workspace_id_for_mention, resolve_workspace_id_for_session, retroactive_auto_done_scan,
    retroactive_auto_done_scan_paths, save_workspace_projection, save_workspace_projection_to_path,
    save_workspace_work_items_projection_to_path, transact_workspace_state,
    transact_workspace_state_at, try_resolve_workspace_assignment_for_session,
    try_resolve_workspace_id_for_session, update_workspace_projection_with_journal,
    update_workspace_projection_with_journal_for_work_event_root,
    update_workspace_projection_with_journal_paths,
    update_workspace_projection_with_journal_paths_at, workspace_projection_stale_reason,
    workspace_work_event_from_board_entry, worktree_sources_needing_backfill, ClassifiedProjection,
    PruneAction, PruneSkipReason, PruneSummary, ResumeOwnerBleedRepairReport, StaleReason,
    WorkItemsCache, WorkItemsRebuildOutcome, WorkspaceRetentionConfig, WorkspaceSessionAssignment,
    WorktreeReconcileSource, WORKSPACE_AGENT_IDENTITY_RESET_VERSION, WORK_ITEMS_REBUILD_VERSION,
};
pub(crate) use persistence::{
    with_workspace_current_and_work_items_lock, with_workspace_work_items_lock, write_atomic,
};
pub use projection::{
    workspace_projection_default_created_at, GitDetails, WorkspaceCleanupCandidate,
    WorkspaceCleanupReason, WorkspaceJournalEntry, WorkspaceLaunchUpdate, WorkspaceProjection,
    WorkspaceProjectionUpdate, WorkspaceStartUpdate,
};
pub use work_items::{
    DuplicateWorkEventProvenance, WorkAgentRef, WorkEvent, WorkEventApplyOutcome, WorkEventKind,
    WorkItem, WorkItemsProjection, WorkspaceExecutionContainerRef, WorkspaceIssueLink,
    WorkspacePrLink, WorkspaceWorkAgentRef, WorkspaceWorkEvent, WorkspaceWorkEventKind,
    WorkspaceWorkItem, WorkspaceWorkItemsCache, WorkspaceWorkItemsProjection,
    WorkspaceWorkItemsRebuildOutcome,
};
