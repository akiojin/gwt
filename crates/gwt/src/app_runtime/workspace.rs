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
use std::sync::Arc;
use std::thread;

use gwt::BranchCleanupOperationStore;

use super::{
    active_work_cleanup_candidate_view_from_candidate,
    active_work_projection_from_saved_with_journal, cleanup_selected_branches_with_progress,
    list_branch_entries_with_active_sessions, non_empty_workspace_text,
    resume_branch_refs_snapshot, work_session_index, workspace_journal_entry_view_from_entry,
    workspace_work_item_view_from_item, ActiveAgentSession, AppEventProxy, AppRuntime,
    BackendEvent, BranchCleanupOptions, BranchEntriesPhase, OutboundEvent, ResumeBranchIndex,
    UserEvent, WindowPreset, WorkspaceResumeContext, WORKSPACE_CLEANUP_EVENT_ID,
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
) -> gwt_core::error::Result<Option<String>> {
    let Some(branch_name) = branch_name
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    else {
        return Ok(None);
    };
    let Some(projection) = gwt_core::workspace_projection::load_workspace_projection(project_root)?
    else {
        return Ok(None);
    };
    let projection_branch = projection
        .git_details
        .as_ref()
        .and_then(|details| details.branch.as_deref())
        .map(str::trim);
    if projection_branch != Some(branch_name) {
        return Ok(None);
    }
    Ok(projection
        .owner
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty()))
}

pub(super) fn merge_active_sessions_into_projection<'a>(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: impl IntoIterator<Item = &'a ActiveAgentSession>,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    for session in sessions {
        let existing = projection
            .latest_agent_for_session(&session.session_id)
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
    // SPEC-2359 US-80 (FR-429): live agent sessions have no authority over
    // shell liveness, so keep every Shell Work here — otherwise the broadcast
    // rebuild would prune a just-registered Shell Work that has no agent
    // session. Shell Works are pruned only by their own PTY / window lifecycle.
    projection.retain_live_agents_keep_shells(
        sessions.iter().map(|session| session.session_id.as_str()),
        updated_at,
    );
}

/// SPEC-2359 US-80 (FR-427): register a Start-Work Shell as a first-class Work
/// in the persisted projection so it appears in the Active Work / Workspace
/// overview like an agent. Shell Works carry the window id as their identity
/// (no agent session) and reuse the agent canonical Work id derivation via the
/// shell's branch / worktree, so a shell groups into the same Work row as an
/// agent on the same branch. `retain_live_agents_keep_shells` drops only dead
/// agents (FR-429): launching the shell never prunes other live or paused
/// shells.
pub(super) fn save_shell_work_projection(
    project_root: &Path,
    window_id: &str,
    worktree_path: Option<PathBuf>,
    branch: Option<String>,
    live_session_ids: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::mutate_workspace_projection(project_root, |projection| {
        projection.retain_live_agents_keep_shells(live_session_ids.iter().map(String::as_str), now);
        let summary = gwt_core::workspace_projection::WorkspaceAgentSummary::shell_work(
            window_id.to_string(),
            worktree_path,
            branch,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            now,
        );
        projection.upsert_agent_summary(summary);
        projection.updated_at = now;
        Ok(())
    })
    .map_err(|error| error.to_string())
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

#[derive(Debug, Clone, Copy)]
pub(super) enum WorkspaceLaunchProjectionKind {
    StartWork,
    Resume { created_by_start_work: bool },
}

pub(super) struct WorkspaceLaunchTransition<'a> {
    pub(super) work_id: Option<String>,
    pub(super) base_branch: Option<&'a str>,
    pub(super) linked_issue_number: Option<u64>,
    /// Trusted execution owner (#3426). When present it overrides any
    /// presentation-derived `resume_context.owner` so the Work projection can
    /// never fork from the ECR/ledger owner kind.
    pub(super) canonical_owner: Option<gwt::cli::execution_state::ExecutionOwnerKey>,
    pub(super) resume_context: Option<&'a WorkspaceResumeContext>,
    pub(super) kind: WorkspaceLaunchProjectionKind,
    pub(super) live_session_ids: &'a std::collections::HashSet<String>,
    pub(super) now: chrono::DateTime<chrono::Utc>,
}

impl WorkspaceLaunchTransition<'_> {
    fn owner(&self) -> Option<String> {
        self.canonical_owner
            .map(workspace_owner_label)
            .or_else(|| {
                self.resume_context
                    .and_then(|context| non_empty_workspace_text(context.owner.as_deref()))
            })
            .or_else(|| {
                self.linked_issue_number
                    .map(|number| format!("Issue #{number}"))
            })
    }
}

