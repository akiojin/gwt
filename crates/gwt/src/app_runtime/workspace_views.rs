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
//!   (`save_unassigned_workspace_launch_projection`,
//!   `save_start_work_workspace_projection`,
//!   `save_resumed_workspace_projection`)
//! - [`AppRuntime::active_work_projection_for_tab`] and the projection
//!   reply / broadcast / prune handlers
//!
//! Behavior-preserving move: `INFLIGHT_LAUNCH_TTL` / `inflight_launch_key`
//! are launch-side and stay in `mod.rs` (Pass 2 moves them to `launch.rs`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::{
    active_agent_summary_from_session, current_git_branch, local_branch_exists,
    merge_active_sessions_into_projection, normalize_branch_name, origin_remote_ref,
    retain_live_workspace_agents, same_worktree_path, save_workspace_launch_projection,
    workspace_cleanup_candidate_for_projection, workspace_projection_owner_title,
    ActiveAgentSession, AppRuntime, BackendEvent, ClientId, IssueBranchLinkStore, OutboundEvent,
    ProjectTabRuntime, WorkspaceLaunchProjectionKind, WorkspaceResumeContext,
};

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

fn managed_hook_health_view_for_project(
    project_root: &Path,
    sessions_dir: &Path,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ManagedHookHealthView> {
    let mut input = gwt::cli::hook::health::ManagedHookHealthInput::new(project_root);
    if let Some(session) = sessions.iter().min_by_key(|session| &session.session_id) {
        input = input.with_runtime_state_path(gwt_agent::runtime_state_path(
            sessions_dir,
            &session.session_id,
        ));
    }
    let health = gwt::cli::hook::health::read_managed_hook_health(&input);
    let should_show = health.status != gwt::cli::hook::health::ManagedHookHealthStatus::Inactive
        || health.pending_discussion.is_some()
        || health.pending_goal.is_some()
        || !health.slow_handlers.is_empty()
        || !health.issues.is_empty();
    should_show.then(|| managed_hook_health_view_from_health(health))
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

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): the canonical Work identity for an
/// active agent. `agent_session_id` is the primary key so that "1 agent
/// session : 1 Work" holds — two agents on the same branch but with distinct
/// `session_id`s resolve to distinct Work rows. Legacy agents that report an
/// empty `session_id` fall back to the historical branch/worktree-derived
/// identity, then `workspace_id`, then the provided `legacy_fallback`.
fn active_work_agent_work_id(
    project_root: &Path,
    agent: &gwt::ActiveWorkAgentView,
    legacy_fallback: Option<&str>,
) -> Option<String> {
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
    let worktree_matches = details
        .worktree_path
        .as_deref()
        .zip(agent.worktree_path.as_deref())
        .is_some_and(|(left, right)| left == Path::new(right));
    branch_matches || worktree_matches
}

fn find_active_work_history<'a>(
    work_id: &str,
    first_agent: Option<&gwt::ActiveWorkAgentView>,
    works: &'a [gwt::WorkspaceHistoryView],
) -> Option<&'a gwt::WorkspaceHistoryView> {
    works.iter().find(|item| item.id == work_id).or_else(|| {
        works.iter().find(|item| {
            item.execution_containers.iter().any(|container| {
                let branch_matches = first_agent
                    .and_then(|agent| agent.branch.as_deref())
                    .zip(container.branch.as_deref())
                    .is_some_and(|(left, right)| left == right);
                let worktree_matches = first_agent
                    .and_then(|agent| agent.worktree_path.as_deref())
                    .zip(container.worktree_path.as_deref())
                    .is_some_and(|(left, right)| Path::new(left) == Path::new(right));
                branch_matches || worktree_matches
            })
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
            active_work_agent_work_id(&projection.project_root, agent, Some(&projection.id))
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
            let history = find_active_work_history(&work_id, first_agent, works);
            let container = history.and_then(|item| item.execution_containers.first());
            let is_current_projection = work_id == projection.id
                || projection_matches_active_work(projection, &work_id)
                || first_agent
                    .is_some_and(|agent| agent_matches_projection_git_details(projection, agent));
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
            let branch_value = if is_current_projection {
                projection
                    .git_details
                    .as_ref()
                    .and_then(|details| details.branch.clone())
            } else {
                container
                    .and_then(|value| value.branch.clone())
                    .or_else(|| first_agent.and_then(|agent| agent.branch.clone()))
            };
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
                worktree_path: if is_current_projection {
                    projection.git_details.as_ref().and_then(|details| {
                        details
                            .worktree_path
                            .as_ref()
                            .map(|path| path.display().to_string())
                    })
                } else {
                    container
                        .and_then(|value| value.worktree_path.clone())
                        .or_else(|| first_agent.and_then(|agent| agent.worktree_path.clone()))
                },
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
    // the Work surface as Paused until closed. Dedupe against the live rows by id
    // and by branch/worktree so a resumed (live again) Work surfaces once as
    // Active, and the launch-recorded history row (keyed by the projection id but
    // covered by a live session) never produces a phantom Paused duplicate.
    append_paused_work_items(&mut active_works, works, journal_title_by_session);
    active_works
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): append Paused `active_works` rows for
/// retained Work-history items that have no live agent group. A history item is
/// Paused when it is incomplete (not Done) and is not already represented by a
/// live row (matched by Work id or by branch/worktree execution container). Done
/// items are skipped here — close/cleanup is handled in a later slice.
fn append_paused_work_items(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    works: &[gwt::WorkspaceHistoryView],
    journal_title_by_session: &std::collections::HashMap<String, String>,
) {
    for work in works {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): terminal closes (Done and
        // Discarded) leave the active Work surface. Both are excluded so a
        // closed Work never re-appears as a Paused row.
        if work.status_category == "done" || work.status_category == "discarded" {
            continue;
        }
        if active_work_already_present(active_works, work) {
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
    }
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a Work-history item is already
/// represented by an existing (live) `active_works` row when their ids match or
/// when they share a branch / worktree identity. Used to dedupe Paused rows so a
/// resumed Work and the launch-recorded history row do not duplicate the live
/// Active row.
fn active_work_already_present(
    active_works: &[gwt::ActiveWorkItemView],
    work: &gwt::WorkspaceHistoryView,
) -> bool {
    active_works.iter().any(|existing| {
        if existing.id == work.id {
            return true;
        }
        // A live Work synthesized without git_details carries no execution
        // container, so also dedupe by shared agent session id (the launch /
        // synthesized history row and the live row reference the same session).
        let session_matches = existing.agents.iter().any(|live_agent| {
            !live_agent.session_id.trim().is_empty()
                && work
                    .agents
                    .iter()
                    .any(|history_agent| history_agent.session_id == live_agent.session_id)
        });
        if session_matches {
            return true;
        }
        work.execution_containers.iter().any(|container| {
            let branch_matches = existing
                .branch
                .as_deref()
                .map(normalize_branch_name)
                .zip(container.branch.as_deref().map(normalize_branch_name))
                .is_some_and(|(left, right)| left == right);
            let worktree_matches = existing
                .worktree_path
                .as_deref()
                .zip(container.worktree_path.as_deref())
                .is_some_and(|(left, right)| Path::new(left) == Path::new(right));
            branch_matches || worktree_matches
        })
    })
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
    project_root: &Path,
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
            .map(|agent| workspace_work_agent_view_from_ref(agent, session_index, project_root))
            .collect(),
        execution_containers: item
            .execution_containers
            .iter()
            .map(workspace_execution_container_view_from_ref)
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
    project_root: &Path,
) -> gwt::WorkspaceHistoryAgentView {
    // A Work's `session_id` is the gwt session id (the launch). It keys into the
    // persisted Session whose forward-only `session_history` is the Session list
    // (agent-tool conversation UUIDs) under this Work; the latest
    // `agent_session_id` marks the currently active Session.
    let sessions = session_index
        .get(agent.session_id.as_str())
        .map(|session| {
            let latest = session.agent_session_id.as_deref();
            let exact_resume_available = session_exact_resume_materializable(project_root, session);
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
    repo_path: &Path,
    branch: Option<&str>,
    worktree_path: &Path,
) -> WorkspaceResumeContext {
    let item = gwt_core::workspace_projection::load_workspace_work_items(repo_path)
        .ok()
        .flatten()
        .and_then(|projection| {
            gwt_core::workspace_projection::find_work_item_for_container(
                &projection,
                repo_path,
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
        .is_some_and(|(left, right)| same_worktree_path(left, right) || left == right);
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
    project_root: &Path,
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
            };
            let history_view =
                workspace_work_agent_view_from_ref(&agent_ref, session_index, project_root);
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
            let mut sorted: Vec<gwt::ActiveWorkAgentView> = std::mem::take(&mut work.agents);
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
            work.agents = kept;
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped as u32);
        }
        // User verification 2026-06-17 (follow-up): Workspace detail is a
        // session summary, not a live process inventory. Per agent identity
        // only the latest history entry stays; live duplicates collapse too.
        {
            let mut sorted: Vec<gwt::ActiveWorkAgentView> = std::mem::take(&mut work.agents);
            sorted.sort_by(compare_active_work_agents_newest_first);
            let mut seen_identities: std::collections::HashSet<String> =
                std::collections::HashSet::new();
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
            work.agents = kept;
        }
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
/// collapse downstream dedups), numeric counts sum, and `merged_into_base`
/// ORs. Old branchless ids keep their own key, so legacy rows never vanish
/// or fuse.
pub(super) fn assign_and_merge_workspace_groups(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    project_root: &Path,
) {
    for work in active_works.iter_mut() {
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
                let active_agents = target.active_agents + work.active_agents;
                let blocked_agents = target.blocked_agents + work.blocked_agents;
                let session_agent_total = target.session_agent_total + work.session_agent_total;
                let merged_into_base = target.merged_into_base || work.merged_into_base;
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
                target.active_agents = active_agents;
                target.blocked_agents = blocked_agents;
                target.session_agent_total = session_agent_total;
                target.merged_into_base = merged_into_base;
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
            .map(|branch| local_branches.is_some_and(|set| set.contains(&branch)));
        // Branchless rows are never "remote": there is nothing to fetch.
        work.remote_only = matches!(branch_local, Some(false));
    }
}

/// SPEC-2359 W-15 (FR-386): flag rows whose branch is merged into a base on
/// origin (background scan cache) or whose recorded PR state is merged — the
/// "safe to delete" signal. Display-only; no automatic close (US-61).
pub(super) fn mark_merged_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    merged_branches: Option<&HashMap<String, chrono::DateTime<chrono::Utc>>>,
) {
    for work in active_works.iter_mut() {
        if active_work_has_dirty_worktree(work) {
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

fn active_work_has_dirty_worktree(work: &gwt::ActiveWorkItemView) -> bool {
    work.worktree_path
        .as_deref()
        .map(Path::new)
        .filter(|path| active_work_path_is_git_toplevel(path))
        .is_some_and(|path| {
            gwt_git::diff::get_status(path)
                .map(|entries| !entries.is_empty())
                .unwrap_or(false)
        })
}

fn active_work_path_is_git_toplevel(path: &Path) -> bool {
    let Ok(path) = dunce::canonicalize(path) else {
        return false;
    };
    let Ok(output) = gwt_core::process::run_git_logged(
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        Some(&path),
    ) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let toplevel = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if toplevel.as_os_str().is_empty() {
        return false;
    }
    dunce::canonicalize(toplevel).is_ok_and(|toplevel| toplevel == path)
}

/// SPEC-2359 US-78: cleanup eligibility is backend-owned per Workspace row.
/// `merged_into_base` remains a display badge; this candidate is the action
/// gate after filtering out live-agent branches/worktrees and remote-only rows.
pub(super) fn mark_workspace_cleanup_candidates(
    active_works: &mut [gwt::ActiveWorkItemView],
    cleanup_ready_branches: Option<&HashMap<String, String>>,
    sessions: &[&ActiveAgentSession],
    live_process_worktree_paths: &HashSet<PathBuf>,
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
        let Some(reason) = cleanup_reason_for_work(work, cleanup_ready_branches, &branch) else {
            continue;
        };
        let worktree_path = work.worktree_path.as_deref().map(Path::new);
        if sessions.iter().any(|session| {
            active_agent_session_matches_work(session, Some(branch.as_str()), worktree_path)
        }) {
            work.cleanup_blocked_reason = Some("live_agent".to_string());
            continue;
        }
        if worktree_path
            .and_then(normalize_existing_worktree_path)
            .is_some_and(|path| live_process_worktree_paths.contains(&path))
        {
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
    branch: &str,
) -> Option<String> {
    if active_work_has_dirty_worktree(work) {
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

fn live_process_worktree_paths_for_cleanup(
    active_works: &[gwt::ActiveWorkItemView],
    cleanup_ready_branches: Option<&HashMap<String, String>>,
    projection_cleanup_candidate: Option<&gwt::ActiveWorkCleanupCandidateView>,
) -> HashSet<PathBuf> {
    let mut candidate_paths = active_works
        .iter()
        .filter(|work| !work.remote_only)
        .filter(|work| {
            work.branch
                .as_deref()
                .map(normalize_branch_name)
                .is_some_and(|branch| branch.starts_with("work/"))
        })
        .filter(|work| {
            work.branch
                .as_deref()
                .map(normalize_branch_name)
                .and_then(|branch| cleanup_reason_for_work(work, cleanup_ready_branches, &branch))
                .is_some()
        })
        .filter_map(|work| {
            work.worktree_path
                .as_deref()
                .map(Path::new)
                .and_then(normalize_existing_worktree_path)
        })
        .collect::<Vec<_>>();
    if let Some(path) = projection_cleanup_candidate
        .and_then(|candidate| candidate.worktree_path.as_deref())
        .map(Path::new)
        .and_then(normalize_existing_worktree_path)
    {
        candidate_paths.push(path);
    }
    candidate_paths.sort();
    candidate_paths.dedup();
    if candidate_paths.is_empty() {
        return HashSet::new();
    }

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );

    let mut live_paths = HashSet::new();
    for process in system.processes().values() {
        let Some(cwd) = process.cwd().and_then(normalize_existing_worktree_path) else {
            continue;
        };
        for candidate in &candidate_paths {
            if cwd == *candidate || cwd.starts_with(candidate) {
                live_paths.insert(candidate.clone());
            }
        }
    }
    live_paths
}

fn normalize_existing_worktree_path(path: &Path) -> Option<PathBuf> {
    dunce::canonicalize(path).ok()
}

fn cleanup_candidate_has_live_process(
    candidate: &gwt::ActiveWorkCleanupCandidateView,
    live_process_worktree_paths: &HashSet<PathBuf>,
) -> bool {
    candidate
        .worktree_path
        .as_deref()
        .map(Path::new)
        .and_then(normalize_existing_worktree_path)
        .is_some_and(|path| live_process_worktree_paths.contains(&path))
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
    let worktree_matches = worktree_path.is_some_and(|path| {
        same_worktree_path(&session.worktree_path, path) || session.worktree_path == path
    });
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
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(project_root)?;
    let cache_root = gwt_core::paths::gwt_cache_dir()
        .join("issues")
        .join(repo_hash.as_str());
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

fn save_unassigned_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::load_or_default_workspace_projection(project_root)
            .map_err(|error| error.to_string())?;
    projection.project_root = project_root.to_path_buf();
    projection.register_unassigned_agent(unassigned_agent_summary_from_session(session, now));
    projection.updated_at = now;
    gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
        .map_err(|error| error.to_string())
}

pub(super) fn save_start_work_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    _base_branch: &str,
    linked_issue_number: Option<u64>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    live_session_ids: &std::collections::HashSet<String>,
) -> Result<(), String> {
    if workspace_resume_context.is_none() {
        return save_unassigned_workspace_launch_projection(project_root, session);
    }
    save_workspace_launch_projection(
        project_root,
        session,
        Some(_base_branch),
        linked_issue_number,
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
        Some(workspace_resume_context),
        WorkspaceLaunchProjectionKind::Resume {
            created_by_start_work: session.branch_name.starts_with("work/"),
        },
        live_session_ids,
    )
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
        let projection = self.active_work_projection_for_tab(tab_id, tab)?;
        Some(OutboundEvent::reply(
            client_id,
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
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

    /// Like `active_work_projection_broadcast_for_active_tab`, but always emits an event
    /// when an active tab exists — falling back to an empty projection so that frontends
    /// clear stale per-project data when the tab focus moves to a project without
    /// any saved projection or live agent sessions.
    pub(super) fn active_work_projection_broadcast_on_tab_change(&self) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self
            .active_work_projection_for_tab(tab_id, tab)
            .unwrap_or_else(|| empty_active_work_projection_view(tab_id, tab));
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
            let workspaces = self
                .work_items_cache
                .borrow_mut()
                .load_or_synthesize(&tab.project_root)
                .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                    updated_at,
                    work_items: Vec::new(),
                })
                .work_items
                .iter()
                .map(|item| {
                    workspace_work_item_view_from_item(item, &session_index, &tab.project_root)
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
                &tab.project_root,
            );
            // SPEC-2359 W-15 (FR-386): "safe to delete" badge inputs — the
            // background merge-scan cache plus the recorded PR state.
            mark_merged_active_works(
                &mut view.active_works,
                self.work_merged_branches.get(&tab.project_root),
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
            let live_process_worktree_paths = live_process_worktree_paths_for_cleanup(
                &view.active_works,
                cleanup_ready_branches,
                view.cleanup_candidate.as_ref(),
            );
            if view.cleanup_candidate.as_ref().is_some_and(|candidate| {
                cleanup_candidate_has_live_process(candidate, &live_process_worktree_paths)
            }) {
                view.cleanup_candidate = None;
            }
            mark_workspace_cleanup_candidates(
                &mut view.active_works,
                cleanup_ready_branches,
                &sessions,
                &live_process_worktree_paths,
            );
            return Some(view);
        }

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
            pr_number: None,
            pr_url: None,
            pr_state: None,
            board_refs: Vec::new(),
            agents: agents.clone(),
            // SPEC-2359 Phase W-12 (FR-349): synthesized from live sessions, so
            // the owning agent is Running and not user-closed → Active.
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
            managed_hook_health: managed_hook_health_view_for_project(
                &tab.project_root,
                &self.sessions_dir,
                &sessions,
            ),
            active_work_count: active_works.len(),
            active_works,
            agents,
            unassigned_agents: Vec::new(),
        })
    }

    pub(crate) fn handle_workspace_projection_changed_events(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Ok(Some(projection)) =
            gwt_core::workspace_projection::load_workspace_projection(project_root)
        else {
            return Vec::new();
        };
        self.apply_workspace_projection_title_sync(project_root, &projection)
    }
}
