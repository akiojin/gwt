//! Active Work / Workspace projection view builders split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - The wire-format mappers for workspace status / lifecycle / event kinds
//!   (`workspace_status_category_wire`, `workspace_work_event_kind_wire`, ...)
//! - The Active Work projection view pipeline
//!   (`active_work_projection_from_saved_with_journal`,
//!   `active_work_items_from_projection`,
//!   `attach_registry_sessions_to_active_works`,
//!   `assign_and_merge_workspace_groups`, merge/remote-only marking, ...)
//! - Workspace resume-context derivation
//!   (`workspace_resume_context_from_projection` /
//!   `workspace_resume_context_from_journal` and the branch-existence checks
//!   consumed by `wizard.rs` through `super`)
//! - Launch-side projection persistence helpers
//!   (`save_start_work_workspace_projection`,
//!   `save_resumed_workspace_projection`)
//! - [`AppRuntime::active_work_projection_for_tab`] and the projection
//!   reply / broadcast / prune handlers
//!
//! Behavior-preserving move: `INFLIGHT_LAUNCH_TTL` / `inflight_launch_key`
//! are launch-side and stay in `mod.rs` (Pass 2 moves them to `launch.rs`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use super::{
    active_agent_summary_from_session, current_git_branch, local_branch_exists,
    merge_active_sessions_into_projection, normalize_branch_name, origin_remote_ref,
    retain_live_workspace_agents, save_workspace_launch_projection,
    workspace_cleanup_candidate_for_projection, workspace_projection_owner_title,
    ActiveAgentSession, AppRuntime, BackendEvent, ClientId, IssueBranchLinkStore, OutboundEvent,
    ProjectTabRuntime, WorkspaceLaunchProjectionKind, WorkspaceResumeContext,
};

fn projection_worktree_path_key(path: &Path) -> PathBuf {
    let mut key = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if key.file_name().is_some() {
                    key.pop();
                } else if !path.is_absolute() {
                    key.push(component.as_os_str());
                }
            }
            _ => key.push(component.as_os_str()),
        }
    }
    key
}

fn projection_worktree_paths_match(left: &Path, right: &Path) -> bool {
    left == right || projection_worktree_path_key(left) == projection_worktree_path_key(right)
}

fn workspace_status_category_wire(
    category: gwt_core::workspace_projection::WorkspaceStatusCategory,
) -> &'static str {
    use gwt_core::workspace_projection::WorkspaceStatusCategory;

    match category {
        WorkspaceStatusCategory::Active => "active",
        WorkspaceStatusCategory::Idle => "idle",
        WorkspaceStatusCategory::Blocked => "blocked",
        WorkspaceStatusCategory::Done => "done",
        WorkspaceStatusCategory::Unknown => "unknown",
    }
}

/// SPEC-2359 Phase W-12 (FR-349): map the agent-session Work lifecycle state
/// to its snake_case wire string for [`gwt::ActiveWorkItemView::lifecycle_state`].
fn work_active_lifecycle_state_wire(
    state: gwt_core::workspace_projection::WorkActiveLifecycleState,
) -> &'static str {
    use gwt_core::workspace_projection::WorkActiveLifecycleState;

    match state {
        WorkActiveLifecycleState::Active => "active",
        WorkActiveLifecycleState::Paused => "paused",
        WorkActiveLifecycleState::Done => "done",
        WorkActiveLifecycleState::Discarded => "discarded",
    }
}

pub(super) const WORKSPACE_OVERVIEW_JOURNAL_LIMIT: usize = 8;
pub(super) const WORKSPACE_CLEANUP_EVENT_ID: &str = "__workspace_cleanup__";

#[cfg(test)]
pub(super) fn active_work_projection_from_saved(
    projection: gwt_core::workspace_projection::WorkspaceProjection,
) -> gwt::ActiveWorkProjectionView {
    let cleanup_candidate = projection
        .cleanup_candidate(false)
        .map(active_work_cleanup_candidate_view_from_candidate);
    active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        Vec::new(),
        cleanup_candidate,
    )
}

pub(super) fn active_work_projection_from_saved_with_journal(
    projection: gwt_core::workspace_projection::WorkspaceProjection,
    journal_entries: Vec<gwt::WorkspaceJournalEntryView>,
    works: Vec<gwt::WorkspaceHistoryView>,
    cleanup_candidate: Option<gwt::ActiveWorkCleanupCandidateView>,
) -> gwt::ActiveWorkProjectionView {
    let project_root = projection.project_root.clone();
    let mut agents = projection
        .agents
        .iter()
        .filter(|agent| {
            agent.is_assigned() || workspace_agent_summary_work_id(&project_root, agent).is_some()
        })
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        active_work_agent_priority_rank(left)
            .cmp(&active_work_agent_priority_rank(right))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let active_agents = agents
        .iter()
        .filter(|agent| agent.status_category == "active")
        .count();
    let blocked_agents = agents
        .iter()
        .filter(|agent| agent.status_category == "blocked")
        .count();
    let agent_branch = agents.iter().find_map(|agent| agent.branch.clone());
    let agent_worktree = agents.iter().find_map(|agent| agent.worktree_path.clone());
    let status_category =
        workspace_status_category_wire(projection.effective_status_category()).to_string();
    let (branch, worktree_path, pr_number, pr_url, pr_state, pr_created_at) =
        match projection.git_details.as_ref() {
            Some(details) => (
                details.branch.clone().or(agent_branch),
                details
                    .worktree_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .or(agent_worktree),
                details.pr_number,
                details.pr_url.clone(),
                details.pr_state.clone(),
                details
                    .pr_created_at
                    .map(|created_at| created_at.to_rfc3339()),
            ),
            None => (agent_branch, agent_worktree, None, None, None, None),
        };
    let mut unassigned_agents = projection
        .agents
        .iter()
        .filter(|agent| {
            agent.is_unassigned() && workspace_agent_summary_work_id(&project_root, agent).is_none()
        })
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    unassigned_agents.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    // SPEC-3075: surface the agent-declared `title-summary` purpose recorded in
    // the journal so historical Works whose WorkItem title is only the branch
    // still get a human-readable rail summary.
    let journal_title_by_session = journal_title_summary_by_session(&journal_entries);
    let active_works =
        active_work_items_from_projection(&projection, &agents, &works, &journal_title_by_session);
    let active_work_count = active_works.len();

    gwt::ActiveWorkProjectionView {
        id: projection.id,
        title: projection.title,
        status_category,
        status_text: projection.status_text,
        summary: projection.summary,
        progress_summary: projection.progress_summary,
        owner: projection.owner,
        next_action: projection.next_action,
        active_agents,
        blocked_agents,
        branch,
        worktree_path,
        pr_number,
        pr_url,
        pr_state,
        pr_created_at,
        board_refs: projection.board_refs,
        journal_entries,
        works,
        cleanup_candidate,
        managed_hook_health: None,
        active_work_count,
        active_works,
        agents,
        unassigned_agents,
    }
}

fn empty_active_work_projection_view(
    tab_id: &str,
    tab: &ProjectTabRuntime,
) -> gwt::ActiveWorkProjectionView {
    gwt::ActiveWorkProjectionView {
        id: tab_id.to_string(),
        title: format!("{} Work", tab.title),
        status_category: "idle".to_string(),
        status_text: String::new(),
        summary: None,
        progress_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: None,
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        board_refs: Vec::new(),
        journal_entries: Vec::new(),
        works: Vec::new(),
        cleanup_candidate: None,
        managed_hook_health: None,
        active_work_count: 0,
        active_works: Vec::new(),
        agents: Vec::new(),
        unassigned_agents: Vec::new(),
    }
}