/// Canonical Work owner label for a trusted execution owner key.
pub(super) fn workspace_owner_label(owner: gwt::cli::execution_state::ExecutionOwnerKey) -> String {
    match owner.kind {
        gwt::cli::execution_state::ExecutionOwnerKind::Spec => {
            format!("SPEC-{}", owner.number)
        }
        gwt::cli::execution_state::ExecutionOwnerKind::Issue => {
            format!("Issue #{}", owner.number)
        }
    }
}

impl WorkspaceLaunchProjectionKind {
    fn work_event_kind(self) -> gwt_core::workspace_projection::WorkEventKind {
        match self {
            Self::StartWork => gwt_core::workspace_projection::WorkEventKind::Start,
            Self::Resume { .. } => gwt_core::workspace_projection::WorkEventKind::Resume,
        }
    }

    fn created_by_start_work(self) -> bool {
        match self {
            Self::StartWork => true,
            Self::Resume {
                created_by_start_work,
            } => created_by_start_work,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn save_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    canonical_owner: Option<gwt::cli::execution_state::ExecutionOwnerKey>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    launch_kind: WorkspaceLaunchProjectionKind,
    live_session_ids: Option<&std::collections::HashSet<String>>,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    gwt_core::workspace_projection::transact_workspace_state_for_work_event_root(
        project_root,
        &session.worktree_path,
        |projection, work_items, _work_items_persisted| {
            // A worker's launch-time snapshot cannot prove another concurrent
            // launch is dead. Keep all persisted agents until the GUI requests
            // a projection refresh with its current live-session registry.
            let preserved_sessions;
            let live_session_ids = match live_session_ids {
                Some(live) => live,
                None => {
                    preserved_sessions = projection
                        .agents
                        .iter()
                        .map(|agent| agent.session_id.clone())
                        .collect();
                    &preserved_sessions
                }
            };
            let work_events = apply_workspace_launch_for_current_work(
                project_root,
                projection,
                work_items,
                session,
                WorkspaceLaunchTransition {
                    work_id: None,
                    base_branch,
                    linked_issue_number,
                    canonical_owner,
                    resume_context: workspace_resume_context,
                    kind: launch_kind,
                    live_session_ids,
                    now,
                },
                false,
            )?;
            Ok(((), work_events))
        },
    )
    .map_err(|error| error.to_string())
}

/// Select the Work lifetime before attaching a new Session, under the same
/// transaction as launch publication (and fresh ECR activation).
pub(super) fn apply_workspace_launch_for_current_work(
    project_root: &Path,
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    work_items: &gwt_core::workspace_projection::WorkItemsProjection,
    session: &ActiveAgentSession,
    mut transition: WorkspaceLaunchTransition<'_>,
    allow_successor_supersession: bool,
) -> gwt_core::error::Result<Vec<gwt_core::workspace_projection::WorkEvent>> {
    use gwt_core::workspace_projection::{
        can_upgrade_work_owner, canonical_work_id, current_work_id_for_launch, successor_work_id,
        workspace_group_key_for_item, WorkEventKind, WorkspaceStatusCategory,
    };
    let canonical_id = canonical_work_id(
        project_root,
        Some(&session.branch_name),
        Some(&session.worktree_path),
    );
    let owner = transition.owner();
    let mut resolved = std::borrow::Cow::Borrowed(work_items);
    let mut events = Vec::new();
    let mut predecessor_id = None;
    let mut owner_successor = false;
    loop {
        transition.work_id = current_work_id_for_launch(
            &resolved,
            project_root,
            Some(&session.branch_name),
            Some(&session.worktree_path),
            transition.canonical_owner.and(owner.as_deref()),
        )?;
        if let Some(current) = resolved
            .work_items
            .iter()
            .find(|item| Some(&item.id) == transition.work_id.as_ref())
            .filter(|item| item.is_terminal() || Some(&item.id) != canonical_id.as_ref())
        {
            let owner_matches = current.owner == owner
                || (current.owner.is_none() && transition.canonical_owner.is_some())
                || can_upgrade_work_owner(current.owner.as_deref(), owner.as_deref());
            let fresh_owner_lifetime = !owner_matches
                && current.is_terminal()
                && allow_successor_supersession
                && transition.canonical_owner.is_some()
                && session
                    .branch_name
                    .strip_prefix("work/issue-")
                    .and_then(|number| number.parse::<u64>().ok())
                    == transition.canonical_owner.map(|owner| owner.number);
            // The authenticated launch proves this Session's local path. Work's
            // branch identity spans hosts, including history with no saved path.
            let container_matches = current.execution_containers.iter().all(|container| {
                canonical_work_id(
                    project_root,
                    container.branch.as_deref(),
                    container.worktree_path.as_deref(),
                ) == canonical_id
            });
            if (!owner_matches && !fresh_owner_lifetime) || !container_matches {
                return Err(gwt_core::error::GwtError::Other(format!(
                    "cannot launch successor of Work {}: owner or execution container mismatch",
                    current.id
                )));
            }
            if resolved.work_items.iter().any(|item| {
                item.is_terminal()
                    && Some(workspace_group_key_for_item(project_root, item)) == canonical_id
                    && item
                        .agents
                        .iter()
                        .any(|agent| agent.session_id == session.session_id)
            }) {
                return Err(gwt_core::error::GwtError::Other(format!(
                    "Session {} belongs to a terminal Work; start a new Session for the successor",
                    session.session_id
                )));
            }
            if current.is_terminal() {
                let successor_id = successor_work_id(&current.id);
                let occupied_rows = resolved
                    .work_items
                    .iter()
                    .filter(|item| item.id == successor_id)
                    .collect::<Vec<_>>();
                if occupied_rows.len() > 1 {
                    return Err(gwt_core::error::GwtError::Other(format!(
                    "successor {successor_id} is duplicated; use workspace.close with close_kind superseded for exact-target diagnosis before issue.monitor.requeue"
                )));
                }
                if let Some(occupied) = occupied_rows.first() {
                    // Earlier refused launches could leave an Idle orphan at
                    // this deterministic ID. Complete its missing lineage under
                    // fresh execution authority, preserving historical events.
                    // Inline histories are compacted, so absence of Start in the
                    // retained tail cannot prove that a lifetime never started.
                    let recoverable_orphan = !occupied.is_terminal()
                        && occupied.status_category == WorkspaceStatusCategory::Idle
                        && occupied.related_work_item_ids.is_empty()
                        && (occupied.owner == owner
                            || can_upgrade_work_owner(occupied.owner.as_deref(), owner.as_deref()))
                        && !occupied.execution_containers.is_empty()
                        && occupied.execution_containers.iter().all(|container| {
                            canonical_work_id(
                                project_root,
                                container.branch.as_deref(),
                                container.worktree_path.as_deref(),
                            ) == canonical_id
                        })
                        && occupied
                            .events
                            .iter()
                            .all(|event| event.related_work_item_id.is_none())
                        && occupied
                            .agents
                            .iter()
                            .all(|agent| !transition.live_session_ids.contains(&agent.session_id));
                    if !recoverable_orphan || allow_successor_supersession {
                        if allow_successor_supersession && transition.canonical_owner.is_some() {
                            let audit = invalid_successor_supersession_event(
                                project_root,
                                current,
                                occupied,
                                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                                    branch: Some(session.branch_name.clone()),
                                    worktree_path: Some(session.worktree_path.clone()),
                                    pr_number: None,
                                    pr_url: None,
                                    pr_state: None,
                                },
                                SuccessorSupersessionAuthority::ExecutionOwner(
                                    owner.as_deref().expect("authenticated launch owner"),
                                ),
                                transition.live_session_ids,
                                transition.now,
                            )?;
                            transition.now = audit.updated_at;
                            if resolved.to_mut().apply_event(audit.clone())
                                != gwt_core::workspace_projection::WorkEventApplyOutcome::Applied
                            {
                                return Err(gwt_core::error::GwtError::Other(
                                "invalid successor supersession audit could not advance state; use workspace.close with close_kind superseded for exact-target diagnosis".to_string()
                            ));
                            }
                            events.push(audit);
                            continue;
                        }
                        return Err(gwt_core::error::GwtError::Other(format!(
                        "cannot launch successor of Work {}: successor {successor_id} has invalid lineage; the registered PM can use workspace.close with work_id {successor_id}, close_kind superseded, then issue.monitor.requeue", current.id)));
                    }
                }
                predecessor_id = Some(current.id.clone());
                owner_successor = fresh_owner_lifetime;
                transition.work_id = Some(successor_id);
            }
        }
        break;
    }
    let mut event = apply_workspace_launch_transition(projection, session, transition);
    if let Some(predecessor_id) = predecessor_id {
        event.kind = WorkEventKind::Start;
        event.related_work_item_id = Some(predecessor_id);
        if owner_successor {
            event.id = format!("owner-successor-{}", event.work_item_id);
        }
    }
    events.push(event);
    Ok(events)
}

/// Authority to retire invalid occupancy without changing historical owners.
#[derive(Clone, Copy)]
pub(super) enum SuccessorSupersessionAuthority<'a> {
    ExecutionOwner(&'a str),
    RegisteredPm,
}

/// Build the same bounded repair for authenticated fresh activation and an
/// explicit registered-PM close. Never attach the new Session to old history.
pub(super) fn invalid_successor_supersession_event(
    project_root: &Path,
    predecessor: &gwt_core::workspace_projection::WorkItem,
    occupied: &gwt_core::workspace_projection::WorkItem,
    container: gwt_core::workspace_projection::WorkspaceExecutionContainerRef,
    authority: SuccessorSupersessionAuthority<'_>,
    live_session_ids: &std::collections::HashSet<String>,
    now: chrono::DateTime<chrono::Utc>,
) -> gwt_core::error::Result<gwt_core::workspace_projection::WorkEvent> {
    use gwt_core::workspace_projection::{
        can_upgrade_work_owner, canonical_work_id, successor_work_id, WorkEvent, WorkEventKind,
    };
    let recovery = format!(
        "the registered PM can use workspace.close with work_id {}, close_kind superseded, then issue.monitor.requeue",
        occupied.id
    );
    let refuse = |reason: &str| {
        gwt_core::error::GwtError::Other(format!(
            "cannot supersede invalid successor {}: {reason}; {recovery}",
            occupied.id
        ))
    };
    if !predecessor.is_terminal() || successor_work_id(&predecessor.id) != occupied.id {
        return Err(refuse("exact terminal predecessor is required"));
    }
    if occupied
        .agents
        .iter()
        .any(|agent| live_session_ids.contains(&agent.session_id))
    {
        return Err(refuse(
            "a member Session is live; use pane.close on its window first",
        ));
    }
    let owner_matches = |actual: Option<&str>| match authority {
        SuccessorSupersessionAuthority::RegisteredPm => true,
        SuccessorSupersessionAuthority::ExecutionOwner(owner) => actual.is_none_or(|actual| {
            actual == owner
                || can_upgrade_work_owner(Some(actual), Some(owner))
                || can_upgrade_work_owner(Some(owner), Some(actual))
        }),
    };
    if !owner_matches(predecessor.owner.as_deref())
        || !owner_matches(occupied.owner.as_deref())
        || occupied
            .events
            .iter()
            .any(|event| !owner_matches(event.owner.as_deref()))
    {
        return Err(refuse(
            "known owner conflict; repair requires registered PM authority",
        ));
    }
    let canonical_id = canonical_work_id(
        project_root,
        container.branch.as_deref(),
        container.worktree_path.as_deref(),
    );
    if canonical_id.is_none() {
        return Err(refuse("canonical execution container is required"));
    }
    let container_matches =
        |candidate: &gwt_core::workspace_projection::WorkspaceExecutionContainerRef| {
            canonical_work_id(
                project_root,
                candidate.branch.as_deref(),
                candidate.worktree_path.as_deref(),
            ) == canonical_id
        };
    if occupied.execution_containers.iter().any(|candidate| !container_matches(candidate))
        || occupied.events.iter().filter_map(|event| event.execution_container.as_ref()).any(|candidate| !container_matches(candidate))
        || occupied.duplicate_event_containers.values().flatten().any(|provenance| {
            // Duplicate provenance is serialized either as a complete event
            // or a legacy container; both retain independent branch evidence.
            match provenance {
                gwt_core::workspace_projection::DuplicateWorkEventProvenance::Event(event) =>
                    event.execution_container.as_ref().is_some_and(|candidate| !container_matches(candidate))
                    || !owner_matches(event.owner.as_deref())
                    || event.related_work_item_id.as_deref().is_some_and(|parent| parent != predecessor.id),
                gwt_core::workspace_projection::DuplicateWorkEventProvenance::LegacyContainer(candidate) => !container_matches(candidate),
            }
        })
    {
        return Err(refuse("foreign execution container evidence"));
    }
    if occupied
        .related_work_item_ids
        .iter()
        .any(|parent| parent != &predecessor.id)
        || occupied
            .events
            .iter()
            .filter_map(|event| event.related_work_item_id.as_deref())
            .any(|parent| parent != predecessor.id)
    {
        return Err(refuse("unrelated Work lineage"));
    }
    validate_superseded_member_runtimes(&gwt_core::paths::gwt_sessions_dir(), occupied)?;
    let latest = occupied
        .events
        .iter()
        .map(|event| event.updated_at)
        .chain([occupied.updated_at])
        .chain(occupied.completed_at)
        .chain(occupied.discarded_at)
        .chain(occupied.events_compacted_through())
        .max()
        .expect("occupied Work timestamp");
    let after_history = latest
        .checked_add_signed(chrono::Duration::nanoseconds(1))
        .ok_or_else(|| refuse("history timestamp has no safe successor instant"))?;
    let mut event = WorkEvent::new(WorkEventKind::Discard, &occupied.id, now.max(after_history));
    event.id = format!("superseded-{}", occupied.id);
    event.summary =
        Some("Superseded invalid successor occupancy by authenticated recovery".to_string());
    event.related_work_item_id = Some(predecessor.id.clone());
    event.execution_container = Some(container);
    event.bind_successor_supersession_identity()?;
    Ok(event)
}

/// Recheck every recorded member while the caller holds the execution owner
/// lease. Runtime proof is published before a child is released, including
/// children not yet visible in this Host's GUI registry.
pub(super) fn validate_superseded_member_runtimes(
    sessions_dir: &Path,
    occupied: &gwt_core::workspace_projection::WorkItem,
) -> gwt_core::error::Result<()> {
    use gwt::cli::execution_state::{
        classify_exact_session_runtime, ExactSessionRuntimeDisposition,
    };
    let members = occupied
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .chain(
            occupied
                .events
                .iter()
                .filter_map(|event| event.agent_session_id.as_deref()),
        )
        .chain(occupied.duplicate_event_containers.values().flatten().filter_map(|provenance| {
            match provenance {
                gwt_core::workspace_projection::DuplicateWorkEventProvenance::Event(event) => event.agent_session_id.as_deref(),
                gwt_core::workspace_projection::DuplicateWorkEventProvenance::LegacyContainer(_) => None,
            }
        }))
        .collect::<std::collections::BTreeSet<_>>();
    for session_id in members {
        let refuse = || {
            gwt_core::GwtError::Other(format!(
            "invalid successor member {session_id} has live or unprovable runtime; the registered PM can use pane.close, then workspace.close with close_kind superseded and issue.monitor.requeue"
        ))
        };
        if gwt_agent::validate_session_id_path_component(session_id).is_err() {
            return Err(refuse());
        }
        let identity =
            match gwt_agent::inspect_session_path(&sessions_dir.join(format!("{session_id}.toml")))
            {
                gwt_agent::SessionPathState::Present(session) => {
                    if session.id != session_id {
                        return Err(refuse());
                    }
                    let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                        .map_err(|_| refuse())?;
                    if identity.is_none() {
                        // A stale legacy sidecar is not evidence of a live
                        // writer. Reuse the exact runtime census, including
                        // its uncertainty checks; Stopped alone grants nothing.
                        let observed =
                            gwt::session_inventory::observe_session(&session, sessions_dir);
                        if !observed.sessions.is_empty() || !observed.uncertainties.is_empty() {
                            return Err(refuse());
                        }
                        continue;
                    }
                    identity
                }
                gwt_agent::SessionPathState::Missing => None,
                gwt_agent::SessionPathState::Error(error) => return Err(error.into()),
            };
        if let Some(identity) = identity {
            match classify_exact_session_runtime(sessions_dir, &identity)? {
                ExactSessionRuntimeDisposition::Live | ExactSessionRuntimeDisposition::Unknown => {
                    return Err(refuse())
                }
                _ => continue,
            }
        }
        // Legacy/missing Sessions have no exact identity to classify. Only
        // total sidecar absence proves that no Host is running one.
        let namespaces = match std::fs::read_dir(sessions_dir.join("runtime")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for namespace in namespaces {
            if namespace?
                .path()
                .join(format!("{session_id}.json"))
                .try_exists()?
            {
                return Err(refuse());
            }
        }
    }
    Ok(())
}

pub(super) fn apply_workspace_launch_transition(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    session: &ActiveAgentSession,
    transition: WorkspaceLaunchTransition<'_>,
) -> gwt_core::workspace_projection::WorkEvent {
    let owner = transition.owner();
    // #3065: drop dead agent entries before computing the running-agents
    // status text. Agent launch never prunes Shell Work.
    projection.retain_live_agents_keep_shells(
        transition.live_session_ids.iter().map(String::as_str),
        transition.now,
    );
    projection.apply_launch(
        gwt_core::workspace_projection::WorkspaceLaunchUpdate {
            work_id: transition.work_id,
            title: transition
                .resume_context
                .and_then(|context| non_empty_workspace_text(context.title.as_deref())),
            summary: transition
                .resume_context
                .and_then(|context| non_empty_workspace_text(context.summary.as_deref())),
            owner,
            next_action: transition
                .resume_context
                .and_then(|context| non_empty_workspace_text(context.next_action.as_deref())),
            branch: session.branch_name.clone(),
            worktree_path: session.worktree_path.clone(),
            base_branch: transition.base_branch.map(str::to_string),
            created_by_start_work: transition.kind.created_by_start_work(),
        },
        active_agent_summary_from_session(session, transition.now),
        transition.now,
    );
    workspace_work_event_from_launch_projection(
        projection,
        session,
        transition.kind.work_event_kind(),
        transition.now,
    )
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

/// Issue #4433: everything the Workspace cleanup worker needs. Grouped into a
/// struct so the reconnect bookkeeping (`operation_id`, `operations`) does not
/// push the spawn helpers past clippy's argument budget.
pub(super) struct WorkspaceCleanupAsyncTask {
    pub(super) context: super::ProjectContext,
    pub(super) project_root: PathBuf,
    pub(super) active_session_branches: std::collections::HashSet<String>,
    pub(super) branch: String,
    pub(super) operation_id: String,
    pub(super) operations: Arc<BranchCleanupOperationStore>,
    pub(super) options: BranchCleanupOptions,
}

/// Issue #4433: same as [`WorkspaceCleanupAsyncTask`], for the Branches
/// window cleanup worker.
struct BranchCleanupAsyncTask {
    context: super::ProjectContext,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branches: Vec<String>,
    operation_id: String,
    operations: Arc<BranchCleanupOperationStore>,
    options: BranchCleanupOptions,
}

pub(super) fn spawn_workspace_cleanup_async(proxy: AppEventProxy, task: WorkspaceCleanupAsyncTask) {
    let WorkspaceCleanupAsyncTask {
        context,
        project_root,
        active_session_branches,
        branch,
        operation_id,
        operations,
        options,
    } = task;
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_context = context.clone();
                    let progress_operation_id = operation_id.clone();
                    let progress_operations = operations.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        std::slice::from_ref(&branch),
                        options,
                        move |progress| {
                            // Issue #4433: broadcast, so a client that
                            // reconnected mid-cleanup still receives progress.
                            progress_operations.record_progress(
                                WORKSPACE_CLEANUP_EVENT_ID,
                                &progress_operation_id,
                                &progress,
                            );
                            progress_proxy.send(UserEvent::ProjectDispatch {
                                context: progress_context.clone(),
                                events: vec![OutboundEvent::project(
                                    progress_context.project_key.clone(),
                                    BackendEvent::BranchCleanupProgress {
                                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                                        operation_id: progress_operation_id.clone(),
                                        branch: progress.branch,
                                        execution_branch: progress.execution_branch,
                                        index: progress.index,
                                        total: progress.total,
                                        phase: progress.phase,
                                        message: progress.message,
                                    },
                                )],
                            });
                        },
                    );
                    operations.record_result(WORKSPACE_CLEANUP_EVENT_ID, &operation_id, &results);
                    let mut events = vec![OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                            operation_id: operation_id.clone(),
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
                // Issue #4433: broadcast so a reconnected client is not left
                // waiting on a cleanup that already failed to start.
                Err(error) => vec![OutboundEvent::project(
                    context.project_key.clone(),
                    BackendEvent::BranchError {
                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::ProjectDispatch { context, events });
    });
}

fn clear_workspace_cleanup_git_details_event(project_root: &Path) -> Option<OutboundEvent> {
    let projection =
        match gwt_core::workspace_projection::mutate_existing_workspace_projection_for_cleanup(
            project_root,
            |projection| {
                projection.clear_git_details_to_idle(chrono::Utc::now());
                Ok(projection.clone())
            },
        ) {
            Ok(Some(projection)) => projection,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    error = %error,
                    "workspace projection cleanup state update skipped"
                );
                return None;
            }
        };
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
    // Issue #3611: this runs on the branch-cleanup worker, so one bulk ref
    // snapshot is affordable — but a per-Session Git probe is not, and the
    // event loop must never inherit one through this view builder.
    let known_branch_refs = resume_branch_refs_snapshot(project_root);
    let resume_branches = ResumeBranchIndex::scanned(Some(&known_branch_refs));
    let workspaces =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(project_root)
            .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                updated_at: projection.updated_at,
                work_items: Vec::new(),
            })
            .work_items
            .iter()
            .map(|item| workspace_work_item_view_from_item(item, &session_index, resume_branches))
            .collect::<Vec<_>>();
    Some(OutboundEvent::project(
        gwt_core::paths::resolve_project_scope(project_root).hash,
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

/// Issue #4433: rebuild the wire event a stored cleanup snapshot stands for.
fn branch_cleanup_snapshot_event(
    id: &str,
    operation_id: &str,
    snapshot: gwt::BranchCleanupOperationSnapshot,
) -> BackendEvent {
    match snapshot {
        gwt::BranchCleanupOperationSnapshot::Progress(progress) => {
            BackendEvent::BranchCleanupProgress {
                id: id.to_string(),
                operation_id: operation_id.to_string(),
                branch: progress.branch,
                execution_branch: progress.execution_branch,
                index: progress.index,
                total: progress.total,
                phase: progress.phase,
                message: progress.message,
            }
        }
        gwt::BranchCleanupOperationSnapshot::Result(results) => BackendEvent::BranchCleanupResult {
            id: id.to_string(),
            operation_id: operation_id.to_string(),
            results,
        },
    }
}

fn spawn_branch_cleanup_async(proxy: AppEventProxy, task: BranchCleanupAsyncTask) {
    let BranchCleanupAsyncTask {
        context,
        window_id,
        project_root,
        active_session_branches,
        branches,
        operation_id,
        operations,
        options,
    } = task;
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_context = context.clone();
                    let progress_window_id = window_id.clone();
                    let progress_operation_id = operation_id.clone();
                    let progress_operations = operations.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        &branches,
                        options,
                        move |progress| {
                            // Issue #4433: broadcast, so a client that
                            // reconnected mid-cleanup still receives progress.
                            progress_operations.record_progress(
                                &progress_window_id,
                                &progress_operation_id,
                                &progress,
                            );
                            progress_proxy.send(UserEvent::ProjectDispatch {
                                context: progress_context.clone(),
                                events: vec![OutboundEvent::project(
                                    progress_context.project_key.clone(),
                                    BackendEvent::BranchCleanupProgress {
                                        id: progress_window_id.clone(),
                                        operation_id: progress_operation_id.clone(),
                                        branch: progress.branch,
                                        execution_branch: progress.execution_branch,
                                        index: progress.index,
                                        total: progress.total,
                                        phase: progress.phase,
                                        message: progress.message,
                                    },
                                )],
                            });
                        },
                    );
                    operations.record_result(&window_id, &operation_id, &results);
                    let mut events = vec![OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: window_id.clone(),
                            operation_id: operation_id.clone(),
                            results,
                        },
                    )];
                    match list_branch_entries_with_active_sessions(
                        &project_root,
                        &active_session_branches,
                    ) {
                        Ok(entries) => {
                            events.push(OutboundEvent::project(
                                context.project_key.clone(),
                                BackendEvent::BranchEntries {
                                    id: window_id.clone(),
                                    phase: BranchEntriesPhase::Hydrated,
                                    entries,
                                    // SPEC-2009 FR-067: fresh load id from the shared
                                    // sequence so the post-cleanup reload is never
                                    // dropped as stale by the frontend.
                                    load_id: gwt::next_branch_load_id(),
                                },
                            ))
                        }
                        Err(error) => events.push(OutboundEvent::project(
                            context.project_key.clone(),
                            BackendEvent::BranchError {
                                id: window_id.clone(),
                                message: error.to_string(),
                            },
                        )),
                    }
                    events
                }
                Err(error) => vec![OutboundEvent::project(
                    context.project_key.clone(),
                    BackendEvent::BranchError {
                        id: window_id,
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::ProjectDispatch { context, events });
    });
}

