//! Workspace projection / resume / cleanup helpers split out of
//! `app_runtime/mod.rs` for SPEC-2077 US-11 / FR-044 (Phase H5,
//! arch-review handoff).
//!
//! Owns the free-function cluster that:
//! - projects [`ActiveAgentSession`]s into the persisted
//!   `WorkspaceProjection` ([`active_agent_summary_from_session`],
//!   [`merge_active_sessions_into_projection`],
//!   [`retain_live_workspace_agents`],
//!   [`workspace_projection_for_current_resume`])
//! - derives Active Work resume / cleanup state
//!   ([`workspace_projection_owner_title`],
//!   [`workspace_cleanup_candidate_for_projection`])
//! - persists launch projections and work events
//!   ([`save_workspace_launch_projection`],
//!   [`workspace_work_event_from_launch_projection`])
//! - runs the Workspace cleanup flow keyed by
//!   `WORKSPACE_CLEANUP_EVENT_ID` ([`spawn_workspace_cleanup_async`],
//!   [`clear_workspace_cleanup_git_details_event`])
//!
//! Behavior-preserving move: shared view helpers
//! (`active_work_projection_from_saved_with_journal`,
//! `non_empty_workspace_text`, ...) stay in `mod.rs` and are imported via
//! `super`.

use std::path::{Path, PathBuf};
use std::thread;

use super::{
    active_work_cleanup_candidate_view_from_candidate,
    active_work_projection_from_saved_with_journal, cleanup_selected_branches_with_progress,
    list_branch_entries_with_active_sessions, non_empty_workspace_text, work_session_index,
    workspace_journal_entry_view_from_entry, workspace_work_item_view_from_item,
    ActiveAgentSession, AppEventProxy, BackendEvent, BranchCleanupOptions, ClientId, OutboundEvent,
    UserEvent, WorkspaceResumeContext, WORKSPACE_CLEANUP_EVENT_ID,
    WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};

pub(super) fn active_agent_summary_from_session(
    session: &ActiveAgentSession,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
    gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: session.session_id.clone(),
        window_id: Some(session.window_id.clone()),
        agent_id: session.agent_id.clone(),
        display_name: session.display_name.clone(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(session.worktree_path.clone()),
        branch: Some(session.branch_name.clone()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at,
    }
}

pub(super) fn workspace_projection_owner_title(
    project_root: &Path,
    branch_name: Option<&str>,
) -> Option<String> {
    let branch_name = branch_name?.trim();
    if branch_name.is_empty() {
        return None;
    }
    let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
        .ok()
        .flatten()?;
    let projection_branch = projection.git_details.as_ref()?.branch.as_deref()?.trim();
    if projection_branch != branch_name {
        return None;
    }
    let owner = projection.owner?.trim().to_string();
    (!owner.is_empty()).then_some(owner)
}

pub(super) fn merge_active_sessions_into_projection<'a>(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: impl IntoIterator<Item = &'a ActiveAgentSession>,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    for session in sessions {
        let existing = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session.session_id)
            .or_else(|| {
                projection
                    .agents
                    .iter()
                    .find(|agent| agent.window_id.as_deref() == Some(session.window_id.as_str()))
            });
        let mut summary = active_agent_summary_from_session(session, updated_at);
        if let Some(existing) = existing {
            summary.affiliation_status = existing.affiliation_status;
            summary.workspace_id = existing.workspace_id.clone();
            summary.title_summary = existing.title_summary.clone();
            summary.current_focus = existing.current_focus.clone();
            summary.last_board_entry_id = existing.last_board_entry_id.clone();
            summary.last_board_entry_kind = existing.last_board_entry_kind.clone();
            summary.coordination_scope = existing.coordination_scope.clone();
        } else {
            summary.affiliation_status =
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
            summary.workspace_id = None;
        }
        projection.upsert_agent_summary(summary);
    }
}

pub(super) fn retain_live_workspace_agents(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    projection.retain_live_agents(
        sessions.iter().map(|session| session.session_id.as_str()),
        updated_at,
    );
}

pub(super) fn workspace_projection_for_current_resume(
    mut projection: gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    tab_title: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceProjection {
    merge_active_sessions_into_projection(&mut projection, sessions.iter().copied(), updated_at);
    retain_live_workspace_agents(&mut projection, sessions, updated_at);
    if !projection.has_current_agents() {
        projection.reset_idle_identity(tab_title, updated_at);
    }
    projection
}

pub(super) fn workspace_cleanup_candidate_for_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ActiveWorkCleanupCandidateView> {
    let branch = projection.git_details.as_ref()?.branch.as_deref()?;
    let branch_has_live_agent = sessions.iter().any(|session| session.branch_name == branch);
    let candidate = projection.cleanup_candidate(branch_has_live_agent)?;
    Some(active_work_cleanup_candidate_view_from_candidate(candidate))
}

pub(super) fn save_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    created_by_start_work: bool,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::load_or_default_workspace_projection(project_root)
            .map_err(|error| error.to_string())?;
    projection.project_root = project_root.to_path_buf();
    let work_id = gwt_core::workspace_projection::canonical_work_id(
        project_root,
        Some(session.branch_name.as_str()),
        Some(session.worktree_path.as_path()),
    );
    let owner = workspace_resume_context
        .and_then(|context| non_empty_workspace_text(context.owner.as_deref()))
        .or_else(|| linked_issue_number.map(|issue_number| format!("Issue #{issue_number}")));
    let agent = active_agent_summary_from_session(session, now);
    projection.apply_launch(
        gwt_core::workspace_projection::WorkspaceLaunchUpdate {
            work_id,
            title: workspace_resume_context
                .and_then(|context| non_empty_workspace_text(context.title.as_deref())),
            summary: workspace_resume_context
                .and_then(|context| non_empty_workspace_text(context.summary.as_deref())),
            owner,
            next_action: workspace_resume_context
                .and_then(|context| non_empty_workspace_text(context.next_action.as_deref())),
            branch: session.branch_name.clone(),
            worktree_path: session.worktree_path.clone(),
            base_branch: base_branch.map(str::to_string),
            created_by_start_work,
        },
        agent,
        now,
    );

    gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
        .map_err(|error| error.to_string())?;
    let work_event_kind = if workspace_resume_context.is_some() {
        gwt_core::workspace_projection::WorkEventKind::Resume
    } else {
        gwt_core::workspace_projection::WorkEventKind::Start
    };
    let work_event =
        workspace_work_event_from_launch_projection(&projection, session, work_event_kind, now);
    gwt_core::workspace_projection::record_workspace_work_event(project_root, work_event)
        .map_err(|error| error.to_string())
}