fn active_work_projection_from_live_sessions(
    tab_id: &str,
    tab: &ProjectTabRuntime,
    sessions: &[&ActiveAgentSession],
    managed_hook_health: Option<gwt::ManagedHookHealthView>,
) -> Option<gwt::ActiveWorkProjectionView> {
    let first = sessions.first()?;
    let active_agents = sessions.len();
    let now = chrono::Utc::now();
    let mut agents = sessions
        .iter()
        .map(|session| {
            let summary = active_agent_summary_from_session(session, now);
            active_work_agent_view_from_summary(&summary)
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let active_works = vec![gwt::ActiveWorkItemView {
        id: tab_id.to_string(),
        title: format!("{} Work", tab.title),
        status_category: "active".to_string(),
        status_text: if active_agents == 1 {
            "1 active agent".to_string()
        } else {
            format!("{active_agents} active agents")
        },
        summary: None,
        progress_summary: None,
        work_summary: None,
        owner: None,
        next_action: Some("Check Board for latest updates".to_string()),
        active_agents,
        blocked_agents: 0,
        branch: Some(first.branch_name.clone()),
        worktree_path: Some(first.worktree_path.display().to_string()),
        managed_hook_health: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        board_refs: Vec::new(),
        agents: agents.clone(),
        works: Vec::new(),
        lifecycle_state: work_active_lifecycle_state_wire(
            gwt_core::workspace_projection::recompute_work_active_lifecycle(
                gwt_core::workspace_projection::WorkAgentRuntime::Running,
                None,
            ),
        )
        .to_string(),
        closed_at: None,
        session_agent_total: 0,
        merged_into_base: false,
        workspace_key: None,
        remote_only: false,
        done_equivalent: false,
        cleanup_candidate: None,
        cleanup_blocked_reason: None,
        updated_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }];
    Some(gwt::ActiveWorkProjectionView {
        id: tab_id.to_string(),
        title: format!("{} workspace", tab.title),
        status_category: "active".to_string(),
        status_text: if active_agents == 1 {
            "1 active agent".to_string()
        } else {
            format!("{active_agents} active agents")
        },
        summary: None,
        progress_summary: None,
        owner: None,
        next_action: Some("Check Board for latest updates".to_string()),
        active_agents,
        blocked_agents: 0,
        branch: Some(first.branch_name.clone()),
        worktree_path: Some(first.worktree_path.display().to_string()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        board_refs: Vec::new(),
        journal_entries: Vec::new(),
        works: Vec::new(),
        cleanup_candidate: None,
        managed_hook_health,
        active_work_count: active_works.len(),
        active_works,
        agents,
        unassigned_agents: Vec::new(),
    })
}

fn managed_hook_health_view_for_project(
    project_root: &Path,
    sessions_dir: &Path,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ManagedHookHealthView> {
    managed_hook_health_view_for_worktree(project_root, sessions_dir, sessions)
}

pub(super) fn managed_hook_health_view_for_worktree(
    worktree: &Path,
    sessions_dir: &Path,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ManagedHookHealthView> {
    let mut input = gwt::cli::hook::health::ManagedHookHealthInput::new(worktree);
    input.runtime_state_path = None;
    let selected_runtime_state = sessions
        .iter()
        .map(|session| {
            let path = gwt_agent::runtime_state_path(sessions_dir, &session.session_id);
            let updated_at = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| {
                    value
                        .get("updated_at")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                });
            (updated_at, session.session_id.as_str(), path)
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)))
        .map(|(_, _, path)| path);
    if let Some(runtime_state_path) = selected_runtime_state {
        input = input.with_runtime_state_path(runtime_state_path);
    }
    let health = gwt::cli::hook::health::read_managed_hook_health(&input);
    let should_show = health.status != gwt::cli::hook::health::ManagedHookHealthStatus::Inactive
        || health.pending_discussion.is_some()
        || health.pending_goal.is_some()
        || !health.slow_handlers.is_empty()
        || !health.issues.is_empty();
    should_show.then(|| managed_hook_health_view_from_health(health))
}

fn attach_managed_hook_health_to_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    sessions_dir: &Path,
    sessions: &[&ActiveAgentSession],
) {
    for work in active_works {
        let Some(worktree) = work.worktree_path.as_deref().map(Path::new) else {
            continue;
        };
        let matching_sessions = sessions
            .iter()
            .copied()
            .filter(|session| projection_worktree_paths_match(&session.worktree_path, worktree))
            .collect::<Vec<_>>();
        work.managed_hook_health =
            managed_hook_health_view_for_worktree(worktree, sessions_dir, &matching_sessions);
    }
}

fn managed_hook_health_status_wire(
    status: gwt::cli::hook::health::ManagedHookHealthStatus,
) -> &'static str {
    match status {
        gwt::cli::hook::health::ManagedHookHealthStatus::Ready => "ready",
        gwt::cli::hook::health::ManagedHookHealthStatus::NeedsAttention => "needs_attention",
        gwt::cli::hook::health::ManagedHookHealthStatus::SelfHealed => "self_healed",
        gwt::cli::hook::health::ManagedHookHealthStatus::Degraded => "degraded",
        gwt::cli::hook::health::ManagedHookHealthStatus::Inactive => "inactive",
        gwt::cli::hook::health::ManagedHookHealthStatus::WaitingForFirstHookEvent => {
            "waiting_for_first_hook_event"
        }
    }
}

fn managed_hook_health_view_from_health(
    health: gwt::cli::hook::health::ManagedHookHealth,
) -> gwt::ManagedHookHealthView {
    gwt::ManagedHookHealthView {
        status: managed_hook_health_status_wire(health.status).to_string(),
        last_event: health.last_event,
        last_event_at: health.last_event_at,
        pending_discussion: health.pending_discussion.map(|pending| {
            gwt::ManagedHookPendingDiscussionView {
                proposal_label: pending.proposal_label,
                proposal_title: pending.proposal_title,
                next_question: pending.next_question,
            }
        }),
        pending_goal: health
            .pending_goal
            .map(|goal| gwt::ManagedHookPendingGoalView {
                proposal_label: goal.proposal_label,
                proposal_title: goal.proposal_title,
                condition: goal.condition,
            }),
        slow_handlers: health
            .slow_handlers
            .into_iter()
            .map(|handler| gwt::ManagedHookSlowHandlerView {
                event: handler.event,
                handler: handler.handler,
                status: handler.status,
                duration_ms: handler.duration_ms.max(0.0).round() as u64,
                occurred_at: handler.occurred_at,
            })
            .collect(),
        issues: health.issues,
    }
}

fn workspace_agent_summary_work_id(
    project_root: &Path,
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> Option<String> {
    gwt_core::workspace_projection::canonical_work_id(
        project_root,
        agent.branch.as_deref(),
        agent.worktree_path.as_deref(),
    )
}

/// Resolve the Work identity for an active agent. A persisted assignment wins
/// only when the repo-global WorkItems SOT contains that Work; this keeps a
/// launch-published canonical id stable across Active → Paused. Older
/// projection-only agents retain the session-derived identity fallback.
fn active_work_agent_work_id(
    project_root: &Path,
    agent: &gwt::ActiveWorkAgentView,
    works: &[gwt::WorkspaceHistoryView],
    legacy_fallback: Option<&str>,
) -> Option<String> {
    if let Some(workspace_id) = agent
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|workspace_id| works.iter().any(|work| work.id == *workspace_id))
    {
        return Some(workspace_id.to_string());
    }
    let session_id = agent.session_id.trim();
    if !session_id.is_empty() {
        return Some(format!("work-session-{session_id}"));
    }
    let worktree_path = agent.worktree_path.as_deref().map(Path::new);
    gwt_core::workspace_projection::canonical_work_id(
        project_root,
        agent.branch.as_deref(),
        worktree_path,
    )
    .or_else(|| {
        agent
            .workspace_id
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
    .or_else(|| legacy_fallback.map(str::to_string))
}

fn projection_matches_active_work(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    work_id: &str,
) -> bool {
    projection
        .git_details
        .as_ref()
        .and_then(|details| {
            gwt_core::workspace_projection::canonical_work_id(
                &projection.project_root,
                details.branch.as_deref(),
                details.worktree_path.as_deref(),
            )
        })
        .as_deref()
        == Some(work_id)
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): with `agent_session_id` as the
/// primary Work identity, a session-derived `work_id` no longer matches the
/// branch-derived id computed from the projection's `git_details`. The current
/// projection's Work row is now identified by checking whether the group's
/// representative agent shares the projection's branch or worktree, so the
/// title / status_text / summary / PR selection driven by `is_current_projection`
/// keeps choosing the live projection values.
fn agent_matches_projection_git_details(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    agent: &gwt::ActiveWorkAgentView,
) -> bool {
    let Some(details) = projection.git_details.as_ref() else {
        return false;
    };
    let branch_matches = details
        .branch
        .as_deref()
        .map(normalize_branch_name)
        .zip(agent.branch.as_deref().map(normalize_branch_name))
        .is_some_and(|(left, right)| left == right);
    let branch_conflicts = details.branch.is_some() && agent.branch.is_some() && !branch_matches;
    let worktree_matches = details
        .worktree_path
        .as_deref()
        .zip(agent.worktree_path.as_deref())
        .is_some_and(|(left, right)| projection_worktree_paths_match(left, Path::new(right)));
    let worktree_conflicts =
        details.worktree_path.is_some() && agent.worktree_path.is_some() && !worktree_matches;
    (branch_matches || worktree_matches) && !branch_conflicts && !worktree_conflicts
}

fn history_git_identity_conflicts(
    branch: Option<&str>,
    worktree: Option<&str>,
    work: &gwt::WorkspaceHistoryView,
) -> bool {
    #[cfg(test)]
    HISTORY_GIT_IDENTITY_CONFLICT_CHECKS.with(|count| count.set(count.get() + 1));
    let branch = branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_branch_name);
    let worktree = worktree.map(str::trim).filter(|value| !value.is_empty());
    let container_branches = work
        .execution_containers
        .iter()
        .filter_map(|container| container.branch.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_branch_name)
        .collect::<Vec<_>>();
    let container_worktrees = work
        .execution_containers
        .iter()
        .filter_map(|container| container.worktree_path.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let branch_matches = branch
        .as_deref()
        .is_some_and(|left| container_branches.iter().any(|right| left == right));
    let worktree_matches = worktree.is_some_and(|left| {
        container_worktrees
            .iter()
            .any(|right| projection_worktree_paths_match(Path::new(left), Path::new(right)))
    });
    let branch_conflicts = branch.is_some() && !container_branches.is_empty() && !branch_matches;
    let worktree_conflicts =
        worktree.is_some() && !container_worktrees.is_empty() && !worktree_matches;
    branch_conflicts || worktree_conflicts
}

#[cfg(test)]
thread_local! {
    static HISTORY_GIT_IDENTITY_CONFLICT_CHECKS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_history_git_identity_conflict_checks() {
    HISTORY_GIT_IDENTITY_CONFLICT_CHECKS.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn history_git_identity_conflict_checks() -> usize {
    HISTORY_GIT_IDENTITY_CONFLICT_CHECKS.with(std::cell::Cell::get)
}

fn find_active_work_history<'a>(
    work_id: &str,
    session_id: Option<&str>,
    branch: Option<&str>,
    worktree: Option<&str>,
    works: &'a [gwt::WorkspaceHistoryView],
) -> Option<&'a gwt::WorkspaceHistoryView> {
    works.iter().find(|item| item.id == work_id).or_else(|| {
        let session_id = session_id?.trim();
        works.iter().find(|item| {
            if session_id.is_empty() || history_git_identity_conflicts(branch, worktree, item) {
                return false;
            }
            item.agents
                .iter()
                .any(|history_agent| history_agent.session_id == session_id)
        })
    })
}

/// SPEC-3075: title shapes that are identifiers, not a declared work purpose.
/// Resume events leak the agent's `gwt-*` skill name into the recorded title,
/// and backfill paths leave the work-item id or a bare UUID — none answer "what
/// work was running", so the rail summary derivation skips them.
pub(super) fn is_identifier_like_title(text: &str) -> bool {
    is_gwt_skill_name(text) || is_work_item_id(text) || is_uuid_like(text)
}

fn is_gwt_skill_name(text: &str) -> bool {
    // ^gwt-[a-z0-9-]+$
    match text.strip_prefix("gwt-") {
        Some(rest) => {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        }
        None => false,
    }
}

fn is_work_item_id(text: &str) -> bool {
    // ^work-[a-z0-9-]+-[0-9a-f]{6,}$ — "work-" prefix, then a lowercase/digit/'-'
    // body, then a final '-'-separated segment of 6+ hex chars (the id suffix).
    let Some(rest) = text.strip_prefix("work-") else {
        return false;
    };
    let Some((body, tail)) = rest.rsplit_once('-') else {
        return false;
    };
    let body_ok = !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let tail_ok = tail.len() >= 6 && tail.chars().all(|c| c.is_ascii_hexdigit());
    body_ok && tail_ok
}

fn is_uuid_like(text: &str) -> bool {
    let segments: [usize; 5] = [8, 4, 4, 4, 12];
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == segments.len()
        && parts
            .iter()
            .zip(segments.iter())
            .all(|(part, len)| part.len() == *len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// SPEC-3075: a purpose candidate is non-empty text that is neither the branch
/// name nor an identifier shape (skill name / work id / UUID).
fn purpose_candidate(value: Option<&str>, branch: Option<&str>) -> Option<String> {
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    let text = value.map(str::trim).filter(|value| !value.is_empty())?;
    if Some(text) == branch || is_identifier_like_title(text) {
        return None;
    }
    Some(text.to_string())
}

/// SPEC-3075: the agent-declared-purpose tier of the "what work was running"
/// Workspace summary. Surfaces the `title-summary` the LLM sets — live agent
/// focus, then the recorded journal purpose, then a non-identifier recorded
/// title. Returns `None` when no declared purpose is known; the caller then
/// layers the PR title / branch tip commit subject on top (see
/// [`apply_work_summary_external_sources`]) before falling back to the branch.
/// The owner is shown as a separate meta chip, not folded into this summary.
/// Display-only; never mutates Work identity.
pub(super) fn derive_work_summary(
    agent_title_summary: Option<&str>,
    journal_title_summary: Option<&str>,
    recorded_title: Option<&str>,
    branch: Option<&str>,
) -> Option<String> {
    purpose_candidate(agent_title_summary, branch)
        .or_else(|| purpose_candidate(journal_title_summary, branch))
        .or_else(|| purpose_candidate(recorded_title, branch))
}

/// SPEC-3075: most-recent agent-declared `title-summary` per agent session, read
/// from the journal so a historical Work that recorded a purpose still surfaces
/// it on the rail even when its WorkItem title is only the branch.
fn journal_title_summary_by_session(
    journal_entries: &[gwt::WorkspaceJournalEntryView],
) -> std::collections::HashMap<String, String> {
    let mut map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    for entry in journal_entries {
        let (Some(session), Some(summary)) = (
            entry.agent_session_id.as_ref(),
            entry.agent_title_summary.as_ref(),
        ) else {
            continue;
        };
        if summary.trim().is_empty() {
            continue;
        }
        match map.get(session) {
            Some((seen_at, _)) if seen_at.as_str() >= entry.updated_at.as_str() => {}
            _ => {
                map.insert(session.clone(), (entry.updated_at.clone(), summary.clone()));
            }
        }
    }
    map.into_iter()
        .map(|(session, (_, summary))| (session, summary))
        .collect()
}

/// SPEC-3075: fill the rail row's `work_summary` "what work was running" label
/// from the external (background-scanned) sources, in priority order.
///
/// First the PR title — the human-written purpose — which OVERRIDES the
/// agent-declared `title-summary` already in `work_summary` (the user's chosen
/// precedence). Then, for rows still missing a summary, the AI-polished summary
/// (FR-006, present only when AI is enabled — it cleans merge/release commit
/// noise), and finally the raw branch tip commit subject — the historical
/// fallback for the ~96% of Workspaces that predate `title-summary`. All maps
/// come from background scan caches, mirroring [`mark_merged_active_works`]; no
/// git, network, or AI call runs on this view-build path.
pub(super) fn apply_work_summary_external_sources(
    active_works: &mut [gwt::ActiveWorkItemView],
    pr_titles: Option<&std::collections::HashMap<String, String>>,
    ai_summaries: Option<&std::collections::HashMap<String, String>>,
    tip_subjects: Option<&std::collections::HashMap<String, String>>,
) {
    let lookup = |map: Option<&std::collections::HashMap<String, String>>, branch: &str| {
        map.and_then(|map| {
            map.get(branch)
                .or_else(|| map.get(&format!("origin/{branch}")))
        })
        .and_then(|value| purpose_candidate(Some(value.as_str()), Some(branch)))
        .filter(|value| !super::is_summary_noise(value))
    };
    for work in active_works.iter_mut() {
        let Some(branch) = work
            .branch
            .as_deref()
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
        else {
            continue;
        };
        // 1. PR title — top priority, overrides any declared title-summary.
        if let Some(title) = lookup(pr_titles, &branch) {
            work.work_summary = Some(title);
            continue;
        }
        // 2/3. AI summary then raw commit subject — only fill a row with no
        // declared purpose. The AI-polished summary cleans the noise the raw
        // commit subject would otherwise show, so it wins over it.
        if work.work_summary.is_some() {
            continue;
        }
        if let Some(summary) =
            lookup(ai_summaries, &branch).or_else(|| lookup(tip_subjects, &branch))
        {
            work.work_summary = Some(summary);
        }
    }
}

fn active_work_items_from_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    agents: &[gwt::ActiveWorkAgentView],
    works: &[gwt::WorkspaceHistoryView],
    journal_title_by_session: &std::collections::HashMap<String, String>,
) -> Vec<gwt::ActiveWorkItemView> {
    let mut grouped: Vec<(String, Vec<gwt::ActiveWorkAgentView>)> = Vec::new();
    for agent in agents {
        let work_id =
            active_work_agent_work_id(&projection.project_root, agent, works, Some(&projection.id))
                .unwrap_or_else(|| projection.id.clone());
        if let Some((_, group_agents)) = grouped.iter_mut().find(|(id, _)| id == &work_id) {
            group_agents.push(agent.clone());
        } else {
            grouped.push((work_id, vec![agent.clone()]));
        }
    }

    let mut active_works = grouped
        .into_iter()
        .map(|(work_id, agents)| {
            let first_agent = agents.first();
            let is_current_projection = work_id == projection.id
                || projection_matches_active_work(projection, &work_id)
                || first_agent
                    .is_some_and(|agent| agent_matches_projection_git_details(projection, agent));
            // Effective live identity is the Agent value plus current
            // projection fallback for a missing dimension. Use the same
            // identity for history selection and final row rendering.
            let live_branch_value =
                first_agent
                    .and_then(|agent| agent.branch.clone())
                    .or_else(|| {
                        if is_current_projection {
                            projection
                                .git_details
                                .as_ref()
                                .and_then(|details| details.branch.clone())
                        } else {
                            None
                        }
                    });
            let live_worktree_value = first_agent
                .and_then(|agent| agent.worktree_path.clone())
                .or_else(|| {
                    if is_current_projection {
                        projection.git_details.as_ref().and_then(|details| {
                            details
                                .worktree_path
                                .as_ref()
                                .map(|path| path.display().to_string())
                        })
                    } else {
                        None
                    }
                });
            let history = find_active_work_history(
                &work_id,
                first_agent.map(|agent| agent.session_id.as_str()),
                live_branch_value.as_deref(),
                live_worktree_value.as_deref(),
                works,
            );
            let container = history.and_then(|item| item.execution_containers.first());
            let active_agents = agents
                .iter()
                .filter(|agent| agent.status_category == "active")
                .count();
            let blocked_agents = agents
                .iter()
                .filter(|agent| agent.status_category == "blocked")
                .count();
            // FR-403: live rows sort by their freshest agent activity.
            let row_updated_at = agents
                .iter()
                .map(|agent| agent.updated_at.clone())
                .max()
                .unwrap_or_default();
            let status_category = if blocked_agents > 0 {
                "blocked".to_string()
            } else if active_agents > 0 {
                "active".to_string()
            } else if let Some(history) = history {
                history.status_category.clone()
            } else {
                workspace_status_category_wire(projection.effective_status_category()).to_string()
            };
            let status_text = if is_current_projection {
                projection.status_text.clone()
            } else {
                history
                    .and_then(|item| item.summary.clone().or_else(|| item.intent.clone()))
                    .unwrap_or_else(|| {
                        if blocked_agents > 0 {
                            format!("{blocked_agents} blocked agents")
                        } else if active_agents == 1 {
                            "1 active agent".to_string()
                        } else {
                            format!("{} active agents", agents.len())
                        }
                    })
            };
            let owner_value = history.and_then(|item| item.owner.clone()).or_else(|| {
                is_current_projection
                    .then(|| projection.owner.clone())
                    .flatten()
            });
            // Keep the live Agent's git identity authoritative on every live
            // row. Projection/history metadata may fill a missing dimension,
            // but cannot erase a conflict before shared-Session deduplication.
            let branch_value =
                live_branch_value.or_else(|| container.and_then(|value| value.branch.clone()));
            let worktree_value = live_worktree_value
                .or_else(|| container.and_then(|value| value.worktree_path.clone()));
            // SPEC-3075: the agent-declared-purpose tier of the rail "what work
            // was running" summary. PR title / commit subject are layered on
            // top later (apply_work_summary_external_sources); branch is the
            // final fallback. Display-only — never the Work identity.
            let work_summary = derive_work_summary(
                first_agent.and_then(|agent| agent.title_summary.as_deref()),
                agents
                    .iter()
                    .find_map(|agent| journal_title_by_session.get(&agent.session_id))
                    .map(String::as_str),
                history.map(|item| item.title.as_str()),
                branch_value.as_deref(),
            );
            gwt::ActiveWorkItemView {
                id: work_id.clone(),
                // SPEC-3075 FR-002/FR-004: the Work title is its *identity*
                // (purpose). `current_focus` is the agent's live "what now"
                // (status) and must never become the Work title — otherwise a
                // status line like "...execution mode..." leaks in as the
                // identity. `title_summary` is the agent-declared purpose, so it
                // stays as a fallback; `current_focus` is removed entirely.
                title: history
                    .map(|item| item.title.clone())
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| is_current_projection.then(|| projection.title.clone()))
                    .or_else(|| first_agent.and_then(|agent| agent.title_summary.clone()))
                    .unwrap_or(work_id),
                status_category,
                status_text,
                summary: history
                    .and_then(|item| item.summary.clone().or_else(|| item.intent.clone()))
                    .or_else(|| {
                        is_current_projection
                            .then(|| projection.summary.clone())
                            .flatten()
                    }),
                progress_summary: history
                    .and_then(|item| item.progress_summary.clone())
                    .or_else(|| {
                        is_current_projection
                            .then(|| projection.progress_summary.clone())
                            .flatten()
                    }),
                work_summary,
                owner: owner_value,
                next_action: if is_current_projection {
                    projection.next_action.clone()
                } else {
                    None
                },
                active_agents,
                blocked_agents,
                branch: branch_value,
                worktree_path: worktree_value,
                managed_hook_health: None,
                pr_number: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_number)
                } else {
                    container.and_then(|value| value.pr_number)
                },
                pr_url: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_url.clone())
                } else {
                    container.and_then(|value| value.pr_url.clone())
                },
                pr_state: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_state.clone())
                } else {
                    container.and_then(|value| value.pr_state.clone())
                },
                board_refs: if is_current_projection {
                    projection.board_refs.clone()
                } else {
                    history
                        .map(|item| item.board_refs.clone())
                        .unwrap_or_default()
                },
                agents,
                // SPEC-2359 Phase W-12 (FR-349): active_work_items groups live
                // assigned agents, so the owning agent session is Running and
                // not user-closed → Active.
                works: Vec::new(),
                lifecycle_state: work_active_lifecycle_state_wire(
                    gwt_core::workspace_projection::recompute_work_active_lifecycle(
                        gwt_core::workspace_projection::WorkAgentRuntime::Running,
                        None,
                    ),
                )
                .to_string(),
                closed_at: None,
                session_agent_total: 0,
                merged_into_base: false,
                workspace_key: None,
                remote_only: false,
                done_equivalent: false,
                cleanup_candidate: None,
                cleanup_blocked_reason: None,
                updated_at: row_updated_at,
            }
        })
        .collect::<Vec<_>>();

    // SPEC-2359 Phase W-12 Slice 5a (FR-350): merge in Paused Work — items that
    // persist in the work history but have no live agent group. These are Works
    // whose owning agent stopped without an explicit user close, so they stay on
    // the Work surface as Paused until closed. Dedupe against existing rows by
    // exact Work id or shared Session identity so a resumed Work surfaces once
    // as Active without hiding a different launch on the same Workspace.
    append_paused_work_items(&mut active_works, works, journal_title_by_session);
    active_works
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): append Paused `active_works` rows for
/// retained Work-history items that have no live agent group. A history item is
/// Paused when it is incomplete (not Done) and is not already represented by a
/// live row (matched by exact Work id or shared Session identity without
/// conflicting git identity). Done items are skipped here — close/cleanup is
/// handled in a later slice.
fn append_paused_work_items(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    works: &[gwt::WorkspaceHistoryView],
    journal_title_by_session: &std::collections::HashMap<String, String>,
) {
    let mut presence = ActiveWorkPresenceIndex::from_active_works(active_works);
    for work in works {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): terminal closes (Done and
        // Discarded) leave the active Work surface. Both are excluded so a
        // closed Work never re-appears as a Paused row.
        if work.status_category == "done" || work.status_category == "discarded" {
            continue;
        }
        if presence.contains_history(work) {
            continue;
        }
        let container = work.execution_containers.first();
        let branch = container.and_then(|value| value.branch.clone());
        let worktree_path = container.and_then(|value| value.worktree_path.clone());
        let title = Some(work.title.clone())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| work.summary.clone())
            .or_else(|| work.intent.clone())
            .unwrap_or_else(|| work.id.clone());
        let status_text = work
            .summary
            .clone()
            .or_else(|| work.intent.clone())
            .unwrap_or_else(|| "Paused".to_string());
        // SPEC-3075: a paused/backfill Work has no live agent — surface the
        // purpose recorded in the journal (agent `title-summary`), then a
        // non-identifier recorded title. PR title / commit subject layer on top
        // later; None falls back to the branch as the rail label.
        let work_summary = derive_work_summary(
            None,
            work.agents
                .iter()
                .find_map(|agent| journal_title_by_session.get(&agent.session_id))
                .map(String::as_str),
            Some(work.title.as_str()),
            branch.as_deref(),
        );
        active_works.push(gwt::ActiveWorkItemView {
            id: work.id.clone(),
            title,
            // Paused Work has no running agent; surface an idle runtime status.
            status_category: "idle".to_string(),
            status_text,
            summary: work.summary.clone().or_else(|| work.intent.clone()),
            progress_summary: work.progress_summary.clone(),
            work_summary,
            owner: work.owner.clone(),
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch,
            worktree_path,
            managed_hook_health: None,
            pr_number: container.and_then(|value| value.pr_number),
            pr_url: container.and_then(|value| value.pr_url.clone()),
            pr_state: container.and_then(|value| value.pr_state.clone()),
            board_refs: work.board_refs.clone(),
            // Carry the persisted Work's agents (each with its Session history)
            // so a Paused Workspace still renders Work → Session in the detail.
            agents: work
                .agents
                .iter()
                .map(paused_work_agent_view_from_history)
                .collect(),
            // No live agent session owns this Work and it is not user-closed →
            // WorkAgentRuntime::None resolves to Paused (FR-350).
            works: Vec::new(),
            lifecycle_state: work_active_lifecycle_state_wire(
                gwt_core::workspace_projection::recompute_work_active_lifecycle(
                    gwt_core::workspace_projection::WorkAgentRuntime::None,
                    None,
                ),
            )
            .to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            cleanup_candidate: None,
            cleanup_blocked_reason: None,
            // FR-403: paused/backfill rows carry the record's last update.
            updated_at: work.updated_at.clone(),
        });
        presence.register(
            active_works
                .last()
                .expect("a paused Work was appended immediately before indexing"),
        );
    }
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a Work-history item is already
/// represented by an existing `active_works` row when their ids match, or when
/// they share a Session identity without conflicting git identities. Branch /
/// worktree identify the parent Workspace and cannot collapse distinct Works.
///
/// Issue #3213: a shared agent session id never collapses two Works whose git
/// identities conflict — a stray session ref recorded under another branch's
/// Work must not swallow the real owner's row (mirrors
/// `active_work_agent_matches_workspace_row_identity`).
#[derive(Clone)]
struct ActiveWorkGitIdentity {
    branch: Option<String>,
    worktree_path: Option<String>,
}