impl AppRuntime {
    /// A cached rail refresh cannot prove that an unreadable works.json has
    /// recovered: file permissions are not part of its cache signature.
    pub(crate) fn recheck_workspace_state_after_projection(&self, project_root: &Path) {
        let Some(context) = self.project_context_for_root(project_root) else {
            return;
        };
        let pending = self.project_state(&context).is_some_and(|state| {
            state
                .workspace_state_notice
                .as_ref()
                .is_some_and(|notice| notice.kind == gwt::WorkspaceStateNoticeKind::LoadError)
        });
        if pending {
            crate::spawn_workspace_projection_reload(
                &self.blocking_tasks,
                self.proxy.clone(),
                context,
            );
        }
    }

    pub(crate) fn handle_workspace_state_load_failed(
        &mut self,
        project_root: &Path,
        error: gwt_core::WorkspaceStateLoadError,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        tracing::warn!(path = %error.path.display(), error = %error.message, "workspace state load failed");
        let notice = gwt::WorkspaceStateNoticeView {
            path: error.path.display().to_string(),
            message: error.message,
            kind: gwt::WorkspaceStateNoticeKind::LoadError,
        };
        self.project_state_mut(&context)
            .expect("current project")
            .workspace_state_notice = Some(notice.clone());
        vec![OutboundEvent::project(
            context.project_key,
            BackendEvent::WorkspaceStateNotice {
                notice: Some(notice),
            },
        )]
    }

