use std::path::Path;

use chrono::{DateTime, Utc};
use gwt_core::paths::gwt_projects_dir;
use gwt_core::workspace_projection::{
    apply_prune_plan, classify_workspace_projections, load_or_default_workspace_projection,
    load_or_synthesize_workspace_work_items, record_workspace_work_event,
    save_workspace_projection, update_workspace_projection_with_journal, ClassifiedProjection,
    PruneAction, PruneSkipReason, WorkEvent, WorkEventKind, WorkItem, WorkspaceAgentSummary,
    WorkspaceExecutionContainerRef, WorkspaceProjection, WorkspaceProjectionUpdate,
    WorkspaceRetentionConfig, WorkspaceStartUpdate, WorkspaceStatusCategory,
};
use gwt_github::{ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, WorkspaceCommand};

pub fn parse(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "update" => parse_update(rest),
        "candidates" => parse_candidates(rest),
        "join" => parse_join(rest),
        "create" => parse_create(rest),
        "ensure" => parse_ensure(rest),
        "projection-list" => parse_projection_list(rest),
        "projection-prune" => parse_projection_prune(rest),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_projection_list(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut stale = false;
    let mut all = false;
    for arg in args {
        match arg.as_str() {
            "--stale" => stale = true,
            "--all" => all = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
    }
    Ok(WorkspaceCommand::ProjectionList { stale, all })
}

fn parse_projection_prune(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut dry_run = false;
    let mut ids: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--id" => {
                let value = parse_required_value(args, i, "--id")?;
                ids.push(value);
                i += 2;
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
    }
    Ok(WorkspaceCommand::ProjectionPrune { dry_run, ids })
}

fn parse_required_value(
    args: &[String],
    index: usize,
    flag: &'static str,
) -> Result<String, CliParseError> {
    args.get(index + 1)
        .cloned()
        .ok_or(CliParseError::MissingFlag(flag))
}

fn parse_update(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut title = None;
    let mut status = None;
    let mut status_text = None;
    let mut summary = None;
    let mut next_action = None;
    let mut owner = None;
    let mut agent_session = None;
    let mut current_focus = None;
    let mut title_summary = None;
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).ok_or(CliParseError::Usage)?.clone();
        match args[i].as_str() {
            "--title" => title = Some(value),
            "--status" => status = Some(value),
            "--status-text" => status_text = Some(value),
            "--summary" => summary = Some(value),
            "--next-action" => next_action = Some(value),
            "--owner" => owner = Some(value),
            "--agent-session" => agent_session = Some(value),
            "--current-focus" => current_focus = Some(value),
            "--title-summary" => title_summary = Some(value),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    if agent_session.is_none() && (current_focus.is_some() || title_summary.is_some()) {
        return Err(CliParseError::MissingFlag("--agent-session"));
    }
    if let Some(value) = title_summary.as_deref() {
        super::validate_title_summary_work_name("--title-summary", value)?;
    }
    Ok(WorkspaceCommand::Update {
        title,
        status,
        status_text,
        summary,
        next_action,
        owner,
        agent_session,
        current_focus,
        title_summary,
    })
}

fn parse_candidates(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    Ok(WorkspaceCommand::Candidates {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
    })
}

fn parse_join(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut workspace_id = None;
    let mut current_focus = None;
    let mut title_summary = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            "--workspace" | "--workspace-id" => {
                workspace_id = Some(parse_required_value(args, i, "--workspace")?)
            }
            "--current-focus" => {
                current_focus = Some(parse_required_value(args, i, "--current-focus")?)
            }
            "--title-summary" => {
                title_summary = Some(parse_required_value(args, i, "--title-summary")?)
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    if let Some(value) = title_summary.as_deref() {
        super::validate_title_summary_work_name("--title-summary", value)?;
    }
    Ok(WorkspaceCommand::Join {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
        workspace_id: workspace_id.ok_or(CliParseError::MissingFlag("--workspace"))?,
        current_focus,
        title_summary,
    })
}

fn parse_create(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut title_summary = None;
    let mut current_focus = None;
    let mut spec = None;
    let mut issue = None;
    let mut split_from = None;
    let mut boundary = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            "--title-summary" => {
                title_summary = Some(parse_required_value(args, i, "--title-summary")?)
            }
            "--current-focus" => {
                current_focus = Some(parse_required_value(args, i, "--current-focus")?)
            }
            "--spec" => {
                spec = Some(
                    parse_required_value(args, i, "--spec")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--issue" => {
                issue = Some(
                    parse_required_value(args, i, "--issue")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--split-from" => split_from = Some(parse_required_value(args, i, "--split-from")?),
            "--boundary" => boundary = Some(parse_required_value(args, i, "--boundary")?),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    let title_summary = title_summary.ok_or(CliParseError::MissingFlag("--title-summary"))?;
    super::validate_title_summary_work_name("--title-summary", &title_summary)?;
    Ok(WorkspaceCommand::Create {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
        title_summary,
        current_focus,
        spec,
        issue,
        split_from,
        boundary,
    })
}

fn parse_ensure(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut title_summary = None;
    let mut current_focus = None;
    let mut spec = None;
    let mut issue = None;
    let mut topic = None;
    let mut boundary = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            "--title-summary" => {
                title_summary = Some(parse_required_value(args, i, "--title-summary")?)
            }
            "--current-focus" => {
                current_focus = Some(parse_required_value(args, i, "--current-focus")?)
            }
            "--spec" => {
                spec = Some(
                    parse_required_value(args, i, "--spec")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--issue" => {
                issue = Some(
                    parse_required_value(args, i, "--issue")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--topic" => topic = Some(parse_required_value(args, i, "--topic")?),
            "--boundary" => boundary = Some(parse_required_value(args, i, "--boundary")?),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    let title_summary = title_summary.ok_or(CliParseError::MissingFlag("--title-summary"))?;
    super::validate_title_summary_work_name("--title-summary", &title_summary)?;
    Ok(WorkspaceCommand::Ensure {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
        title_summary,
        current_focus,
        spec,
        issue,
        topic,
        boundary,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceEnsureInput {
    pub agent_session: String,
    pub title_summary: String,
    pub current_focus: Option<String>,
    pub spec: Option<u64>,
    pub issue: Option<u64>,
    pub topic: Option<String>,
    pub boundary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspaceEnsureDisposition {
    AlreadyAssigned,
    Joined,
    Created,
}

impl WorkspaceEnsureDisposition {
    fn as_str(self) -> &'static str {
        match self {
            WorkspaceEnsureDisposition::AlreadyAssigned => "already-assigned",
            WorkspaceEnsureDisposition::Joined => "joined",
            WorkspaceEnsureDisposition::Created => "created",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceEnsureResult {
    pub workspace_id: String,
    pub disposition: WorkspaceEnsureDisposition,
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: WorkspaceCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        WorkspaceCommand::Update {
            title,
            status,
            status_text,
            summary,
            next_action,
            owner,
            agent_session,
            current_focus,
            title_summary,
        } => {
            let project_state_root = agent_session
                .as_deref()
                .map(|session_id| {
                    crate::agent_project_state::project_state_root_for_agent_session_or_fallback(
                        env.repo_path(),
                        session_id,
                    )
                })
                .unwrap_or_else(|| env.repo_path().to_path_buf());
            if let Some(session_id) = agent_session.as_deref() {
                crate::agent_project_state::repair_split_agent_state_if_needed(
                    &project_state_root,
                    env.repo_path(),
                    session_id,
                )
                .map_err(core_error)?;
            }
            let update = WorkspaceProjectionUpdate {
                title,
                status_category: status
                    .as_deref()
                    .map(parse_status_category)
                    .transpose()
                    .map_err(string_error)?,
                status_text,
                owner,
                next_action,
                summary,
                agent_session_id: agent_session,
                agent_current_focus: current_focus,
                agent_title_summary: title_summary,
            };
            let entry = update_workspace_projection_with_journal(&project_state_root, update)
                .map_err(|error| string_error(error.to_string()))?;
            publish_workspace_change(&project_state_root);
            out.push_str(&format!("workspace updated: {}\n", entry.id));
            Ok(0)
        }
        WorkspaceCommand::Candidates { agent_session } => {
            let projection =
                load_or_synthesize_workspace_work_items(env.repo_path()).map_err(core_error)?;
            let current_intent = current_agent_intent(env.repo_path(), &agent_session)?;
            let mut candidates = projection
                .work_items
                .iter()
                .filter(|item| item.is_incomplete())
                .filter(|item| {
                    !item
                        .agents
                        .iter()
                        .any(|agent| agent.session_id == agent_session)
                })
                .map(|item| {
                    let score = workspace_similarity_score(
                        current_intent.as_deref().unwrap_or_default(),
                        &workspace_item_text(item),
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            if candidates.is_empty() {
                out.push_str("workspace candidates: none\n");
            } else {
                for (score, item) in candidates {
                    out.push_str(&format!(
                        "{}\t{}\t{}\tscore={score}\n",
                        item.id,
                        status_category_wire(item.status_category),
                        item.title
                    ));
                }
            }
            Ok(0)
        }
        WorkspaceCommand::Join {
            agent_session,
            workspace_id,
            current_focus,
            title_summary,
        } => {
            let mut projection =
                load_or_default_workspace_projection(env.repo_path()).map_err(core_error)?;
            let Some(item) = workspace_item_by_id(env.repo_path(), &workspace_id)? else {
                return Err(string_error(format!("workspace not found: {workspace_id}")));
            };
            assign_agent_to_workspace(
                &mut projection,
                &agent_session,
                &workspace_id,
                current_focus,
                title_summary,
            )?;
            apply_workspace_item_to_projection(&mut projection, &item);
            save_workspace_projection(env.repo_path(), &projection).map_err(core_error)?;
            publish_workspace_change(env.repo_path());
            out.push_str(&format!("workspace joined: {workspace_id}\n"));
            Ok(0)
        }
        WorkspaceCommand::Create {
            agent_session,
            title_summary,
            current_focus,
            spec,
            issue,
            split_from,
            boundary,
        } => {
            let existing =
                load_or_synthesize_workspace_work_items(env.repo_path()).map_err(core_error)?;
            let mut projection =
                load_or_default_workspace_projection(env.repo_path()).map_err(core_error)?;
            let Some(agent) = projection
                .agents
                .iter()
                .find(|agent| agent.session_id == agent_session)
            else {
                return Err(string_error(format!(
                    "agent session not found: {agent_session}"
                )));
            };
            let agent_display_name = agent.display_name.clone();
            // SPEC-2359 W16-2 (FR-389): mint the canonical (machine-
            // independent, branch-keyed) Work id when the agent has a branch
            // so every machine's records join the same Workspace.
            let canonical_id = gwt_core::workspace_projection::canonical_work_id(
                env.repo_path(),
                agent.branch.as_deref(),
                agent.worktree_path.as_deref(),
            );
            let canonical_joins_existing = canonical_id.as_deref().is_some_and(|id| {
                existing
                    .work_items
                    .iter()
                    .any(|item| item.is_incomplete() && item.id == id)
            });
            if !canonical_joins_existing
                && split_from.is_none()
                && boundary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                let new_text = [Some(title_summary.as_str()), current_focus.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(item) = existing.work_items.iter().find(|item| {
                    item.is_incomplete()
                        && workspace_similarity_score(&new_text, &workspace_item_text(item)) >= 2
                }) {
                    return Err(string_error(format!(
                        "similar Workspace exists: {} ({})",
                        item.title, item.id
                    )));
                }
            }
            let workspace_id = canonical_id
                .unwrap_or_else(|| format!("workspace-{}", Utc::now().timestamp_millis()));
            let owner = spec
                .map(|number| format!("SPEC-{number}"))
                .or_else(|| issue.map(|number| format!("Issue #{number}")));
            let now = Utc::now();
            let mut event = WorkEvent::new(WorkEventKind::Start, workspace_id.clone(), now);
            event.title = Some(title_summary.clone());
            event.intent = current_focus.clone();
            event.summary = current_focus
                .clone()
                .or_else(|| Some(title_summary.clone()));
            event.status_category = Some(WorkspaceStatusCategory::Active);
            event.owner = owner.clone();
            event.next_action = Some("Coordinate on Board before implementation".to_string());
            event.agent_session_id = Some(agent_session.clone());
            event.agent_id = Some(agent.agent_id.clone());
            event.display_name = Some(agent.display_name.clone());
            event.execution_container = Some(WorkspaceExecutionContainerRef {
                branch: agent.branch.clone(),
                worktree_path: agent.worktree_path.clone(),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            });
            if let Some(split_from) = split_from {
                event.kind = WorkEventKind::Split;
                event.related_work_item_id = Some(split_from);
            }
            if let Some(boundary) = boundary {
                event.next_action = Some(format!("Boundary: {boundary}"));
            }
            record_workspace_work_event(env.repo_path(), event).map_err(core_error)?;
            projection.start_work(
                WorkspaceStartUpdate {
                    workspace_id: workspace_id.clone(),
                    title: title_summary.clone(),
                    status_text: current_focus.clone(),
                    // SPEC-2359 Phase U-6 (FR-134): when current_focus is
                    // omitted, fall back to title_summary so the Workspace
                    // Overview Summary section never renders empty for
                    // newly-created workspaces.
                    summary: current_focus
                        .clone()
                        .or_else(|| Some(title_summary.clone())),
                    owner,
                    next_action: "Coordinate on Board before implementation".to_string(),
                },
                now,
            );
            // SPEC-2359 Phase U-6 (FR-131, FR-135): record creation metadata
            // and initial lifecycle stage so the new Card preview / Detail
            // pane has informative chips from Day-0 without waiting for
            // retroactive migration.
            projection.created_at = now;
            projection.creator = Some(agent_display_name);
            projection.lifecycle_stage =
                gwt_core::workspace_projection::WorkspaceLifecycleStage::Active;
            assign_agent_to_workspace(
                &mut projection,
                &agent_session,
                &workspace_id,
                current_focus,
                Some(title_summary),
            )?;
            projection.updated_at = now;
            save_workspace_projection(env.repo_path(), &projection).map_err(core_error)?;
            publish_workspace_change(env.repo_path());
            out.push_str(&format!("workspace created: {workspace_id}\n"));
            Ok(0)
        }
        WorkspaceCommand::Ensure {
            agent_session,
            title_summary,
            current_focus,
            spec,
            issue,
            topic,
            boundary,
        } => {
            let result = ensure_workspace_for_agent(
                env.repo_path(),
                WorkspaceEnsureInput {
                    agent_session,
                    title_summary,
                    current_focus,
                    spec,
                    issue,
                    topic,
                    boundary,
                },
            )?;
            out.push_str(&format!(
                "workspace ensured: {} ({})\n",
                result.workspace_id,
                result.disposition.as_str()
            ));
            Ok(0)
        }
        WorkspaceCommand::ProjectionList { stale, all } => {
            let scan_root = gwt_projects_dir();
            run_projection_list_with_scan_root(
                &scan_root,
                &WorkspaceRetentionConfig::default(),
                Utc::now(),
                stale,
                all,
                |_| false,
                out,
            )
        }
        WorkspaceCommand::ProjectionPrune { dry_run, ids } => {
            let scan_root = gwt_projects_dir();
            run_projection_prune_with_scan_root(
                &scan_root,
                &WorkspaceRetentionConfig::default(),
                Utc::now(),
                dry_run,
                &ids,
                |_| false,
                out,
            )
        }
    }
}

/// SPEC-2359 US-41 (FR-153): implement `workspace.projection_list` over a
/// caller-provided `scan_root` so the production path uses `gwt_projects_dir()`
/// and tests can pass a tempdir. `is_active_session` bridges in the live-window
/// registry from `app_runtime` (default `false` in CLI-only contexts).
fn run_projection_list_with_scan_root<F>(
    scan_root: &Path,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
    stale: bool,
    all: bool,
    is_active_session: F,
    out: &mut String,
) -> Result<i32, SpecOpsError>
where
    F: Fn(&WorkspaceProjection) -> bool,
{
    let plan = classify_workspace_projections(scan_root, config, now, is_active_session);
    let filtered = filter_projection_list(&plan, stale, all);
    out.push_str(&format!(
        "# workspace projection list (mode: {}, count: {})\n",
        list_mode_label(stale, all),
        filtered.len()
    ));
    for entry in filtered {
        let reason = entry
            .stale_reason
            .map(|r| r.as_str().to_string())
            .unwrap_or_else(|| "-".to_string());
        let action = format_prune_action(&entry.action);
        out.push_str(&format!(
            "{} | {} | {:?} | {} | {} | {}\n",
            entry.workspace_id,
            entry.project_root.display(),
            entry.lifecycle_stage,
            reason,
            action,
            entry.updated_at.format("%Y-%m-%dT%H:%M:%SZ"),
        ));
    }
    Ok(0)
}

/// SPEC-2359 US-41 (FR-153, FR-154): implement `workspace.projection_prune`
/// over a caller-provided `scan_root`. `ids` lets the user scope the prune to
/// specific workspace IDs; empty means "every classified entry".
fn run_projection_prune_with_scan_root<F>(
    scan_root: &Path,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
    dry_run: bool,
    ids: &[String],
    is_active_session: F,
    out: &mut String,
) -> Result<i32, SpecOpsError>
where
    F: Fn(&WorkspaceProjection) -> bool,
{
    let plan = classify_workspace_projections(scan_root, config, now, is_active_session);
    let filtered: Vec<ClassifiedProjection> = if ids.is_empty() {
        plan
    } else {
        plan.into_iter()
            .filter(|item| ids.iter().any(|id| id == &item.workspace_id))
            .collect()
    };
    let summary = apply_prune_plan(&filtered, dry_run).map_err(core_error)?;
    let mode = if dry_run { "DRY-RUN" } else { "APPLIED" };
    out.push_str(&format!(
        "{}: archive={} delete={} skip={}\n",
        mode, summary.archived, summary.deleted, summary.skipped,
    ));
    Ok(0)
}

fn filter_projection_list(
    plan: &[ClassifiedProjection],
    stale: bool,
    all: bool,
) -> Vec<&ClassifiedProjection> {
    if all {
        plan.iter().collect()
    } else if stale {
        plan.iter()
            .filter(|entry| {
                !matches!(
                    entry.action,
                    PruneAction::Skip {
                        reason: PruneSkipReason::NotStale,
                    }
                )
            })
            .collect()
    } else {
        plan.iter()
            .filter(|entry| matches!(entry.action, PruneAction::Archive | PruneAction::Delete))
            .collect()
    }
}

fn list_mode_label(stale: bool, all: bool) -> &'static str {
    match (stale, all) {
        (_, true) => "all",
        (true, _) => "stale-or-archived",
        _ => "actionable",
    }
}

fn format_prune_action(action: &PruneAction) -> String {
    match action {
        PruneAction::Skip { reason } => format!("skip:{:?}", reason),
        PruneAction::Archive => "archive".to_string(),
        PruneAction::Delete => "delete".to_string(),
    }
}

pub(super) fn ensure_workspace_for_agent(
    repo_path: &std::path::Path,
    input: WorkspaceEnsureInput,
) -> Result<WorkspaceEnsureResult, SpecOpsError> {
    let mut projection = load_or_default_workspace_projection(repo_path).map_err(core_error)?;
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == input.agent_session)
        .cloned()
    else {
        return Err(string_error(format!(
            "agent session not found: {}",
            input.agent_session
        )));
    };
    let owner = owner_from_spec_or_issue(input.spec, input.issue);
    if agent.is_assigned() {
        if let Some(workspace_id) = agent.workspace_id.as_deref() {
            if let Some(item) =
                workspace_item_by_id(repo_path, workspace_id)?.filter(WorkItem::is_incomplete)
            {
                apply_workspace_item_to_projection(&mut projection, &item);
            }
            assign_agent_to_workspace(
                &mut projection,
                &input.agent_session,
                workspace_id,
                input.current_focus,
                Some(input.title_summary),
            )?;
            save_workspace_projection(repo_path, &projection).map_err(core_error)?;
            publish_workspace_change(repo_path);
            return Ok(WorkspaceEnsureResult {
                workspace_id: workspace_id.to_string(),
                disposition: WorkspaceEnsureDisposition::AlreadyAssigned,
            });
        }
    }

    let existing = load_or_synthesize_workspace_work_items(repo_path).map_err(core_error)?;
    // SPEC-2359 W16-2 (FR-389): when the canonical (branch-keyed) Work id
    // already names an incomplete item, join it directly — the similarity
    // guard never blocks same-branch convergence.
    if let Some(canonical_id) = gwt_core::workspace_projection::canonical_work_id(
        repo_path,
        agent.branch.as_deref(),
        agent.worktree_path.as_deref(),
    ) {
        if existing
            .work_items
            .iter()
            .any(|item| item.is_incomplete() && item.id == canonical_id)
        {
            record_workspace_join_event(repo_path, &canonical_id, &input, owner.clone(), &agent)?;
            assign_agent_to_workspace(
                &mut projection,
                &input.agent_session,
                &canonical_id,
                input.current_focus,
                Some(input.title_summary),
            )?;
            if let Some(item) = existing
                .work_items
                .iter()
                .find(|item| item.id == canonical_id)
            {
                apply_workspace_item_to_projection(&mut projection, item);
            }
            save_workspace_projection(repo_path, &projection).map_err(core_error)?;
            publish_workspace_change(repo_path);
            return Ok(WorkspaceEnsureResult {
                workspace_id: canonical_id,
                disposition: WorkspaceEnsureDisposition::Joined,
            });
        }
    }
    let ensure_text = workspace_ensure_text(&input, owner.as_deref());
    if let Some(item) = best_workspace_candidate(&existing.work_items, &ensure_text) {
        let workspace_id = item.id.clone();
        record_workspace_join_event(repo_path, &workspace_id, &input, owner.clone(), &agent)?;
        assign_agent_to_workspace(
            &mut projection,
            &input.agent_session,
            &workspace_id,
            input.current_focus,
            Some(input.title_summary),
        )?;
        apply_workspace_item_to_projection(&mut projection, item);
        save_workspace_projection(repo_path, &projection).map_err(core_error)?;
        publish_workspace_change(repo_path);
        return Ok(WorkspaceEnsureResult {
            workspace_id,
            disposition: WorkspaceEnsureDisposition::Joined,
        });
    }

    let workspace_id =
        create_workspace_for_agent(repo_path, &mut projection, &input, owner, &agent)?;
    save_workspace_projection(repo_path, &projection).map_err(core_error)?;
    publish_workspace_change(repo_path);
    Ok(WorkspaceEnsureResult {
        workspace_id,
        disposition: WorkspaceEnsureDisposition::Created,
    })
}

fn owner_from_spec_or_issue(spec: Option<u64>, issue: Option<u64>) -> Option<String> {
    spec.map(|number| format!("SPEC-{number}"))
        .or_else(|| issue.map(|number| format!("Issue #{number}")))
}

fn workspace_ensure_text(input: &WorkspaceEnsureInput, owner: Option<&str>) -> String {
    [
        Some(input.title_summary.as_str()),
        input.current_focus.as_deref(),
        input.topic.as_deref(),
        owner,
        input.boundary.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
}

fn best_workspace_candidate<'a>(
    work_items: &'a [WorkItem],
    ensure_text: &str,
) -> Option<&'a WorkItem> {
    work_items
        .iter()
        .filter(|item| item.is_incomplete())
        .map(|item| {
            let score = workspace_similarity_score(ensure_text, &workspace_item_text(item));
            (score, item)
        })
        .filter(|(score, _)| *score >= 2)
        .max_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| left.updated_at.cmp(&right.updated_at))
        })
        .map(|(_, item)| item)
}

fn record_workspace_join_event(
    repo_path: &std::path::Path,
    workspace_id: &str,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    agent: &WorkspaceAgentSummary,
) -> Result<(), SpecOpsError> {
    let now = Utc::now();
    let mut event = WorkEvent::new(WorkEventKind::Claim, workspace_id.to_string(), now);
    event.intent = input.current_focus.clone();
    event.summary = Some(format!("Joined Workspace: {}", input.title_summary));
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = owner;
    event.next_action = input
        .boundary
        .as_deref()
        .map(|boundary| format!("Boundary: {boundary}"));
    event.agent_session_id = Some(input.agent_session.clone());
    event.agent_id = Some(agent.agent_id.clone());
    event.display_name = Some(agent.display_name.clone());
    event.execution_container = Some(workspace_execution_container_from_agent(agent));
    record_workspace_work_event(repo_path, event).map_err(core_error)
}

fn create_workspace_for_agent(
    repo_path: &std::path::Path,
    projection: &mut WorkspaceProjection,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    agent: &WorkspaceAgentSummary,
) -> Result<String, SpecOpsError> {
    // SPEC-2359 W16-2 (FR-389): canonical, machine-independent Work id when
    // the agent has a branch / worktree; millis fallback for branchless agents.
    let workspace_id = gwt_core::workspace_projection::canonical_work_id(
        repo_path,
        agent.branch.as_deref(),
        agent.worktree_path.as_deref(),
    )
    .unwrap_or_else(|| format!("workspace-{}", Utc::now().timestamp_millis()));
    let now = Utc::now();
    let mut event = WorkEvent::new(WorkEventKind::Start, workspace_id.clone(), now);
    event.title = Some(input.title_summary.clone());
    event.intent = input
        .current_focus
        .clone()
        .or_else(|| Some(input.title_summary.clone()));
    event.summary = input
        .current_focus
        .clone()
        .or_else(|| Some(input.title_summary.clone()));
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = owner.clone();
    event.next_action = Some(
        input
            .boundary
            .as_deref()
            .map(|boundary| format!("Boundary: {boundary}"))
            .unwrap_or_else(|| "Coordinate on Board before implementation".to_string()),
    );
    event.agent_session_id = Some(input.agent_session.clone());
    event.agent_id = Some(agent.agent_id.clone());
    event.display_name = Some(agent.display_name.clone());
    event.execution_container = Some(workspace_execution_container_from_agent(agent));
    record_workspace_work_event(repo_path, event).map_err(core_error)?;

    projection.start_work(
        WorkspaceStartUpdate {
            workspace_id: workspace_id.clone(),
            title: input.title_summary.clone(),
            status_text: input.current_focus.clone(),
            summary: input.current_focus.clone(),
            owner,
            next_action: input
                .boundary
                .as_deref()
                .map(|boundary| format!("Boundary: {boundary}"))
                .unwrap_or_else(|| "Coordinate on Board before implementation".to_string()),
        },
        now,
    );
    assign_agent_to_workspace(
        projection,
        &input.agent_session,
        &workspace_id,
        input.current_focus.clone(),
        Some(input.title_summary.clone()),
    )?;
    Ok(workspace_id)
}

fn workspace_execution_container_from_agent(
    agent: &WorkspaceAgentSummary,
) -> WorkspaceExecutionContainerRef {
    WorkspaceExecutionContainerRef {
        branch: agent.branch.clone(),
        worktree_path: agent.worktree_path.clone(),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    }
}

fn core_error(error: gwt_core::error::GwtError) -> SpecOpsError {
    string_error(error.to_string())
}

fn status_category_wire(category: WorkspaceStatusCategory) -> &'static str {
    match category {
        WorkspaceStatusCategory::Active => "active",
        WorkspaceStatusCategory::Idle => "idle",
        WorkspaceStatusCategory::Blocked => "blocked",
        WorkspaceStatusCategory::Done => "done",
        WorkspaceStatusCategory::Unknown => "unknown",
    }
}

fn current_agent_intent(
    repo_path: &std::path::Path,
    agent_session: &str,
) -> Result<Option<String>, SpecOpsError> {
    let projection = load_or_default_workspace_projection(repo_path).map_err(core_error)?;
    Ok(projection
        .agents
        .iter()
        .find(|agent| agent.session_id == agent_session)
        .map(|agent| {
            [
                agent.title_summary.as_deref(),
                agent.current_focus.as_deref(),
                agent.coordination_scope.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
        }))
}

fn workspace_item_by_id(
    repo_path: &std::path::Path,
    workspace_id: &str,
) -> Result<Option<WorkItem>, SpecOpsError> {
    Ok(load_or_synthesize_workspace_work_items(repo_path)
        .map_err(core_error)?
        .work_items
        .into_iter()
        .find(|item| item.id == workspace_id))
}

fn apply_workspace_item_to_projection(projection: &mut WorkspaceProjection, item: &WorkItem) {
    projection.apply_work_item(item, Utc::now());
}

fn assign_agent_to_workspace(
    projection: &mut WorkspaceProjection,
    agent_session: &str,
    workspace_id: &str,
    current_focus: Option<String>,
    title_summary: Option<String>,
) -> Result<(), SpecOpsError> {
    if !projection.assign_agent(
        agent_session,
        workspace_id,
        current_focus,
        title_summary,
        Utc::now(),
    ) {
        return Err(string_error(format!(
            "agent session not found: {agent_session}"
        )));
    }
    Ok(())
}

fn workspace_item_text(item: &WorkItem) -> String {
    let mut parts = vec![item.title.as_str()];
    if let Some(intent) = item.intent.as_deref() {
        parts.push(intent);
    }
    if let Some(summary) = item.summary.as_deref() {
        parts.push(summary);
    }
    if let Some(owner) = item.owner.as_deref() {
        parts.push(owner);
    }
    parts.join("\n")
}

fn workspace_similarity_score(left: &str, right: &str) -> usize {
    let left_tokens = workspace_tokens(left);
    if left_tokens.is_empty() {
        return 0;
    }
    let right_tokens = workspace_tokens(right);
    left_tokens
        .iter()
        .filter(|token| right_tokens.contains(*token))
        .count()
}

fn workspace_tokens(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_lowercase())
        .collect()
}

#[cfg(unix)]
pub(crate) fn publish_workspace_change(project_root: &std::path::Path) {
    let result = crate::daemon_publisher::publish_event(
        project_root,
        "workspace",
        serde_json::json!({"projection": "updated"}),
    );
    if let Err(err) = result {
        tracing::debug!(
            error = %err,
            project_root = %project_root.display(),
            "workspace.update: daemon publish failed (non-fatal)"
        );
    }
}

#[cfg(not(unix))]
pub(crate) fn publish_workspace_change(_project_root: &std::path::Path) {}

fn parse_status_category(value: &str) -> Result<WorkspaceStatusCategory, String> {
    match value {
        "active" => Ok(WorkspaceStatusCategory::Active),
        "idle" => Ok(WorkspaceStatusCategory::Idle),
        "blocked" => Ok(WorkspaceStatusCategory::Blocked),
        "done" => Ok(WorkspaceStatusCategory::Done),
        "unknown" => Ok(WorkspaceStatusCategory::Unknown),
        other => Err(format!("unknown workspace status '{other}'")),
    }
}

fn string_error(error: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::env::TestEnv;
    use gwt_core::workspace_projection::{
        load_workspace_projection, load_workspace_work_items, record_workspace_work_event,
        save_workspace_projection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceProjection,
    };
    use std::ffi::OsString;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct ScopedHome {
        previous_home: Option<OsString>,
    }

    impl ScopedHome {
        fn set(path: &std::path::Path) -> Self {
            let previous_home = std::env::var_os("HOME");
            std::env::set_var("HOME", path);
            Self { previous_home }
        }
    }

    impl Drop for ScopedHome {
        fn drop(&mut self) {
            if let Some(previous_home) = self.previous_home.as_ref() {
                std::env::set_var("HOME", previous_home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    fn unassigned_agent(session_id: &str) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Implement Workspace history".to_string()),
            title_summary: Some("Workspace history".to_string()),
            worktree_path: None,
            branch: Some("work/20260511-0100".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: Utc::now(),
        }
    }

    fn assigned_agent_with_window(
        session_id: &str,
        window_id: &str,
        worktree_path: &Path,
    ) -> WorkspaceAgentSummary {
        let mut agent = unassigned_agent(session_id);
        agent.window_id = Some(window_id.to_string());
        agent.current_focus = None;
        agent.title_summary = None;
        agent.worktree_path = Some(worktree_path.to_path_buf());
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent
    }

    fn write_session_with_project_state_root(
        session_id: &str,
        worktree_path: &Path,
        project_state_root: &Path,
    ) {
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = gwt_agent::Session::new(
            worktree_path,
            "work/20260601-0934",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.project_state_root = Some(project_state_root.to_path_buf());
        session.save(&sessions_dir).expect("write session");
    }

    #[test]
    fn parse_workspace_update_accepts_summary_fields() {
        let parsed = parse(&[
            s("update"),
            s("--title"),
            s("Fix Active Work"),
            s("--status"),
            s("active"),
            s("--summary"),
            s("Workspace state is current"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Update {
                title: Some("Fix Active Work".to_string()),
                status: Some("active".to_string()),
                status_text: None,
                summary: Some("Workspace state is current".to_string()),
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            }
        );
    }

    #[test]
    fn parse_workspace_update_accepts_agent_title_summary() {
        let parsed = parse(&[
            s("update"),
            s("--agent-session"),
            s("session-1"),
            s("--current-focus"),
            s("Implementing the title-summary contract across Board and Workspace"),
            s("--title-summary"),
            s("Title summary contract"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some(
                    "Implementing the title-summary contract across Board and Workspace"
                        .to_string()
                ),
                title_summary: Some("Title summary contract".to_string()),
            }
        );
    }

    #[test]
    fn parse_workspace_update_requires_agent_session_for_agent_title_summary() {
        let err = parse(&[
            s("update"),
            s("--title-summary"),
            s("Title summary contract"),
        ])
        .expect_err("agent title summary requires agent session");

        assert!(matches!(err, CliParseError::MissingFlag("--agent-session")));
    }

    #[test]
    fn parse_workspace_update_rejects_status_like_agent_title_summary() {
        let err = parse(&[
            s("update"),
            s("--agent-session"),
            s("session-1"),
            s("--current-focus"),
            s("Finished implementing the Agent title improvement"),
            s("--title-summary"),
            s("エージェントタイトル改善完了"),
        ])
        .expect_err("title-summary must describe the work, not its status");

        let message = err.to_string();
        assert!(message.contains("--title-summary"), "{message}");
        assert!(message.contains("work name"), "{message}");
        assert!(message.contains("status"), "{message}");
    }

    #[test]
    fn parse_workspace_create_accepts_assignment_fields() {
        let parsed = parse(&[
            s("create"),
            s("--agent-session"),
            s("session-1"),
            s("--title-summary"),
            s("Workspace history"),
            s("--current-focus"),
            s("Implementing Workspace history"),
            s("--spec"),
            s("2359"),
            s("--split-from"),
            s("workspace-existing"),
            s("--boundary"),
            s("UI only"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implementing Workspace history".to_string()),
                spec: Some(2359),
                issue: None,
                split_from: Some("workspace-existing".to_string()),
                boundary: Some("UI only".to_string()),
            }
        );
    }

    #[test]
    fn parse_workspace_candidates_and_join_commands() {
        let candidates = parse(&[s("candidates"), s("--agent-session"), s("session-1")])
            .expect("parse candidates");
        assert_eq!(
            candidates,
            WorkspaceCommand::Candidates {
                agent_session: "session-1".to_string()
            }
        );

        let join = parse(&[
            s("join"),
            s("--agent-session"),
            s("session-1"),
            s("--workspace"),
            s("workspace-existing"),
            s("--current-focus"),
            s("Continue Workspace history"),
            s("--title-summary"),
            s("Workspace history"),
        ])
        .expect("parse join");
        assert_eq!(
            join,
            WorkspaceCommand::Join {
                agent_session: "session-1".to_string(),
                workspace_id: "workspace-existing".to_string(),
                current_focus: Some("Continue Workspace history".to_string()),
                title_summary: Some("Workspace history".to_string()),
            }
        );
    }

    #[test]
    fn parse_workspace_ensure_accepts_materialization_fields() {
        let parsed = parse(&[
            s("ensure"),
            s("--agent-session"),
            s("session-1"),
            s("--title-summary"),
            s("Workspace materialization"),
            s("--current-focus"),
            s("Ensure actionable Unassigned Agents join a Workspace"),
            s("--spec"),
            s("2359"),
            s("--topic"),
            s("workspace-materialization"),
            s("--boundary"),
            s("CLI and Board write path"),
        ])
        .expect("parse ensure");

        assert_eq!(
            parsed,
            WorkspaceCommand::Ensure {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some(
                    "Ensure actionable Unassigned Agents join a Workspace".to_string()
                ),
                spec: Some(2359),
                issue: None,
                topic: Some("workspace-materialization".to_string()),
                boundary: Some("CLI and Board write path".to_string()),
            }
        );
    }

    #[test]
    fn workspace_update_persists_workspace_status() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: Some("Workspace coordination".to_string()),
                status: Some("blocked".to_string()),
                status_text: Some("Waiting on Board alignment".to_string()),
                summary: Some("Align Workspace ownership before edits".to_string()),
                next_action: Some("Post Board request".to_string()),
                owner: Some("SPEC-2359".to_string()),
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut out,
        )
        .expect("update workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace updated:"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(saved.title, "Work coordination");
        assert_eq!(saved.status_category, WorkspaceStatusCategory::Blocked);
        assert_eq!(saved.owner.as_deref(), Some("SPEC-2359"));
    }

    #[test]
    fn workspace_update_agent_session_uses_stored_project_state_root() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);

        let mut canonical = WorkspaceProjection::default_for_project(&project_root);
        canonical.agents.push(assigned_agent_with_window(
            "session-1",
            "project::agent-1",
            &worktree,
        ));
        save_workspace_projection(&project_root, &canonical).expect("save canonical projection");

        let mut env = TestEnv::new(worktree.clone());
        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some("Implement canonical Project State identity".to_string()),
                title_summary: Some("Project State identity".to_string()),
            },
            &mut out,
        )
        .expect("update workspace");

        assert_eq!(code, 0);
        let saved = load_workspace_projection(&project_root)
            .expect("load canonical projection")
            .expect("canonical projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("canonical agent");
        assert_eq!(
            agent.title_summary.as_deref(),
            Some("Project State identity")
        );
        assert_eq!(
            agent.current_focus.as_deref(),
            Some("Implement canonical Project State identity")
        );
        assert!(
            load_workspace_projection(&worktree)
                .expect("load worktree projection")
                .is_none(),
            "agent workspace update must not create a split Project State under the worktree root"
        );
    }

    #[test]
    fn workspace_update_repairs_split_agent_title_into_canonical_root() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);

        let mut canonical = WorkspaceProjection::default_for_project(&project_root);
        canonical.agents.push(assigned_agent_with_window(
            "session-1",
            "project::agent-1",
            &worktree,
        ));
        save_workspace_projection(&project_root, &canonical).expect("save canonical projection");

        let mut split = WorkspaceProjection::default_for_project(&worktree);
        let mut split_agent =
            assigned_agent_with_window("session-1", "project::agent-1", &worktree);
        split_agent.title_summary = Some("Split root title".to_string());
        split_agent.current_focus = Some("Previously written to worktree root".to_string());
        split.agents.push(split_agent);
        save_workspace_projection(&worktree, &split).expect("save split projection");

        let mut env = TestEnv::new(worktree.clone());
        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some("Continue from canonical Project State".to_string()),
                title_summary: None,
            },
            &mut out,
        )
        .expect("update workspace");

        assert_eq!(code, 0);
        let saved = load_workspace_projection(&project_root)
            .expect("load canonical projection")
            .expect("canonical projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("canonical agent");
        assert_eq!(
            agent.title_summary.as_deref(),
            Some("Split root title"),
            "the first canonical update after the fix must recover the title written to the old split root"
        );
        assert_eq!(
            agent.current_focus.as_deref(),
            Some("Continue from canonical Project State")
        );
    }

    #[test]
    fn workspace_join_assigns_unassigned_agent_to_existing_workspace() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace history".to_string());
        event.summary = Some("Existing Workspace".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Join {
                agent_session: "session-1".to_string(),
                workspace_id: "workspace-existing".to_string(),
                current_focus: Some("Continue Workspace history".to_string()),
                title_summary: Some("Workspace history".to_string()),
            },
            &mut out,
        )
        .expect("join workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace joined: workspace-existing"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), Some("workspace-existing"));
        assert_eq!(saved.id, "workspace-existing");
    }

    #[test]
    fn workspace_create_records_workspace_and_assigns_agent() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implement Workspace history".to_string()),
                spec: Some(2359),
                issue: None,
                split_from: None,
                boundary: Some("history slice".to_string()),
            },
            &mut out,
        )
        .expect("create workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace created: work-"), "{out}");
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let workspace_id = saved.id.clone();
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), Some(workspace_id.as_str()));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(items.work_items.len(), 1);
        assert_eq!(items.work_items[0].id, workspace_id);
        assert_eq!(items.work_items[0].title, "Work history");
    }

    /// SPEC-2359 Phase U-6 (FR-131, FR-134, FR-135, FR-136): a workspace
    /// created without `--current-focus` must still have a non-empty
    /// `summary` (auto-filled from `title_summary`), a real `created_at`
    /// timestamp, the originating Agent's `display_name` as `creator`, and
    /// an initial `WorkEvent { kind: Start }` so the Workspace
    /// Overview Lifecycle section is never empty on Day-0.
    #[test]
    fn workspace_create_autofills_summary_and_metadata_when_current_focus_missing() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace U-6 autofill".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                split_from: None,
                boundary: None,
            },
            &mut out,
        )
        .expect("create workspace");

        assert_eq!(code, 0);
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(
            saved.summary.as_deref(),
            Some("Workspace U-6 autofill"),
            "summary must fall back to title_summary when --current-focus is omitted"
        );
        assert_eq!(
            saved.lifecycle_stage,
            gwt_core::workspace_projection::WorkspaceLifecycleStage::Active,
            "lifecycle_stage must initialize to Active on workspace create"
        );
        assert_ne!(
            saved.created_at,
            gwt_core::workspace_projection::workspace_projection_default_created_at(),
            "created_at must be a real timestamp, not the migration sentinel"
        );
        assert!(
            saved.creator.is_some(),
            "creator must capture the originating Agent's display_name"
        );

        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(
            items.work_items.len(),
            1,
            "Workspace Overview Lifecycle requires at least one Day-0 event"
        );
    }

    #[test]
    fn workspace_candidates_lists_similar_incomplete_workspaces() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace history".to_string());
        event.intent = Some("Implement Workspace history with affiliation state".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Candidates {
                agent_session: "session-1".to_string(),
            },
            &mut out,
        )
        .expect("list candidates");

        assert_eq!(code, 0);
        assert!(out.contains("workspace-existing"), "{out}");
        assert!(out.contains("Work history"), "{out}");
        assert!(out.contains("score="), "{out}");
    }

    #[test]
    fn workspace_create_rejects_similar_workspace_without_split_boundary() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace history".to_string());
        event.intent = Some("Implement Workspace history with affiliation state".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let err = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implement Workspace history affiliation".to_string()),
                spec: None,
                issue: Some(2359),
                split_from: None,
                boundary: None,
            },
            &mut out,
        )
        .expect_err("similar Workspace should be rejected");

        assert!(err.to_string().contains("similar Workspace exists"));
    }

    #[test]
    fn workspace_create_allows_explicit_split_boundary_for_similar_workspace() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace history".to_string());
        event.intent = Some("Implement Workspace history with affiliation state".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implement Workspace history affiliation".to_string()),
                spec: None,
                issue: Some(2359),
                split_from: Some("workspace-existing".to_string()),
                boundary: Some("new affiliation state tests only".to_string()),
            },
            &mut out,
        )
        .expect("explicit split boundary should create a new Workspace");

        assert_eq!(code, 0);
        // SPEC-2359 W16-2: branch-bearing agents mint the canonical work- id.
        assert!(out.contains("workspace created: work-"), "{out}");
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_ne!(saved.id, "workspace-existing");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(agent.workspace_id.as_deref(), Some(saved.id.as_str()));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert!(items
            .work_items
            .iter()
            .any(|item| item.id == "workspace-existing"));
        assert!(items.work_items.iter().any(|item| item.id == saved.id));
    }

    #[test]
    fn workspace_ensure_joins_existing_canonical_branch_workspace_bypassing_similarity() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let repo = tempfile::tempdir().expect("repo");

        // Existing incomplete Work keyed by the canonical branch id.
        let canonical_id = gwt_core::workspace_projection::canonical_work_id(
            repo.path(),
            Some("work/canonical"),
            None,
        )
        .expect("canonical id");
        let now = Utc::now();
        let mut start = WorkEvent::new(WorkEventKind::Start, canonical_id.clone(), now);
        start.title = Some("totally different wording".to_string());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(repo.path(), start).expect("seed canonical work");

        // Live agent on the same branch with entirely dissimilar text.
        let mut projection = load_or_default_workspace_projection(repo.path()).expect("projection");
        let mut canonical_agent = unassigned_agent("session-canonical");
        canonical_agent.branch = Some("work/canonical".to_string());
        projection.agents.push(canonical_agent);
        save_workspace_projection(repo.path(), &projection).expect("save projection");

        let result = ensure_workspace_for_agent(
            repo.path(),
            WorkspaceEnsureInput {
                agent_session: "session-canonical".to_string(),
                title_summary: "no lexical overlap at all".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("ensure");
        assert_eq!(result.workspace_id, canonical_id);
        assert!(matches!(
            result.disposition,
            WorkspaceEnsureDisposition::Joined
        ));
    }

    #[test]
    fn workspace_create_for_agent_mints_canonical_id_for_branch() {
        let _guard = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedHome::set(home.path());
        let repo = tempfile::tempdir().expect("repo");

        let mut projection = load_or_default_workspace_projection(repo.path()).expect("projection");
        let mut agent = unassigned_agent("session-mint");
        agent.branch = Some("work/minted".to_string());
        agent.worktree_path = None;
        projection.agents.push(agent.clone());
        let workspace_id = create_workspace_for_agent(
            repo.path(),
            &mut projection,
            &WorkspaceEnsureInput {
                agent_session: "session-mint".to_string(),
                title_summary: "mint".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
            None,
            &agent,
        )
        .expect("create");
        let expected = gwt_core::workspace_projection::canonical_work_id(
            repo.path(),
            Some("work/minted"),
            None,
        )
        .expect("canonical id");
        assert_eq!(workspace_id, expected);
    }

    #[test]
    fn workspace_ensure_joins_similar_incomplete_workspace() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace materialization".to_string());
        event.intent = Some("Ensure actionable Unassigned Agents join Workspace".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Ensure {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some(
                    "Ensure actionable Unassigned Agents join Workspace".to_string(),
                ),
                spec: Some(2359),
                issue: None,
                topic: Some("workspace-materialization".to_string()),
                boundary: None,
            },
            &mut out,
        )
        .expect("ensure workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace ensured: workspace-existing (joined)"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), Some("workspace-existing"));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert!(items.work_items[0]
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-1"));
    }

    #[test]
    fn workspace_ensure_creates_workspace_when_no_candidate_matches() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Ensure {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some("Create Workspace from actionable intent".to_string()),
                spec: Some(2359),
                issue: None,
                topic: Some("workspace-materialization".to_string()),
                boundary: None,
            },
            &mut out,
        )
        .expect("ensure workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace ensured: work-"), "{out}");
        assert!(out.contains("(created)"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let workspace_id = saved.id.clone();
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(agent.workspace_id.as_deref(), Some(workspace_id.as_str()));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(items.work_items.len(), 1);
        assert_eq!(items.work_items[0].title, "Work materialization");
        assert_eq!(items.work_items[0].owner.as_deref(), Some("SPEC-2359"));
    }

    #[test]
    fn workspace_ensure_is_idempotent_for_already_assigned_agent() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut agent = unassigned_agent("session-1");
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some("workspace-existing".to_string());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.id = "workspace-existing".to_string();
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace materialization".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Ensure {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some("Continue current Workspace".to_string()),
                spec: Some(2359),
                issue: None,
                topic: None,
                boundary: None,
            },
            &mut out,
        )
        .expect("ensure workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace ensured: workspace-existing (already-assigned)"));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(items.work_items.len(), 1);
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn parse_workspace_projection_list_defaults_to_no_flags() {
        let cmd = parse(&args(&["projection-list"])).expect("parse projection-list");
        assert_eq!(
            cmd,
            WorkspaceCommand::ProjectionList {
                stale: false,
                all: false,
            }
        );
    }

    #[test]
    fn parse_workspace_projection_list_accepts_stale_and_all_flags() {
        let cmd = parse(&args(&["projection-list", "--stale", "--all"]))
            .expect("parse projection-list --stale --all");
        assert_eq!(
            cmd,
            WorkspaceCommand::ProjectionList {
                stale: true,
                all: true,
            }
        );
    }

    #[test]
    fn parse_workspace_projection_prune_defaults_to_apply_mode() {
        let cmd = parse(&args(&["projection-prune"])).expect("parse projection-prune");
        assert_eq!(
            cmd,
            WorkspaceCommand::ProjectionPrune {
                dry_run: false,
                ids: Vec::new(),
            }
        );
    }

    #[test]
    fn parse_workspace_projection_prune_accepts_dry_run() {
        let cmd = parse(&args(&["projection-prune", "--dry-run"]))
            .expect("parse projection-prune --dry-run");
        assert_eq!(
            cmd,
            WorkspaceCommand::ProjectionPrune {
                dry_run: true,
                ids: Vec::new(),
            }
        );
    }

    #[test]
    fn parse_workspace_projection_prune_accepts_repeated_ids() {
        let cmd = parse(&args(&[
            "projection-prune",
            "--id",
            "abc-123",
            "--id",
            "def-456",
        ]))
        .expect("parse projection-prune --id ... --id ...");
        assert_eq!(
            cmd,
            WorkspaceCommand::ProjectionPrune {
                dry_run: false,
                ids: vec!["abc-123".to_string(), "def-456".to_string()],
            }
        );
    }

    #[test]
    fn parse_workspace_projection_prune_rejects_unknown_flag() {
        let err =
            parse(&args(&["projection-prune", "--bogus"])).expect_err("unknown flag should fail");
        assert!(matches!(err, CliParseError::UnknownSubcommand(_)));
    }

    use gwt_core::workspace_projection::{
        save_workspace_projection_to_path, WorkspaceLifecycleStage,
    };

    fn seed_stale_workspace(
        scan_root: &std::path::Path,
        id: &str,
        hash: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
        lifecycle: WorkspaceLifecycleStage,
    ) {
        let project_dir = scan_root.join(hash);
        let workspace_dir = project_dir.join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
        let mut projection = WorkspaceProjection::default_for_project(&project_dir);
        projection.id = id.to_string();
        projection.updated_at = updated_at;
        projection.lifecycle_stage = lifecycle;
        save_workspace_projection_to_path(&workspace_dir.join("current.json"), &projection)
            .expect("save");
    }

    #[test]
    fn run_projection_list_with_scan_root_emits_actionable_only_by_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = Utc::now();
        seed_stale_workspace(
            tmp.path(),
            "ws-stale",
            "stale-hash",
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        seed_stale_workspace(
            tmp.path(),
            "ws-fresh",
            "fresh-hash",
            now,
            WorkspaceLifecycleStage::Active,
        );

        let mut out = String::new();
        let code = run_projection_list_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            now,
            false,
            false,
            |_| false,
            &mut out,
        )
        .expect("list");
        assert_eq!(code, 0);
        assert!(out.contains("ws-stale"), "stale workspace must be listed");
        assert!(
            !out.contains("ws-fresh"),
            "fresh workspace must be filtered out in default (actionable) mode",
        );
        assert!(out.contains("mode: actionable"));
    }

    #[test]
    fn run_projection_list_with_scan_root_includes_fresh_when_all_flag_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = Utc::now();
        seed_stale_workspace(
            tmp.path(),
            "ws-fresh",
            "fresh-hash",
            now,
            WorkspaceLifecycleStage::Active,
        );

        let mut out = String::new();
        let code = run_projection_list_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            now,
            false,
            true,
            |_| false,
            &mut out,
        )
        .expect("list");
        assert_eq!(code, 0);
        assert!(out.contains("ws-fresh"));
        assert!(out.contains("mode: all"));
    }

    #[test]
    fn run_projection_prune_with_scan_root_dry_run_reports_plan_without_changes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = Utc::now();
        seed_stale_workspace(
            tmp.path(),
            "ws-archive-me",
            "stale-hash",
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );

        let mut out = String::new();
        let code = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            now,
            true,
            &[],
            |_| false,
            &mut out,
        )
        .expect("prune dry-run");
        assert_eq!(code, 0);
        assert!(out.contains("DRY-RUN: archive=1 delete=0 skip=0"));
        // dry-run should not mutate lifecycle_stage
        let projection_path = tmp.path().join("stale-hash/workspace/current.json");
        let loaded =
            gwt_core::workspace_projection::load_workspace_projection_from_path(&projection_path)
                .expect("load")
                .expect("present");
        assert_eq!(loaded.lifecycle_stage, WorkspaceLifecycleStage::Active);
    }

    #[test]
    fn run_projection_prune_with_scan_root_apply_persists_archive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = Utc::now();
        seed_stale_workspace(
            tmp.path(),
            "ws-archive-me",
            "stale-hash",
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );

        let mut out = String::new();
        let code = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            now,
            false,
            &[],
            |_| false,
            &mut out,
        )
        .expect("prune apply");
        assert_eq!(code, 0);
        assert!(out.contains("APPLIED: archive=1"));

        let projection_path = tmp.path().join("stale-hash/workspace/current.json");
        let loaded =
            gwt_core::workspace_projection::load_workspace_projection_from_path(&projection_path)
                .expect("load")
                .expect("present");
        assert_eq!(loaded.lifecycle_stage, WorkspaceLifecycleStage::Archived);
    }

    #[test]
    fn run_projection_prune_with_scan_root_filters_by_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = Utc::now();
        seed_stale_workspace(
            tmp.path(),
            "ws-keep",
            "keep-hash",
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );
        seed_stale_workspace(
            tmp.path(),
            "ws-take",
            "take-hash",
            now - chrono::Duration::days(40),
            WorkspaceLifecycleStage::Active,
        );

        let mut out = String::new();
        let _ = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            now,
            false,
            &["ws-take".to_string()],
            |_| false,
            &mut out,
        )
        .expect("prune by id");
        assert!(out.contains("APPLIED: archive=1"));

        let keep = gwt_core::workspace_projection::load_workspace_projection_from_path(
            &tmp.path().join("keep-hash/workspace/current.json"),
        )
        .expect("load keep")
        .expect("present");
        assert_eq!(
            keep.lifecycle_stage,
            WorkspaceLifecycleStage::Active,
            "id filter must leave non-matching workspaces untouched",
        );
        let take = gwt_core::workspace_projection::load_workspace_projection_from_path(
            &tmp.path().join("take-hash/workspace/current.json"),
        )
        .expect("load take")
        .expect("present");
        assert_eq!(take.lifecycle_stage, WorkspaceLifecycleStage::Archived);
    }
}