#[derive(Default)]
struct ActiveWorkPresenceIndex {
    ids: HashSet<String>,
    git_identities_by_session: HashMap<String, Vec<ActiveWorkGitIdentity>>,
}

impl ActiveWorkPresenceIndex {
    fn from_active_works(active_works: &[gwt::ActiveWorkItemView]) -> Self {
        let mut index = Self::default();
        for work in active_works {
            index.register(work);
        }
        index
    }

    fn register(&mut self, work: &gwt::ActiveWorkItemView) {
        self.ids.insert(work.id.clone());
        let identity = ActiveWorkGitIdentity {
            branch: work.branch.clone(),
            worktree_path: work.worktree_path.clone(),
        };
        for session_id in work
            .agents
            .iter()
            .map(|agent| agent.session_id.trim())
            .filter(|session_id| !session_id.is_empty())
        {
            self.git_identities_by_session
                .entry(session_id.to_string())
                .or_default()
                .push(identity.clone());
        }
    }

    fn contains_history(&self, work: &gwt::WorkspaceHistoryView) -> bool {
        if self.ids.contains(&work.id) {
            return true;
        }
        work.agents
            .iter()
            .map(|agent| agent.session_id.trim())
            .filter(|session_id| !session_id.is_empty())
            .filter_map(|session_id| self.git_identities_by_session.get(session_id))
            .flatten()
            .any(|identity| {
                !history_git_identity_conflicts(
                    identity.branch.as_deref(),
                    identity.worktree_path.as_deref(),
                    work,
                )
            })
    }
}

pub(super) fn active_work_cleanup_candidate_view_from_candidate(
    candidate: gwt_core::workspace_projection::WorkspaceCleanupCandidate,
) -> gwt::ActiveWorkCleanupCandidateView {
    gwt::ActiveWorkCleanupCandidateView {
        branch: candidate.branch,
        worktree_path: candidate
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        reason: candidate.reason.as_str().to_string(),
        default_delete_remote: candidate.default_delete_remote,
        remote_delete_available: candidate.remote_delete_available,
    }
}

pub(super) fn workspace_journal_entry_view_from_entry(
    entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> gwt::WorkspaceJournalEntryView {
    gwt::WorkspaceJournalEntryView {
        id: entry.id.clone(),
        updated_at: entry
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        title: entry.title.clone(),
        status_category: entry
            .status_category
            .map(workspace_status_category_wire)
            .map(str::to_string),
        status_text: entry.status_text.clone(),
        summary: entry.summary.clone(),
        progress_summary: entry.progress_summary.clone(),
        owner: entry.owner.clone(),
        next_action: entry.next_action.clone(),
        agent_session_id: entry.agent_session_id.clone(),
        agent_current_focus: entry.agent_current_focus.clone(),
        agent_title_summary: entry.agent_title_summary.clone(),
    }
}

/// Index agent sessions by their gwt session id (the Work / launch id) so the
/// view builder can attach each Work's Session history.
pub(super) fn work_session_index(
    sessions: &[gwt_agent::Session],
) -> std::collections::HashMap<&str, &gwt_agent::Session> {
    sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect()
}

pub(crate) fn workspace_work_item_view_from_item(
    item: &gwt_core::workspace_projection::WorkItem,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
    resume_branches: ResumeBranchIndex<'_>,
) -> gwt::WorkspaceHistoryView {
    gwt::WorkspaceHistoryView {
        id: item.id.clone(),
        title: item.title.clone(),
        intent: item.intent.clone(),
        summary: item.summary.clone(),
        progress_summary: item.progress_summary.clone(),
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a discarded Work surfaces as the
        // dedicated `"discarded"` status so the Work surface and the Paused
        // exclusion treat it as a terminal close distinct from Done.
        status_category: if item.discarded {
            "discarded".to_string()
        } else {
            workspace_status_category_wire(item.status_category).to_string()
        },
        owner: item.owner.clone(),
        created_at: item
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        updated_at: item
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        completed_at: item
            .completed_at
            .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        agents: item
            .agents
            .iter()
            .map(|agent| workspace_work_agent_view_from_ref(agent, session_index, resume_branches))
            .collect(),
        execution_containers: item
            .execution_containers
            .iter()
            .map(|container| {
                workspace_execution_container_view_from_ref(
                    container,
                    item.agents.first().map(|agent| agent.session_id.as_str()),
                )
            })
            .collect(),
        board_refs: item.board_refs.clone(),
        related_workspace_ids: item.related_work_item_ids.clone(),
        events: item
            .events
            .iter()
            .map(workspace_work_event_view_from_event)
            .collect(),
    }
}

pub(super) fn workspace_work_agent_view_from_ref(
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
    resume_branches: ResumeBranchIndex<'_>,
) -> gwt::WorkspaceHistoryAgentView {
    // A Work's `session_id` is the gwt session id (the launch). It keys into the
    // persisted Session whose forward-only `session_history` is the Session list
    // (agent-tool conversation UUIDs) under this Work; the latest
    // `agent_session_id` marks the currently active Session.
    let sessions = session_index
        .get(agent.session_id.as_str())
        .map(|session| {
            let latest = session.agent_session_id.as_deref();
            let exact_resume_available =
                resume_branches.session_exact_resume_materializable(session);
            // Render Sessions in stable chronological order (oldest first) so
            // clock skew or delayed persistence cannot scramble the timeline;
            // the append order alone is not guaranteed monotonic.
            let mut entries: Vec<_> = session.session_history.iter().collect();
            entries.sort_by_key(|entry| entry.started_at);
            if entries.is_empty() {
                // SPEC-2359 W-16 (FR-402 follow-up): `session_history` is newer
                // than most ledger TOMLs (zero coverage on long-lived machines),
                // but the latest conversation pointer still exists. Synthesize
                // it as the single Session row instead of "No session yet".
                return latest
                    .map(|conversation| {
                        vec![gwt::WorkspaceHistorySessionView {
                            agent_session_id: conversation.to_string(),
                            started_at: session
                                .updated_at
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            is_active: true,
                            resumable: exact_resume_available
                                && session.is_resumable_conversation(conversation),
                        }]
                    })
                    .unwrap_or_default();
            }
            entries
                .into_iter()
                .map(|entry| gwt::WorkspaceHistorySessionView {
                    agent_session_id: entry.agent_session_id.clone(),
                    started_at: entry
                        .started_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    is_active: latest == Some(entry.agent_session_id.as_str()),
                    // A Session whose conversation handle is structurally
                    // unusable (empty / Codex placeholder) is history-only; the
                    // surface hides its Resume control. A machine-local ledger
                    // whose worktree and branch are gone is also history-only;
                    // Workspace Continue remains the fallback.
                    resumable: exact_resume_available
                        && session.is_resumable_conversation(&entry.agent_session_id),
                })
                .collect()
        })
        .unwrap_or_default();
    // Work records written without agent metadata (older record paths)
    // would render as an anonymous "Agent" group (user verification
    // 2026-06-12) — borrow identity from the ledger TOML when available.
    let ledger = session_index.get(agent.session_id.as_str());
    let display_name = agent
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            ledger
                .map(|session| session.display_name.clone())
                .filter(|name| !name.trim().is_empty())
        });
    let agent_id = agent
        .agent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| ledger.map(|session| session.agent_id.command().to_string()));
    gwt::WorkspaceHistoryAgentView {
        session_id: agent.session_id.clone(),
        agent_id,
        display_name,
        updated_at: agent
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        sessions,
    }
}

fn workspace_execution_container_view_from_ref(
    container: &gwt_core::workspace_projection::WorkspaceExecutionContainerRef,
    session_id: Option<&str>,
) -> gwt::WorkspaceExecutionContainerView {
    gwt::WorkspaceExecutionContainerView {
        branch: container.branch.clone(),
        worktree_path: container
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        pr_number: container.pr_number,
        pr_url: container.pr_url.clone(),
        pr_state: container.pr_state.clone(),
        diagnosis: container.worktree_path.as_deref().map(|worktree| {
            workspace_execution_diagnosis_view(gwt::cli::execution_state::diagnose_for_projection(
                worktree, session_id,
            ))
        }),
    }
}

pub(super) fn workspace_execution_diagnosis_view(
    snapshot: gwt::cli::execution_state::ExecutionDiagnosisSnapshot,
) -> gwt::WorkspaceExecutionDiagnosisView {
    let ecr_status = match snapshot.ecr_status {
        gwt::cli::execution_state::ExecutionDiagnosisState::Active => "active",
        gwt::cli::execution_state::ExecutionDiagnosisState::Completed => "completed",
        gwt::cli::execution_state::ExecutionDiagnosisState::Blocked => "blocked",
        gwt::cli::execution_state::ExecutionDiagnosisState::Missing => "missing",
        gwt::cli::execution_state::ExecutionDiagnosisState::Corrupt => "corrupt",
    };
    let binding_state = match snapshot.binding_state {
        gwt::cli::execution_state::ExecutionBindingState::Bound => "bound",
        gwt::cli::execution_state::ExecutionBindingState::Missing => "missing",
        gwt::cli::execution_state::ExecutionBindingState::Stale => "stale",
        gwt::cli::execution_state::ExecutionBindingState::Terminal => "terminal",
        gwt::cli::execution_state::ExecutionBindingState::HostUnreachable => "host_unreachable",
        gwt::cli::execution_state::ExecutionBindingState::Unknown => "unknown",
        gwt::cli::execution_state::ExecutionBindingState::Corrupt => "corrupt",
    };
    gwt::WorkspaceExecutionDiagnosisView {
        schema_version: snapshot.schema_version,
        ecr_status: ecr_status.to_string(),
        owner_kind: snapshot.owner_kind.map(|kind| kind.as_str().to_string()),
        owner_number: snapshot.owner_number,
        blocked_reason: snapshot.blocked_reason,
        missing_verification: snapshot.missing_verification,
        generation_id: snapshot.generation_id,
        binding_state: binding_state.to_string(),
        binding_cause: snapshot.binding_cause,
        verification_state: snapshot.verification_state,
        trivial_reason: snapshot.trivial_reason,
        generated_outputs: snapshot.generated_outputs,
        capability_generation: snapshot.capability_generation,
        continuation: snapshot
            .continuation
            .map(|value| serde_json::to_value(value).expect("continuation serializes")),
        workspace_update_applicable: snapshot.workspace_update_applicable,
        workspace_update_applicability_reason: snapshot.workspace_update_applicability_reason,
        obligation_revival: snapshot
            .obligation_revival
            .map(|value| serde_json::to_value(value).expect("obligation revival serializes")),
        binding_repair: snapshot
            .binding_repair
            .map(|value| serde_json::to_value(value).expect("binding repair serializes")),
        repair: snapshot
            .repair
            .map(|value| serde_json::to_value(value).expect("repair serializes")),
        work_event_receipt_generation_id: snapshot.work_event_receipt_generation_id,
        work_event_receipt_matches_current_generation: snapshot
            .work_event_receipt_matches_current_generation,
        settlement: snapshot
            .settlement
            .map(|status| serde_json::to_value(status).expect("settlement status serializes")),
        settlement_severity: snapshot.settlement_severity,
        settlement_obligation_open: snapshot.settlement_obligation_open,
        open_obligations: snapshot.open_obligations,
        available_recoveries: snapshot.available_recoveries,
        warnings: snapshot.warnings,
    }
}

fn workspace_work_event_view_from_event(
    event: &gwt_core::workspace_projection::WorkEvent,
) -> gwt::WorkspaceHistoryEventView {
    gwt::WorkspaceHistoryEventView {
        id: event.id.clone(),
        workspace_id: event.work_item_id.clone(),
        kind: workspace_work_event_kind_wire(event.kind).to_string(),
        title: event.title.clone(),
        intent: event.intent.clone(),
        summary: event.summary.clone(),
        progress_summary: event.progress_summary.clone(),
        status_category: event
            .status_category
            .map(workspace_status_category_wire)
            .map(str::to_string),
        owner: event.owner.clone(),
        next_action: event.next_action.clone(),
        agent_session_id: event.agent_session_id.clone(),
        board_entry_id: event.board_entry_id.clone(),
        related_workspace_id: event.related_work_item_id.clone(),
        updated_at: event
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

pub(super) fn workspace_work_event_kind_wire(
    kind: gwt_core::workspace_projection::WorkEventKind,
) -> &'static str {
    use gwt_core::workspace_projection::WorkEventKind;

    match kind {
        WorkEventKind::Start => "start",
        WorkEventKind::Claim => "claim",
        WorkEventKind::Update => "update",
        WorkEventKind::Blocked => "blocked",
        WorkEventKind::Handoff => "handoff",
        WorkEventKind::Resume => "resume",
        WorkEventKind::Split => "split",
        WorkEventKind::Merge => "merge",
        WorkEventKind::Pr => "pr",
        WorkEventKind::Pause => "pause",
        WorkEventKind::Done => "done",
        WorkEventKind::Discard => "discard",
        WorkEventKind::Backfill => "backfill",
    }
}

pub(super) fn non_empty_workspace_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn workspace_resume_context_from_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
) -> WorkspaceResumeContext {
    WorkspaceResumeContext {
        title: non_empty_workspace_text(Some(&projection.title)),
        owner: non_empty_workspace_text(projection.owner.as_deref()),
        summary: non_empty_workspace_text(projection.summary.as_deref()),
        next_action: non_empty_workspace_text(projection.next_action.as_deref()),
    }
}