    pub(crate) fn handle_workspace_state_loaded(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        let state = self.project_state_mut(&context).expect("current project");
        let recovered = state
            .workspace_state_notice
            .as_ref()
            .is_some_and(|notice| notice.kind == gwt::WorkspaceStateNoticeKind::LoadError);
        if !recovered {
            return Vec::new();
        }
        state.workspace_state_notice = None;
        self.spawn_work_events_ingest(project_root.to_path_buf(), true);
        let _ = self.active_work_projection_broadcast_for_tab(&context.tab_id);
        vec![OutboundEvent::project(
            context.project_key,
            BackendEvent::WorkspaceStateNotice { notice: None },
        )]
    }

    pub(crate) fn run_branch_cleanup_events(
        &self,
        client_id: &str,
        id: &str,
        branches: &[String],
        delete_remote: bool,
        force_filesystem_delete: bool,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: format!("Window preset {:?} is not a Work surface", window.preset),
                },
            )];
        }

        let Some(context) = self.project_context(&address.tab_id) else {
            return Vec::new();
        };
        let Some(state) = self.project_state(&context) else {
            return Vec::new();
        };
        spawn_branch_cleanup_async(
            self.proxy.clone(),
            BranchCleanupAsyncTask {
                context: context.clone(),
                window_id: id.to_string(),
                project_root: tab.project_root.clone(),
                active_session_branches: self.active_session_branches_for_tab(&address.tab_id),
                branches: branches.to_vec(),
                operation_id: operation_id.to_string(),
                operations: state.branch_cleanup_operations.clone(),
                options: BranchCleanupOptions {
                    delete_remote,
                    force_filesystem_delete,
                },
            },
        );
        Vec::new()
    }

    pub(crate) fn run_workspace_cleanup_events(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
        branch: &str,
        delete_remote: bool,
        force_filesystem_delete: bool,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let tab_id = &context.tab_id;
        let Some(tab) = self.tab(tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };

        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        spawn_workspace_cleanup_async(
            self.proxy.clone(),
            WorkspaceCleanupAsyncTask {
                context: context.clone(),
                project_root: tab.project_root.clone(),
                active_session_branches: self.active_session_branches_for_tab(tab_id),
                branch: branch.to_string(),
                operation_id: operation_id.to_string(),
                operations: state.branch_cleanup_operations.clone(),
                options: BranchCleanupOptions {
                    delete_remote,
                    force_filesystem_delete,
                },
            },
        );
        Vec::new()
    }

    /// Issue #4433 AC-2: replay the current state of an in-flight cleanup to a
    /// client that reconnected (and therefore has a new `client_id`) while the
    /// cleanup was still running. An unknown operation replies with nothing —
    /// silence must never be rendered as a cleanup failure.
    pub(crate) fn sync_branch_cleanup_events(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
        id: &str,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        let Some(snapshot) = state.branch_cleanup_operations.snapshot(id, operation_id) else {
            return Vec::new();
        };
        vec![OutboundEvent::reply(
            client_id,
            branch_cleanup_snapshot_event(id, operation_id, snapshot),
        )]
    }

    /// Issue #4433 AC-2: replay every live cleanup to a client during its
    /// initial sync. A reloaded page lost the operation ids it was watching, so
    /// it cannot ask for them by name.
    pub(crate) fn live_branch_cleanup_replies(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        state
            .branch_cleanup_operations
            .live_operations()
            .into_iter()
            .map(|(id, operation_id, snapshot)| {
                OutboundEvent::reply(
                    client_id,
                    branch_cleanup_snapshot_event(&id, &operation_id, snapshot),
                )
            })
            .collect()
    }

    /// Issue #4433 AC-3: the client consumed the operation's result, so drop
    /// the snapshot instead of replaying it into the next cleanup.
    pub(crate) fn clear_branch_cleanup_status_events(
        &self,
        context: &super::ProjectContext,
        id: &str,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        if let Some(state) = self.project_state(context) {
            state.branch_cleanup_operations.clear(id, operation_id);
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use gwt_core::test_support::{env_lock, ScopedEnvVar};

    #[test]
    fn cleanup_completion_does_not_recreate_projection_deleted_while_waiting_for_lock() {
        let _env_guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("create home");
        let _home = ScopedEnvVar::set("HOME", &home);
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create project root");

        let projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
        gwt_core::workspace_projection::save_workspace_projection(&project_root, &projection)
            .expect("save current projection");
        let legacy_path = gwt_core::paths::gwt_project_dir_for_repo_path(&project_root)
            .join("workspace/current.json");
        // Retired HOME residue predates the current writer, which rejects it.
        let legacy_bytes = serde_json::to_vec(&projection).expect("serialize legacy fixture");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).expect("create legacy directory");
        std::fs::write(&legacy_path, &legacy_bytes).expect("seed remaining legacy projection");
        let current_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(current_path.with_file_name("works.lock"))
            .expect("open project lock");
        lock.lock_exclusive().expect("lock project state");

        let root_for_cleanup = project_root.clone();
        let handle = std::thread::spawn(move || {
            clear_workspace_cleanup_git_details_event(&root_for_cleanup)
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            !handle.is_finished(),
            "cleanup completion must wait for the project lock"
        );

        std::fs::remove_file(&current_path).expect("remove current projection while locked");
        FileExt::unlock(&lock).expect("unlock project state");
        let event = handle.join().expect("join cleanup completion");

        assert_eq!(
            (event.is_none(), current_path.exists()),
            (true, false),
            "a deleted current projection must not be broadcast or recreated by cleanup completion"
        );
        assert_eq!(
            std::fs::read(&legacy_path).expect("read preserved legacy projection"),
            legacy_bytes,
            "cleanup must preserve retired HOME bytes when canonical state disappears"
        );
        let error = gwt_core::workspace_projection::load_workspace_projection(&project_root)
            .expect_err("remaining retired HOME state must be refused");
        assert!(
            matches!(
                error,
                gwt_core::error::GwtError::WorkspaceStateLoad(ref detail)
                    if detail.kind == gwt_core::error::WorkspaceStateLoadErrorKind::LegacyLayout
                        && detail.path == legacy_path
            ),
            "a normal load must refuse retired state without recreating canonical state: {error}"
        );
    }
}