fn workspace_work_event_from_launch_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    session: &ActiveAgentSession,
    kind: gwt_core::workspace_projection::WorkEventKind,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkEvent {
    let mut event =
        gwt_core::workspace_projection::WorkEvent::new(kind, projection.id.clone(), updated_at);
    event.title = Some(projection.title.clone());
    event.intent = projection
        .summary
        .clone()
        .or_else(|| projection.next_action.clone());
    event.summary = Some(projection.status_text.clone());
    event.status_category = Some(projection.status_category);
    event.owner = projection.owner.clone();
    event.next_action = projection.next_action.clone();
    event.agent_session_id = Some(session.session_id.clone());
    event.agent_id = Some(session.agent_id.to_string());
    event.display_name = Some(session.display_name.clone());
    event.execution_container = projection.git_details.as_ref().map(|details| {
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: details.branch.clone(),
            worktree_path: details.worktree_path.clone(),
            pr_number: details.pr_number,
            pr_url: details.pr_url.clone(),
            pr_state: details.pr_state.clone(),
        }
    });
    event
}

pub(super) fn spawn_workspace_cleanup_async(
    proxy: AppEventProxy,
    client_id: ClientId,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branch: String,
    options: BranchCleanupOptions,
) {
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_client_id = client_id.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        std::slice::from_ref(&branch),
                        options,
                        move |progress| {
                            progress_proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                                progress_client_id.clone(),
                                BackendEvent::BranchCleanupProgress {
                                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                                    branch: progress.branch,
                                    execution_branch: progress.execution_branch,
                                    index: progress.index,
                                    total: progress.total,
                                    phase: progress.phase,
                                    message: progress.message,
                                },
                            )]));
                        },
                    );
                    let mut events = vec![OutboundEvent::reply(
                        client_id.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                            results: results.clone(),
                        },
                    )];
                    if results.iter().any(|result| {
                        result.branch == branch
                            && matches!(
                                result.status,
                                gwt::BranchCleanupResultStatus::Success
                                    | gwt::BranchCleanupResultStatus::Partial
                            )
                    }) {
                        // SPEC-2359 US-37 / FR-118: emit Done only after the
                        // matching workspace cleanup actually succeeded.
                        let _ =
                            gwt_core::workspace_projection::emit_workspace_done_event_for_branch(
                                &project_root,
                                &branch,
                                chrono::Utc::now(),
                            );
                        if let Some(event) =
                            clear_workspace_cleanup_git_details_event(&project_root)
                        {
                            events.push(event);
                        }
                    }
                    events
                }
                Err(error) => vec![OutboundEvent::reply(
                    client_id.clone(),
                    BackendEvent::BranchError {
                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::Dispatch(events));
    });
}

fn clear_workspace_cleanup_git_details_event(project_root: &Path) -> Option<OutboundEvent> {
    let mut projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
        .ok()
        .flatten()?;
    projection.clear_git_details_to_idle(chrono::Utc::now());
    if let Err(error) =
        gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
    {
        tracing::warn!(
            project_root = %project_root.display(),
            error = %error,
            "workspace projection cleanup state update skipped"
        );
        return None;
    }
    let journal_entries = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
        project_root,
        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
    )
    .unwrap_or_default()
    .iter()
    .map(workspace_journal_entry_view_from_entry)
    .collect::<Vec<_>>();
    // Rare post-cleanup path with no runtime handle: a one-shot cache load
    // matches the previous eager loader's cost and semantics.
    let agent_sessions = crate::session_ledger_cache::SessionLedgerCache::new()
        .load(&gwt_core::paths::gwt_sessions_dir());
    let session_index = work_session_index(&agent_sessions);
    let workspaces =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(project_root)
            .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                updated_at: projection.updated_at,
                work_items: Vec::new(),
            })
            .work_items
            .iter()
            .map(|item| workspace_work_item_view_from_item(item, &session_index))
            .collect::<Vec<_>>();
    Some(OutboundEvent::broadcast(
        BackendEvent::ActiveWorkProjection {
            projection: Box::new(active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                None,
            )),
        },
    ))
}