pub(super) fn workspace_resume_context_from_journal(
    entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> WorkspaceResumeContext {
    WorkspaceResumeContext {
        title: non_empty_workspace_text(entry.title.as_deref())
            .or_else(|| non_empty_workspace_text(entry.agent_title_summary.as_deref())),
        owner: non_empty_workspace_text(entry.owner.as_deref()),
        summary: non_empty_workspace_text(entry.summary.as_deref())
            .or_else(|| non_empty_workspace_text(entry.agent_current_focus.as_deref()))
            .or_else(|| non_empty_workspace_text(entry.status_text.as_deref())),
        next_action: non_empty_workspace_text(entry.next_action.as_deref()),
    }
}

/// #3065: build the Workspace Resume context from the resumed branch's own
/// Work item. The repo-shared current projection (`current.json`) must NOT be
/// the source here: it carries the identity of whatever Work last wrote it,
/// and replaying that identity into a different Work's resume event is how
/// one Work's owner/title leaked into every other Workspace row. When no
/// Work item matches the container, the context is neutral — never the
/// shared identity.
pub(super) fn workspace_resume_context_for_work_item(
    project_state_root: &Path,
    branch: Option<&str>,
    worktree_path: &Path,
) -> WorkspaceResumeContext {
    let item = gwt_core::workspace_projection::load_workspace_work_items(project_state_root)
        .ok()
        .flatten()
        .and_then(|projection| {
            gwt_core::workspace_projection::find_work_item_for_container(
                &projection,
                project_state_root,
                branch,
                Some(worktree_path),
            )
            .cloned()
        });
    match item {
        Some(item) => WorkspaceResumeContext {
            title: non_empty_workspace_text(Some(&item.title)),
            owner: non_empty_workspace_text(item.owner.as_deref()),
            summary: non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        },
        None => WorkspaceResumeContext {
            title: None,
            owner: None,
            summary: None,
            next_action: None,
        },
    }
}

pub(super) fn workspace_resume_owner_issue_number(owner: Option<&str>) -> Option<u64> {
    let owner = owner?.trim();
    if owner.is_empty() {
        return None;
    }
    let lower = owner.to_ascii_lowercase();
    if !(owner.starts_with('#') || lower.contains("issue") || lower.contains("spec")) {
        return None;
    }

    let mut digits = String::new();
    let mut started = false;
    for character in owner.chars() {
        if character.is_ascii_digit() {
            started = true;
            digits.push(character);
        } else if started {
            break;
        }
    }
    digits.parse::<u64>().ok()
}

pub(super) fn linked_issue_workspace_context(
    project_root: &Path,
    issue_number: u64,
    owner_label: impl Into<String>,
) -> WorkspaceResumeContext {
    let owner_label = owner_label.into();
    WorkspaceResumeContext {
        title: issue_title_from_cache(project_root, issue_number)
            .or_else(|| Some(owner_label.clone())),
        owner: Some(owner_label),
        summary: None,
        next_action: None,
    }
}

pub(super) fn workspace_resume_branch_from_journal_project_root(
    project_root: &Path,
    active_project_root: &Path,
) -> Option<String> {
    if let Ok(branch) = current_git_branch(project_root) {
        let branch = normalize_branch_name(branch.trim());
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    let main_repo_path = gwt_git::worktree::main_worktree_root(active_project_root).ok()?;
    let layout_root = main_repo_path.parent()?;
    let normalized_project_root = normalize_existing_path_prefix(project_root);
    let normalized_layout_root = normalize_existing_path_prefix(layout_root);
    let relative_path = normalized_project_root
        .strip_prefix(&normalized_layout_root)
        .ok()?;
    let branch = relative_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if branch.is_empty() {
        return None;
    }
    Some(branch)
}

fn normalize_existing_path_prefix(path: &Path) -> PathBuf {
    if path.exists() {
        return std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    }

    let mut missing_components = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            return path.to_path_buf();
        };
        missing_components.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return path.to_path_buf();
        };
        current = parent;
    }

    let mut normalized = std::fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
    for component in missing_components.iter().rev() {
        normalized.push(component);
    }
    normalized
}

pub(super) fn workspace_resume_branch_exists(project_root: &Path, branch_name: &str) -> bool {
    let branch_name = normalize_branch_name(branch_name.trim());
    if branch_name.is_empty() {
        return false;
    }
    let Ok(main_repo_path) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    if local_branch_exists(&main_repo_path, &branch_name).unwrap_or(false) {
        return true;
    }
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    manager
        .remote_branch_exists(&origin_remote_ref(&branch_name))
        .unwrap_or(false)
}

pub(super) fn session_exact_resume_materializable(
    project_root: &Path,
    session: &gwt_agent::Session,
) -> bool {
    if session.worktree_path.as_path().exists() {
        return true;
    }
    workspace_resume_branch_exists(project_root, &session.branch)
}

/// Issue #3611: every short branch name (`work/x`, `origin/work/x`) that exists
/// in `project_root`, resolved in ONE `for-each-ref` spawn.
///
/// This is the same inventory [`workspace_resume_branch_exists`] derives with a
/// `git rev-parse --git-common-dir` plus two `git show-ref --verify` spawns
/// **per branch**. Resolving it once is what keeps the Git process count flat
/// as the Session count grows (AC-2); the GUI event loop never calls it — it
/// consumes the snapshot published by the background merge scan instead.
pub(crate) fn resume_branch_refs_snapshot(project_root: &Path) -> HashSet<String> {
    gwt_git::refs::branch_tip_committer_times(project_root)
        .map(|tips| tips.into_keys().collect())
        .unwrap_or_default()
}

/// Issue #3611: resolves "can this Session's exact worktree be re-materialized?"
/// for the projection view builders without spawning Git per Session.
///
/// The GUI event loop is single-threaded, so the former per-Session
/// `rev-parse` + `show-ref` pair stalled it for seconds on repositories with
/// many historical Sessions and starved the `pane.*` route (#3510). Every
/// builder now answers from a branch-ref snapshot: the background merge scan
/// publishes it for the event-loop path, and blocking-task builders resolve
/// their own with [`resume_branch_refs_snapshot`].
#[derive(Clone, Copy)]
pub(crate) struct ResumeBranchIndex<'a> {
    /// `None` means "this project has not been scanned yet". A Session is then
    /// left optimistically resumable: the Launch Wizard re-verifies branch
    /// existence before materializing, so an unnecessary Resume control is
    /// recoverable while a missing one hides a working Resume.
    known_branch_refs: Option<&'a HashSet<String>>,
}

impl<'a> ResumeBranchIndex<'a> {
    pub(crate) fn scanned(known_branch_refs: Option<&'a HashSet<String>>) -> Self {
        Self { known_branch_refs }
    }

    pub(crate) fn branch_exists(&self, branch_name: &str) -> bool {
        let branch_name = normalize_branch_name(branch_name.trim());
        if branch_name.is_empty() {
            return false;
        }
        let Some(known_branch_refs) = self.known_branch_refs else {
            return true;
        };
        known_branch_refs.contains(&branch_name)
            || known_branch_refs.contains(&origin_remote_ref(&branch_name))
    }

    pub(crate) fn session_exact_resume_materializable(&self, session: &gwt_agent::Session) -> bool {
        session.worktree_path.as_path().exists() || self.branch_exists(&session.branch)
    }
}

fn active_work_agent_priority_rank(agent: &gwt::ActiveWorkAgentView) -> u8 {
    match agent.status_category.as_str() {
        "blocked" => 0,
        "active" => match agent.last_board_entry_kind.as_deref() {
            Some("handoff") => 1,
            Some("next") => 2,
            Some("claim") => 3,
            Some("decision") => 4,
            Some("status") => 5,
            _ => 6,
        },
        "idle" => 7,
        "done" => 8,
        _ => 9,
    }
}

fn active_work_agent_view_from_summary(
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> gwt::ActiveWorkAgentView {
    let affiliation_status = match agent.affiliation_status {
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned => "unassigned",
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned => "assigned",
    };
    gwt::ActiveWorkAgentView {
        session_id: agent.session_id.clone(),
        window_id: agent.window_id.clone(),
        agent_id: agent.agent_id.clone(),
        display_name: agent.display_name.clone(),
        affiliation_status: affiliation_status.to_string(),
        workspace_id: agent.workspace_id.clone(),
        status_category: workspace_status_category_wire(agent.status_category).to_string(),
        current_focus: agent.current_focus.clone(),
        title_summary: agent.title_summary.clone(),
        branch: agent.branch.clone(),
        worktree_path: agent
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        last_board_entry_id: agent.last_board_entry_id.clone(),
        last_board_entry_kind: agent
            .last_board_entry_kind
            .as_ref()
            .map(|kind| kind.as_str().to_string()),
        coordination_scope: agent.coordination_scope.clone(),
        updated_at: agent.updated_at.to_rfc3339(),
        // Live projection summaries do not carry conversation history; Paused
        // Works fill this in from the persisted Session via
        // `paused_work_agent_view_from_history`.
        sessions: Vec::new(),
    }
}

fn active_work_agent_identity_key(agent: &gwt::ActiveWorkAgentView) -> Option<String> {
    for raw in [&agent.agent_id, &agent.display_name] {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        if let Some(agent_id) = gwt_agent::resolve_agent_id(value) {
            return Some(format!("agent:{}", agent_id.command()));
        }
    }

    [&agent.agent_id, &agent.display_name]
        .into_iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(|value| format!("label:{}", value.to_lowercase()))
}

fn recompute_active_work_agent_counters(work: &mut gwt::ActiveWorkItemView) {
    work.active_agents = work
        .agents
        .iter()
        .filter(|agent| matches!(agent.status_category.as_str(), "active" | "running"))
        .count();
    work.blocked_agents = work
        .agents
        .iter()
        .filter(|agent| agent.status_category == "blocked")
        .count();
}

fn collapse_active_work_agents_by_conversation(
    agents: &mut Vec<gwt::ActiveWorkAgentView>,
) -> usize {
    let mut sorted = std::mem::take(agents);
    sorted.sort_by(compare_active_work_agents_newest_first);
    let mut seen_conversations: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut kept: Vec<gwt::ActiveWorkAgentView> = Vec::with_capacity(sorted.len());
    let mut dropped = 0usize;
    for agent in sorted {
        let conversation = agent
            .sessions
            .iter()
            .find(|session| session.is_active)
            .or_else(|| agent.sessions.first())
            .map(|session| session.agent_session_id.clone());
        match conversation {
            Some(conversation) if !conversation.is_empty() => {
                if let Some(&index) = seen_conversations.get(&conversation) {
                    if kept[index].display_name.trim().is_empty()
                        && !agent.display_name.trim().is_empty()
                    {
                        kept[index].display_name = agent.display_name.clone();
                    }
                    dropped += 1;
                } else {
                    seen_conversations.insert(conversation, kept.len());
                    kept.push(agent);
                }
            }
            _ => kept.push(agent),
        }
    }
    *agents = kept;
    dropped
}

fn retain_summary_active_work_agent_per_identity(agents: &mut Vec<gwt::ActiveWorkAgentView>) {
    let mut sorted = std::mem::take(agents);
    sorted.sort_by(compare_active_work_agents_for_summary);
    let mut seen_identities: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept: Vec<gwt::ActiveWorkAgentView> = Vec::with_capacity(sorted.len());
    for agent in sorted {
        let Some(identity) = active_work_agent_identity_key(&agent) else {
            kept.push(agent);
            continue;
        };
        if seen_identities.insert(identity) {
            kept.push(agent);
        }
    }
    *agents = kept;
}

fn compare_active_work_agents_for_summary(
    left: &gwt::ActiveWorkAgentView,
    right: &gwt::ActiveWorkAgentView,
) -> std::cmp::Ordering {
    left.sessions
        .is_empty()
        .cmp(&right.sessions.is_empty())
        .then_with(|| compare_active_work_agents_newest_first(left, right))
}

fn active_work_payload_agents(
    agents: &[gwt::ActiveWorkAgentView],
    cap: usize,
) -> Vec<gwt::ActiveWorkAgentView> {
    let mut summary_agents = agents.to_vec();
    retain_summary_active_work_agent_per_identity(&mut summary_agents);
    summary_agents.truncate(cap);

    let mut selected_session_ids: HashSet<String> = summary_agents
        .iter()
        .map(|agent| agent.session_id.clone())
        .collect();
    let mut payload_agents = summary_agents;
    let mut remaining_agents = agents.to_vec();
    remaining_agents.sort_by(compare_active_work_agents_newest_first);
    for agent in remaining_agents {
        if payload_agents.len() >= cap {
            break;
        }
        if selected_session_ids.insert(agent.session_id.clone()) {
            payload_agents.push(agent);
        }
    }
    payload_agents.sort_by(compare_active_work_agents_newest_first);
    payload_agents
}

fn sync_active_workspace_child_agents(
    work: &mut gwt::ActiveWorkItemView,
    payload_agents: &[gwt::ActiveWorkAgentView],
) {
    if work.works.len() == 1 {
        work.works[0].agents = payload_agents.to_vec();
        return;
    }

    let child_session_ids: Vec<HashSet<String>> = work
        .works
        .iter()
        .map(|child| {
            child
                .agents
                .iter()
                .map(|agent| agent.session_id.clone())
                .collect()
        })
        .collect();
    for child in &mut work.works {
        child.agents.clear();
    }

    for agent in payload_agents {
        let canonical_child_id = format!("work-session-{}", agent.session_id);
        let target_index = work
            .works
            .iter()
            .position(|child| child.id == canonical_child_id)
            .or_else(|| {
                child_session_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, session_ids)| session_ids.contains(agent.session_id.as_str()))
                    .max_by(|(left_index, _), (right_index, _)| {
                        let left = &work.works[*left_index];
                        let right = &work.works[*right_index];
                        left.updated_at
                            .cmp(&right.updated_at)
                            .then_with(|| left.id.cmp(&right.id))
                    })
                    .map(|(index, _)| index)
            });
        if let Some(index) = target_index {
            work.works[index].agents.push(agent.clone());
        }
    }
}

fn compare_active_work_agents_newest_first(
    left: &gwt::ActiveWorkAgentView,
    right: &gwt::ActiveWorkAgentView,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.session_id.cmp(&left.session_id))
        .then_with(|| right.window_id.cmp(&left.window_id))
        .then_with(|| right.display_name.cmp(&left.display_name))
        .then_with(|| right.agent_id.cmp(&left.agent_id))
}

fn active_work_agent_matches_workspace_row_identity(
    row_branch: Option<&str>,
    row_worktree: Option<&Path>,
    agent: &gwt::ActiveWorkAgentView,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
) -> bool {
    let row_branch = row_branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_branch_name);
    let row_has_git_identity = row_branch.is_some() || row_worktree.is_some();

    let ledger = session_index.get(agent.session_id.as_str());
    let agent_branch = ledger
        .map(|session| session.branch.as_str())
        .or(agent.branch.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_branch_name);
    let agent_worktree = ledger
        .map(|session| session.worktree_path.as_path())
        .or_else(|| agent.worktree_path.as_deref().map(Path::new));

    if !row_has_git_identity {
        return true;
    }

    let branch_matches = row_branch
        .as_deref()
        .zip(agent_branch.as_deref())
        .is_some_and(|(left, right)| left == right);
    let worktree_matches = row_worktree
        .zip(agent_worktree)
        .is_some_and(|(left, right)| projection_worktree_paths_match(left, right));
    if branch_matches || worktree_matches {
        return true;
    }

    let branch_conflicts = row_branch.is_some() && agent_branch.is_some();
    let worktree_conflicts = row_worktree.is_some() && agent_worktree.is_some();
    !(branch_conflicts || worktree_conflicts)
}

