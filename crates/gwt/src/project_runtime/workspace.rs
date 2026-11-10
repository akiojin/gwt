//! Work launch publication and successor transitions owned by project runtimes.

use crate as gwt;
use crate::launch_runtime::intake_hook_config_is_disposable;
use crate::launch_runtime::same_worktree_path;
use crate::worktree_form::is_ephemeral_worktree_path;
use std::path::Path;

use super::launch_payload::{ActiveAgentSession, WorkspaceResumeContext};

/// SPEC-2359 US-80 (FR-427): register a Start-Work Shell as a first-class Work
/// in the persisted projection so it appears in the Active Work / Workspace
/// overview like an agent. Shell Works carry the window id as their identity
/// (no agent session) and reuse the agent canonical Work id derivation via the
/// shell's branch / worktree, so a shell groups into the same Work row as an
/// agent on the same branch. `retain_live_agents_keep_shells` drops only dead
/// agents (FR-429): launching the shell never prunes other live or paused
/// shells.
pub fn save_shell_work_projection(
    project_root: &Path,
    window_id: &str,
    worktree_path: Option<std::path::PathBuf>,
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

pub fn active_agent_summary_from_session(
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

pub fn non_empty_workspace_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn unassigned_agent_summary_from_session(
    session: &ActiveAgentSession,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
    let mut summary = active_agent_summary_from_session(session, updated_at);
    summary.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    summary.workspace_id = None;
    summary
}

#[derive(Debug, Clone, Copy)]
pub enum WorkspaceLaunchProjectionKind {
    StartWork,
    Resume { created_by_start_work: bool },
}

pub struct WorkspaceLaunchTransition<'a> {
    pub work_id: Option<String>,
    pub base_branch: Option<&'a str>,
    pub linked_issue_number: Option<u64>,
    /// Trusted execution owner (#3426). When present it overrides any
    /// presentation-derived `resume_context.owner` so the Work projection can
    /// never fork from the ECR/ledger owner kind.
    pub canonical_owner: Option<crate::cli::execution_state::ExecutionOwnerKey>,
    pub resume_context: Option<&'a WorkspaceResumeContext>,
    pub kind: WorkspaceLaunchProjectionKind,
    pub live_session_ids: &'a std::collections::HashSet<String>,
    pub now: chrono::DateTime<chrono::Utc>,
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
pub fn workspace_owner_label(owner: crate::cli::execution_state::ExecutionOwnerKey) -> String {
    match owner.kind {
        crate::cli::execution_state::ExecutionOwnerKind::Spec => {
            format!("SPEC-{}", owner.number)
        }
        crate::cli::execution_state::ExecutionOwnerKind::Issue => {
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
pub fn save_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    canonical_owner: Option<crate::cli::execution_state::ExecutionOwnerKey>,
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
pub fn apply_workspace_launch_for_current_work(
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
pub enum SuccessorSupersessionAuthority<'a> {
    ExecutionOwner(&'a str),
    RegisteredPm,
}

/// Build the same bounded repair for authenticated fresh activation and an
/// explicit registered-PM close. Never attach the new Session to old history.
pub fn invalid_successor_supersession_event(
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
    event.bind_portable_successor_supersession_identity()?;
    Ok(event)
}

/// Recheck every recorded member while the caller holds the execution owner
/// lease. Runtime proof is published before a child is released, including
/// children not yet visible in this Host's GUI registry.
pub fn validate_superseded_member_runtimes(
    sessions_dir: &Path,
    occupied: &gwt_core::workspace_projection::WorkItem,
) -> gwt_core::error::Result<()> {
    use crate::cli::execution_state::{
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
                            crate::session_inventory::observe_session(&session, sessions_dir);
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

pub fn apply_workspace_launch_transition(
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

pub fn save_start_work_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: &str,
    linked_issue_number: Option<u64>,
    canonical_owner: Option<crate::cli::execution_state::ExecutionOwnerKey>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    live_session_ids: Option<&std::collections::HashSet<String>>,
) -> Result<(), String> {
    if workspace_resume_context.is_none() && linked_issue_number.is_none() {
        let now = chrono::Utc::now();
        return gwt_core::workspace_projection::mutate_workspace_projection(
            project_root,
            |projection| {
                projection
                    .register_unassigned_agent(unassigned_agent_summary_from_session(session, now));
                projection.updated_at = now;
                Ok(())
            },
        )
        .map_err(|error| error.to_string());
    }
    save_workspace_launch_projection(
        project_root,
        session,
        Some(base_branch),
        linked_issue_number,
        canonical_owner,
        workspace_resume_context,
        WorkspaceLaunchProjectionKind::StartWork,
        live_session_ids,
    )
}

pub fn save_resumed_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    canonical_owner: Option<crate::cli::execution_state::ExecutionOwnerKey>,
    workspace_resume_context: &WorkspaceResumeContext,
    live_session_ids: Option<&std::collections::HashSet<String>>,
) -> Result<(), String> {
    save_workspace_launch_projection(
        project_root,
        session,
        base_branch,
        linked_issue_number,
        canonical_owner,
        Some(workspace_resume_context),
        WorkspaceLaunchProjectionKind::Resume {
            created_by_start_work: session.branch_name.starts_with("work/"),
        },
        live_session_ids,
    )
}

pub fn session_uses_ephemeral_worktree_for_project(
    project_root: Option<&Path>,
    session: &ActiveAgentSession,
) -> bool {
    if !is_ephemeral_worktree_path(&session.worktree_path) {
        return false;
    }
    let Some(main_repo_path) =
        project_root.and_then(|root| gwt_git::worktree::main_worktree_root(root).ok())
    else {
        return !session.worktree_path.exists();
    };
    match gwt_git::WorktreeManager::new(&main_repo_path).list() {
        Ok(worktrees) => worktrees
            .iter()
            .find(|info| same_worktree_path(&info.path, &session.worktree_path))
            // On a branch → a real worktree. Detached → ephemeral.
            .is_none_or(|info| info.branch.is_none()),
        // Cannot enumerate: fall back to "gone means it was ephemeral".
        Err(_) => !session.worktree_path.exists(),
    }
}

pub fn finalize_ephemeral_worktree_for_project(
    project_root: Option<&Path>,
    session: &ActiveAgentSession,
) {
    let worktree_path = session.worktree_path.as_path();
    let main_repo_path = project_root
        .and_then(|root| gwt_git::worktree::main_worktree_root(root).ok())
        .unwrap_or_else(|| worktree_path.to_path_buf());
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);

    match manager.ephemeral_worktree_has_local_work_with(worktree_path, |entry| {
        intake_hook_config_is_disposable(worktree_path, entry)
    }) {
        Ok(true) => {
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                "ephemeral worktree has local work (changes, ignored files, or commits); keeping it so nothing is lost"
            );
            return;
        }
        Ok(false) => {}
        Err(error) => {
            // Fail closed: if we cannot prove the worktree is empty, keep it.
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                error = %error,
                "could not determine ephemeral worktree cleanliness; keeping it"
            );
            return;
        }
    }

    if let Err(error) =
        gwt::managed_assets::cleanup_worktree_with_codex_project_trust(worktree_path, || {
            manager
                .remove_force(worktree_path)
                .map_err(|error| std::io::Error::other(error.to_string()))
        })
    {
        tracing::warn!(
            worktree_path = %worktree_path.display(),
            error = %error,
            "failed to revoke Codex project trust and remove clean ephemeral worktree"
        );
    }
}

pub fn paused_work_record_task(
    project_root: &Path,
    session: &ActiveAgentSession,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> Option<Box<dyn FnOnce() + Send + 'static>> {
    let session_id = session.session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let work_items_root = project_root.to_path_buf();
    let session_id = session_id.to_string();
    let log_session_id = session.session_id.clone();
    let fallback_branch = session.branch_name.clone();
    let fallback_worktree_path = session.worktree_path.clone();
    Some(Box::new(move || {
        let projection =
            gwt_core::workspace_projection::load_workspace_projection(&work_items_root)
                .ok()
                .flatten();
        let agent_summary = projection
            .as_ref()
            .and_then(|projection| projection.latest_agent_for_session(&session_id));
        let assigned_work_id = agent_summary
            .filter(|agent| agent.is_assigned())
            .and_then(|agent| agent.workspace_id.clone());
        let fallback_work_id = format!("work-session-{session_id}");
        // #3065: owner / summary / the title fallback must come from the
        // session's own Work item, never from the repo-shared projection.
        let agent_title = agent_summary
            .and_then(|agent| {
                agent
                    .title_summary
                    .clone()
                    .or_else(|| agent.current_focus.clone())
            })
            .filter(|value| !value.trim().is_empty());
        let board_refs = projection
            .as_ref()
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default();
        let branch = agent_summary
            .and_then(|agent| agent.branch.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone())
            })
            .or(Some(fallback_branch))
            .filter(|value| !value.trim().is_empty());
        let worktree_path = agent_summary
            .and_then(|agent| agent.worktree_path.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.worktree_path.clone())
            })
            .or(Some(fallback_worktree_path));
        let git_details = projection
            .as_ref()
            .and_then(|projection| projection.git_details.clone());
        let execution_container = (branch.is_some() || worktree_path.is_some()).then(|| {
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch,
                worktree_path,
                pr_number: git_details.as_ref().and_then(|details| details.pr_number),
                pr_url: git_details
                    .as_ref()
                    .and_then(|details| details.pr_url.clone()),
                pr_state: git_details
                    .as_ref()
                    .and_then(|details| details.pr_state.clone()),
            }
        });
        let lookup_branch = execution_container
            .as_ref()
            .and_then(|container| container.branch.clone());
        let lookup_worktree = execution_container
            .as_ref()
            .and_then(|container| container.worktree_path.clone());
        // #3065: resolve identity from the session's own Work item. The
        // works.json IO already happens on this background thread for the
        // record itself, so the lookup adds no UI-loop cost.
        let works = gwt_core::workspace_projection::load_workspace_work_items(&work_items_root)
            .ok()
            .flatten();
        let work_id = assigned_work_id
            .filter(|work_id| {
                works
                    .as_ref()
                    .is_some_and(|works| works.work_items.iter().any(|item| item.id == *work_id))
            })
            .unwrap_or(fallback_work_id);
        let own_item = works.as_ref().and_then(|works| {
            gwt_core::workspace_projection::find_work_item_for_container(
                works,
                &work_items_root,
                lookup_branch.as_deref(),
                lookup_worktree.as_deref(),
            )
            .map(|item| {
                (
                    item.title.clone(),
                    item.summary.clone().or_else(|| item.intent.clone()),
                    item.owner.clone(),
                )
            })
        });
        let (item_title, summary, owner) = own_item.unwrap_or((String::new(), None, None));
        let title =
            agent_title.or_else(|| Some(item_title).filter(|value| !value.trim().is_empty()));
        if let Err(error) = gwt_core::workspace_projection::record_workspace_work_paused_event(
            &work_items_root,
            &work_id,
            title.as_deref(),
            summary.as_deref(),
            owner.as_deref(),
            &board_refs,
            execution_container,
            Some(&session_id),
            updated_at,
        ) {
            tracing::warn!(
                error = %error,
                work_items_root = %work_items_root.display(),
                session_id = %log_session_id,
                work_id = %work_id,
                "failed to persist Paused Work for stopped Agent session"
            );
        }
    }))
}