/// Convert a persisted Work's agent (a launch, carrying its Session history) to
/// the active-surface agent view so Paused Workspaces render their Work →
/// Session list instead of an empty agent list.
/// SPEC-2359 Phase W-16 (FR-402): attach machine-local ledger sessions to
/// each Workspace (branch) row. Sessions whose TOML carries this project's
/// repo hash and the row's branch join the row's agents (deduped by gwt
/// session id, capped per [`crate::workspace_session_registry`]); the
/// uncapped count rides `session_agent_total` so the frontend can render
/// "+N more sessions".
pub(super) fn attach_registry_sessions_to_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    agent_sessions: &[gwt_agent::Session],
    project_repo_hash: Option<gwt_core::repo_hash::RepoHash>,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
    resume_branches: ResumeBranchIndex<'_>,
) {
    let registry = crate::workspace_session_registry::branch_session_registry(
        agent_sessions,
        project_repo_hash.as_ref().map(|hash| hash.as_str()),
    );
    let cap = crate::workspace_session_registry::REGISTRY_SESSION_CAP;
    for work in active_works.iter_mut() {
        let row_branch = work.branch.clone();
        let row_worktree = work.worktree_path.as_deref().map(PathBuf::from);
        work.agents.retain(|agent| {
            active_work_agent_matches_workspace_row_identity(
                row_branch.as_deref(),
                row_worktree.as_deref(),
                agent,
                session_index,
            )
        });
        let existing: Vec<&str> = work
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect();
        let (additions, extra_total) =
            crate::workspace_session_registry::registry_sessions_for_branch(
                &registry,
                work.branch.as_deref(),
                &existing,
                cap,
            );
        work.session_agent_total = (work.agents.len() + extra_total) as u32;
        for session in additions {
            let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
                session_id: session.id.clone(),
                agent_id: Some(session.agent_id.command().to_string()),
                display_name: Some(session.display_name.clone()),
                updated_at: session.last_activity_at,
                attached_by: None,
            };
            let history_view =
                workspace_work_agent_view_from_ref(&agent_ref, session_index, resume_branches);
            work.agents
                .push(paused_work_agent_view_from_history(&history_view));
        }
        // User verification 2026-06-12 (follow-up): ghost record agents —
        // ledger TOML gone, no identity recorded, no conversation — render
        // as a dead "Agent / No session yet" group whose Resume cannot work.
        // Drop them from the view; the Work row itself stays.
        {
            let before = work.agents.len();
            work.agents.retain(|agent| {
                !agent.display_name.trim().is_empty()
                    || !agent.agent_id.trim().is_empty()
                    || !agent.sessions.is_empty()
            });
            let dropped = (before - work.agents.len()) as u32;
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped);
        }
        // User verification 2026-06-12: a Resume creates a new gwt session for
        // the SAME agent conversation, which used to render as two Work rows
        // ("Agent" + "Claude Code") carrying one conversation id. Collapse
        // agents whose latest conversation matches — newest updated_at wins
        // and borrows the duplicate's display_name when its own is empty.
        {
            let dropped = collapse_active_work_agents_by_conversation(&mut work.agents);
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped as u32);
        }
        // Select one bounded Agent set for every child Work before the parent
        // summary collapses identities. The latest Agent per identity is
        // reserved first so the summary remains stable; remaining slots keep
        // distinct conversations visible in their owning child Work.
        let payload_agents = active_work_payload_agents(&work.agents, cap);
        sync_active_workspace_child_agents(work, &payload_agents);
        for child in &mut work.works {
            retain_summary_active_work_agent_per_identity(&mut child.agents);
        }
        // User verification 2026-06-17 (follow-up): Workspace detail is a
        // session summary, not a live process inventory. Per agent identity
        // only one history entry stays; a usable conversation wins before
        // recency so empty replacement records cannot hide it. Child Works use
        // the same rule to avoid rendering Current beside "No session yet".
        retain_summary_active_work_agent_per_identity(&mut work.agents);
        recompute_active_work_agent_counters(work);
        // The cap applies to the row's TOTAL agents: a decomposed legacy row
        // can carry hundreds of record agents, and the workspace payload feeds
        // every connected client (unbounded fan-out amplifies the WebSocket
        // eviction storm). Keep the newest agents; the uncapped count already
        // rides `session_agent_total`. RFC3339 UTC strings sort lexically.
        if work.agents.len() > cap {
            work.agents.sort_by(compare_active_work_agents_newest_first);
            work.agents.truncate(cap);
        }
    }
    // SPEC-2359 Phase W-16 (FR-403): order the list by last update, newest
    // first — the row stamp or its freshest agent/ledger session, whichever
    // is newer. RFC3339 UTC strings compare lexically.
    let row_sort_key = |work: &gwt::ActiveWorkItemView| -> String {
        work.agents
            .iter()
            .map(|agent| agent.updated_at.clone())
            .chain(std::iter::once(work.updated_at.clone()))
            .max()
            .unwrap_or_default()
    };
    active_works.sort_by_key(|work| std::cmp::Reverse(row_sort_key(work)));
}

/// SPEC-2359 W16-2 (FR-389 / SC-259): assign every row its Workspace
/// grouping key (canonical branch identity → canonical worktree identity →
/// own id) and merge rows that share a key into ONE Workspace row. The
/// newest row is the representative; agents concatenate (the identity
/// collapse downstream dedups), numeric counts sum, `merged_into_base` ORs,
/// and missing PR metadata is filled from another row in the same branch
/// group. Old branchless ids keep their own key, so legacy rows never vanish
/// or fuse.
pub(super) fn assign_and_merge_workspace_groups(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    project_root: &Path,
) {
    assign_and_merge_workspace_groups_impl(active_works, project_root, true);
}

fn assign_and_merge_workspace_groups_cache_only(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    project_root: &Path,
) {
    assign_and_merge_workspace_groups_impl(active_works, project_root, false);
}

fn assign_and_merge_workspace_groups_impl(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    project_root: &Path,
    include_execution_diagnosis: bool,
) {
    for work in active_works.iter_mut() {
        if work.works.is_empty() {
            let child = active_workspace_child_work(work, include_execution_diagnosis);
            work.works.push(child);
        }
        let branch = work
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let worktree = work.worktree_path.as_deref().map(std::path::Path::new);
        let key = gwt_core::workspace_projection::canonical_work_id(project_root, branch, None)
            .or_else(|| {
                gwt_core::workspace_projection::canonical_work_id(project_root, None, worktree)
            })
            .unwrap_or_else(|| work.id.clone());
        work.workspace_key = Some(key);
    }

    let mut merged: Vec<gwt::ActiveWorkItemView> = Vec::with_capacity(active_works.len());
    let mut index_by_key: HashMap<String, usize> = HashMap::new();
    for work in active_works.drain(..) {
        let key = work
            .workspace_key
            .clone()
            .unwrap_or_else(|| work.id.clone());
        match index_by_key.get(&key) {
            Some(&slot) => {
                let target = &mut merged[slot];
                let newer = work.updated_at > target.updated_at;
                let mut agents = std::mem::take(&mut target.agents);
                agents.extend(work.agents.iter().cloned());
                let mut child_works = std::mem::take(&mut target.works);
                child_works.extend(work.works.iter().cloned());
                child_works.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
                let mut seen_work_ids = HashSet::new();
                child_works.retain(|child| seen_work_ids.insert(child.id.clone()));
                let active_agents = target.active_agents + work.active_agents;
                let blocked_agents = target.blocked_agents + work.blocked_agents;
                let session_agent_total = target.session_agent_total + work.session_agent_total;
                let merged_into_base = target.merged_into_base || work.merged_into_base;
                let (pr_number, pr_url, pr_state) = if newer {
                    (
                        work.pr_number.or(target.pr_number),
                        work.pr_url.clone().or_else(|| target.pr_url.clone()),
                        work.pr_state.clone().or_else(|| target.pr_state.clone()),
                    )
                } else {
                    (
                        target.pr_number.or(work.pr_number),
                        target.pr_url.clone().or_else(|| work.pr_url.clone()),
                        target.pr_state.clone().or_else(|| work.pr_state.clone()),
                    )
                };
                if newer {
                    let key = target.workspace_key.clone();
                    // SPEC-3075 FR-004: a session-derived row's title/owner is
                    // agent content (the live session), not the branch's
                    // identity. When a fresher session row merges into a
                    // branch-backed Work, take its fresher status but preserve
                    // the branch-backed identity so another agent's content
                    // never surfaces as this Work's title.
                    let target_branch_backed = !target.id.starts_with("work-session-");
                    let work_session_derived = work.id.starts_with("work-session-");
                    let preserved_identity = (target_branch_backed && work_session_derived)
                        .then(|| (target.title.clone(), target.owner.clone()));
                    *target = work;
                    target.workspace_key = key;
                    if let Some((title, owner)) = preserved_identity {
                        target.title = title;
                        if owner.is_some() {
                            target.owner = owner;
                        }
                    }
                }
                target.agents = agents;
                target.works = child_works;
                target.active_agents = active_agents;
                target.blocked_agents = blocked_agents;
                target.session_agent_total = session_agent_total;
                target.merged_into_base = merged_into_base;
                target.pr_number = pr_number;
                target.pr_url = pr_url;
                target.pr_state = pr_state;
                if target.branch.is_none() {
                    // keep any branch the group knows about
                    target.branch = merged_branch_fallback(&target.agents);
                }
            }
            None => {
                index_by_key.insert(key, merged.len());
                merged.push(work);
            }
        }
    }
    *active_works = merged;
}

fn active_workspace_child_work(
    work: &gwt::ActiveWorkItemView,
    include_execution_diagnosis: bool,
) -> gwt::ActiveWorkspaceWorkView {
    let lifecycle_state = work.lifecycle_state.clone();
    let manual_close_allowed = lifecycle_state == "paused" && work.active_agents == 0;
    let close_blocked_reason = (!manual_close_allowed
        && !matches!(lifecycle_state.as_str(), "done" | "discarded"))
    .then(|| "live_agent".to_string());
    gwt::ActiveWorkspaceWorkView {
        id: work.id.clone(),
        title: work.title.clone(),
        work_summary: work.work_summary.clone(),
        status_category: work.status_category.clone(),
        status_text: work.status_text.clone(),
        owner: work.owner.clone(),
        lifecycle_state,
        closed_at: work.closed_at.clone(),
        manual_close_allowed,
        close_blocked_reason,
        agents: work.agents.clone(),
        execution_diagnosis: if include_execution_diagnosis {
            work.worktree_path.as_deref().map(|worktree| {
                workspace_execution_diagnosis_view(
                    gwt::cli::execution_state::diagnose_for_projection(
                        Path::new(worktree),
                        work.agents.first().map(|agent| agent.session_id.as_str()),
                    ),
                )
            })
        } else {
            None
        },
        updated_at: work.updated_at.clone(),
    }
}

fn merged_branch_fallback(agents: &[gwt::ActiveWorkAgentView]) -> Option<String> {
    agents.iter().find_map(|agent| agent.branch.clone())
}

/// SPEC-2359 W16-3 (FR-390): flag rows whose branch exists only as a fetched
/// remote ref — no recorded worktree path and no local worktree for the
/// branch. Display-only marking (FR-381/FR-390: rendering generates no
/// events); the existing Launch path materializes a worktree on demand.
pub(super) fn mark_remote_only_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    local_branches: Option<&std::collections::HashSet<String>>,
) {
    for work in active_works.iter_mut() {
        let has_worktree = work
            .worktree_path
            .as_deref()
            .map(str::trim)
            .is_some_and(|path| !path.is_empty());
        if has_worktree {
            work.remote_only = false;
            continue;
        }
        let branch_local = work
            .branch
            .as_deref()
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
            .and_then(|branch| local_branches.map(|set| set.contains(&branch)));
        // Branchless rows are never "remote": there is nothing to fetch.
        work.remote_only = matches!(branch_local, Some(false));
        if work.remote_only {
            for child in &mut work.works {
                child.manual_close_allowed = false;
                child.close_blocked_reason = Some("remote_environment_unknown".to_string());
            }
        }
    }
}

/// SPEC-2359 W-15 (FR-386): flag rows whose branch is merged into a base on
/// origin (background scan cache) or whose recorded PR state is merged — the
/// "safe to delete" signal. Display-only; no automatic close (US-61).
pub(super) fn mark_merged_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    merged_branches: Option<&HashMap<String, chrono::DateTime<chrono::Utc>>>,
    dirty_branches: Option<&HashSet<String>>,
) {
    for work in active_works.iter_mut() {
        if active_work_dirty_from_cache(work, dirty_branches).unwrap_or(true) {
            work.merged_into_base = false;
            work.done_equivalent = false;
            continue;
        }
        let merge_reference = work
            .branch
            .as_deref()
            .map(crate::runtime_support::normalize_branch_name)
            .and_then(|branch| merged_branches.and_then(|map| map.get(&branch)))
            .copied();
        let by_pr = work
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
        work.merged_into_base = merge_reference.is_some() || by_pr;

        // SPEC-2359 W16-4 (FR-391): merged ∧ stale → derived Done-equivalent.
        // Membership rides the scan verdict ONLY (pr_state stays badge-only);
        // explicit terminal closes keep their own lifecycle; no event is ever
        // recorded from this classification (US-61).
        let terminal = matches!(work.lifecycle_state.as_str(), "done" | "discarded");
        let last_activity = work
            .agents
            .iter()
            .map(|agent| agent.updated_at.as_str())
            .chain(std::iter::once(work.updated_at.as_str()))
            .filter_map(|stamp| {
                chrono::DateTime::parse_from_rfc3339(stamp)
                    .ok()
                    .map(|value| value.with_timezone(&chrono::Utc))
            })
            .max();
        work.done_equivalent = !terminal
            && last_activity.is_some_and(|last| {
                gwt_core::workspace_projection::derive_merged_done_equivalent(
                    merge_reference.is_some(),
                    last,
                    merge_reference,
                )
            });
    }
}

fn active_work_dirty_from_cache(
    work: &gwt::ActiveWorkItemView,
    dirty_branches: Option<&HashSet<String>>,
) -> Option<bool> {
    let has_worktree = work
        .worktree_path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty());
    if !has_worktree {
        return Some(false);
    }
    let branch = work
        .branch
        .as_deref()
        .map(normalize_branch_name)
        .filter(|branch| !branch.is_empty())?;
    Some(dirty_branches?.contains(&branch))
}

/// SPEC-2359 US-78: cleanup eligibility is backend-owned per Workspace row.
/// `merged_into_base` remains a display badge; this candidate is the action
/// gate after filtering out live-agent branches/worktrees and remote-only rows.
pub(super) fn mark_workspace_cleanup_candidates(
    active_works: &mut [gwt::ActiveWorkItemView],
    cleanup_ready_branches: Option<&HashMap<String, String>>,
    dirty_branches: Option<&HashSet<String>>,
    sessions: &[&ActiveAgentSession],
    live_process_branches: Option<&HashSet<String>>,
) {
    for work in active_works.iter_mut() {
        work.cleanup_candidate = None;
        work.cleanup_blocked_reason = None;
        if work.remote_only {
            continue;
        }
        let Some(branch) = work
            .branch
            .as_deref()
            .map(normalize_branch_name)
            .filter(|branch| branch.starts_with("work/"))
        else {
            continue;
        };
        let Some(reason) =
            cleanup_reason_for_work(work, cleanup_ready_branches, dirty_branches, &branch)
        else {
            continue;
        };
        let worktree_path = work.worktree_path.as_deref().map(Path::new);
        if sessions.iter().any(|session| {
            active_agent_session_matches_work(session, Some(branch.as_str()), worktree_path)
        }) {
            work.cleanup_blocked_reason = Some("live_agent".to_string());
            continue;
        }
        let Some(live_process_branches) = live_process_branches else {
            work.cleanup_blocked_reason = Some("process_liveness_unknown".to_string());
            continue;
        };
        if live_process_branches.contains(&branch) {
            work.cleanup_blocked_reason = Some("live_process".to_string());
            continue;
        }
        work.cleanup_candidate = Some(gwt::ActiveWorkCleanupCandidateView {
            branch: branch.to_string(),
            worktree_path: work.worktree_path.clone(),
            reason,
            default_delete_remote: false,
            remote_delete_available: true,
        });
    }
}

fn cleanup_reason_for_work(
    work: &gwt::ActiveWorkItemView,
    cleanup_ready_branches: Option<&HashMap<String, String>>,
    dirty_branches: Option<&HashSet<String>>,
    branch: &str,
) -> Option<String> {
    if active_work_dirty_from_cache(work, dirty_branches).unwrap_or(true) {
        return None;
    }
    if let Some(reason) = cleanup_ready_branches
        .and_then(|map| map.get(branch))
        .cloned()
    {
        return Some(reason);
    }
    if work.merged_into_base
        || work
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"))
    {
        return Some(
            gwt_core::workspace_projection::WorkspaceCleanupReason::PrMerged
                .as_str()
                .to_string(),
        );
    }
    None
}

fn cleanup_candidate_has_live_process(
    candidate: &gwt::ActiveWorkCleanupCandidateView,
    live_process_branches: Option<&HashSet<String>>,
) -> bool {
    let Some(live_process_branches) = live_process_branches else {
        return true;
    };
    let branch = normalize_branch_name(&candidate.branch);
    live_process_branches.contains(&branch)
}

fn paused_work_agent_view_from_history(
    agent: &gwt::WorkspaceHistoryAgentView,
) -> gwt::ActiveWorkAgentView {
    gwt::ActiveWorkAgentView {
        session_id: agent.session_id.clone(),
        window_id: None,
        agent_id: agent.agent_id.clone().unwrap_or_default(),
        display_name: agent.display_name.clone().unwrap_or_default(),
        affiliation_status: "assigned".to_string(),
        workspace_id: None,
        status_category: "idle".to_string(),
        current_focus: None,
        title_summary: None,
        branch: None,
        worktree_path: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        updated_at: agent.updated_at.clone(),
        sessions: agent.sessions.clone(),
    }
}

pub(super) fn active_agent_session_matches_work(
    session: &ActiveAgentSession,
    normalized_branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> bool {
    let branch_matches = normalized_branch
        .is_some_and(|branch| normalize_branch_name(session.branch_name.trim()) == branch);
    let worktree_matches = worktree_path
        .is_some_and(|path| projection_worktree_paths_match(&session.worktree_path, path));
    branch_matches || worktree_matches
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

pub(super) fn agent_launch_purpose_title(
    project_root: &Path,
    linked_issue_number: Option<u64>,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> Option<String> {
    linked_issue_number
        .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        .or_else(|| {
            linked_issue_number_for_branch(project_root, branch_name, issue_link_cache_dir)
                .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        })
        .or_else(|| workspace_projection_owner_title(project_root, branch_name))
}

fn issue_title_from_cache(project_root: &Path, issue_number: u64) -> Option<String> {
    // #3426: use the canonical workspace-home-aware cache resolution (with the
    // detached fallback) so managed roots read the same cache as execution
    // owner-kind detection and title sync.
    let cache_root = gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
    let entry =
        gwt_github::Cache::new(cache_root).load_entry(gwt_github::IssueNumber(issue_number))?;
    let title = entry.snapshot.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn linked_issue_number_for_branch(
    project_root: &Path,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> Option<u64> {
    let branch_name = branch_name?.trim();
    if branch_name.is_empty() {
        return None;
    }
    let repo_hash = gwt::index_worker::detect_repo_hash(project_root)?;
    let path = issue_link_cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let bytes = std::fs::read(path).ok()?;
    let store = serde_json::from_slice::<IssueBranchLinkStore>(&bytes).ok()?;
    store.branches.get(branch_name).copied()
}

pub(super) fn save_start_work_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: &str,
    linked_issue_number: Option<u64>,
    canonical_owner: Option<gwt::cli::execution_state::ExecutionOwnerKey>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    live_session_ids: &std::collections::HashSet<String>,
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

pub(super) fn save_resumed_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    workspace_resume_context: &WorkspaceResumeContext,
    live_session_ids: &std::collections::HashSet<String>,
) -> Result<(), String> {
    save_workspace_launch_projection(
        project_root,
        session,
        base_branch,
        linked_issue_number,
        None,
        Some(workspace_resume_context),
        WorkspaceLaunchProjectionKind::Resume {
            created_by_start_work: session.branch_name.starts_with("work/"),
        },
        live_session_ids,
    )
}

/// Build the lifecycle-event payload without copying the unbounded historical
/// Work and journal vectors. The cache remains the owner of those vectors; a
/// later background materialization sends the complete projection.
fn bounded_active_work_agent_snapshot(
    cached: &gwt::ActiveWorkAgentView,
) -> gwt::ActiveWorkAgentView {
    gwt::ActiveWorkAgentView {
        session_id: cached.session_id.clone(),
        window_id: cached.window_id.clone(),
        agent_id: cached.agent_id.clone(),
        display_name: cached.display_name.clone(),
        affiliation_status: cached.affiliation_status.clone(),
        workspace_id: cached.workspace_id.clone(),
        status_category: cached.status_category.clone(),
        current_focus: cached.current_focus.clone(),
        title_summary: cached.title_summary.clone(),
        branch: cached.branch.clone(),
        worktree_path: cached.worktree_path.clone(),
        last_board_entry_id: cached.last_board_entry_id.clone(),
        last_board_entry_kind: cached.last_board_entry_kind.clone(),
        coordination_scope: cached.coordination_scope.clone(),
        updated_at: cached.updated_at.clone(),
        sessions: Vec::new(),
    }
}

fn bounded_active_workspace_work_snapshot(
    cached: &gwt::ActiveWorkspaceWorkView,
) -> gwt::ActiveWorkspaceWorkView {
    gwt::ActiveWorkspaceWorkView {
        id: cached.id.clone(),
        title: cached.title.clone(),
        work_summary: cached.work_summary.clone(),
        status_category: cached.status_category.clone(),
        status_text: cached.status_text.clone(),
        owner: cached.owner.clone(),
        lifecycle_state: cached.lifecycle_state.clone(),
        closed_at: cached.closed_at.clone(),
        manual_close_allowed: cached.manual_close_allowed,
        close_blocked_reason: cached.close_blocked_reason.clone(),
        agents: cached
            .agents
            .iter()
            .map(bounded_active_work_agent_snapshot)
            .collect(),
        execution_diagnosis: cached.execution_diagnosis.clone(),
        updated_at: cached.updated_at.clone(),
    }
}

fn bounded_active_work_item_snapshot(cached: &gwt::ActiveWorkItemView) -> gwt::ActiveWorkItemView {
    gwt::ActiveWorkItemView {
        id: cached.id.clone(),
        title: cached.title.clone(),
        status_category: cached.status_category.clone(),
        status_text: cached.status_text.clone(),
        summary: cached.summary.clone(),
        progress_summary: cached.progress_summary.clone(),
        work_summary: cached.work_summary.clone(),
        owner: cached.owner.clone(),
        next_action: cached.next_action.clone(),
        active_agents: cached.active_agents,
        blocked_agents: cached.blocked_agents,
        branch: cached.branch.clone(),
        worktree_path: cached.worktree_path.clone(),
        managed_hook_health: cached.managed_hook_health.clone(),
        pr_number: cached.pr_number,
        pr_url: cached.pr_url.clone(),
        pr_state: cached.pr_state.clone(),
        board_refs: cached.board_refs.clone(),
        agents: cached
            .agents
            .iter()
            .map(bounded_active_work_agent_snapshot)
            .collect(),
        works: cached
            .works
            .iter()
            .map(bounded_active_workspace_work_snapshot)
            .collect(),
        lifecycle_state: cached.lifecycle_state.clone(),
        closed_at: cached.closed_at.clone(),
        session_agent_total: cached.session_agent_total,
        updated_at: cached.updated_at.clone(),
        merged_into_base: cached.merged_into_base,
        workspace_key: cached.workspace_key.clone(),
        remote_only: cached.remote_only,
        done_equivalent: cached.done_equivalent,
        cleanup_candidate: cached.cleanup_candidate.clone(),
        cleanup_blocked_reason: cached.cleanup_blocked_reason.clone(),
    }
}

fn bounded_active_work_projection_snapshot(
    cached: &gwt::ActiveWorkProjectionView,
) -> gwt::ActiveWorkProjectionView {
    gwt::ActiveWorkProjectionView {
        id: cached.id.clone(),
        title: cached.title.clone(),
        status_category: cached.status_category.clone(),
        status_text: cached.status_text.clone(),
        summary: cached.summary.clone(),
        progress_summary: cached.progress_summary.clone(),
        owner: cached.owner.clone(),
        next_action: cached.next_action.clone(),
        active_agents: cached.active_agents,
        blocked_agents: cached.blocked_agents,
        branch: cached.branch.clone(),
        worktree_path: cached.worktree_path.clone(),
        pr_number: cached.pr_number,
        pr_url: cached.pr_url.clone(),
        pr_state: cached.pr_state.clone(),
        pr_created_at: cached.pr_created_at.clone(),
        board_refs: cached.board_refs.clone(),
        journal_entries: Vec::new(),
        works: Vec::new(),
        cleanup_candidate: cached.cleanup_candidate.clone(),
        managed_hook_health: cached.managed_hook_health.clone(),
        active_work_count: cached.active_work_count,
        active_works: cached
            .active_works
            .iter()
            .map(bounded_active_work_item_snapshot)
            .collect(),
        agents: cached
            .agents
            .iter()
            .map(bounded_active_work_agent_snapshot)
            .collect(),
        unassigned_agents: cached
            .unassigned_agents
            .iter()
            .map(bounded_active_work_agent_snapshot)
            .collect(),
    }
}

type CachedAgentPool = HashMap<String, VecDeque<gwt::ActiveWorkAgentView>>;

fn take_cached_agents(agents: &mut Vec<gwt::ActiveWorkAgentView>, pool: &mut CachedAgentPool) {
    for agent in std::mem::take(agents) {
        pool.entry(agent.session_id.clone())
            .or_default()
            .push_back(agent);
    }
}

fn take_replaced_cached_agents(
    agents: &mut Vec<gwt::ActiveWorkAgentView>,
    replaced_session_ids: &HashSet<String>,
    pool: &mut CachedAgentPool,
) {
    let mut retained = Vec::with_capacity(agents.len());
    for agent in std::mem::take(agents) {
        if replaced_session_ids.contains(&agent.session_id) {
            pool.entry(agent.session_id.clone())
                .or_default()
                .push_back(agent);
        } else {
            retained.push(agent);
        }
    }
    *agents = retained;
}

fn move_cached_agents_into_fresh(
    fresh: &mut Vec<gwt::ActiveWorkAgentView>,
    pool: &mut CachedAgentPool,
) {
    let mut reconciled = Vec::with_capacity(fresh.len());
    for fresh_agent in std::mem::take(fresh) {
        let cached_agent = pool
            .get_mut(&fresh_agent.session_id)
            .and_then(|agents| agents.pop_front());
        if let Some(mut cached_agent) = cached_agent {
            let sessions = std::mem::take(&mut cached_agent.sessions);
            cached_agent = fresh_agent;
            cached_agent.sessions = sessions;
            reconciled.push(cached_agent);
        } else {
            reconciled.push(fresh_agent);
        }
    }
    *fresh = reconciled;
}

fn append_fresh_agents(
    retained: &mut Vec<gwt::ActiveWorkAgentView>,
    mut fresh: Vec<gwt::ActiveWorkAgentView>,
) {
    let fresh_session_ids = fresh
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect::<HashSet<_>>();
    retained.retain(|agent| !fresh_session_ids.contains(agent.session_id.as_str()));
    retained.append(&mut fresh);
}

fn cached_workspace_group_key(work: &gwt::ActiveWorkItemView, project_root: &Path) -> String {
    work.workspace_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .or_else(|| {
            gwt_core::workspace_projection::canonical_work_id(
                project_root,
                work.branch.as_deref(),
                None,
            )
        })
        .or_else(|| {
            gwt_core::workspace_projection::canonical_work_id(
                project_root,
                None,
                work.worktree_path.as_deref().map(Path::new),
            )
        })
        .unwrap_or_else(|| work.id.clone())
}

fn refresh_cached_workspace_runtime_state(work: &mut gwt::ActiveWorkItemView) {
    for child in &mut work.works {
        let active = child
            .agents
            .iter()
            .filter(|agent| matches!(agent.status_category.as_str(), "active" | "running"))
            .count();
        let blocked = child
            .agents
            .iter()
            .filter(|agent| agent.status_category == "blocked")
            .count();
        if blocked > 0 {
            child.lifecycle_state = "active".to_string();
            child.status_category = "blocked".to_string();
            child.status_text = if blocked == 1 {
                "1 blocked agent".to_string()
            } else {
                format!("{blocked} blocked agents")
            };
            child.manual_close_allowed = false;
            child.close_blocked_reason = Some("live_agent".to_string());
        } else if active > 0 {
            child.lifecycle_state = "active".to_string();
            child.status_category = "active".to_string();
            child.status_text = if active == 1 {
                "1 active agent".to_string()
            } else {
                format!("{active} active agents")
            };
            child.manual_close_allowed = false;
            child.close_blocked_reason = Some("live_agent".to_string());
        } else if child.lifecycle_state == "active" {
            child.lifecycle_state = "paused".to_string();
            child.status_category = "idle".to_string();
            child.status_text = "Paused".to_string();
            if child.close_blocked_reason.as_deref() != Some("remote_environment_unknown") {
                child.manual_close_allowed = true;
                child.close_blocked_reason = None;
            }
        }
    }

    recompute_active_work_agent_counters(work);
    if work.blocked_agents > 0 {
        work.lifecycle_state = "active".to_string();
        work.status_category = "blocked".to_string();
        work.status_text = if work.blocked_agents == 1 {
            "1 blocked agent".to_string()
        } else {
            format!("{} blocked agents", work.blocked_agents)
        };
    } else if work.active_agents > 0 {
        work.lifecycle_state = "active".to_string();
        work.status_category = "active".to_string();
        work.status_text = if work.active_agents == 1 {
            "1 active agent".to_string()
        } else {
            format!("{} active agents", work.active_agents)
        };
    } else if work.lifecycle_state == "active" {
        work.lifecycle_state = "paused".to_string();
        work.status_category = "idle".to_string();
        work.status_text = "Paused".to_string();
        work.next_action = None;
    }
}

fn merge_fresh_cached_child(
    target: &mut gwt::ActiveWorkspaceWorkView,
    mut fresh: gwt::ActiveWorkspaceWorkView,
) {
    let retained_work_summary = target.work_summary.take();
    let retained_owner = target.owner.take();
    let retained_diagnosis = target.execution_diagnosis.take();
    let retained_remote_guard =
        if target.close_blocked_reason.as_deref() == Some("remote_environment_unknown") {
            target.close_blocked_reason.clone()
        } else {
            None
        };

    append_fresh_agents(&mut target.agents, std::mem::take(&mut fresh.agents));
    target.status_category = fresh.status_category;
    target.status_text = fresh.status_text;
    target.work_summary = fresh.work_summary.or(retained_work_summary);
    target.owner = fresh.owner.or(retained_owner);
    target.lifecycle_state = fresh.lifecycle_state;
    target.closed_at = fresh.closed_at;
    target.manual_close_allowed = fresh.manual_close_allowed;
    target.close_blocked_reason = fresh.close_blocked_reason;
    target.execution_diagnosis = retained_diagnosis.or(fresh.execution_diagnosis);
    target.updated_at = fresh.updated_at;
    if target.manual_close_allowed && retained_remote_guard.is_some() {
        target.manual_close_allowed = false;
        target.close_blocked_reason = retained_remote_guard;
    }
}

fn merge_fresh_cached_root(
    target: &mut gwt::ActiveWorkItemView,
    mut fresh: gwt::ActiveWorkItemView,
    previous_child_by_session: &HashMap<String, String>,
    cached_child_ids: &HashSet<String>,
) {
    let retained_summary = target.summary.take();
    let retained_progress_summary = target.progress_summary.take();
    let retained_work_summary = target.work_summary.take();
    let retained_owner = target.owner.take();
    let retained_branch = target.branch.take();
    let retained_worktree_path = target.worktree_path.take();
    let retained_pr_url = target.pr_url.take();
    let retained_pr_state = target.pr_state.take();
    let retained_board_refs = std::mem::take(&mut target.board_refs);

    append_fresh_agents(&mut target.agents, std::mem::take(&mut fresh.agents));
    let mut target_child_index = target
        .works
        .iter()
        .enumerate()
        .map(|(index, child)| (child.id.clone(), index))
        .collect::<HashMap<_, _>>();
    for mut fresh_child in std::mem::take(&mut fresh.works) {
        let routed_child_id = fresh_child.agents.iter().find_map(|agent| {
            previous_child_by_session
                .get(&agent.session_id)
                .or_else(|| {
                    agent
                        .workspace_id
                        .as_ref()
                        .filter(|workspace_id| cached_child_ids.contains(workspace_id.as_str()))
                })
                .cloned()
        });
        let target_index = routed_child_id
            .as_deref()
            .and_then(|id| target_child_index.get(id).copied())
            .or_else(|| target_child_index.get(&fresh_child.id).copied());
        if let Some(index) = target_index {
            merge_fresh_cached_child(&mut target.works[index], fresh_child);
        } else {
            if let Some(routed_child_id) = routed_child_id {
                fresh_child.id = routed_child_id;
            }
            target_child_index.insert(fresh_child.id.clone(), target.works.len());
            target.works.push(fresh_child);
        }
    }

    target.status_category = fresh.status_category;
    target.status_text = fresh.status_text;
    target.summary = fresh.summary.or(retained_summary);
    target.progress_summary = fresh.progress_summary.or(retained_progress_summary);
    target.work_summary = fresh.work_summary.or(retained_work_summary);
    target.owner = fresh.owner.or(retained_owner);
    target.next_action = fresh.next_action;
    target.branch = fresh.branch.or(retained_branch);
    target.worktree_path = fresh.worktree_path.or(retained_worktree_path);
    target.pr_number = fresh.pr_number.or(target.pr_number);
    target.pr_url = fresh.pr_url.or(retained_pr_url);
    target.pr_state = fresh.pr_state.or(retained_pr_state);
    if fresh.board_refs.is_empty() {
        target.board_refs = retained_board_refs;
    } else {
        target.board_refs = fresh.board_refs;
    }
    target.lifecycle_state = fresh.lifecycle_state;
    target.closed_at = fresh.closed_at;
    target.session_agent_total = target.session_agent_total.max(fresh.session_agent_total);
    target.updated_at = fresh.updated_at;
}

/// Reconcile only the authoritative agent membership carried by `fresh`.
/// Historical Works/journal remain owned by the cached projection and are
/// neither cloned nor fed back through the full projection builder.
fn merge_workspace_projection_membership_cache_only(
    cached: &mut gwt::ActiveWorkProjectionView,
    project_root: &Path,
    fresh: &gwt_core::workspace_projection::WorkspaceProjection,
) {
    let previous_authoritative_session_ids = cached
        .agents
        .iter()
        .chain(cached.unassigned_agents.iter())
        .map(|agent| agent.session_id.clone())
        .collect::<HashSet<_>>();
    let fresh_session_ids = fresh
        .agents
        .iter()
        .map(|agent| agent.session_id.clone())
        .collect::<HashSet<_>>();
    let replaced_session_ids = previous_authoritative_session_ids
        .union(&fresh_session_ids)
        .cloned()
        .collect::<HashSet<_>>();

    let mut previous_child_by_session = HashMap::new();
    let mut cached_child_ids = HashSet::new();
    for work in &cached.active_works {
        for child in &work.works {
            cached_child_ids.insert(child.id.clone());
            for agent in &child.agents {
                if fresh_session_ids.contains(&agent.session_id) {
                    previous_child_by_session
                        .entry(agent.session_id.clone())
                        .or_insert_with(|| child.id.clone());
                }
            }
        }
    }

    let mut top_level_agents = CachedAgentPool::new();
    take_cached_agents(&mut cached.agents, &mut top_level_agents);
    take_cached_agents(&mut cached.unassigned_agents, &mut top_level_agents);

    let mut nested_agents = CachedAgentPool::new();
    for work in &mut cached.active_works {
        take_replaced_cached_agents(&mut work.agents, &replaced_session_ids, &mut nested_agents);
        for child in &mut work.works {
            take_replaced_cached_agents(
                &mut child.agents,
                &replaced_session_ids,
                &mut nested_agents,
            );
        }
    }

    // `WorkspaceProjection` is the current-state record (metadata plus
    // `WorkspaceAgentSummary` membership); it has no Work/journal/Session
    // history fields. This clone is therefore proportional only to fresh
    // visible membership.
    let mut fresh_view =
        active_work_projection_from_saved_with_journal(fresh.clone(), Vec::new(), Vec::new(), None);
    assign_and_merge_workspace_groups_cache_only(&mut fresh_view.active_works, project_root);
    move_cached_agents_into_fresh(&mut fresh_view.agents, &mut top_level_agents);
    move_cached_agents_into_fresh(&mut fresh_view.unassigned_agents, &mut top_level_agents);
    for work in &mut fresh_view.active_works {
        move_cached_agents_into_fresh(&mut work.agents, &mut nested_agents);
        for child in &mut work.works {
            move_cached_agents_into_fresh(&mut child.agents, &mut nested_agents);
        }
    }

    cached.id = fresh_view.id;
    cached.title = fresh_view.title;
    cached.status_category = fresh_view.status_category;
    cached.status_text = fresh_view.status_text;
    cached.summary = fresh_view.summary;
    cached.progress_summary = fresh_view.progress_summary;
    cached.owner = fresh_view.owner;
    cached.next_action = fresh_view.next_action;
    cached.active_agents = fresh_view.active_agents;
    cached.blocked_agents = fresh_view.blocked_agents;
    cached.branch = fresh_view.branch;
    cached.worktree_path = fresh_view.worktree_path;
    cached.pr_number = fresh_view.pr_number;
    cached.pr_url = fresh_view.pr_url;
    cached.pr_state = fresh_view.pr_state;
    cached.pr_created_at = fresh_view.pr_created_at;
    cached.board_refs = fresh_view.board_refs;
    cached.agents = fresh_view.agents;
    cached.unassigned_agents = fresh_view.unassigned_agents;

    let mut root_index = cached
        .active_works
        .iter()
        .enumerate()
        .map(|(index, work)| (cached_workspace_group_key(work, project_root), index))
        .collect::<HashMap<_, _>>();
    for fresh_root in fresh_view.active_works {
        let key = cached_workspace_group_key(&fresh_root, project_root);
        if let Some(&index) = root_index.get(&key) {
            merge_fresh_cached_root(
                &mut cached.active_works[index],
                fresh_root,
                &previous_child_by_session,
                &cached_child_ids,
            );
        } else {
            root_index.insert(key, cached.active_works.len());
            cached.active_works.push(fresh_root);
        }
    }
    for work in &mut cached.active_works {
        refresh_cached_workspace_runtime_state(work);
    }
    cached.active_work_count = cached.active_works.len();
}

impl AppRuntime {
    /// SPEC-2359 US-41 (FR-153, FR-154, FR-155): handle
    /// [`FrontendEvent::WorkspaceProjectionPrune`] by classifying every
    /// projection under `~/.gwt/projects/`, applying or previewing the plan,
    /// and replying with a count summary or an error.
    ///
    /// Note: `is_active_session` is `|_| false` here as a first-pass; a
    /// follow-up commit will bridge the live-window registry so currently
    /// running Agents block their owning Workspace from prune.
    pub(super) fn workspace_projection_prune_events(
        &self,
        client_id: ClientId,
        dry_run: bool,
        ids: Vec<String>,
    ) -> Vec<OutboundEvent> {
        use gwt_core::paths::gwt_projects_dir;
        use gwt_core::workspace_projection::{
            apply_prune_plan, classify_workspace_projections, WorkspaceRetentionConfig,
        };

        let scan_root = gwt_projects_dir();
        let now = chrono::Utc::now();
        let config = WorkspaceRetentionConfig::default();
        let live_session_ids: std::collections::HashSet<String> =
            self.active_agent_sessions.keys().cloned().collect();
        let is_active_session =
            |projection: &gwt_core::workspace_projection::WorkspaceProjection| {
                projection
                    .agents
                    .iter()
                    .any(|agent| live_session_ids.contains(&agent.session_id))
            };
        let plan = classify_workspace_projections(&scan_root, &config, now, is_active_session);
        let filtered: Vec<_> = if ids.is_empty() {
            plan
        } else {
            plan.into_iter()
                .filter(|item| ids.iter().any(|id| id == &item.workspace_id))
                .collect()
        };

        match apply_prune_plan(&filtered, dry_run) {
            Ok(summary) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::WorkspaceProjectionPruneResult {
                    mode: if dry_run {
                        "dry_run".to_string()
                    } else {
                        "applied".to_string()
                    },
                    archived: summary.archived,
                    deleted: summary.deleted,
                    skipped: summary.skipped,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::WorkspaceProjectionPruneError {
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(super) fn active_work_projection_reply(&self, client_id: &str) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self.cached_or_in_memory_active_work_projection_for_tab(tab_id, tab);
        Some(OutboundEvent::reply(
            client_id,
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    fn in_memory_active_work_projection_for_tab(
        &self,
        tab_id: &str,
        tab: &ProjectTabRuntime,
    ) -> gwt::ActiveWorkProjectionView {
        let sessions = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            return empty_active_work_projection_view(tab_id, tab);
        }
        active_work_projection_from_live_sessions(tab_id, tab, &sessions, None)
            .unwrap_or_else(|| empty_active_work_projection_view(tab_id, tab))
    }

    fn cached_or_in_memory_active_work_projection_for_tab(
        &self,
        tab_id: &str,
        tab: &ProjectTabRuntime,
    ) -> gwt::ActiveWorkProjectionView {
        self.active_work_projection_cache
            .borrow()
            .get(tab_id)
            .cloned()
            .unwrap_or_else(|| self.in_memory_active_work_projection_for_tab(tab_id, tab))
    }

    /// Apply the accepted in-memory stop to the already materialized Work view
    /// without touching disk. Durable WorkItems/Session reconciliation follows
    /// on the background close finalizer, but the immediate cache replay must
    /// never advertise the detached agent as still active.
    pub(crate) fn mark_cached_active_work_session_stopped(
        &self,
        tab_id: &str,
        session_id: &str,
        window_id: &str,
    ) {
        fn mark_agent_stopped(
            agent: &mut gwt::ActiveWorkAgentView,
            session_id: &str,
            window_id: &str,
        ) {
            if agent.session_id != session_id && agent.window_id.as_deref() != Some(window_id) {
                return;
            }
            agent.window_id = None;
            agent.status_category = "idle".to_string();
            for session in &mut agent.sessions {
                session.is_active = false;
            }
        }

        let mut cache = self.active_work_projection_cache.borrow_mut();
        let Some(projection) = cache.get_mut(tab_id) else {
            return;
        };
        for agent in projection
            .agents
            .iter_mut()
            .chain(projection.unassigned_agents.iter_mut())
        {
            mark_agent_stopped(agent, session_id, window_id);
        }
        for work in &mut projection.active_works {
            for agent in &mut work.agents {
                mark_agent_stopped(agent, session_id, window_id);
            }
            for child in &mut work.works {
                for agent in &mut child.agents {
                    mark_agent_stopped(agent, session_id, window_id);
                }
                let child_active_agents = child
                    .agents
                    .iter()
                    .filter(|agent| agent.status_category == "active")
                    .count();
                let child_blocked_agents = child
                    .agents
                    .iter()
                    .filter(|agent| agent.status_category == "blocked")
                    .count();
                if child_blocked_agents > 0 {
                    child.lifecycle_state = "active".to_string();
                    child.status_category = "blocked".to_string();
                    child.manual_close_allowed = false;
                    child.close_blocked_reason = Some("live_agent".to_string());
                    child.status_text = if child_blocked_agents == 1 {
                        "1 blocked agent".to_string()
                    } else {
                        format!("{child_blocked_agents} blocked agents")
                    };
                } else if child_active_agents == 0 && child.lifecycle_state == "active" {
                    child.lifecycle_state = "paused".to_string();
                    child.status_category = "idle".to_string();
                    if child.status_text.trim().is_empty() {
                        child.status_text = "Paused".to_string();
                    }
                    if child.close_blocked_reason.as_deref() != Some("remote_environment_unknown") {
                        child.manual_close_allowed = true;
                        child.close_blocked_reason = None;
                    }
                }
            }
            work.active_agents = work
                .agents
                .iter()
                .filter(|agent| agent.status_category == "active")
                .count();
            work.blocked_agents = work
                .agents
                .iter()
                .filter(|agent| agent.status_category == "blocked")
                .count();
            if work.blocked_agents > 0 {
                work.lifecycle_state = "active".to_string();
                work.status_category = "blocked".to_string();
                work.status_text = if work.blocked_agents == 1 {
                    "1 blocked agent".to_string()
                } else {
                    format!("{} blocked agents", work.blocked_agents)
                };
            } else if work.active_agents == 0 && work.lifecycle_state == "active" {
                work.lifecycle_state = "paused".to_string();
                work.status_category = "idle".to_string();
                if work.status_text.trim().is_empty()
                    || work.status_text.ends_with(" active agent")
                    || work.status_text.ends_with(" active agents")
                {
                    work.status_text = "Paused".to_string();
                }
                work.next_action = None;
            }
        }
        projection.active_agents = projection
            .agents
            .iter()
            .filter(|agent| agent.status_category == "active")
            .count();
        projection.blocked_agents = projection
            .agents
            .iter()
            .filter(|agent| agent.status_category == "blocked")
            .count();
        if projection.blocked_agents > 0 {
            projection.status_category = "blocked".to_string();
            projection.status_text = if projection.blocked_agents == 1 {
                "1 blocked agent".to_string()
            } else {
                format!("{} blocked agents", projection.blocked_agents)
            };
        } else if projection.active_agents == 0 {
            projection.status_category = "idle".to_string();
            projection.status_text = "Paused".to_string();
            projection.next_action = None;
        } else {
            projection.status_text = if projection.active_agents == 1 {
                "1 active agent".to_string()
            } else {
                format!("{} active agents", projection.active_agents)
            };
        }
    }

    /// Merge one disk-watcher payload into the already materialized Active
    /// Work cache without touching Session, WorkItems, journal, Git, or hook
    /// health stores. The watcher has already paid to load `current.json`, so
    /// its per-agent state is fresh; the cache retains the expensive history
    /// and enrichment assembled by the background projection builder.
    pub(crate) fn merge_workspace_projection_into_cached_active_work(
        &self,
        project_root: &Path,
        fresh: &gwt_core::workspace_projection::WorkspaceProjection,
    ) {
        let Some(tab_id) = self
            .tabs
            .iter()
            .find(|tab| projection_worktree_paths_match(&tab.project_root, project_root))
            .map(|tab| tab.id.clone())
        else {
            return;
        };
        let mut cache = self.active_work_projection_cache.borrow_mut();
        if let Some(projection) = cache.get_mut(&tab_id) {
            merge_workspace_projection_membership_cache_only(projection, project_root, fresh);
            return;
        }

        // A watcher can win the race with the first full background
        // materialization. Seed the cache from its authoritative current-state
        // membership instead of falling back to possibly stale live-session
        // bookkeeping. `WorkspaceProjection` contains no historical Session,
        // Work, or journal vectors, so this cold-cache construction is bounded
        // by visible membership.
        let mut projection = active_work_projection_from_saved_with_journal(
            fresh.clone(),
            Vec::new(),
            Vec::new(),
            None,
        );
        assign_and_merge_workspace_groups_cache_only(&mut projection.active_works, project_root);
        cache.insert(tab_id, projection);
    }

    /// Rebuild the cache for the project whose background completion just
    /// arrived. Completion events can belong to an inactive tab; rebuilding
    /// only the active tab leaves the target cache stale, while tab-change is
    /// intentionally cache-only to keep the GUI event path process-free.
    pub(crate) fn refresh_active_work_projection_for_project_root(
        &self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| projection_worktree_paths_match(&tab.project_root, project_root))
        else {
            return Vec::new();
        };
        let tab_id = tab.id.clone();
        let projection = self.active_work_projection_for_tab(&tab_id, tab);
        if self.active_tab_id.as_deref() != Some(tab_id.as_str()) {
            return Vec::new();
        }
        projection
            .map(|projection| {
                vec![OutboundEvent::broadcast(
                    BackendEvent::ActiveWorkProjection {
                        projection: Box::new(projection),
                    },
                )]
            })
            .unwrap_or_default()
    }

    pub(crate) fn active_work_projection_broadcast_for_active_tab(&self) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self.active_work_projection_for_tab(tab_id, tab)?;
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    /// Issue #3783: lifecycle acknowledgements must not enter the disk-backed
    /// projection builder. The authoritative Work/Session files are updated by
    /// the close finalizer and their normal background refresh replaces this
    /// cache snapshot after the acknowledgement is already on the wire.
    pub(crate) fn cached_active_work_projection_broadcast_for_active_tab(
        &self,
    ) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let has_cached_projection = self
            .active_work_projection_cache
            .borrow()
            .contains_key(tab_id);
        let has_live_session = self
            .active_agent_sessions
            .values()
            .any(|session| session.tab_id == *tab_id);
        if !has_cached_projection && !has_live_session {
            return None;
        }
        let cached_projection = self
            .active_work_projection_cache
            .borrow()
            .get(tab_id)
            .map(bounded_active_work_projection_snapshot);
        let projection = cached_projection
            .unwrap_or_else(|| self.in_memory_active_work_projection_for_tab(tab_id, tab));
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjectionPatch {
                projection: Box::new(projection),
            },
        ))
    }

    /// A Workspace watcher notification carries the same bounded membership
    /// patch as a lifecycle acknowledgement. The browser preserves its
    /// existing history for the exact projection id, so Tao neither clones nor
    /// serializes unbounded Work/Session vectors here.
    pub(crate) fn cached_active_work_projection_broadcast_for_workspace_watcher(
        &self,
    ) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let cached_projection = self
            .active_work_projection_cache
            .borrow()
            .get(tab_id)
            .map(bounded_active_work_projection_snapshot);
        let projection = cached_projection
            .unwrap_or_else(|| self.in_memory_active_work_projection_for_tab(tab_id, tab));
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjectionPatch {
                projection: Box::new(projection),
            },
        ))
    }

    /// Materialize a bounded projection from process-local state only. This is
    /// used by auto-close paths that must emit an authoritative empty/updated
    /// Work surface even when no prior projection cache exists.
    pub(crate) fn in_memory_active_work_projection_broadcast_for_active_tab(
        &self,
    ) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjectionPatch {
                projection: Box::new(self.in_memory_active_work_projection_for_tab(tab_id, tab)),
            },
        ))
    }

    /// Like `active_work_projection_broadcast_for_active_tab`, but always emits an event
    /// when an active tab exists — falling back to an empty projection so that frontends
    /// clear stale per-project data when the tab focus moves to a project without
    /// any saved projection or live agent sessions.
    pub(super) fn active_work_projection_broadcast_on_tab_change(&self) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self.cached_or_in_memory_active_work_projection_for_tab(tab_id, tab);
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    pub(super) fn active_work_projection_for_tab(
        &self,
        tab_id: &str,
        tab: &ProjectTabRuntime,
    ) -> Option<gwt::ActiveWorkProjectionView> {
        #[cfg(test)]
        FULL_ACTIVE_WORK_PROJECTION_BUILDS.with(|count| count.set(count.get() + 1));
        let sessions = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .collect::<Vec<_>>();
        let saved_projection =
            gwt_core::workspace_projection::load_workspace_projection(&tab.project_root)
                .ok()
                .flatten();
        // SPEC-2359 Phase W-15 (FR-379/FR-382): the Workspace list is the
        // union of existing worktrees and unclosed records, independent of
        // live agents and of whether the project was ever launched here. When
        // no projection has been saved yet (fresh home / never-launched
        // project) but Work records exist (e.g. worktree backfill), synthesize
        // a default projection so the records still surface.
        let loaded_projection = saved_projection.or_else(|| {
            self.work_items_cache
                .borrow_mut()
                .load_or_synthesize(&tab.project_root)
                .ok()
                .filter(|works| !works.work_items.is_empty())
                .map(|_| {
                    gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                        &tab.project_root,
                    )
                })
        });
        if let Some(projection) = loaded_projection {
            let mut projection = projection;
            let had_saved_agents = !projection.agents.is_empty();
            let cleanup_candidate =
                workspace_cleanup_candidate_for_projection(&projection, &sessions);
            merge_active_sessions_into_projection(
                &mut projection,
                sessions.iter().copied(),
                chrono::Utc::now(),
            );
            let updated_at = chrono::Utc::now();
            retain_live_workspace_agents(&mut projection, &sessions, updated_at);
            // SPEC-2359 US-80 (FR-428): derive each Shell Work's status from its
            // live PTY — running → Active, otherwise (exited or post-restart) →
            // Idle — so the rail never shows a dead shell as Active.
            projection.reconcile_shell_status(
                |window_id| {
                    matches!(
                        self.window_pty_statuses.get(window_id),
                        Some(crate::WindowProcessStatus::Running)
                    )
                },
                updated_at,
            );
            if had_saved_agents && !projection.has_current_agents() {
                projection.reset_idle_identity(&tab.title, updated_at);
            }
            let journal_entries =
                gwt_core::workspace_projection::load_recent_workspace_journal_entries(
                    &tab.project_root,
                    WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
                )
                .unwrap_or_default()
                .iter()
                .map(workspace_journal_entry_view_from_entry)
                .collect::<Vec<_>>();
            let agent_sessions = self
                .session_ledger_cache
                .borrow_mut()
                .load(&self.sessions_dir);
            let session_index = work_session_index(&agent_sessions);
            // Issue #3611: resumability is answered from the background merge
            // scan's branch snapshot. Probing branches here would spawn Git
            // once per Session on the event-loop thread.
            let resume_branches =
                ResumeBranchIndex::scanned(self.work_known_branch_refs.get(&tab.project_root));
            // Current and WorkItems share the stable Project State identity.
            // The exact worktree is an event destination, never a second
            // WorkItems discovery root.
            let work_items = self
                .work_items_cache
                .borrow_mut()
                .load_or_synthesize(&tab.project_root)
                .map(|items| items.work_items)
                .unwrap_or_default();
            let workspaces = work_items
                .iter()
                .map(|item| {
                    workspace_work_item_view_from_item(item, &session_index, resume_branches)
                })
                .collect::<Vec<_>>();
            let mut view = active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                cleanup_candidate,
            );
            view.managed_hook_health = managed_hook_health_view_for_project(
                &tab.project_root,
                &self.sessions_dir,
                &sessions,
            );
            // SPEC-2359 W16-2 (FR-389): group Works sharing a canonical
            // branch into one Workspace row before the ledger attach, so the
            // attach / identity-collapse / cap run once per Workspace.
            assign_and_merge_workspace_groups(&mut view.active_works, &tab.project_root);
            // SPEC-2359 Phase W-16 (FR-402): attach the machine-local session
            // ledger to each Workspace (branch) row so sessions surface even
            // when works.json never recorded an agent for the branch.
            attach_registry_sessions_to_active_works(
                &mut view.active_works,
                &agent_sessions,
                gwt_core::repo_hash::detect_repo_hash(&tab.project_root),
                &session_index,
                resume_branches,
            );
            attach_managed_hook_health_to_active_works(
                &mut view.active_works,
                &self.sessions_dir,
                &sessions,
            );
            // SPEC-2359 W-15 (FR-386): "safe to delete" badge inputs — the
            // background merge-scan cache plus the recorded PR state.
            let dirty_branches = self.work_dirty_branches.get(&tab.project_root);
            mark_merged_active_works(
                &mut view.active_works,
                self.work_merged_branches.get(&tab.project_root),
                dirty_branches,
            );
            // SPEC-3075: fill the rail summary — PR title (top), then the
            // AI-polished summary (FR-006), then the raw branch tip commit
            // subject for Works with no recorded purpose (all from background
            // scan caches).
            apply_work_summary_external_sources(
                &mut view.active_works,
                self.work_pr_titles.get(&tab.project_root),
                self.work_ai_summaries.get(&tab.project_root),
                self.work_tip_subjects.get(&tab.project_root),
            );
            // SPEC-2359 W16-3 (FR-390): "Remote" rows — branch known only
            // from fetched refs, no local worktree (cache lookup only).
            mark_remote_only_active_works(
                &mut view.active_works,
                self.local_worktree_branches.borrow().get(&tab.project_root),
            );
            let cleanup_ready_branches = self.work_cleanup_ready_branches.get(&tab.project_root);
            let live_process_branches = self.work_live_process_branches.get(&tab.project_root);
            if view.cleanup_candidate.as_ref().is_some_and(|candidate| {
                cleanup_candidate_has_live_process(candidate, live_process_branches)
            }) {
                view.cleanup_candidate = None;
            }
            mark_workspace_cleanup_candidates(
                &mut view.active_works,
                cleanup_ready_branches,
                dirty_branches,
                &sessions,
                live_process_branches,
            );
            self.active_work_projection_cache
                .borrow_mut()
                .insert(tab_id.to_string(), view.clone());
            return Some(view);
        }

        let mut view = active_work_projection_from_live_sessions(
            tab_id,
            tab,
            &sessions,
            managed_hook_health_view_for_project(&tab.project_root, &self.sessions_dir, &sessions),
        );
        if let Some(view) = view.as_mut() {
            attach_managed_hook_health_to_active_works(
                &mut view.active_works,
                &self.sessions_dir,
                &sessions,
            );
        }
        let mut cache = self.active_work_projection_cache.borrow_mut();
        if let Some(view) = view.as_ref() {
            cache.insert(tab_id.to_string(), view.clone());
        } else {
            cache.remove(tab_id);
        }
        view
    }

    pub(crate) fn handle_workspace_projection_changed_events(
        &mut self,
        project_root: &Path,
        projection: &gwt_core::workspace_projection::WorkspaceProjection,
    ) -> Vec<OutboundEvent> {
        self.apply_workspace_projection_title_sync_cache_only(project_root, projection)
    }
}

#[cfg(test)]
thread_local! {
    static FULL_ACTIVE_WORK_PROJECTION_BUILDS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_full_active_work_projection_builds() {
    FULL_ACTIVE_WORK_PROJECTION_BUILDS.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn full_active_work_projection_builds() -> usize {
    FULL_ACTIVE_WORK_PROJECTION_BUILDS.with(std::cell::Cell::get)
}

#[cfg(test)]
mod bounded_cache_merge_tests {
    use super::*;

    fn agent(
        session_id: &str,
        branch: &str,
        status: gwt_core::workspace_projection::WorkspaceStatusCategory,
    ) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
        gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: Some(format!("tab-1::{session_id}")),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: status,
            current_focus: None,
            title_summary: None,
            worktree_path: Some(PathBuf::from(format!("/repo/{branch}"))),
            branch: Some(branch.to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        }
    }

    fn projection(
        project_root: &Path,
        agents: Vec<gwt_core::workspace_projection::WorkspaceAgentSummary>,
    ) -> gwt_core::workspace_projection::WorkspaceProjection {
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(project_root);
        projection.agents = agents;
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection
    }

    fn history(session_id: &str) -> gwt::WorkspaceHistorySessionView {
        gwt::WorkspaceHistorySessionView {
            agent_session_id: format!("conversation-{session_id}"),
            started_at: "2026-08-29T00:00:00Z".to_string(),
            is_active: true,
            resumable: true,
        }
    }

    fn install_history(
        view: &mut gwt::ActiveWorkProjectionView,
        session_id: &str,
        sessions: &[gwt::WorkspaceHistorySessionView],
    ) {
        for agent in view
            .agents
            .iter_mut()
            .chain(view.unassigned_agents.iter_mut())
        {
            if agent.session_id == session_id {
                agent.sessions = sessions.to_vec();
            }
        }
        for work in &mut view.active_works {
            for agent in &mut work.agents {
                if agent.session_id == session_id {
                    agent.sessions = sessions.to_vec();
                }
            }
            for child in &mut work.works {
                for agent in &mut child.agents {
                    if agent.session_id == session_id {
                        agent.sessions = sessions.to_vec();
                    }
                }
            }
        }
    }

    fn all_agents(
        view: &gwt::ActiveWorkProjectionView,
    ) -> impl Iterator<Item = &gwt::ActiveWorkAgentView> {
        view.agents
            .iter()
            .chain(view.unassigned_agents.iter())
            .chain(view.active_works.iter().flat_map(|work| work.agents.iter()))
            .chain(
                view.active_works
                    .iter()
                    .flat_map(|work| work.works.iter())
                    .flat_map(|work| work.agents.iter()),
            )
    }

    fn session_history_allocations(
        view: &gwt::ActiveWorkProjectionView,
        session_id: &str,
    ) -> Vec<usize> {
        let mut allocations = all_agents(view)
            .filter(|agent| agent.session_id == session_id && !agent.sessions.is_empty())
            .map(|agent| agent.sessions.as_ptr() as usize)
            .collect::<Vec<_>>();
        allocations.sort_unstable();
        allocations
    }

    #[test]
    fn cache_broadcast_snapshot_omits_unbounded_history_vectors() {
        let root = Path::new("/repo");
        let mut cached = active_work_projection_from_saved(projection(
            root,
            vec![agent(
                "session-live",
                "work/live",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            )],
        ));
        assign_and_merge_workspace_groups_cache_only(&mut cached.active_works, root);
        let retained_history = vec![history("session-live")];
        install_history(&mut cached, "session-live", &retained_history);
        cached.journal_entries.push(gwt::WorkspaceJournalEntryView {
            id: "journal-sentinel".to_string(),
            updated_at: "2026-08-29T00:00:00Z".to_string(),
            title: None,
            status_category: None,
            status_text: None,
            summary: None,
            progress_summary: None,
            owner: None,
            next_action: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        });
        cached.works.push(gwt::WorkspaceHistoryView {
            id: "history-sentinel".to_string(),
            title: "History sentinel".to_string(),
            intent: None,
            summary: None,
            progress_summary: None,
            status_category: "done".to_string(),
            owner: None,
            created_at: "2026-08-29T00:00:00Z".to_string(),
            updated_at: "2026-08-29T00:00:00Z".to_string(),
            completed_at: Some("2026-08-29T00:00:00Z".to_string()),
            agents: Vec::new(),
            execution_containers: Vec::new(),
            board_refs: Vec::new(),
            related_workspace_ids: Vec::new(),
            events: Vec::new(),
        });

        let outbound = bounded_active_work_projection_snapshot(&cached);

        assert!(outbound.works.is_empty());
        assert!(outbound.journal_entries.is_empty());
        assert!(all_agents(&outbound).all(|agent| agent.sessions.is_empty()));
        assert!(all_agents(&cached)
            .filter(|agent| agent.session_id == "session-live")
            .all(|agent| agent.sessions == retained_history));
        let outbound_session_ids = all_agents(&outbound)
            .map(|agent| agent.session_id.as_str())
            .collect::<HashSet<_>>();
        let cached_session_ids = all_agents(&cached)
            .map(|agent| agent.session_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(outbound_session_ids, cached_session_ids);
        assert_eq!(cached.works[0].id, "history-sentinel");
        assert_eq!(cached.journal_entries[0].id, "journal-sentinel");
    }

    #[test]
    fn watcher_cache_snapshot_is_bounded_and_leaves_history_in_cache() {
        let root = Path::new("/repo");
        let mut cached = active_work_projection_from_saved(projection(
            root,
            vec![agent(
                "session-live",
                "work/live",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            )],
        ));
        assign_and_merge_workspace_groups_cache_only(&mut cached.active_works, root);
        let retained_history = vec![history("session-live")];
        install_history(&mut cached, "session-live", &retained_history);
        cached.journal_entries.push(gwt::WorkspaceJournalEntryView {
            id: "journal-sentinel".to_string(),
            updated_at: "2026-08-29T00:00:00Z".to_string(),
            title: None,
            status_category: None,
            status_text: None,
            summary: None,
            progress_summary: None,
            owner: None,
            next_action: None,
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: None,
        });
        cached.works.push(gwt::WorkspaceHistoryView {
            id: "history-sentinel".to_string(),
            title: "History sentinel".to_string(),
            intent: None,
            summary: None,
            progress_summary: None,
            status_category: "done".to_string(),
            owner: None,
            created_at: "2026-08-29T00:00:00Z".to_string(),
            updated_at: "2026-08-29T00:00:00Z".to_string(),
            completed_at: Some("2026-08-29T00:00:00Z".to_string()),
            agents: Vec::new(),
            execution_containers: Vec::new(),
            board_refs: Vec::new(),
            related_workspace_ids: Vec::new(),
            events: Vec::new(),
        });

        let outbound = bounded_active_work_projection_snapshot(&cached);

        assert!(outbound.works.is_empty());
        assert!(outbound.journal_entries.is_empty());
        assert!(all_agents(&outbound).all(|agent| agent.sessions.is_empty()));
        assert!(all_agents(&cached)
            .filter(|agent| agent.session_id == "session-live")
            .all(|agent| agent.sessions == retained_history));
        assert_eq!(cached.works[0].id, "history-sentinel");
        assert_eq!(cached.journal_entries[0].id, "journal-sentinel");
    }

    #[test]
    fn cache_membership_merge_is_authoritative_without_rebuilding_history() {
        let root = Path::new("/repo");
        let mut cached = active_work_projection_from_saved(projection(
            root,
            vec![
                agent(
                    "session-keep",
                    "work/existing",
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
                agent(
                    "session-remove",
                    "work/existing",
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
            ],
        ));
        assign_and_merge_workspace_groups_cache_only(&mut cached.active_works, root);
        let retained_history = vec![history("session-keep")];
        install_history(&mut cached, "session-keep", &retained_history);
        let history_allocations_before = session_history_allocations(&cached, "session-keep");
        let existing = cached
            .active_works
            .iter_mut()
            .find(|work| work.branch.as_deref() == Some("work/existing"))
            .expect("existing grouped Work");
        existing.work_summary = Some("cached enrichment".to_string());
        existing.merged_into_base = true;
        let mut historical_agent = existing.agents[0].clone();
        historical_agent.session_id = "session-history-only".to_string();
        historical_agent.status_category = "idle".to_string();
        historical_agent.sessions = vec![history("session-history-only")];
        existing.agents.push(historical_agent);
        cached.works.push(gwt::WorkspaceHistoryView {
            id: "history-must-not-be-consumed".to_string(),
            title: "History".to_string(),
            intent: None,
            summary: None,
            progress_summary: None,
            status_category: "done".to_string(),
            owner: None,
            created_at: "2026-08-29T00:00:00Z".to_string(),
            updated_at: "2026-08-29T00:00:00Z".to_string(),
            completed_at: Some("2026-08-29T00:00:00Z".to_string()),
            agents: Vec::new(),
            execution_containers: Vec::new(),
            board_refs: Vec::new(),
            related_workspace_ids: Vec::new(),
            events: Vec::new(),
        });
        let history_before = cached.works.clone();
        let fresh = projection(
            root,
            vec![
                agent(
                    "session-keep",
                    "work/existing",
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked,
                ),
                agent(
                    "session-add",
                    "work/new",
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
            ],
        );

        reset_history_git_identity_conflict_checks();
        merge_workspace_projection_membership_cache_only(&mut cached, root, &fresh);

        let session_ids = all_agents(&cached)
            .map(|agent| agent.session_id.as_str())
            .collect::<HashSet<_>>();
        assert!(session_ids.contains("session-keep"));
        assert!(session_ids.contains("session-add"));
        assert!(session_ids.contains("session-history-only"));
        assert!(!session_ids.contains("session-remove"));
        assert!(all_agents(&cached)
            .filter(|agent| agent.session_id == "session-keep")
            .all(|agent| agent.sessions == retained_history));
        assert_eq!(
            session_history_allocations(&cached, "session-keep"),
            history_allocations_before,
            "cache reconciliation must move each existing Session history Vec without cloning it"
        );
        let existing = cached
            .active_works
            .iter()
            .find(|work| work.branch.as_deref() == Some("work/existing"))
            .expect("existing root group remains");
        assert_eq!(existing.work_summary.as_deref(), Some("cached enrichment"));
        assert!(existing.merged_into_base);
        assert!(cached
            .active_works
            .iter()
            .any(|work| work.branch.as_deref() == Some("work/new")
                && work
                    .agents
                    .iter()
                    .any(|agent| agent.session_id == "session-add")));
        assert_eq!(cached.works, history_before);
        assert_eq!(history_git_identity_conflict_checks(), 0);
    }
}
