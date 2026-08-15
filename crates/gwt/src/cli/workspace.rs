use std::path::Path;

use chrono::{DateTime, Utc};
use gwt_core::error::GwtError;
use gwt_core::paths::{
    gwt_projects_dir, gwt_workspace_projection_path_for_repo_path,
    gwt_workspace_work_items_path_for_repo_path,
};
use gwt_core::workspace_projection::{
    apply_prune_plan, classify_workspace_projections, load_or_default_workspace_projection,
    load_or_synthesize_workspace_work_items, load_workspace_projection_from_path,
    load_workspace_work_items_from_path, transact_workspace_state,
    transact_workspace_state_for_work_event_root_with_preflight,
    update_workspace_projection_with_journal_for_resolved_work_target, ClassifiedProjection,
    PruneAction, PruneSkipReason, TrackedWorkEventPolicy, WorkEvent, WorkEventKind, WorkItem,
    WorkItemsProjection, WorkspaceAgentSummary, WorkspaceExecutionContainerRef,
    WorkspaceProjection, WorkspaceProjectionUpdate, WorkspaceRetentionConfig, WorkspaceStartUpdate,
    WorkspaceStatusCategory,
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
    let mut progress_summary = None;
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
            "--progress-summary" => progress_summary = Some(value),
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
        progress_summary,
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

struct WorkspaceUpdateBridgeAuthority {
    identity: gwt_agent::SessionExecutionIdentity,
    project_state_root: std::path::PathBuf,
    work_id: String,
    owner: Option<String>,
    known_journal_entry_ids: Option<Vec<String>>,
    known_work_event_ids: Vec<String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_runtime_binding: Option<gwt_agent::DockerRuntimeBinding>,
    local_continuation_eligible: bool,
}

enum WorkspaceUpdateBridgeAuthoritySnapshot {
    Exact(Box<WorkspaceUpdateBridgeAuthority>),
    NeedsEnsure,
    Unavailable,
}

fn snapshot_workspace_update_bridge_authority(
    repo_path: &std::path::Path,
    session_id: &str,
) -> WorkspaceUpdateBridgeAuthoritySnapshot {
    let recovery = match crate::agent_project_state::validated_workspace_recovery_session(
        repo_path, session_id,
    ) {
        Ok(Some(recovery)) => recovery,
        Ok(None) | Err(_) => return WorkspaceUpdateBridgeAuthoritySnapshot::Unavailable,
    };
    let (recovery, policy, host_recovery) = match recovery {
        crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(recovery) => {
            (recovery, WorkspaceEnsurePolicy::HostMayBootstrap, true)
        }
        crate::agent_project_state::ValidatedWorkspaceEnsureSession::Docker(recovery) => {
            (recovery, WorkspaceEnsurePolicy::DockerExistingOnly, false)
        }
    };
    let projection = load_workspace_projection_from_path(
        &gwt_workspace_projection_path_for_repo_path(&recovery.project_state_root),
    )
    .ok()
    .flatten();
    let work_items = load_workspace_work_items_from_path(
        &gwt_workspace_work_items_path_for_repo_path(&recovery.project_state_root),
    )
    .ok()
    .flatten();
    let (Some(projection), Some(work_items)) = (projection, work_items) else {
        return WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure;
    };
    let expected_owner = match resolve_workspace_ensure_owner(Some(&recovery.session), None, None) {
        Ok(owner) => owner,
        Err(_) => return WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure,
    };
    let input = WorkspaceEnsureInput {
        agent_session: session_id.to_string(),
        title_summary: String::new(),
        current_focus: None,
        spec: None,
        issue: None,
        topic: None,
        boundary: None,
    };
    let authority = validate_workspace_ensure_recovery_state(
        &recovery,
        &input,
        expected_owner.as_deref(),
        policy,
        &projection,
        &work_items,
    );
    let Ok(WorkspaceEnsureAuthorityState::ExactExisting {
        canonical_id: work_id,
        canonicalize_work_agent_id,
        canonicalize_work_owner,
    }) = authority
    else {
        return WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure;
    };
    if canonicalize_work_agent_id
        || canonicalize_work_owner
        || !projection
            .latest_agent_for_session(session_id)
            .is_some_and(|agent| {
                agent.is_assigned()
                    && agent.workspace_id.as_deref() == Some(work_id.as_str())
                    && agent.agent_id == recovery.session.agent_id.command()
            })
    {
        return WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure;
    }
    let known_journal_entry_ids =
        match gwt_core::workspace_projection::load_recent_workspace_journal_entries(
            &recovery.project_state_root,
            usize::MAX,
        ) {
            Ok(entries) => Some(entries.into_iter().map(|entry| entry.id).collect()),
            Err(_) => None,
        };
    let known_work_event_ids = work_items
        .work_items
        .iter()
        .find(|item| item.id == work_id)
        .map(|item| item.events.iter().map(|event| event.id.clone()).collect())
        .unwrap_or_default();
    let identity = match gwt_agent::SessionExecutionIdentity::from_session(&recovery.session) {
        Ok(Some(identity)) => identity,
        Ok(None) | Err(_) => return WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure,
    };
    let runtime_target = recovery.session.runtime_target;
    let docker_runtime_binding = recovery.session.docker_runtime_binding.clone();
    let local_continuation_eligible = host_recovery
        && runtime_target == gwt_agent::LaunchRuntimeTarget::Host
        && docker_runtime_binding.is_none();
    WorkspaceUpdateBridgeAuthoritySnapshot::Exact(Box::new(WorkspaceUpdateBridgeAuthority {
        identity,
        project_state_root: recovery.project_state_root,
        work_id,
        owner: expected_owner,
        known_journal_entry_ids,
        known_work_event_ids,
        runtime_target,
        docker_runtime_binding,
        local_continuation_eligible,
    }))
}

fn validate_workspace_update_bridge_receipt(
    session_id: &str,
    authority: &WorkspaceUpdateBridgeAuthority,
    request: &crate::AgentWorkspaceUpdateRequest,
    receipt: &crate::AgentWorkspaceUpdateReceipt,
) -> Result<(), String> {
    if receipt.work_id != authority.work_id {
        return Err(
            "[receipt_mismatch] Host workspace bridge returned a receipt for a different Work; no local fallback was attempted"
                .to_string(),
        );
    }
    if authority
        .known_journal_entry_ids
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| id == &receipt.journal_entry_id))
    {
        return Err(
            "[receipt_mismatch] Host workspace bridge returned stale journal receipt evidence; no local fallback was attempted"
                .to_string(),
        );
    }
    if let Some(known_journal_entry_ids) = authority.known_journal_entry_ids.as_ref() {
        if let Ok(entries) = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
            &authority.project_state_root,
            usize::MAX,
        ) {
            let receipt_entries = entries
                .iter()
                .filter(|entry| entry.id == receipt.journal_entry_id)
                .collect::<Vec<_>>();
            match receipt_entries.as_slice() {
                [entry]
                    if !known_journal_entry_ids
                        .iter()
                        .any(|id| id == &entry.id)
                        && workspace_journal_entry_matches_update(
                            session_id, authority, request, entry,
                        ) =>
                {
                    return Ok(());
                }
                [] => {}
                _ => {
                    return Err(
                        "[receipt_mismatch] Host workspace bridge receipt journal does not match the authenticated update; no local fallback was attempted"
                            .to_string(),
                    )
                }
            }
        }
    }
    validate_workspace_update_work_event_readback(
        session_id,
        authority,
        request,
        &receipt.journal_entry_id,
    )
}

fn workspace_journal_entry_matches_update(
    session_id: &str,
    authority: &WorkspaceUpdateBridgeAuthority,
    request: &crate::AgentWorkspaceUpdateRequest,
    entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> bool {
    let canonical_entry_root = dunce::canonicalize(&entry.project_root).ok();
    let intent = &request.intent;
    canonical_entry_root.as_deref() == Some(authority.project_state_root.as_path())
        && entry.agent_session_id.as_deref() == Some(session_id)
        && entry.owner == authority.owner
        && entry.title == intent.title
        && entry.status_category == intent.status_category
        && entry.status_text == intent.status_text
        && entry.next_action == intent.next_action
        && entry.summary == intent.summary
        && entry.progress_summary == intent.progress_summary
        && entry.agent_current_focus == intent.current_focus
        && entry.agent_title_summary == intent.title_summary
}

fn validate_workspace_update_work_event_readback(
    session_id: &str,
    authority: &WorkspaceUpdateBridgeAuthority,
    request: &crate::AgentWorkspaceUpdateRequest,
    receipt_evidence_id: &str,
) -> Result<(), String> {
    let work_items = load_workspace_work_items_from_path(
        &gwt_workspace_work_items_path_for_repo_path(&authority.project_state_root),
    )
    .map_err(|_| {
        "[receipt_mismatch] Host workspace bridge Work event evidence could not be read; no local fallback was attempted"
            .to_string()
    })?
    .ok_or_else(|| {
        "[receipt_mismatch] Host workspace bridge Work event evidence is missing; no local fallback was attempted"
            .to_string()
    })?;
    let matching_works = work_items
        .work_items
        .iter()
        .filter(|item| item.id == authority.work_id)
        .collect::<Vec<_>>();
    let [work] = matching_works.as_slice() else {
        return Err(
            "[receipt_mismatch] Host workspace bridge Work event evidence is ambiguous; no local fallback was attempted"
                .to_string(),
        );
    };
    let expected_kind = match request.intent.status_category {
        Some(WorkspaceStatusCategory::Done) => WorkEventKind::Done,
        Some(WorkspaceStatusCategory::Blocked) => WorkEventKind::Blocked,
        _ => WorkEventKind::Update,
    };
    let expected_title = request
        .intent
        .title
        .as_ref()
        .or(request.intent.title_summary.as_ref());
    let expected_summary = request
        .intent
        .summary
        .as_ref()
        .or(request.intent.status_text.as_ref());
    let matching_new_events = work
        .events
        .iter()
        .filter(|event| {
            event.id == receipt_evidence_id
                && !authority
                    .known_work_event_ids
                    .iter()
                    .any(|id| id == &event.id)
                && event.kind == expected_kind
                && event.work_item_id == authority.work_id
                && event.agent_session_id.as_deref() == Some(session_id)
                && event.owner == authority.owner
                && event.title.as_ref() == expected_title
                && event.intent == request.intent.current_focus
                && event.summary.as_ref() == expected_summary
                && event.progress_summary == request.intent.progress_summary
                && event.status_category == request.intent.status_category
                && event.next_action == request.intent.next_action
        })
        .count();
    if matching_new_events != 1 {
        return Err(
            "[receipt_mismatch] Host workspace bridge receipt did not identify one exact new Work event; no local fallback was attempted"
                .to_string(),
        );
    }
    Ok(())
}

fn continue_workspace_update_after_typed_ensure_required(
    session_id: &str,
    authority: WorkspaceUpdateBridgeAuthority,
    request: crate::AgentWorkspaceUpdateRequest,
) -> Result<crate::AgentWorkspaceUpdateReceipt, String> {
    if !authority.local_continuation_eligible {
        return Err(
            "typed workspace.ensure compatibility continuation is available only for an exact Host Session authority"
                .to_string(),
        );
    }
    let binding = authority.identity.execution_binding.clone();
    let work_id = authority.work_id.clone();
    let project_state_root = authority.project_state_root.clone();
    let work_event_root = authority.identity.worktree_path.clone();
    let expected_runtime_target = authority.runtime_target;
    let expected_docker_runtime_binding = authority.docker_runtime_binding.clone();
    let terminal_update = request.intent.status_category == Some(WorkspaceStatusCategory::Done);
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    let result = crate::cli::execution_state::with_current_active_session_execution_identity_global_lease(
        &gwt_core::paths::gwt_sessions_dir(),
        &authority.identity,
        |settlement_trusted_dir| -> Result<crate::AgentWorkspaceUpdateReceipt, String> {
            let current = gwt_agent::Session::load(&session_path).map_err(|_| {
                "typed workspace.ensure compatibility continuation could not reload the durable Session runtime authority"
                    .to_string()
            })?;
            if current.runtime_target != expected_runtime_target
                || current.docker_runtime_binding != expected_docker_runtime_binding
                || current.runtime_target != gwt_agent::LaunchRuntimeTarget::Host
                || current.docker_runtime_binding.is_some()
            {
                return Err(
                    "typed workspace.ensure compatibility continuation was refused because the Session runtime target or Docker binding changed"
                        .to_string(),
                );
            }
            crate::agent_project_state::apply_bound_authenticated_workspace_update_for_exact_work_with_held_global_lease(
                &project_state_root,
                session_id,
                &binding,
                &work_id,
                settlement_trusted_dir,
                request,
            )
            .map_err(|error| {
                format!("typed workspace.ensure compatibility continuation was refused: {error}")
            })
        },
    )
    .map_err(|_| {
        "typed workspace.ensure compatibility continuation could not validate the durable authority"
            .to_string()
    })?;
    let result = result.ok_or_else(|| {
        "typed workspace.ensure compatibility continuation was refused because the Session or execution binding changed"
            .to_string()
    })?;
    let receipt = result?;
    if terminal_update {
        if let Err(error) = crate::cli::verification_record::save_work_event_settlement_record(
            &work_event_root,
            session_id,
            true,
        ) {
            tracing::warn!(
                ?error,
                "terminal compatibility continuation persisted; retaining the write-ahead settlement receipt after refresh failure"
            );
        }
    }
    publish_workspace_change(&project_state_root);
    Ok(receipt)
}

struct LocalWorkspaceUpdateTransaction<'a> {
    invocation_repo_path: &'a Path,
    session_id: &'a str,
    target: &'a crate::agent_project_state::SessionWorkMutationTarget,
    tracked_event_policy: TrackedWorkEventPolicy,
    opens_work_settlement: bool,
}

fn persist_local_workspace_update(
    transaction: &LocalWorkspaceUpdateTransaction<'_>,
    update: WorkspaceProjectionUpdate,
    settlement_trusted_dir: Option<&Path>,
) -> gwt_core::error::Result<gwt_core::workspace_projection::WorkspaceJournalEntry> {
    let persistence_target = transaction.target.persistence_target();
    update_workspace_projection_with_journal_for_resolved_work_target(
        &persistence_target,
        update,
        transaction.tracked_event_policy,
        |_, _| {
            let refreshed = crate::agent_project_state::resolve_session_work_mutation_target(
                transaction.invocation_repo_path,
                transaction.session_id,
            )?;
            if refreshed != *transaction.target {
                return Err(GwtError::Other(
                    "Session-bound workspace target changed before commit".to_string(),
                ));
            }
            Ok(())
        },
        |event, journal_entry| {
            if !transaction.opens_work_settlement {
                return Ok(());
            }
            let trusted_dir = settlement_trusted_dir.ok_or_else(|| {
                GwtError::Other(
                    "workspace.update terminal Work event settlement lease is missing".to_string(),
                )
            })?;
            crate::cli::verification_record::prepare_work_event_settlement_record_with_held_lease(
                trusted_dir,
                &transaction.target.work_event_root,
                &transaction.target.session_id,
                event,
                journal_entry,
            )
            .map(|_| ())
            .map_err(|error| {
                GwtError::Other(format!(
                    "workspace.update could not reserve the terminal Work event settlement obligation before mutation: {error}"
                ))
            })
        },
    )
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
            progress_summary,
            next_action,
            owner,
            agent_session,
            current_focus,
            title_summary,
        } => {
            let status_category = status
                .as_deref()
                .map(parse_status_category)
                .transpose()
                .map_err(string_error)?;
            let session_id = std::env::var(gwt_agent::session::GWT_SESSION_ID_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    string_error(
                        "workspace.update requires ambient GWT_SESSION_ID for mutation".to_string(),
                    )
                })?;
            if agent_session
                .as_deref()
                .is_some_and(|explicit| explicit != session_id)
            {
                return Err(string_error(
                    "workspace.update agent Session must exactly match ambient GWT_SESSION_ID"
                        .to_string(),
                ));
            }
            let legacy_repo_path = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let intent = crate::AgentWorkspaceUpdateIntent {
                title,
                status_category,
                status_text,
                summary,
                progress_summary,
                next_action,
                owner,
                current_focus,
                title_summary,
            };
            if let Some(target) =
                crate::daemon_runtime::HookForwardTarget::from_env_strict().map_err(string_error)?
            {
                // Snapshot any locally readable exact durable authority before contacting the
                // Host. A readable legacy or invalid canonical state must pass workspace.ensure
                // first. When container isolation makes local authority unavailable, authenticated
                // Host 2xx remains authoritative, but no local continuation is allowed.
                let bridge_authority = match snapshot_workspace_update_bridge_authority(
                    &legacy_repo_path,
                    &session_id,
                ) {
                    WorkspaceUpdateBridgeAuthoritySnapshot::Exact(authority) => Some(*authority),
                    WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure => {
                        return Err(string_error(
                            "[workspace_ensure_required] managed workspace.update requires an exact canonical Work authority; run workspace.ensure before retrying"
                                .to_string(),
                        ));
                    }
                    WorkspaceUpdateBridgeAuthoritySnapshot::Unavailable => None,
                };
                // A complete bridge capability makes Host the authority boundary. Do not read a
                // container-local Session ledger to redirect this observation: it may be absent or
                // deliberately isolated, and Host revalidates the claimed Session against these
                // exact invocation facts before committing anything.
                let observation = crate::observe_agent_runtime(&legacy_repo_path)
                    .map_err(|error| string_error(error.to_string()))?;
                let request = crate::AgentWorkspaceUpdateRequest {
                    schema_version: crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                    claimed_session_id: session_id.clone(),
                    observation,
                    intent,
                };
                let receipt =
                    match crate::daemon_runtime::send_workspace_update_via_agent_bridge_detailed(
                        &target, &request,
                    ) {
                        Ok(receipt) => {
                            if let Some(authority) = bridge_authority.as_ref() {
                                validate_workspace_update_bridge_receipt(
                                    &session_id,
                                    authority,
                                    &request,
                                    &receipt,
                                )
                                .map_err(string_error)?;
                            }
                            receipt
                        }
                        Err(error) if error.is_exact_workspace_ensure_required() => {
                            let authority =
                                bridge_authority.ok_or_else(|| string_error(error.to_string()))?;
                            continue_workspace_update_after_typed_ensure_required(
                                &session_id,
                                authority,
                                request,
                            )
                            .map_err(string_error)?
                        }
                        Err(error) => return Err(string_error(error.to_string())),
                    };
                out.push_str(&format!(
                    "workspace updated: {}\n",
                    receipt.journal_entry_id
                ));
                return Ok(0);
            }
            if std::env::var_os(gwt_agent::session::GWT_SESSION_RUNTIME_PATH_ENV).is_some() {
                return Err(string_error(
                    "managed workspace.update is missing its Host bridge capability; relaunch the Session"
                        .to_string(),
                ));
            }
            let operation_repo_path = match crate::agent_project_state::durable_session_runtime_target_if_session_exists(&session_id) {
                Some(Ok(gwt_agent::LaunchRuntimeTarget::Host)) => {
                    match crate::agent_project_state::resolve_execution_recovery_context_if_session_exists(
                        env.repo_path(),
                        &session_id,
                    ) {
                        Some(Ok(context)) => context.worktree().to_path_buf(),
                        Some(Err(error)) => return Err(core_error(error)),
                        None => legacy_repo_path,
                    }
                }
                Some(Ok(gwt_agent::LaunchRuntimeTarget::Docker)) | None => legacy_repo_path,
                Some(Err(error)) => return Err(core_error(error)),
            };
            let target = crate::agent_project_state::resolve_session_work_mutation_target(
                &operation_repo_path,
                &session_id,
            )
            .map_err(core_error)?;
            let tracked_event_policy =
                if crate::cli::execution_state::is_completed(&target.work_event_root) {
                    TrackedWorkEventPolicy::SkipTracked
                } else {
                    TrackedWorkEventPolicy::Persist
                };
            let opens_work_settlement = tracked_event_policy == TrackedWorkEventPolicy::Persist
                && intent.status_category
                    == Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
            tracing::debug!(
                session_id = %target.session_id,
                work_id = %target.work_id,
                branch = %target.branch_identity,
                worktree = %target.worktree_identity.display(),
                project_state_root = %target.project_state_root.display(),
                work_event_root = %target.work_event_root.display(),
                "resolved Session-bound workspace.update target"
            );
            let update = WorkspaceProjectionUpdate {
                title: intent.title,
                status_category: intent.status_category,
                status_text: intent.status_text,
                owner: intent.owner,
                next_action: intent.next_action,
                summary: intent.summary,
                progress_summary: intent.progress_summary,
                agent_session_id: Some(target.session_id.clone()),
                agent_current_focus: intent.current_focus,
                agent_title_summary: intent.title_summary,
            };
            let transaction = LocalWorkspaceUpdateTransaction {
                invocation_repo_path: &operation_repo_path,
                session_id: &session_id,
                target: &target,
                tracked_event_policy,
                opens_work_settlement,
            };
            let entry = if !opens_work_settlement {
                persist_local_workspace_update(&transaction, update, None)
                    .map_err(|error| string_error(error.to_string()))?
            } else {
                let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(
                    &target.work_event_root,
                )
                .ok_or_else(|| {
                    string_error(
                        "workspace.update could not resolve the terminal Work event settlement store before mutation"
                            .to_string(),
                    )
                })?;
                let nested = crate::cli::trusted_store::with_write_lease_for_resolved_dir(
                    &trusted_dir,
                    || -> std::io::Result<_> {
                        Ok(persist_local_workspace_update(
                            &transaction,
                            update,
                            Some(&trusted_dir),
                        ))
                    },
                )
                .map_err(|_| {
                    string_error(
                        "workspace.update could not acquire the terminal Work event settlement lease before mutation"
                            .to_string(),
                    )
                })?;
                nested.map_err(|error| string_error(error.to_string()))?
            };
            if opens_work_settlement {
                if let Err(error) =
                    crate::cli::verification_record::save_work_event_settlement_record(
                        &target.work_event_root,
                        &target.session_id,
                        true,
                    )
                {
                    tracing::warn!(
                        ?error,
                        "terminal Work event persisted; retaining the write-ahead settlement receipt after refresh failure"
                    );
                }
            }
            publish_workspace_change(&target.project_state_root);
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
            transact_workspace_state(env.repo_path(), |projection, work_items, _persisted| {
                let Some(item) = work_items
                    .work_items
                    .iter()
                    .find(|item| item.id == workspace_id)
                else {
                    return Err(GwtError::Other(format!(
                        "workspace not found: {workspace_id}"
                    )));
                };
                if item.is_terminal() {
                    return Err(GwtError::Other(format!(
                        "cannot join terminal Work: {workspace_id}"
                    )));
                }
                let Some(agent) = projection.latest_agent_for_session(&agent_session).cloned()
                else {
                    return Err(GwtError::Other(format!(
                        "agent session not found: {agent_session}"
                    )));
                };
                let event = workspace_claim_event(
                    &workspace_id,
                    &agent_session,
                    current_focus.clone(),
                    title_summary.clone().or_else(|| Some(item.title.clone())),
                    item.owner.clone(),
                    None,
                    &agent,
                );
                assign_agent_to_workspace(
                    projection,
                    &agent_session,
                    &workspace_id,
                    current_focus,
                    title_summary,
                )
                .map_err(spec_ops_as_core_error)?;
                apply_workspace_item_to_projection(projection, item);
                Ok(((), vec![event]))
            })
            .map_err(core_error)?;
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
            let workspace_id =
                transact_workspace_state(env.repo_path(), |projection, existing, _persisted| {
                    let Some(agent) = projection.latest_agent_for_session(&agent_session).cloned()
                    else {
                        return Err(GwtError::Other(format!(
                            "agent session not found: {agent_session}"
                        )));
                    };
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
                                && workspace_similarity_score(&new_text, &workspace_item_text(item))
                                    >= 2
                        }) {
                            return Err(GwtError::Other(format!(
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
                    let next_action = boundary
                        .as_deref()
                        .map(|boundary| format!("Boundary: {boundary}"))
                        .unwrap_or_else(|| "Coordinate on Board before implementation".to_string());
                    let mut event = WorkEvent::new(WorkEventKind::Start, workspace_id.clone(), now);
                    event.title = Some(title_summary.clone());
                    event.intent = current_focus.clone();
                    event.summary = current_focus
                        .clone()
                        .or_else(|| Some(title_summary.clone()));
                    event.status_category = Some(WorkspaceStatusCategory::Active);
                    event.owner = owner.clone();
                    event.next_action = Some(next_action.clone());
                    event.agent_session_id = Some(agent_session.clone());
                    event.agent_id = Some(agent.agent_id.clone());
                    event.display_name = Some(agent.display_name.clone());
                    event.execution_container =
                        Some(workspace_execution_container_from_agent(&agent));
                    if let Some(split_from) = split_from {
                        event.kind = WorkEventKind::Split;
                        event.related_work_item_id = Some(split_from);
                    }
                    projection.start_work(
                        WorkspaceStartUpdate {
                            workspace_id: workspace_id.clone(),
                            title: title_summary.clone(),
                            status_text: current_focus.clone(),
                            summary: current_focus
                                .clone()
                                .or_else(|| Some(title_summary.clone())),
                            owner,
                            next_action,
                        },
                        now,
                    );
                    projection.created_at = now;
                    projection.creator = Some(agent.display_name.clone());
                    projection.lifecycle_stage =
                        gwt_core::workspace_projection::WorkspaceLifecycleStage::Active;
                    assign_agent_to_workspace(
                        projection,
                        &agent_session,
                        &workspace_id,
                        current_focus,
                        Some(title_summary),
                    )
                    .map_err(spec_ops_as_core_error)?;
                    projection.updated_at = now;
                    Ok((workspace_id, vec![event]))
                })
                .map_err(core_error)?;
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
        WorkspaceCommand::WorkPrune {
            dry_run,
            ids,
            project_root,
        } => {
            let target = project_root
                .as_deref()
                .map(Path::new)
                .unwrap_or_else(|| env.repo_path());
            run_work_prune(target, dry_run, &ids, out)
        }
    }
}

/// Issue #3448 AC-1: settle incomplete Works whose owner Issue is already
/// closed. Reads the local Issue cache for owner state, so a cache miss keeps
/// the Work (fail-closed via [`classify_stale_works`]). Closing goes through
/// the canonical `emit_workspace_done_event_if_absent`, which is idempotent
/// and keeps `works.json` a pure fold of the event log.
fn run_work_prune(
    repo_path: &Path,
    dry_run: bool,
    ids: &[String],
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let all_works: Vec<WorkItem> = load_workspace_work_items_from_path(&work_items_path)
        .map_err(core_error)?
        .map(|projection| projection.work_items)
        .unwrap_or_default();
    let works: Vec<WorkItem> = if ids.is_empty() {
        all_works
    } else {
        all_works
            .into_iter()
            .filter(|item| ids.iter().any(|id| id == &item.id))
            .collect()
    };

    let cache =
        crate::issue_cache::issue_cache_root_for_repo_path(repo_path).map(gwt_github::Cache::new);
    let plan = classify_stale_works(&works, |number| {
        let cache = cache.as_ref()?;
        let entry = cache.load_entry(gwt_github::IssueNumber(number))?;
        Some(entry.snapshot.state == gwt_github::client::IssueState::Open)
    });

    let mode = if dry_run { "DRY-RUN" } else { "APPLIED" };
    let mut closed = 0usize;
    let mut failed = 0usize;
    for candidate in &plan.candidates {
        out.push_str(&format!(
            "  close {} — owner #{} (closed) — {}\n",
            candidate.work_id, candidate.owner_number, candidate.title
        ));
        if dry_run {
            continue;
        }
        match gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
            repo_path,
            &candidate.work_id,
            Utc::now(),
        ) {
            Ok(_) => closed += 1,
            Err(error) => {
                failed += 1;
                out.push_str(&format!(
                    "  ! {} could not be closed: {error}\n",
                    candidate.work_id
                ));
            }
        }
    }

    // Second pass: orphaned worktree-scan placeholders. They are discarded
    // rather than completed — they never represented work.
    let orphans = classify_orphaned_backfill_works(&works, |path| path.exists());
    let mut discarded = 0usize;
    for candidate in &orphans.candidates {
        out.push_str(&format!(
            "  discard {} — orphaned worktree placeholder — {}\n",
            candidate.work_id, candidate.title
        ));
        if dry_run {
            continue;
        }
        match gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
            repo_path,
            &candidate.work_id,
            Utc::now(),
        ) {
            Ok(_) => discarded += 1,
            Err(error) => {
                failed += 1;
                out.push_str(&format!(
                    "  ! {} could not be discarded: {error}\n",
                    candidate.work_id
                ));
            }
        }
    }

    let mut reasons: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for skip in &plan.skipped {
        *reasons.entry(skip.reason.as_str()).or_default() += 1;
    }
    let skip_detail = reasons
        .iter()
        .map(|(reason, count)| format!("{reason}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    out.push_str(&format!(
        "{mode}: closed_candidates={} closed={} discard_candidates={} discarded={} failed={} skipped={} [{skip_detail}]\n",
        plan.candidates.len(),
        closed,
        orphans.candidates.len(),
        discarded,
        failed,
        plan.skipped.len(),
    ));
    if failed > 0 {
        return Ok(1);
    }
    Ok(0)
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
    let mut summary = apply_prune_plan(&filtered, dry_run).map_err(core_error)?;
    let legacy_summary =
        prune_empty_workspace_state_files(scan_root, dry_run, ids).map_err(core_error)?;
    summary.archived += legacy_summary.archived;
    summary.deleted += legacy_summary.deleted;
    summary.skipped += legacy_summary.skipped;
    let mode = if dry_run { "DRY-RUN" } else { "APPLIED" };
    out.push_str(&format!(
        "{}: archive={} delete={} skip={}\n",
        mode, summary.archived, summary.deleted, summary.skipped,
    ));
    Ok(0)
}

fn prune_empty_workspace_state_files(
    scan_root: &Path,
    dry_run: bool,
    ids: &[String],
) -> gwt_core::Result<gwt_core::workspace_projection::PruneSummary> {
    let mut summary = gwt_core::workspace_projection::PruneSummary::default();
    if !ids.is_empty() {
        return Ok(summary);
    }

    let entries = match std::fs::read_dir(scan_root) {
        Ok(entries) => entries,
        Err(_) => return Ok(summary),
    };

    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }
        let workspace_path = project_dir.join("workspace.json");
        if !workspace_path.is_file() {
            continue;
        }
        let Ok(state) = crate::load_workspace_state(&workspace_path) else {
            continue;
        };
        if !state.windows.is_empty() {
            continue;
        }
        if !dry_run {
            std::fs::remove_file(&workspace_path).map_err(|err| {
                gwt_core::GwtError::Other(format!(
                    "failed to remove no-window workspace state {}: {}",
                    workspace_path.display(),
                    err
                ))
            })?;
            let _ = std::fs::remove_dir(&project_dir);
        }
        summary.deleted += 1;
    }

    Ok(summary)
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
    let probe = probe_workspace_ensure(repo_path, &input);
    if !probe.executable() {
        return Err(core_error(GwtError::Other(format!(
            "workspace.ensure prerequisite probe refused: {}",
            probe.reason.as_deref().unwrap_or("unavailable")
        ))));
    }
    let recovery = crate::agent_project_state::validated_workspace_recovery_session(
        repo_path,
        &input.agent_session,
    )
    .map_err(core_error)?;
    let owner = resolve_workspace_ensure_owner(
        recovery.as_ref().map(|recovery| match recovery {
            crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(recovery) => {
                &recovery.session
            }
            crate::agent_project_state::ValidatedWorkspaceEnsureSession::Docker(recovery) => {
                &recovery.session
            }
        }),
        input.spec,
        input.issue,
    )
    .map_err(core_error)?;
    let (result, publish_root) = match recovery {
        Some(crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(recovery)) => {
            let exact_session =
                gwt_agent::SessionExecutionIdentity::from_session(&recovery.session)
                    .map_err(|error| {
                        core_error(GwtError::Other(format!(
                            "invalid durable Host Session execution identity: {error}"
                        )))
                    })?
                    .ok_or_else(|| {
                        core_error(GwtError::Other(format!(
                            "durable Host Session {} has no execution identity",
                            recovery.session.id
                        )))
                    })?;
            let result =
                crate::cli::execution_state::with_current_active_session_execution_identity_lease(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &exact_session,
                    || {
                        transact_workspace_state_for_work_event_root_with_preflight(
                            &recovery.project_state_root,
                            &recovery.work_event_root,
                            |projection, existing, _| {
                                validate_workspace_ensure_recovery_state(
                                    &recovery,
                                    &input,
                                    owner.as_deref(),
                                    WorkspaceEnsurePolicy::HostMayBootstrap,
                                    projection,
                                    existing,
                                )
                                .map(|_| ())
                            },
                            |projection, existing, persisted| {
                                apply_workspace_ensure_transition(
                                    &recovery.project_state_root,
                                    projection,
                                    existing,
                                    persisted,
                                    &input,
                                    owner.clone(),
                                    Some(&recovery),
                                )
                            },
                        )
                    },
                )
                .map_err(|error| core_error(GwtError::Io(error)))?
                .ok_or_else(|| {
                    core_error(GwtError::Other(format!(
                        "durable Host Session {} changed before workspace.ensure publication",
                        recovery.session.id
                    )))
                })?
                .map_err(core_error)?;
            (result, recovery.project_state_root)
        }
        Some(crate::agent_project_state::ValidatedWorkspaceEnsureSession::Docker(recovery)) => {
            let exact_session =
                gwt_agent::SessionExecutionIdentity::from_session(&recovery.session)
                    .map_err(|error| {
                        core_error(GwtError::Other(format!(
                            "invalid durable Docker Session execution identity: {error}"
                        )))
                    })?
                    .ok_or_else(|| {
                        core_error(GwtError::Other(format!(
                            "durable Docker Session {} has no execution identity",
                            recovery.session.id
                        )))
                    })?;
            let result =
                crate::cli::execution_state::with_current_active_session_execution_identity_lease(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &exact_session,
                    || {
                        transact_workspace_state_for_work_event_root_with_preflight(
                            &recovery.project_state_root,
                            &recovery.work_event_root,
                            |projection, existing, _| {
                                validate_workspace_ensure_recovery_state(
                                    &recovery,
                                    &input,
                                    owner.as_deref(),
                                    WorkspaceEnsurePolicy::DockerExistingOnly,
                                    projection,
                                    existing,
                                )
                                .map(|_| ())
                            },
                            |projection, existing, _| {
                                apply_existing_docker_workspace_ensure_transition(
                                    projection,
                                    existing,
                                    &input,
                                    owner.as_deref(),
                                    &recovery,
                                )
                            },
                        )
                    },
                )
                .map_err(|error| core_error(GwtError::Io(error)))?
                .ok_or_else(|| {
                    core_error(GwtError::Other(format!(
                        "durable Docker Session {} changed before workspace.ensure validation",
                        recovery.session.id
                    )))
                })?
                .map_err(core_error)?;
            (result, recovery.project_state_root)
        }
        None => {
            return Err(core_error(GwtError::Other(format!(
                "Session ledger is missing for Session {}",
                input.agent_session
            ))));
        }
    };
    publish_workspace_change(&publish_root);
    Ok(result)
}

/// Side-effect-free prerequisite evaluator shared by `execution.status` and
/// the `workspace.ensure` mutation preflight.
pub(crate) fn probe_workspace_ensure(
    repo_path: &Path,
    input: &WorkspaceEnsureInput,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{
        GovernanceCause, GovernanceEffect, GovernanceMetadata, RecoveryProbe,
    };
    let protected = |cause| GovernanceMetadata {
        effect: Some(GovernanceEffect::Protected),
        cause,
        retryable: Some(false),
        ..GovernanceMetadata::default()
    };
    let recovery = match crate::agent_project_state::validated_workspace_recovery_session(
        repo_path,
        &input.agent_session,
    ) {
        Ok(Some(recovery)) => recovery,
        Ok(None) => {
            return RecoveryProbe::unavailable(
                "workspace.ensure",
                protected(Some(GovernanceCause::ManagedIdentity)),
                format!(
                    "Session ledger is missing for Session {}",
                    input.agent_session
                ),
            )
        }
        Err(error) => {
            return RecoveryProbe::unavailable(
                "workspace.ensure",
                protected(Some(GovernanceCause::Authority)),
                error.to_string(),
            )
        }
    };
    let (recovery, policy) = match &recovery {
        crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(recovery) => {
            (recovery, WorkspaceEnsurePolicy::HostMayBootstrap)
        }
        crate::agent_project_state::ValidatedWorkspaceEnsureSession::Docker(recovery) => {
            (recovery, WorkspaceEnsurePolicy::DockerExistingOnly)
        }
    };
    let owner =
        match resolve_workspace_ensure_owner(Some(&recovery.session), input.spec, input.issue) {
            Ok(owner) => owner,
            Err(error) => {
                return RecoveryProbe::unavailable(
                    "workspace.ensure",
                    protected(Some(GovernanceCause::ManagedIdentity)),
                    error.to_string(),
                )
            }
        };
    let projection_path = gwt_workspace_projection_path_for_repo_path(&recovery.project_state_root);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(&recovery.project_state_root);
    let projection = match load_workspace_projection_from_path(&projection_path) {
        Ok(Some(projection)) => projection,
        Ok(None) => WorkspaceProjection::default_for_project(&recovery.project_state_root),
        Err(error) => {
            return RecoveryProbe::unavailable(
                "workspace.ensure",
                protected(Some(GovernanceCause::StructuralGovernance)),
                error.to_string(),
            )
        }
    };
    let work_items = match load_workspace_work_items_from_path(&work_items_path) {
        Ok(Some(work_items)) => work_items,
        Ok(None) => WorkItemsProjection::empty(Utc::now()),
        Err(error) => {
            return RecoveryProbe::unavailable(
                "workspace.ensure",
                protected(Some(GovernanceCause::StructuralGovernance)),
                error.to_string(),
            )
        }
    };
    let generation = recovery
        .session
        .execution_binding
        .as_ref()
        .map(|binding| binding.identity.generation_id.clone());
    let governance = GovernanceMetadata {
        effect: Some(GovernanceEffect::Protected),
        fingerprint: Some(format!(
            "workspace.ensure:{}:{}",
            input.agent_session,
            generation.as_deref().unwrap_or("unbound")
        )),
        retryable: Some(true),
        repository_target: recovery.session.repo_hash.clone(),
        target_state: Some("workspace_assignment".to_string()),
        execution_generation: generation,
        ..GovernanceMetadata::default()
    };
    match validate_workspace_ensure_recovery_state(
        recovery,
        input,
        owner.as_deref(),
        policy,
        &projection,
        &work_items,
    ) {
        Ok(authority) if authority.requires_mutation() => {
            RecoveryProbe::available("workspace.ensure", governance)
        }
        Ok(_) => RecoveryProbe::satisfied("workspace.ensure", governance),
        Err(error) => RecoveryProbe::unavailable(
            "workspace.ensure",
            GovernanceMetadata {
                cause: Some(GovernanceCause::DomainInvalid),
                ..governance
            },
            error.to_string(),
        ),
    }
}

/// Canonical, explicit status candidate used only for read-only diagnosis.
/// Mutation callers must pass their real request to [`probe_workspace_ensure`].
pub(crate) fn workspace_ensure_status_candidate(session_id: &str) -> WorkspaceEnsureInput {
    WorkspaceEnsureInput {
        agent_session: session_id.to_string(),
        title_summary: "Recovered Work".to_string(),
        current_focus: None,
        spec: None,
        issue: None,
        topic: None,
        boundary: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceEnsurePolicy {
    HostMayBootstrap,
    DockerExistingOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceEnsureAuthorityState {
    Missing {
        canonical_id: String,
    },
    ExactExisting {
        canonical_id: String,
        canonicalize_work_agent_id: bool,
        canonicalize_work_owner: bool,
    },
}

impl WorkspaceEnsureAuthorityState {
    fn canonical_id(&self) -> &str {
        match self {
            Self::Missing { canonical_id } | Self::ExactExisting { canonical_id, .. } => {
                canonical_id
            }
        }
    }

    fn requires_mutation(&self) -> bool {
        match self {
            Self::Missing { .. } => true,
            Self::ExactExisting {
                canonicalize_work_agent_id,
                canonicalize_work_owner,
                ..
            } => *canonicalize_work_agent_id || *canonicalize_work_owner,
        }
    }
}

fn workspace_ensure_agent_identity_matches(
    stored: Option<&str>,
    durable: &gwt_agent::AgentId,
) -> bool {
    let Some(stored) = stored.filter(|value| !value.is_empty()) else {
        return false;
    };
    if stored.eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID) {
        return false;
    }
    match durable {
        gwt_agent::AgentId::Custom(expected) => stored == expected,
        _ => gwt_agent::resolve_agent_id(stored).as_ref() == Some(durable),
    }
}

/// Canonicalize a stored Work owner onto the durable `SPEC-<n>` spelling.
///
/// Two legacy spellings reach the same SPEC owner and are safe to upgrade:
/// `Issue #<n>` (a Work started as a plain Issue that later gained the
/// `gwt-spec` label) and `SPEC #<n>` (SPEC #3431 FR-070 — the spelling the
/// knowledge-launch wizard stamped before it was aligned with the binding).
/// Both are one-way: nothing downgrades a durable SPEC owner.
fn workspace_ensure_can_upgrade_owner(stored: Option<&str>, durable: Option<&str>) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    let Some(durable) = durable else {
        return false;
    };
    let Some((prefix, stored_number)) = ["Issue #", "SPEC #"].into_iter().find_map(|prefix| {
        stored
            .strip_prefix(prefix)
            .and_then(|number| number.parse::<u64>().ok())
            .map(|number| (prefix, number))
    }) else {
        return false;
    };
    let Some(durable_number) = durable
        .strip_prefix("SPEC-")
        .and_then(|number| number.parse::<u64>().ok())
    else {
        return false;
    };
    stored == format!("{prefix}{stored_number}")
        && durable == format!("SPEC-{durable_number}")
        && stored_number == durable_number
}

fn validate_workspace_ensure_recovery_state(
    recovery: &crate::agent_project_state::ValidatedWorkspaceRecoverySession,
    input: &WorkspaceEnsureInput,
    expected_owner: Option<&str>,
    policy: WorkspaceEnsurePolicy,
    projection: &WorkspaceProjection,
    existing: &WorkItemsProjection,
) -> gwt_core::error::Result<WorkspaceEnsureAuthorityState> {
    let canonical_id = gwt_core::workspace_projection::canonical_work_id(
        &recovery.project_state_root,
        Some(recovery.branch_identity.as_str()),
        Some(recovery.worktree_identity.as_path()),
    )
    .ok_or_else(|| {
        GwtError::Other(format!(
            "canonical Work identity is unavailable for Session {}",
            input.agent_session
        ))
    })?;
    let durable_agent_id = recovery.session.agent_id.command();
    let session_rows = projection
        .agents
        .iter()
        .filter(|agent| agent.session_id == input.agent_session)
        .collect::<Vec<_>>();
    if session_rows.len() > 1 {
        return Err(GwtError::Other(format!(
            "projection authority for Session {} is ambiguous",
            input.agent_session
        )));
    }
    for agent in session_rows {
        let identity_matches = agent
            .branch
            .as_deref()
            .map(crate::agent_project_state::canonical_branch_identity)
            .as_deref()
            == Some(recovery.branch_identity.as_str())
            && agent
                .worktree_path
                .as_deref()
                .map(crate::agent_project_state::normalize_mutation_path)
                .as_deref()
                == Some(recovery.worktree_identity.as_path());
        if !identity_matches {
            return Err(GwtError::Other(format!(
                "projection identity mismatch for Session {}",
                input.agent_session
            )));
        }
        if !workspace_ensure_agent_identity_matches(
            Some(agent.agent_id.as_str()),
            &recovery.session.agent_id,
        ) {
            return Err(GwtError::Other(format!(
                "projection agent identity mismatch for Session {}: durable={}, projection={}",
                input.agent_session, durable_agent_id, agent.agent_id
            )));
        }
        if agent.is_assigned() && agent.workspace_id.as_deref() != Some(canonical_id.as_str()) {
            return Err(GwtError::Other(format!(
                "Session {} has a noncanonical Workspace assignment",
                input.agent_session
            )));
        }
        if agent.is_unassigned() && agent.workspace_id.is_some() {
            return Err(GwtError::Other(format!(
                "Session {} has an ambiguous Unassigned Workspace id",
                input.agent_session
            )));
        }
    }
    if projection
        .latest_agent_for_session(&input.agent_session)
        .is_none()
        && policy == WorkspaceEnsurePolicy::HostMayBootstrap
        && recovery.session.execution_binding.is_none()
    {
        return Err(GwtError::Other(format!(
            "workspace.ensure bootstrap requires a current durable execution binding for Session {}",
            input.agent_session
        )));
    }
    if let Some(item) = existing.work_items.iter().find(|item| {
        item.id != canonical_id
            && item
                .agents
                .iter()
                .any(|agent| agent.session_id == input.agent_session)
    }) {
        return Err(GwtError::Other(format!(
            "Session {} is already attached to noncanonical Work {}",
            input.agent_session, item.id
        )));
    }
    let canonical_items = existing
        .work_items
        .iter()
        .filter(|item| item.id == canonical_id)
        .collect::<Vec<_>>();
    if canonical_items.len() > 1 {
        return Err(GwtError::Other(format!(
            "canonical Work {canonical_id} is ambiguous"
        )));
    }
    let Some(item) = canonical_items.first().copied() else {
        if policy == WorkspaceEnsurePolicy::DockerExistingOnly {
            return Err(GwtError::Other(format!(
                "Docker workspace.ensure for Session {} cannot recover a missing Work; the originating launch must remain failed",
                input.agent_session
            )));
        }
        return Ok(WorkspaceEnsureAuthorityState::Missing { canonical_id });
    };
    if item.is_terminal() {
        return Err(GwtError::Other(format!(
            "canonical Work {canonical_id} is terminal"
        )));
    }
    let canonicalize_work_owner = item.owner.as_deref() != expected_owner;
    if canonicalize_work_owner
        && !workspace_ensure_can_upgrade_owner(item.owner.as_deref(), expected_owner)
    {
        return Err(GwtError::Other(format!(
            "canonical Work {canonical_id} owner mismatch: durable={}, stored={}",
            expected_owner.unwrap_or("<none>"),
            item.owner.as_deref().unwrap_or("<none>")
        )));
    }
    let session_refs = item
        .agents
        .iter()
        .filter(|agent| agent.session_id == input.agent_session)
        .collect::<Vec<_>>();
    if session_refs.len() > 1 {
        return Err(GwtError::Other(format!(
            "canonical Work {canonical_id} has ambiguous Session agent refs"
        )));
    }
    let session_ref = session_refs.first().copied();
    if let Some(session_ref) = session_ref {
        if !workspace_ensure_agent_identity_matches(
            session_ref.agent_id.as_deref(),
            &recovery.session.agent_id,
        ) {
            return Err(GwtError::Other(format!(
                "Work agent identity mismatch for Session {}: durable={}, stored={}",
                input.agent_session,
                durable_agent_id,
                session_ref.agent_id.as_deref().unwrap_or("<none>")
            )));
        }
    }
    let matching_containers = item
        .execution_containers
        .iter()
        .filter(|container| workspace_execution_container_matches_recovery(container, recovery))
        .count();
    if matching_containers != 1 {
        return Err(GwtError::Other(format!(
            "canonical Work {canonical_id} has ambiguous exact execution containers"
        )));
    }
    if policy == WorkspaceEnsurePolicy::DockerExistingOnly {
        let exact_assigned = projection
            .latest_agent_for_session(&input.agent_session)
            .is_some_and(|agent| {
                agent.is_assigned() && agent.workspace_id.as_deref() == Some(canonical_id.as_str())
            });
        if !exact_assigned {
            return Err(GwtError::Other(format!(
                "Docker workspace.ensure for Session {} requires an exact existing assignment; the originating launch must remain failed",
                input.agent_session
            )));
        }
    }
    Ok(WorkspaceEnsureAuthorityState::ExactExisting {
        canonicalize_work_agent_id: session_ref.and_then(|agent| agent.agent_id.as_deref())
            != Some(durable_agent_id),
        canonicalize_work_owner,
        canonical_id,
    })
}

fn workspace_execution_container_matches_recovery(
    container: &WorkspaceExecutionContainerRef,
    recovery: &crate::agent_project_state::ValidatedWorkspaceRecoverySession,
) -> bool {
    let branch_matches = container
        .branch
        .as_deref()
        .map(crate::agent_project_state::canonical_branch_identity)
        .as_deref()
        == Some(recovery.branch_identity.as_str());
    let worktree_matches = container
        .worktree_path
        .as_deref()
        .map(crate::agent_project_state::normalize_mutation_path)
        .as_deref()
        == Some(recovery.worktree_identity.as_path());
    branch_matches && worktree_matches
}

#[allow(clippy::too_many_arguments)]
fn apply_workspace_ensure_transition(
    work_identity_root: &Path,
    projection: &mut WorkspaceProjection,
    existing: &WorkItemsProjection,
    persisted: bool,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    recovery: Option<&crate::agent_project_state::ValidatedWorkspaceRecoverySession>,
) -> gwt_core::error::Result<(WorkspaceEnsureResult, Vec<WorkEvent>)> {
    if let Some(recovery) = recovery {
        return apply_session_bound_workspace_ensure_transition(
            projection, existing, input, owner, recovery,
        );
    }
    if projection
        .latest_agent_for_session(&input.agent_session)
        .is_none()
    {
        let Some(recovery) = recovery else {
            return Err(GwtError::Other(format!(
                "agent session not found: {}",
                input.agent_session
            )));
        };
        if recovery.session.execution_binding.is_none() {
            return Err(GwtError::Other(format!(
                "workspace.ensure bootstrap requires a current durable execution binding for Session {}",
                input.agent_session
            )));
        }
        crate::cli::hook::register_session_in_projection(projection, &recovery.session, Utc::now());
    }
    let mut agent = projection
        .latest_agent_for_session(&input.agent_session)
        .cloned()
        .ok_or_else(|| {
            GwtError::Other(format!("agent session not found: {}", input.agent_session))
        })?;
    if let Some(recovery) = recovery {
        let session_rows_match = projection
            .agents
            .iter()
            .filter(|stored| stored.session_id == input.agent_session)
            .all(|stored| {
                stored.branch.as_deref() == Some(recovery.branch_identity.as_str())
                    && stored
                        .worktree_path
                        .as_deref()
                        .and_then(|path| dunce::canonicalize(path).ok())
                        .as_deref()
                        == Some(recovery.worktree_identity.as_path())
            });
        if !session_rows_match {
            return Err(GwtError::Other(format!(
                "projection identity mismatch for Session {}",
                input.agent_session
            )));
        }
        agent.branch = Some(recovery.branch_identity.clone());
        agent.worktree_path = Some(recovery.worktree_identity.clone());
        for stored in projection
            .agents
            .iter_mut()
            .filter(|stored| stored.session_id == input.agent_session)
        {
            stored.branch = agent.branch.clone();
            stored.worktree_path = agent.worktree_path.clone();
        }
    }
    let canonical_id = gwt_core::workspace_projection::canonical_work_id(
        work_identity_root,
        recovery
            .map(|recovery| recovery.branch_identity.as_str())
            .or(agent.branch.as_deref()),
        recovery
            .map(|recovery| recovery.worktree_identity.as_path())
            .or(agent.worktree_path.as_deref()),
    );
    let session_bound_canonical = recovery.is_some() && canonical_id.is_some();
    if session_bound_canonical
        && agent.is_assigned()
        && agent.workspace_id.as_deref() != canonical_id.as_deref()
    {
        return Err(GwtError::Other(format!(
            "Session {} has a noncanonical Workspace assignment",
            input.agent_session
        )));
    }
    let noncanonical_work_ids = if session_bound_canonical {
        existing
            .work_items
            .iter()
            .filter(|item| {
                Some(item.id.as_str()) != canonical_id.as_deref()
                    && item
                        .agents
                        .iter()
                        .any(|agent| agent.session_id == input.agent_session)
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if !noncanonical_work_ids.is_empty() {
        return Err(GwtError::Other(format!(
            "Session {} is already attached to noncanonical Work {}",
            input.agent_session,
            noncanonical_work_ids.join(", ")
        )));
    }

    if agent.is_assigned() {
        if let Some(workspace_id) = agent.workspace_id.as_deref() {
            let assignment_is_canonical =
                !session_bound_canonical || canonical_id.as_deref() == Some(workspace_id);
            if assignment_is_canonical {
                let existing_item = persisted
                    .then(|| {
                        existing
                            .work_items
                            .iter()
                            .find(|item| item.id == workspace_id && item.is_incomplete())
                    })
                    .flatten();
                if let Some(item) = existing_item {
                    apply_workspace_item_to_projection(projection, item);
                }
                assign_agent_to_workspace(
                    projection,
                    &input.agent_session,
                    workspace_id,
                    input.current_focus.clone(),
                    Some(input.title_summary.clone()),
                )
                .map_err(spec_ops_as_core_error)?;
                let events = existing_item
                    .is_none()
                    .then(|| workspace_start_event(workspace_id, input, owner.clone(), &agent))
                    .into_iter()
                    .collect();
                return Ok((
                    WorkspaceEnsureResult {
                        workspace_id: workspace_id.to_string(),
                        disposition: WorkspaceEnsureDisposition::AlreadyAssigned,
                    },
                    events,
                ));
            }
        }
    }

    if let Some(item) = canonical_id.as_deref().and_then(|canonical_id| {
        existing
            .work_items
            .iter()
            .find(|item| item.is_incomplete() && item.id == canonical_id)
    }) {
        let workspace_id = item.id.clone();
        assign_agent_to_workspace(
            projection,
            &input.agent_session,
            &workspace_id,
            input.current_focus.clone(),
            Some(input.title_summary.clone()),
        )
        .map_err(spec_ops_as_core_error)?;
        apply_workspace_item_to_projection(projection, item);
        let events = vec![workspace_join_event(
            &workspace_id,
            input,
            owner.clone(),
            &agent,
        )];
        return Ok((
            WorkspaceEnsureResult {
                workspace_id,
                disposition: WorkspaceEnsureDisposition::Joined,
            },
            events,
        ));
    }

    let ensure_text = workspace_ensure_text(input, owner.as_deref());
    if !session_bound_canonical {
        if let Some(item) =
            best_workspace_candidate(&existing.work_items, &ensure_text, owner.as_deref())
        {
            let workspace_id = item.id.clone();
            assign_agent_to_workspace(
                projection,
                &input.agent_session,
                &workspace_id,
                input.current_focus.clone(),
                Some(input.title_summary.clone()),
            )
            .map_err(spec_ops_as_core_error)?;
            apply_workspace_item_to_projection(projection, item);
            let event = workspace_join_event(&workspace_id, input, owner.clone(), &agent);
            return Ok((
                WorkspaceEnsureResult {
                    workspace_id,
                    disposition: WorkspaceEnsureDisposition::Joined,
                },
                vec![event],
            ));
        }
    }

    let (workspace_id, event) =
        create_workspace_for_agent(work_identity_root, projection, input, owner, &agent)
            .map_err(spec_ops_as_core_error)?;
    Ok((
        WorkspaceEnsureResult {
            workspace_id,
            disposition: WorkspaceEnsureDisposition::Created,
        },
        vec![event],
    ))
}

fn apply_session_bound_workspace_ensure_transition(
    projection: &mut WorkspaceProjection,
    existing: &WorkItemsProjection,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    recovery: &crate::agent_project_state::ValidatedWorkspaceRecoverySession,
) -> gwt_core::error::Result<(WorkspaceEnsureResult, Vec<WorkEvent>)> {
    let authority = validate_workspace_ensure_recovery_state(
        recovery,
        input,
        owner.as_deref(),
        WorkspaceEnsurePolicy::HostMayBootstrap,
        projection,
        existing,
    )?;
    let had_exact_assignment = projection
        .latest_agent_for_session(&input.agent_session)
        .is_some_and(|agent| {
            agent.is_assigned() && agent.workspace_id.as_deref() == Some(authority.canonical_id())
        });
    let was_assigned = projection
        .latest_agent_for_session(&input.agent_session)
        .is_some_and(WorkspaceAgentSummary::is_assigned);
    if projection
        .latest_agent_for_session(&input.agent_session)
        .is_none()
    {
        crate::cli::hook::register_session_in_projection(projection, &recovery.session, Utc::now());
    }
    let mut agent = projection
        .latest_agent_for_session(&input.agent_session)
        .cloned()
        .ok_or_else(|| {
            GwtError::Other(format!("agent session not found: {}", input.agent_session))
        })?;
    agent.agent_id = recovery.session.agent_id.command().to_string();
    agent.branch = Some(recovery.branch_identity.clone());
    agent.worktree_path = Some(recovery.worktree_identity.clone());
    if let Some(stored) = projection.latest_agent_for_session_mut(&input.agent_session) {
        stored.agent_id = agent.agent_id.clone();
        stored.branch = agent.branch.clone();
        stored.worktree_path = agent.worktree_path.clone();
    }

    match authority {
        WorkspaceEnsureAuthorityState::ExactExisting {
            canonical_id,
            canonicalize_work_agent_id,
            canonicalize_work_owner,
        } => {
            let item = existing
                .work_items
                .iter()
                .find(|item| item.id == canonical_id)
                .ok_or_else(|| {
                    GwtError::Other(format!(
                        "canonical Work {canonical_id} disappeared during workspace.ensure"
                    ))
                })?;
            apply_workspace_item_to_projection(projection, item);
            if canonicalize_work_owner {
                projection.owner = owner.clone();
            }
            assign_agent_to_workspace(
                projection,
                &input.agent_session,
                &canonical_id,
                input.current_focus.clone(),
                Some(input.title_summary.clone()),
            )
            .map_err(spec_ops_as_core_error)?;
            let events = if canonicalize_work_agent_id || canonicalize_work_owner {
                vec![workspace_authority_correction_event(
                    item,
                    &input.agent_session,
                    recovery.session.agent_id.command(),
                    !had_exact_assignment,
                    if canonicalize_work_owner {
                        owner.as_deref()
                    } else {
                        None
                    },
                )?]
            } else {
                Vec::new()
            };
            Ok((
                WorkspaceEnsureResult {
                    workspace_id: canonical_id,
                    disposition: WorkspaceEnsureDisposition::AlreadyAssigned,
                },
                events,
            ))
        }
        WorkspaceEnsureAuthorityState::Missing { canonical_id } => {
            let (workspace_id, event) = create_workspace_for_agent(
                &recovery.project_state_root,
                projection,
                input,
                owner,
                &agent,
            )
            .map_err(spec_ops_as_core_error)?;
            if workspace_id != canonical_id {
                return Err(GwtError::Other(format!(
                    "canonical Work identity changed during workspace.ensure: expected {canonical_id}, got {workspace_id}"
                )));
            }
            Ok((
                WorkspaceEnsureResult {
                    workspace_id,
                    disposition: if was_assigned {
                        WorkspaceEnsureDisposition::AlreadyAssigned
                    } else {
                        WorkspaceEnsureDisposition::Created
                    },
                },
                vec![event],
            ))
        }
    }
}

fn owner_from_spec_or_issue(spec: Option<u64>, issue: Option<u64>) -> Option<String> {
    spec.map(|number| format!("SPEC-{number}"))
        .or_else(|| issue.map(|number| format!("Issue #{number}")))
}

fn resolve_workspace_ensure_owner(
    session: Option<&gwt_agent::Session>,
    spec: Option<u64>,
    issue: Option<u64>,
) -> gwt_core::error::Result<Option<String>> {
    if spec.is_some() && issue.is_some() {
        return Err(GwtError::Other(
            "workspace.ensure accepts only one of spec or issue".to_string(),
        ));
    }
    let explicit = owner_from_spec_or_issue(spec, issue);
    let Some(session) = session else {
        return Ok(explicit);
    };
    if let Some(binding) = session.execution_binding.as_ref() {
        let durable = match binding.owner_kind.as_str() {
            "spec" => format!("SPEC-{}", binding.owner_number),
            "issue" => format!("Issue #{}", binding.owner_number),
            _ => {
                return Err(GwtError::Other(
                    "invalid durable Session owner kind".to_string(),
                ))
            }
        };
        if explicit.as_deref().is_some_and(|owner| owner != durable) {
            return Err(GwtError::Other(format!(
                "workspace.ensure owner mismatch: durable={durable}, requested={}",
                explicit.as_deref().unwrap_or_default()
            )));
        }
        return Ok(Some(durable));
    }
    if let Some(number) = session.linked_issue_number {
        if spec.is_some_and(|requested| requested != number)
            || issue.is_some_and(|requested| requested != number)
        {
            return Err(GwtError::Other(format!(
                "workspace.ensure owner mismatch: durable={number}, requested={}",
                spec.or(issue).unwrap_or_default()
            )));
        }
        return Ok(explicit.or_else(|| Some(format!("Issue #{number}"))));
    }
    Ok(explicit)
}

fn apply_existing_docker_workspace_ensure_transition(
    projection: &mut WorkspaceProjection,
    existing: &WorkItemsProjection,
    input: &WorkspaceEnsureInput,
    owner: Option<&str>,
    recovery: &crate::agent_project_state::ValidatedWorkspaceRecoverySession,
) -> gwt_core::error::Result<(WorkspaceEnsureResult, Vec<WorkEvent>)> {
    let authority = validate_workspace_ensure_recovery_state(
        recovery,
        input,
        owner,
        WorkspaceEnsurePolicy::DockerExistingOnly,
        projection,
        existing,
    )?;
    let WorkspaceEnsureAuthorityState::ExactExisting {
        canonical_id: workspace_id,
        canonicalize_work_agent_id,
        canonicalize_work_owner,
    } = authority
    else {
        return Err(GwtError::Other(format!(
            "Docker workspace.ensure for Session {} cannot recover a missing Work",
            input.agent_session
        )));
    };
    if let Some(agent) = projection.latest_agent_for_session_mut(&input.agent_session) {
        agent.agent_id = recovery.session.agent_id.command().to_string();
    }
    if canonicalize_work_owner {
        projection.owner = owner.map(str::to_string);
    }
    let item = existing
        .work_items
        .iter()
        .find(|item| item.id == workspace_id.as_str())
        .ok_or_else(|| {
            GwtError::Other(format!(
                "canonical Work {workspace_id} disappeared during Docker workspace.ensure"
            ))
        })?;
    let events = if canonicalize_work_agent_id || canonicalize_work_owner {
        vec![workspace_authority_correction_event(
            item,
            &input.agent_session,
            recovery.session.agent_id.command(),
            false,
            if canonicalize_work_owner { owner } else { None },
        )?]
    } else {
        Vec::new()
    };
    Ok((
        WorkspaceEnsureResult {
            workspace_id,
            disposition: WorkspaceEnsureDisposition::AlreadyAssigned,
        },
        events,
    ))
}

fn workspace_authority_correction_event(
    item: &WorkItem,
    session_id: &str,
    canonical_agent_id: &str,
    establishes_assignment: bool,
    canonical_owner: Option<&str>,
) -> gwt_core::error::Result<WorkEvent> {
    let kind = if establishes_assignment {
        WorkEventKind::Claim
    } else {
        WorkEventKind::Update
    };
    let latest = item
        .events
        .iter()
        .map(|event| event.updated_at)
        .chain(std::iter::once(item.updated_at))
        .max()
        .unwrap_or(item.updated_at);
    let strictly_after_latest = latest
        .checked_add_signed(chrono::Duration::nanoseconds(1))
        .ok_or_else(|| {
            GwtError::Other(format!(
                "canonical Work {} has an unrepresentable future event timestamp",
                item.id
            ))
        })?;
    let mut event = WorkEvent::new(kind, item.id.clone(), Utc::now().max(strictly_after_latest));
    event.agent_session_id = Some(session_id.to_string());
    event.agent_id = Some(canonical_agent_id.to_string());
    event.owner = canonical_owner.map(str::to_string);
    if establishes_assignment {
        event.status_category = Some(item.status_category);
    }
    Ok(event)
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
    owner: Option<&str>,
) -> Option<&'a WorkItem> {
    work_items
        .iter()
        .filter(|item| {
            item.is_incomplete()
                && owner.is_none_or(|owner| {
                    item.owner
                        .as_deref()
                        .is_none_or(|item_owner| item_owner == owner)
                })
        })
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

fn workspace_join_event(
    workspace_id: &str,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    agent: &WorkspaceAgentSummary,
) -> WorkEvent {
    workspace_claim_event(
        workspace_id,
        &input.agent_session,
        input.current_focus.clone(),
        Some(input.title_summary.clone()),
        owner,
        input.boundary.as_deref(),
        agent,
    )
}

#[allow(clippy::too_many_arguments)]
fn workspace_claim_event(
    workspace_id: &str,
    agent_session: &str,
    current_focus: Option<String>,
    title_summary: Option<String>,
    owner: Option<String>,
    boundary: Option<&str>,
    agent: &WorkspaceAgentSummary,
) -> WorkEvent {
    let now = Utc::now();
    let mut event = WorkEvent::new(WorkEventKind::Claim, workspace_id.to_string(), now);
    event.intent = current_focus;
    event.summary = title_summary.map(|title| format!("Joined Workspace: {title}"));
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = owner;
    event.next_action = boundary.map(|boundary| format!("Boundary: {boundary}"));
    event.agent_session_id = Some(agent_session.to_string());
    event.agent_id = Some(agent.agent_id.clone());
    event.display_name = Some(agent.display_name.clone());
    event.execution_container = Some(workspace_execution_container_from_agent(agent));
    event
}

fn create_workspace_for_agent(
    repo_path: &std::path::Path,
    projection: &mut WorkspaceProjection,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    agent: &WorkspaceAgentSummary,
) -> Result<(String, WorkEvent), SpecOpsError> {
    // SPEC-2359 W16-2 (FR-389): canonical, machine-independent Work id when
    // the agent has a branch / worktree; millis fallback for branchless agents.
    let workspace_id = gwt_core::workspace_projection::canonical_work_id(
        repo_path,
        agent.branch.as_deref(),
        agent.worktree_path.as_deref(),
    )
    .unwrap_or_else(|| format!("workspace-{}", Utc::now().timestamp_millis()));
    let now = Utc::now();
    let event = workspace_start_event(&workspace_id, input, owner.clone(), agent);

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
    Ok((workspace_id, event))
}

fn workspace_start_event(
    workspace_id: &str,
    input: &WorkspaceEnsureInput,
    owner: Option<String>,
    agent: &WorkspaceAgentSummary,
) -> WorkEvent {
    let mut event = WorkEvent::new(WorkEventKind::Start, workspace_id, Utc::now());
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
    event.owner = owner;
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
    event
}

fn spec_ops_as_core_error(error: SpecOpsError) -> GwtError {
    GwtError::Other(error.to_string())
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
        .latest_agent_for_session(agent_session)
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

/// Issue #3448: one incomplete Work whose owner Issue is already closed, plus
/// the evidence that made it a candidate. Reported before anything is written
/// so a dry run can be reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleWorkCandidate {
    pub(crate) work_id: String,
    pub(crate) title: String,
    pub(crate) owner: String,
    pub(crate) owner_number: u64,
}

/// A Work that is deliberately left alone, with the reason. Every non-candidate
/// lands here so the operator can audit why nothing happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedWork {
    pub(crate) work_id: String,
    pub(crate) reason: StaleWorkSkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StaleWorkSkipReason {
    /// Already Done or explicitly discarded — nothing to settle.
    AlreadyTerminal,
    /// No owner recorded, so there is no Issue whose state could justify a
    /// close. Fail-closed: an ownerless Work is never auto-closed.
    OwnerMissing,
    /// The owner is recorded but its Issue state is unknown locally (absent
    /// from the Issue cache). Fail-closed for the same reason.
    OwnerStateUnknown,
    /// The owner Issue is still open, so the Work is legitimately active.
    OwnerOpen,
    /// The Work carries real state (owner, attached agent, or non-backfill
    /// history), so the placeholder rule does not apply to it.
    CarriesRealState,
    /// The placeholder still projects an existing worktree.
    WorktreePresent,
}

impl StaleWorkSkipReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyTerminal => "already_terminal",
            Self::OwnerMissing => "owner_missing",
            Self::OwnerStateUnknown => "owner_state_unknown",
            Self::OwnerOpen => "owner_open",
            Self::CarriesRealState => "carries_real_state",
            Self::WorktreePresent => "worktree_present",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StaleWorkPlan {
    pub(crate) candidates: Vec<StaleWorkCandidate>,
    pub(crate) skipped: Vec<SkippedWork>,
}

/// Issue #3448 AC-2: Work owners are recorded with drifting spellings — the
/// same Issue appears as `3327`, `Issue #3327`, `SPEC-3327`, and `SPEC #3327`.
/// Normalize to the Issue number so one Issue is one owner. Anything without a
/// number resolves to `None` and is treated as ownerless.
pub(crate) fn owner_issue_number(owner: Option<&str>) -> Option<u64> {
    let owner = owner?.trim();
    if owner.is_empty() {
        return None;
    }
    let digits: String = owner
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u64>().ok()
}

/// Issue #3448 AC-1/AC-5: decide which incomplete Works belong to an already
/// closed owner Issue. Pure: `issue_is_open` answers "is Issue N open?" and
/// returns `None` when the state cannot be determined locally.
///
/// Fail-closed by construction — a Work is a candidate only when its owner
/// resolves to an Issue that is *known* to be closed. Missing owner, unknown
/// state, and open owners all skip with a recorded reason, so a cache miss can
/// never close live work.
pub(crate) fn classify_stale_works<F>(works: &[WorkItem], issue_is_open: F) -> StaleWorkPlan
where
    F: Fn(u64) -> Option<bool>,
{
    let mut plan = StaleWorkPlan::default();
    for work in works {
        if work.is_terminal() {
            plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::AlreadyTerminal,
            });
            continue;
        }
        let Some(number) = owner_issue_number(work.owner.as_deref()) else {
            plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::OwnerMissing,
            });
            continue;
        };
        match issue_is_open(number) {
            Some(false) => plan.candidates.push(StaleWorkCandidate {
                work_id: work.id.clone(),
                title: work.title.clone(),
                owner: work.owner.clone().unwrap_or_default(),
                owner_number: number,
            }),
            Some(true) => plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::OwnerOpen,
            }),
            None => plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::OwnerStateUnknown,
            }),
        }
    }
    plan
}

/// Issue #3448 / #3447: worktree scanning materializes one placeholder Work per
/// branch (`kind: backfill`, title = branch name, no owner, no agents). When the
/// worktree is later removed the placeholder survives as pure derived noise —
/// 470 of 788 rows on real data. Such a row is `discarded`, not `done`: it never
/// represented work, so marking it complete would be a lie.
///
/// Pure: `worktree_exists` answers "does this path still exist?". Fail-closed —
/// an owner, an attached agent, any non-backfill event, or a still-present
/// worktree each keeps the Work.
pub(crate) fn classify_orphaned_backfill_works<F>(
    works: &[WorkItem],
    worktree_exists: F,
) -> StaleWorkPlan
where
    F: Fn(&Path) -> bool,
{
    let mut plan = StaleWorkPlan::default();
    for work in works {
        if work.is_terminal() {
            plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::AlreadyTerminal,
            });
            continue;
        }
        let placeholder = work
            .owner
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
            && work.agents.is_empty()
            && !work.events.is_empty()
            && work
                .events
                .iter()
                .all(|event| event.kind == WorkEventKind::Backfill);
        if !placeholder {
            plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::CarriesRealState,
            });
            continue;
        }
        let paths: Vec<&Path> = work
            .execution_containers
            .iter()
            .filter_map(|container| container.worktree_path.as_deref())
            .collect();
        // No recorded path means the placeholder cannot be proven orphaned.
        if paths.is_empty() || paths.iter().any(|path| worktree_exists(path)) {
            plan.skipped.push(SkippedWork {
                work_id: work.id.clone(),
                reason: StaleWorkSkipReason::WorktreePresent,
            });
            continue;
        }
        plan.candidates.push(StaleWorkCandidate {
            work_id: work.id.clone(),
            title: work.title.clone(),
            owner: String::new(),
            owner_number: 0,
        });
    }
    plan
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gwt_core::workspace_projection::WorkAgentRef;

    // Issue #3448 AC-1/AC-2/AC-5: closed-owner Work stagnation. `classify_stale_works`
    // is the pure decision core: it names which incomplete Works belong to an
    // owner Issue that is already closed, and — fail-closed — which ones must be
    // skipped because their owner cannot be resolved or an agent may still hold
    // them. Owner spelling is normalized so `3327` / `Issue #3327` / `SPEC-3327`
    // resolve to the same Issue (AC-2).
    mod stale_work_classification {
        use super::*;

        fn work(id: &str, owner: Option<&str>, status: WorkspaceStatusCategory) -> WorkItem {
            let now = Utc::now();
            WorkItem {
                id: id.to_string(),
                title: id.to_string(),
                intent: None,
                summary: None,
                progress_summary: None,
                status_category: status,
                owner: owner.map(str::to_string),
                created_at: now,
                updated_at: now,
                completed_at: None,
                agents: Vec::new(),
                execution_containers: Vec::new(),
                board_refs: Vec::new(),
                related_work_item_ids: Vec::new(),
                events: Vec::new(),
                legacy_metadata_snapshot: None,
                legacy_metadata_authoritative: false,
                legacy_metadata_snapshot_at: None,
                duplicate_event_containers: Default::default(),
                discarded: false,
                discarded_at: None,
            }
        }

        #[test]
        fn owner_spelling_variants_resolve_to_the_same_issue() {
            for spelling in ["3327", "Issue #3327", "SPEC-3327", "SPEC #3327", "#3327"] {
                assert_eq!(
                    super::super::owner_issue_number(Some(spelling)),
                    Some(3327),
                    "owner spelling must normalize: {spelling}"
                );
            }
            assert_eq!(super::super::owner_issue_number(None), None);
            assert_eq!(super::super::owner_issue_number(Some("   ")), None);
        }

        #[test]
        fn closed_owner_work_is_a_candidate_and_open_owner_is_kept() {
            let works = vec![
                work(
                    "w-closed",
                    Some("Issue #3327"),
                    WorkspaceStatusCategory::Active,
                ),
                work("w-open", Some("2359"), WorkspaceStatusCategory::Active),
            ];
            let plan = super::super::classify_stale_works(&works, |number| match number {
                3327 => Some(false),
                2359 => Some(true),
                _ => None,
            });

            let candidates: Vec<&str> = plan
                .candidates
                .iter()
                .map(|item| item.work_id.as_str())
                .collect();
            assert_eq!(candidates, vec!["w-closed"]);
            assert!(
                plan.skipped.iter().any(|item| item.work_id == "w-open"),
                "an open owner keeps its Work active"
            );
        }

        #[test]
        fn unresolvable_owner_fails_closed() {
            let works = vec![
                work("w-no-owner", None, WorkspaceStatusCategory::Active),
                work(
                    "w-unknown",
                    Some("Issue #9999"),
                    WorkspaceStatusCategory::Active,
                ),
            ];
            let plan = super::super::classify_stale_works(&works, |_| None);

            assert!(
                plan.candidates.is_empty(),
                "a Work whose owner cannot be resolved is never closed automatically"
            );
            assert_eq!(plan.skipped.len(), 2);
        }

        // Issue #3448 AC-5: the operation must never widen its own blast radius.
        // A live-looking Work whose owner is closed is still a candidate, but a
        // Work the caller did not name via `ids` must never be touched — the
        // filter is applied before classification, not after.
        // Issue #3448 / #3447: worktree scanning materializes a placeholder Work
        // per branch (`kind: backfill`, title = branch, no owner, no agents).
        // When the worktree is later removed the placeholder is left behind as
        // pure derived noise — 470 of 788 rows on real data. They are discarded,
        // not "done": they never represented work.
        #[test]
        fn orphaned_backfill_placeholder_is_a_discard_candidate() {
            let mut item = work(
                "work-work-issue-3403-bc4a663e",
                None,
                WorkspaceStatusCategory::Idle,
            );
            item.title = "work/issue-3403".to_string();
            item.execution_containers = vec![WorkspaceExecutionContainerRef {
                branch: Some("work/issue-3403".to_string()),
                worktree_path: Some(std::path::PathBuf::from(
                    "/definitely/absent/work/issue-3403",
                )),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            }];
            item.events = vec![WorkEvent::new(
                WorkEventKind::Backfill,
                "work-work-issue-3403-bc4a663e",
                Utc::now(),
            )];

            let plan = super::super::classify_orphaned_backfill_works(&[item], |path| {
                let _ = path;
                false
            });

            assert_eq!(plan.candidates.len(), 1);
            assert_eq!(plan.candidates[0].work_id, "work-work-issue-3403-bc4a663e");
        }

        // Fail-closed: while the worktree still exists the placeholder is the
        // legitimate projection of a live worktree and must survive.
        #[test]
        fn backfill_placeholder_with_a_live_worktree_is_kept() {
            let mut item = work(
                "work-work-issue-3245-aaa",
                None,
                WorkspaceStatusCategory::Idle,
            );
            item.title = "work/issue-3245".to_string();
            item.execution_containers = vec![WorkspaceExecutionContainerRef {
                branch: Some("work/issue-3245".to_string()),
                worktree_path: Some(std::path::PathBuf::from("/present/work/issue-3245")),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            }];
            item.events = vec![WorkEvent::new(
                WorkEventKind::Backfill,
                "work-work-issue-3245-aaa",
                Utc::now(),
            )];

            let plan = super::super::classify_orphaned_backfill_works(&[item], |_| true);

            assert!(
                plan.candidates.is_empty(),
                "a live worktree keeps its placeholder"
            );
        }

        // Real Work must never be swept by the placeholder rule, even when its
        // worktree is gone: an owner, an agent, or any non-backfill event all
        // prove it carried real state.
        #[test]
        fn real_work_is_never_swept_as_a_placeholder() {
            let mut owned = work(
                "w-owner",
                Some("Issue #3327"),
                WorkspaceStatusCategory::Idle,
            );
            owned.events = vec![WorkEvent::new(
                WorkEventKind::Backfill,
                "w-owner",
                Utc::now(),
            )];
            let mut with_agent = work("w-agent", None, WorkspaceStatusCategory::Idle);
            with_agent.events = vec![WorkEvent::new(
                WorkEventKind::Backfill,
                "w-agent",
                Utc::now(),
            )];
            with_agent.agents = vec![WorkAgentRef {
                session_id: "s1".to_string(),
                agent_id: Some("codex".to_string()),
                display_name: Some("Codex".to_string()),
                updated_at: Utc::now(),
                attached_by: None,
            }];
            let mut started = work("w-started", None, WorkspaceStatusCategory::Idle);
            started.events = vec![
                WorkEvent::new(WorkEventKind::Backfill, "w-started", Utc::now()),
                WorkEvent::new(WorkEventKind::Start, "w-started", Utc::now()),
            ];

            let plan = super::super::classify_orphaned_backfill_works(
                &[owned, with_agent, started],
                |_| false,
            );

            assert!(
                plan.candidates.is_empty(),
                "owner / agent / non-backfill history each disqualify the placeholder rule"
            );
        }

        #[test]
        fn id_filter_scopes_the_plan_to_named_works() {
            let works = [
                work("w-a", Some("3327"), WorkspaceStatusCategory::Active),
                work("w-b", Some("3327"), WorkspaceStatusCategory::Active),
            ];
            let named: Vec<WorkItem> = works
                .iter()
                .filter(|item| item.id == "w-a")
                .cloned()
                .collect();
            let plan = super::super::classify_stale_works(&named, |_| Some(false));

            assert_eq!(plan.candidates.len(), 1);
            assert_eq!(plan.candidates[0].work_id, "w-a");
            assert!(
                !plan.skipped.iter().any(|item| item.work_id == "w-b"),
                "an unnamed Work must not even appear in the plan"
            );
        }

        // Issue #3448: the skip reasons are the audit trail. A cache miss and a
        // genuinely open owner are different situations and must stay
        // distinguishable in the report.
        #[test]
        fn skip_reasons_distinguish_unknown_state_from_open_owner() {
            let works = vec![
                work("w-unknown", Some("4242"), WorkspaceStatusCategory::Active),
                work("w-open", Some("2359"), WorkspaceStatusCategory::Active),
            ];
            let plan = super::super::classify_stale_works(&works, |number| match number {
                2359 => Some(true),
                _ => None,
            });

            let reason = |id: &str| {
                plan.skipped
                    .iter()
                    .find(|item| item.work_id == id)
                    .map(|item| item.reason.as_str())
            };
            assert_eq!(reason("w-unknown"), Some("owner_state_unknown"));
            assert_eq!(reason("w-open"), Some("owner_open"));
        }

        #[test]
        fn already_terminal_work_is_not_reclosed() {
            let mut done = work("w-done", Some("3327"), WorkspaceStatusCategory::Done);
            done.completed_at = Some(Utc::now());
            let mut discarded = work("w-discarded", Some("3327"), WorkspaceStatusCategory::Active);
            discarded.discarded = true;
            let works = vec![done, discarded];

            let plan = super::super::classify_stale_works(&works, |_| Some(false));

            assert!(
                plan.candidates.is_empty(),
                "terminal Works are already settled; closing them again emits nothing"
            );
        }
    }

    use crate::cli::env::TestEnv;
    use gwt_core::workspace_projection::{
        load_workspace_projection, load_workspace_work_items, record_workspace_work_event,
        save_workspace_projection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceProjection,
    };
    use std::{
        io::{Read, Write},
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };
    fn s(value: &str) -> String {
        value.to_string()
    }

    struct WorkspaceTestEnvGuard {
        _forward_url: gwt_core::test_support::ScopedEnvVar,
        _forward_token: gwt_core::test_support::ScopedEnvVar,
        _runtime_path: gwt_core::test_support::ScopedEnvVar,
        // Declared last so environment guards restore their values before the
        // process-global environment lock is released.
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn env_guard() -> WorkspaceTestEnvGuard {
        let lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        WorkspaceTestEnvGuard {
            _forward_url: gwt_core::test_support::ScopedEnvVar::unset(
                gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            ),
            _forward_token: gwt_core::test_support::ScopedEnvVar::unset(
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            ),
            _runtime_path: gwt_core::test_support::ScopedEnvVar::unset(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            ),
            _lock: lock,
        }
    }

    /// True once the buffer holds a full HTTP request: headers terminated and,
    /// when `Content-Length` is declared, the whole body received.
    fn request_is_complete(buffer: &[u8]) -> bool {
        let Some(header_end) = buffer
            .windows(4)
            .position(|window| {
                window
                    == b"

"
            })
            .map(|index| index + 4)
        else {
            return false;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        buffer.len() - header_end >= content_length
    }

    struct WorkspaceUpdateSuccessProbe {
        forward_url: String,
        requested: mpsc::Receiver<()>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl WorkspaceUpdateSuccessProbe {
        fn start(work_id: &str) -> Self {
            let listener =
                std::net::TcpListener::bind(("127.0.0.1", 0)).expect("update probe listener");
            listener
                .set_nonblocking(true)
                .expect("nonblocking update probe");
            let address = listener.local_addr().expect("update probe address");
            let (requested_tx, requested) = mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let body = serde_json::json!({
                "schema_version": crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                "work_id": work_id,
                "journal_entry_id": "fake-host-journal"
            })
            .to_string();
            let thread = std::thread::spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // The listener polls non-blocking, and on Windows
                            // the accepted socket inherits that mode. Reading
                            // non-blocking returns WouldBlock immediately, so
                            // the probe would answer and close before the
                            // client finished writing its request and the
                            // client would see a transport failure instead of
                            // the 200. Read the request to completion first.
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                            let mut request = Vec::new();
                            let mut chunk = [0_u8; 8192];
                            loop {
                                match stream.read(&mut chunk) {
                                    Ok(0) => break,
                                    Ok(read) => {
                                        request.extend_from_slice(&chunk[..read]);
                                        if request_is_complete(&request) {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            requested_tx.send(()).expect("record update probe request");
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            stream
                                .write_all(response.as_bytes())
                                .expect("write update probe response");
                            return;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept update probe request: {error}"),
                    }
                }
            });
            Self {
                forward_url: format!("http://{address}/internal/hook-live"),
                requested,
                stop,
                thread: Some(thread),
            }
        }

        fn assert_no_request(mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().expect("join update probe");
            }
            assert!(
                matches!(
                    self.requested.try_recv(),
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
                ),
                "managed workspace.update must stop before contacting Host"
            );
        }

        fn expect_request(mut self) {
            self.requested
                .recv_timeout(Duration::from_secs(2))
                .expect("authenticated Host update request");
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().expect("join update probe");
            }
        }
    }

    impl Drop for WorkspaceUpdateSuccessProbe {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn apply_legacy_workspace_ensure_transition_for_test(
        repo_path: &Path,
        input: WorkspaceEnsureInput,
    ) -> gwt_core::error::Result<WorkspaceEnsureResult> {
        let owner = resolve_workspace_ensure_owner(None, input.spec, input.issue)?;
        transact_workspace_state(repo_path, |projection, existing, persisted| {
            apply_workspace_ensure_transition(
                repo_path, projection, existing, persisted, &input, owner, None,
            )
        })
    }

    struct ScopedHome {
        _home: gwt_core::test_support::ScopedGwtHome,
    }

    impl ScopedHome {
        fn set(path: &std::path::Path) -> Self {
            Self {
                _home: gwt_core::test_support::ScopedGwtHome::set(path),
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
        agent.branch = Some("work/20260601-0934".to_string());
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some("work-session".to_string());
        agent
    }

    fn run_git(args: &[&str], cwd: &Path) {
        let output = gwt_core::process::run_git_logged(args, Some(cwd)).expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_session_git_layout(project_state_root: &Path, worktree_path: &Path) {
        const BRANCH: &str = "work/20260601-0934";
        const REMOTE: &str = "https://example.invalid/acme/workspace-update.git";
        std::fs::create_dir_all(project_state_root).expect("project root");
        if project_state_root == worktree_path {
            crate::cli::trusted_store::init_git_repo_with_origin(worktree_path);
            run_git(&["checkout", "-b", BRANCH], worktree_path);
            run_git(&["remote", "set-url", "origin", REMOTE], worktree_path);
            return;
        }

        let bootstrap = project_state_root.join("bootstrap");
        std::fs::create_dir_all(&bootstrap).expect("bootstrap");
        crate::cli::trusted_store::init_git_repo_with_origin(&bootstrap);
        run_git(&["checkout", "-b", BRANCH], &bootstrap);
        let bare = project_state_root.join("gwt.git");
        let bare_arg = bare.to_str().expect("bare path");
        let bootstrap_arg = bootstrap.to_str().expect("bootstrap path");
        run_git(
            &["clone", "--bare", bootstrap_arg, bare_arg],
            project_state_root,
        );
        run_git(&["remote", "set-url", "origin", REMOTE], &bare);
        if worktree_path.exists() {
            std::fs::remove_dir(worktree_path).expect("remove empty worktree placeholder");
        }
        std::fs::create_dir_all(worktree_path.parent().expect("worktree parent"))
            .expect("worktree parent");
        let worktree_arg = worktree_path.to_str().expect("worktree path");
        run_git(&["worktree", "add", worktree_arg, BRANCH], &bare);
    }

    fn write_session_with_project_state_root(
        session_id: &str,
        worktree_path: &Path,
        project_state_root: &Path,
    ) {
        initialize_session_git_layout(project_state_root, worktree_path);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = gwt_agent::Session::new(
            worktree_path,
            "work/20260601-0934",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.project_state_root = Some(project_state_root.to_path_buf());
        session.linked_issue_number = Some(3412);
        session.save(&sessions_dir).expect("write session");

        let mut event = WorkEvent::new(WorkEventKind::Start, "work-session", Utc::now());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        event.owner = Some("Issue #3412".to_string());
        event.agent_session_id = Some(session_id.to_string());
        event.agent_id = Some("codex".to_string());
        event.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(worktree_path.to_path_buf()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(project_state_root, worktree_path, event);
    }

    fn write_projectionless_session(
        session_id: &str,
        worktree_path: &Path,
        project_state_root: &Path,
        linked_issue_number: u64,
    ) {
        initialize_session_git_layout(project_state_root, worktree_path);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = gwt_agent::Session::new(
            worktree_path,
            "work/20260601-0934",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.project_state_root = Some(project_state_root.to_path_buf());
        session.linked_issue_number = Some(linked_issue_number);
        session
            .save(&sessions_dir)
            .expect("write projectionless session");
    }

    fn write_bound_projectionless_session(
        session_id: &str,
        worktree_path: &Path,
        project_state_root: &Path,
        linked_issue_number: u64,
    ) -> crate::cli::execution_state::ExecutionOwnerKey {
        write_bound_projectionless_session_for_owner(
            session_id,
            worktree_path,
            project_state_root,
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            linked_issue_number,
        )
    }

    fn write_bound_projectionless_session_for_owner(
        session_id: &str,
        worktree_path: &Path,
        project_state_root: &Path,
        owner_kind: crate::cli::execution_state::ExecutionOwnerKind,
        owner_number: u64,
    ) -> crate::cli::execution_state::ExecutionOwnerKey {
        initialize_session_git_layout(project_state_root, worktree_path);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: owner_kind,
            number: owner_number,
        };
        crate::cli::execution_state::materialize_at_launch(
            worktree_path,
            owner.kind,
            owner.number,
            session_id,
            &format!("$gwt-execute #{}", owner.number),
            false,
        )
        .expect("materialize bound execution");
        crate::cli::execution_state::ensure_generation_ledger(
            worktree_path,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize bound generation ledger");
        let identity = crate::cli::execution_state::current_execution_binding(worktree_path, owner)
            .expect("read current execution binding")
            .expect("current execution binding");
        let mut session = gwt_agent::Session::new(
            worktree_path,
            "work/20260601-0934",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.project_state_root = Some(project_state_root.to_path_buf());
        session.linked_issue_number = Some(owner.number);
        session
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session_id.to_string(),
                repo_hash: gwt_core::repo_hash::detect_repo_hash(worktree_path)
                    .expect("repo hash")
                    .to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            }))
            .expect("bind projectionless Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("write bound projectionless Session");
        owner
    }

    fn write_unbound_docker_session(session_id: &str, repo: &Path) {
        initialize_session_git_layout(repo, repo);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session =
            gwt_agent::Session::new(repo, "work/20260601-0934", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.project_state_root = Some(repo.to_path_buf());
        session.linked_issue_number = Some(3412);
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.save(&sessions_dir).expect("write Docker session");
    }

    fn write_docker_session(session_id: &str, repo: &Path) {
        write_docker_session_for_owner(
            session_id,
            repo,
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            3412,
        );
    }

    fn write_docker_session_for_owner(
        session_id: &str,
        repo: &Path,
        owner_kind: crate::cli::execution_state::ExecutionOwnerKind,
        owner_number: u64,
    ) {
        write_unbound_docker_session(session_id, repo);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: owner_kind,
            number: owner_number,
        };
        crate::cli::execution_state::materialize_at_launch(
            repo,
            owner.kind,
            owner.number,
            session_id,
            "$gwt-execute #3412",
            false,
        )
        .expect("materialize Docker execution");
        crate::cli::execution_state::ensure_generation_ledger(
            repo,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize Docker generation ledger");
        let identity = crate::cli::execution_state::current_execution_binding(repo, owner)
            .expect("read Docker binding")
            .expect("Docker binding");
        let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        let mut session =
            gwt_agent::Session::load_and_migrate(&path).expect("load Docker session for binding");
        session
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session_id.to_string(),
                repo_hash: gwt_core::repo_hash::detect_repo_hash(repo)
                    .expect("Docker repo hash")
                    .to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            }))
            .expect("bind Docker Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save bound Docker Session");
    }

    fn workspace_recovery_state_paths(project_state_root: &Path, worktree: &Path) -> Vec<PathBuf> {
        let project_legacy =
            gwt_core::paths::gwt_project_dir_for_repo_path(project_state_root).join("workspace");
        let worktree_legacy =
            gwt_core::paths::gwt_project_dir_for_repo_path(worktree).join("workspace");
        vec![
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(project_state_root),
            project_legacy.join("current.json"),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(project_state_root),
            project_legacy.join("works.json"),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(worktree),
            worktree_legacy.join("works.json"),
            gwt_core::paths::gwt_repo_local_work_events_path(project_state_root),
            gwt_core::paths::gwt_repo_local_work_events_path(worktree),
            gwt_core::paths::gwt_workspace_work_events_path_for_repo_path(project_state_root),
            gwt_core::paths::gwt_workspace_work_events_path_for_repo_path(worktree),
            project_state_root.join(".gitattributes"),
            worktree.join(".gitattributes"),
        ]
    }

    fn workspace_recovery_state_bytes(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
        paths.iter().map(|path| std::fs::read(path).ok()).collect()
    }

    fn seed_exact_workspace_work(
        project_state_root: &Path,
        worktree: &Path,
        session_id: &str,
        owner: Option<&str>,
        agent_id: &str,
    ) -> String {
        let canonical_worktree = dunce::canonicalize(worktree).expect("canonical worktree");
        let work_id = gwt_core::workspace_projection::canonical_work_id(
            project_state_root,
            Some("work/20260601-0934"),
            Some(canonical_worktree.as_path()),
        )
        .expect("canonical Work id");
        let mut start = WorkEvent::new(WorkEventKind::Start, work_id.clone(), Utc::now());
        start.title = Some("Exact recovery Work".to_string());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.owner = owner.map(str::to_string);
        start.agent_session_id = Some(session_id.to_string());
        start.agent_id = Some(agent_id.to_string());
        start.display_name = Some("Codex".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(canonical_worktree),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(project_state_root, worktree, start);
        work_id
    }

    fn seed_workspace_container_shadow(
        project_state_root: &Path,
        worktree: &Path,
        work_id: &str,
        session_id: &str,
        status: WorkspaceStatusCategory,
    ) {
        let canonical_worktree = dunce::canonicalize(worktree).expect("canonical worktree");
        let mut start = WorkEvent::new(WorkEventKind::Start, work_id, Utc::now());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.owner = Some("Issue #9999".to_string());
        start.agent_session_id = Some(session_id.to_string());
        start.agent_id = Some("codex".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(canonical_worktree),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(project_state_root, worktree, start);
        if status == WorkspaceStatusCategory::Active {
            return;
        }
        let kind = match status {
            WorkspaceStatusCategory::Idle => WorkEventKind::Pause,
            WorkspaceStatusCategory::Blocked => WorkEventKind::Blocked,
            WorkspaceStatusCategory::Done => WorkEventKind::Done,
            WorkspaceStatusCategory::Unknown => WorkEventKind::Update,
            WorkspaceStatusCategory::Active => unreachable!(),
        };
        let mut transition = WorkEvent::new(kind, work_id, Utc::now());
        transition.status_category = Some(status);
        transition.agent_session_id = Some(session_id.to_string());
        record_recovery_work_event(project_state_root, worktree, transition);
    }

    fn record_recovery_work_event(project_state_root: &Path, worktree: &Path, event: WorkEvent) {
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(project_state_root);
        let mut work_items =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&work_items_path)
                .expect("load repo-global WorkItems fixture")
                .unwrap_or_else(|| WorkItemsProjection::empty(event.updated_at));
        work_items.apply_event(event.clone());
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &work_items_path,
            &work_items,
        )
        .expect("seed repo-global WorkItems");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(worktree);
        gwt_core::workspace_projection::append_workspace_work_event_to_path(&events_path, &event)
            .expect("seed worktree-local tracked event");
    }

    pub(crate) fn seed_valid_update_target(repo: &Path, session_id: &str) {
        write_session_with_project_state_root(session_id, repo, repo);
        let mut projection = WorkspaceProjection::default_for_project(repo);
        projection.id = "work-session".to_string();
        projection.agents.push(assigned_agent_with_window(
            session_id,
            "project::agent-1",
            repo,
        ));
        save_workspace_projection(repo, &projection).expect("save assigned Session fixture");
    }

    fn save_completed_execution_record(worktree: &Path, session_id: &str, owner: u64) {
        use crate::cli::execution_state::{
            ExecutionControlRecord, ExecutionControlStatus, ExecutionOwnerKind,
        };
        let now = Utc::now();
        let record = ExecutionControlRecord {
            owner_kind: ExecutionOwnerKind::Issue,
            owner_number: owner,
            primary_session_id: session_id.to_string(),
            entrypoint: "launch".to_string(),
            bundled_required_owners: Vec::new(),
            status: ExecutionControlStatus::Completed,
            blocked_reason: None,
            missing_verification: None,
            launched_at: now,
            settled_at: Some(now),
            transfers: Vec::new(),
            recoveries: Vec::new(),
            content_hash: String::new(),
        };
        crate::cli::execution_state::save(worktree, &record).expect("save execution record");
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
                progress_summary: None,
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
                progress_summary: None,
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

    /// Issue #3184: transient helper-workflow phases (browser-check,
    /// verification, merging, server startup) must never become the Agent
    /// title, even via the legacy `--title-summary` flag path.
    #[test]
    fn parse_workspace_update_rejects_transient_activity_title_summary() {
        for label in [
            "browser check",
            "Browser-Check",
            "Headless browser check",
            "Browser check (fresh instance)",
            "verification",
            "merging",
            "server startup",
            "ブラウザチェック",
            "ヘッドレスブラウザチェック",
            "動作確認",
        ] {
            let err = parse(&[
                s("update"),
                s("--agent-session"),
                s("session-1"),
                s("--title-summary"),
                s(label),
            ])
            .expect_err("title-summary must stay the work purpose, not a transient activity");

            let message = err.to_string();
            assert!(message.contains("--title-summary"), "{label}: {message}");
            assert!(message.contains("transient activity"), "{label}: {message}");
            assert!(message.contains("current_focus"), "{label}: {message}");
        }
    }

    /// Issue #3184: work names that merely mention the activity domain stay
    /// valid — only bare activity labels are rejected.
    #[test]
    fn parse_workspace_update_accepts_work_name_mentioning_activity_domain() {
        for label in [
            "browser-check purpose overwrite guard",
            "Fix browser check",
            "Issue #3184 title guard",
            "release verification pipeline",
            "E2E testing harness",
        ] {
            parse(&[
                s("update"),
                s("--agent-session"),
                s("session-1"),
                s("--title-summary"),
                s(label),
            ])
            .unwrap_or_else(|err| panic!("{label} should stay a valid work name: {err}"));
        }
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
        seed_valid_update_target(&repo, "session-status");
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-status",
        );

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: Some("Workspace coordination".to_string()),
                status: Some("blocked".to_string()),
                status_text: Some("Waiting on Board alignment".to_string()),
                summary: Some("Align Workspace ownership before edits".to_string()),
                progress_summary: None,
                next_action: Some("Post Board request".to_string()),
                owner: None,
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
        assert_eq!(saved.owner.as_deref(), Some("Issue #3412"));
    }

    #[test]
    fn terminal_workspace_update_opens_work_event_settlement_obligation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        seed_valid_update_target(&repo, "session-terminal");
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-terminal",
        );
        let mut env = TestEnv::new(repo.clone());

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: Some("done".to_string()),
                status_text: Some("Final Work summary recorded".to_string()),
                summary: Some("Implementation and focused verification complete".to_string()),
                progress_summary: Some("Final cumulative delivery summary".to_string()),
                next_action: Some("Commit and push the tracked Work event".to_string()),
                owner: None,
                agent_session: None,
                current_focus: Some("Settling tracked Work delivery".to_string()),
                title_summary: None,
            },
            &mut out,
        )
        .expect("terminal workspace update");

        assert_eq!(code, 0, "{out}");
        let record = crate::cli::verification_record::load_work_event_settlement_record(&repo)
            .expect("load settlement obligation")
            .expect("terminal update must create a settlement obligation");
        assert!(record.obligation_open);
        assert_eq!(record.session_id, "session-terminal");
        assert!(matches!(
            record.status,
            crate::cli::verification_record::WorkEventSettlementStatus::Blocked(
                crate::cli::verification_record::WorkEventSettlementBlocker::PathDirtyInUnreachableEnvironment {
                    environment: crate::cli::verification_record::WorkEventSettlementEnvironment::MissingUpstream,
                    ..
                }
            )
        ));
        assert_eq!(
            record.status.severity(),
            crate::cli::verification_record::WorkEventSettlementSeverity::Warning
        );
    }

    #[test]
    fn foreign_active_exact_unbound_recovery_reaches_ensure_and_update() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        initialize_session_git_layout(&repo, &repo);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3393,
        };
        let foreign_session_id = "foreign-predecessor-session";
        crate::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            foreign_session_id,
            "$gwt-execute #3393",
            false,
        )
        .expect("materialize foreign Active predecessor");
        crate::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize predecessor ledger");
        let predecessor = crate::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read predecessor binding")
            .expect("predecessor binding");
        let repo_hash = gwt_core::repo_hash::detect_repo_hash(&repo)
            .expect("repo hash")
            .to_string();
        crate::cli::execution_state::record_rebound_continuation_validation(
            &repo,
            owner,
            "foreign-predecessor-rebound-audit",
            foreign_session_id,
            &gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: foreign_session_id.to_string(),
                repo_hash: repo_hash.clone(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity: predecessor,
                capability_generation: 1,
            },
        )
        .expect("record valid foreign rebound audit");

        let session_id = "exact-unbound-successor-session";
        let mut session =
            gwt_agent::Session::new(&repo, "work/20260601-0934", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = None;
        session.execution_binding = None;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        session.docker_runtime_binding = None;
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("write exact unbound Host Session");

        let before = crate::cli::execution_state::diagnose(&repo, Some(session_id));
        assert!(before
            .available_recoveries
            .contains(&"execution.continue".to_string()));
        assert!(!before
            .available_recoveries
            .contains(&"workspace.ensure".to_string()));
        assert!(!before
            .available_recoveries
            .contains(&"execution.repair".to_string()));

        let (continuation, binding) = crate::agent_project_state::continue_authenticated_execution(
            &repo,
            session_id,
            crate::AgentExecutionContinuationRequest {
                schema_version: crate::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "foreign-active-exact-unbound-e2e".to_string(),
            },
        )
        .expect("create exact successor");
        assert_eq!(
            continuation.outcome,
            crate::AgentExecutionContinuationOutcome::SuccessorCreated
        );

        let after_continue = crate::cli::execution_state::diagnose(&repo, Some(session_id));
        assert!(after_continue
            .available_recoveries
            .contains(&"workspace.ensure".to_string()));
        assert!(!after_continue
            .available_recoveries
            .contains(&"execution.repair".to_string()));

        let _session_env =
            crate::cli::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let (repair_code, repair_out) = crate::cli::run_collect(
            &mut env,
            crate::cli::CliCommand::Execution(
                crate::cli::execution_state::ExecutionCommand::Repair {
                    reason: "prove non-corrupt authority".to_string(),
                },
            ),
        )
        .expect("run actual repair operation");
        assert_eq!(repair_code, 2, "{repair_out}");
        assert!(
            repair_out.contains("execution_repair_not_corrupt"),
            "{repair_out}"
        );

        let ensure_input = WorkspaceEnsureInput {
            agent_session: session_id.to_string(),
            title_summary: "P6-A exact recovery".to_string(),
            current_focus: Some("Exercise continuation recovery end to end".to_string()),
            spec: None,
            issue: Some(owner.number),
            topic: Some("recovery".to_string()),
            boundary: Some("continuation and Workspace".to_string()),
        };
        let ensured = ensure_workspace_for_agent(&repo, ensure_input)
            .expect("execute advertised workspace.ensure");
        let after_ensure = crate::cli::execution_state::diagnose(&repo, Some(session_id));
        assert!(after_ensure
            .available_recoveries
            .contains(&"workspace.update".to_string()));
        assert!(!after_ensure
            .available_recoveries
            .contains(&"workspace.ensure".to_string()));
        let update = crate::agent_project_state::apply_bound_authenticated_workspace_update(
            &repo,
            session_id,
            &binding,
            crate::AgentWorkspaceUpdateRequest {
                schema_version: crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                claimed_session_id: session_id.to_string(),
                observation: crate::observe_agent_runtime(&repo)
                    .expect("observe exact successor worktree"),
                intent: crate::AgentWorkspaceUpdateIntent {
                    summary: Some("Recovery reached authenticated Workspace update".to_string()),
                    ..Default::default()
                },
            },
        )
        .expect("execute workspace.update after ensure");
        assert_eq!(update.work_id, ensured.workspace_id);
    }

    #[test]
    fn typed_ensure_required_continuation_keeps_split_roots_and_settlement_evidence_aligned() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_state_root = temp.path().join("workspace-home");
        let worktree = project_state_root.join("work").join("issue-3412");
        let session_id = "session-typed-split-root";
        write_bound_projectionless_session(session_id, &worktree, &project_state_root, 3412);

        let ensured = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Typed split-root continuation".to_string(),
                current_focus: Some("Ensure the exact durable assignment".to_string()),
                spec: None,
                issue: Some(3412),
                topic: None,
                boundary: None,
            },
        )
        .expect("ensure exact split-root Host authority");
        assert_eq!(ensured.disposition, WorkspaceEnsureDisposition::Created);

        let project_state_root =
            dunce::canonicalize(&project_state_root).expect("canonical Project State root");
        let worktree = dunce::canonicalize(&worktree).expect("canonical linked worktree");
        assert_ne!(project_state_root, worktree);

        let legacy_worktree_dir =
            gwt_core::paths::gwt_project_dir_for_repo_path(&worktree).join("workspace");
        std::fs::create_dir_all(&legacy_worktree_dir).expect("create legacy worktree state");
        let legacy_worktree_current = legacy_worktree_dir.join("current.json");
        let legacy_worktree_works = legacy_worktree_dir.join("works.json");
        std::fs::write(&legacy_worktree_current, b"legacy worktree current")
            .expect("seed legacy worktree current");
        std::fs::write(&legacy_worktree_works, b"legacy worktree works")
            .expect("seed legacy worktree Works");
        let legacy_before = [
            std::fs::read(&legacy_worktree_current).expect("read legacy current before"),
            std::fs::read(&legacy_worktree_works).expect("read legacy Works before"),
        ];

        let WorkspaceUpdateBridgeAuthoritySnapshot::Exact(authority) =
            snapshot_workspace_update_bridge_authority(&worktree, session_id)
        else {
            panic!("exact ensured Host authority snapshot");
        };
        assert_eq!(authority.project_state_root, project_state_root);
        let expected_work_id = authority.work_id.clone();
        let expected_execution_identity = authority.identity.execution_binding.identity.clone();
        let receipt = continue_workspace_update_after_typed_ensure_required(
            session_id,
            *authority,
            crate::AgentWorkspaceUpdateRequest {
                schema_version: crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                claimed_session_id: session_id.to_string(),
                observation: crate::observe_agent_runtime(&worktree)
                    .expect("observe exact linked worktree"),
                intent: crate::AgentWorkspaceUpdateIntent {
                    status_category: Some(WorkspaceStatusCategory::Done),
                    summary: Some("Typed split-root continuation completed".to_string()),
                    ..Default::default()
                },
            },
        )
        .expect("apply the bounded typed continuation");
        assert_eq!(receipt.work_id, expected_work_id);

        let current = load_workspace_projection(&project_state_root)
            .expect("load repo-global current")
            .expect("repo-global current exists");
        assert_eq!(current.status_category, WorkspaceStatusCategory::Done);
        let works = load_workspace_work_items(&project_state_root)
            .expect("load repo-global Works")
            .expect("repo-global Works exist");
        assert_eq!(
            works
                .work_items
                .iter()
                .find(|work| work.id == expected_work_id)
                .expect("exact Work exists")
                .status_category,
            WorkspaceStatusCategory::Done
        );

        let tracked_events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let tracked_events = std::fs::read_to_string(&tracked_events_path)
            .expect("read worktree-local tracked events")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<WorkEvent>(line).expect("tracked event JSON"))
            .collect::<Vec<_>>();
        let done_event = tracked_events
            .iter()
            .find(|event| event.kind == WorkEventKind::Done)
            .expect("typed continuation emits one Done event");
        assert_eq!(done_event.work_item_id, expected_work_id);
        assert!(
            !gwt_core::paths::gwt_repo_local_work_events_path(&project_state_root).exists(),
            "Project State root must not receive a tracked Work event"
        );
        assert_eq!(
            [
                std::fs::read(&legacy_worktree_current).expect("read legacy current after"),
                std::fs::read(&legacy_worktree_works).expect("read legacy Works after"),
            ],
            legacy_before,
            "typed continuation must not write legacy linked-worktree projections"
        );

        let journal = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
            &project_state_root,
            1,
        )
        .expect("load repo-global journal")
        .into_iter()
        .next()
        .expect("typed continuation journal entry");
        assert_eq!(journal.id, receipt.journal_entry_id);
        let settlement =
            crate::cli::verification_record::load_work_event_settlement_record(&worktree)
                .expect("load typed continuation settlement")
                .expect("typed continuation settlement exists");
        assert!(settlement.obligation_open);
        assert_eq!(settlement.session_id, session_id);
        assert_eq!(
            settlement.execution_binding.as_ref(),
            Some(&expected_execution_identity)
        );
        assert!(matches!(
            settlement.status,
            crate::cli::verification_record::WorkEventSettlementStatus::Blocked(
                crate::cli::verification_record::WorkEventSettlementBlocker::PathDirtyInUnreachableEnvironment {
                    environment: crate::cli::verification_record::WorkEventSettlementEnvironment::MissingUpstream,
                    ..
                }
            )
        ));
    }

    #[test]
    fn terminal_workspace_update_refuses_before_mutation_when_settlement_store_is_unwritable() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        seed_valid_update_target(&repo, "session-terminal-store-failure");
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-terminal-store-failure",
        );
        let surface_paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo),
            gwt_core::paths::gwt_workspace_journal_path_for_repo_path(&repo),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
            gwt_core::paths::gwt_repo_local_work_events_path(&repo),
        ];
        let before = surface_paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(&repo)
            .expect("fixture has a trusted-store path");
        std::fs::create_dir_all(trusted_dir.parent().expect("trusted-store parent"))
            .expect("create trusted-store parent");
        std::fs::write(&trusted_dir, b"block trusted-store directory creation")
            .expect("make trusted-store path unwritable");
        let mut env = TestEnv::new(repo.clone());
        let mut out = String::new();

        let error = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: Some("done".to_string()),
                status_text: Some("must not persist".to_string()),
                summary: Some("settlement authority is unavailable".to_string()),
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut out,
        )
        .expect_err("unwritable settlement store must reject the terminal update");

        assert!(error.to_string().contains("settlement"), "{error}");
        let after = surface_paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(
            after, before,
            "settlement authority must be reserved before any terminal Work surface mutates"
        );
    }

    #[test]
    fn workspace_update_json_envelope_persists_progress_summary() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        seed_valid_update_target(&repo, "session-progress");
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-progress",
        );
        let mut env = TestEnv::new(repo.clone());
        env.stdin = serde_json::json!({
            "schema_version": 1,
            "operation": "workspace.update",
            "params": {
                "agent_session": "session-progress",
                "purpose": "Workspace detail progress summary",
                "current_focus": "Adding progress summary persistence",
                "summary": "Latest update should stay separate.",
                "progress_summary": "Implemented resume normalization and identified the split WorkEvent root. Now adding a cumulative progress summary field."
            }
        })
        .to_string();

        let code = crate::cli::env::dispatch(&mut env, &["gwtd".to_string()]);

        assert_eq!(
            code,
            0,
            "workspace.update JSON envelope failed: {}",
            String::from_utf8_lossy(&env.stderr)
        );
        let raw = std::fs::read_to_string(
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo),
        )
        .expect("workspace current json");
        let current: serde_json::Value = serde_json::from_str(&raw).expect("current json");
        assert_eq!(
            current["progress_summary"],
            "Implemented resume normalization and identified the split WorkEvent root. Now adding a cumulative progress summary field."
        );
        assert_eq!(
            current["summary"], "Latest update should stay separate.",
            "progress summary must not replace latest-update summary"
        );
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
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );

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
                progress_summary: None,
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
        assert!(
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree).is_file(),
            "agent workspace update must record the Work event in the session worktree"
        );
        assert!(
            !gwt_core::paths::gwt_repo_local_work_events_path(&project_root).exists(),
            "agent workspace update must not write the repo-local Work event to the Project State root"
        );
    }

    /// Issue #3278 regression: when the agent session referenced by
    /// `GWT_SESSION_ID` records a worktree that disagrees with the worktree the
    /// process actually runs in, `workspace.update` must refuse to write — the
    /// tracked Work event and its identity would be attributed to a foreign
    /// Work. The refusal is actionable (names the session, the expected
    /// worktree, and the actual worktree) and leaves tracked logs untouched.
    #[test]
    fn workspace_update_rejects_session_cwd_worktree_mismatch() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let session_worktree = temp.path().join("work").join("issue-session");
        let actual_worktree = temp.path().join("work").join("issue-actual");
        std::fs::create_dir_all(&session_worktree).expect("session worktree");
        std::fs::create_dir_all(&actual_worktree).expect("actual worktree");
        // The session (stale GWT_SESSION_ID) points at a different worktree
        // than the one this process runs in.
        write_session_with_project_state_root("session-1", &session_worktree, &session_worktree);
        let session_events_path =
            gwt_core::paths::gwt_repo_local_work_events_path(&session_worktree);
        let session_events_before =
            std::fs::read(&session_events_path).expect("seeded session Work event log");
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );

        let mut env = TestEnv::new(actual_worktree.clone());
        let mut out = String::new();
        let error = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some("Late coordination update".to_string()),
                title_summary: None,
            },
            &mut out,
        )
        .expect_err("session/cwd worktree mismatch must fail closed");

        let message = error.to_string();
        assert!(
            message.contains("session-1"),
            "error must name the conflicting session: {message}"
        );
        assert!(
            message.contains("issue-session") && message.contains("issue-actual"),
            "error must name both the expected and actual worktree: {message}"
        );
        assert!(
            !gwt_core::paths::gwt_repo_local_work_events_path(&actual_worktree).exists(),
            "refused update must not append a tracked Work event in the actual worktree"
        );
        assert_eq!(
            std::fs::read(&session_events_path).expect("session Work event log after refusal"),
            session_events_before,
            "refused update must leave the session worktree event log byte-equivalent"
        );
    }

    /// Issue #3278 regression: once the Execution Control Record for the
    /// worktree has settled as Completed, a coordination-only
    /// `workspace.update` (the kind a post-merge stale reminder triggers) must
    /// not append to the git-tracked `events.jsonl` — otherwise it re-dirties an
    /// already committed/merged worktree. The machine-local projection still
    /// updates so the Board / titlebar stay live.
    #[test]
    fn workspace_update_skips_tracked_event_after_execution_completed() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );

        let mut canonical = WorkspaceProjection::default_for_project(&project_root);
        canonical.agents.push(assigned_agent_with_window(
            "session-1",
            "project::agent-1",
            &worktree,
        ));
        save_workspace_projection(&project_root, &canonical).expect("save canonical projection");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let events_before = std::fs::read(&events_path).expect("seeded tracked Work event log");

        // The execution for this worktree already completed (final commit +
        // push + PR merge happened before this coordination update).
        save_completed_execution_record(&worktree, "session-1", 3412);

        let mut env = TestEnv::new(worktree.clone());
        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some("Post-merge coordination focus".to_string()),
                title_summary: None,
            },
            &mut out,
        )
        .expect("update workspace");

        assert_eq!(code, 0);
        assert_eq!(
            std::fs::read(&events_path).expect("tracked Work event log after update"),
            events_before,
            "completed-execution coordination update must leave tracked events.jsonl byte-equivalent"
        );
        let saved = load_workspace_projection(&project_root)
            .expect("load canonical projection")
            .expect("canonical projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("canonical agent");
        assert_eq!(
            agent.current_focus.as_deref(),
            Some("Post-merge coordination focus"),
            "the machine-local projection must still reflect the coordination update"
        );
    }

    /// Issue #3184 regression: when a stable purpose is already set, a
    /// `workspace.update` that tries to replace it with a transient activity
    /// label (`browser check` etc.) must be rejected end-to-end and the
    /// projection — the source of the Agent titlebar `dynamic_title` — must
    /// keep the original purpose.
    #[test]
    fn workspace_update_keeps_existing_purpose_when_transient_activity_rejected() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let _session =
            crate::cli::test_support::ScopedEnvVar::unset(gwt_agent::session::GWT_SESSION_ID_ENV);
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260629-0915");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);

        let mut canonical = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = assigned_agent_with_window("session-1", "project::agent-1", &worktree);
        agent.title_summary = Some("UI surface audit".to_string());
        canonical.agents.push(agent);
        save_workspace_projection(&project_root, &canonical).expect("save canonical projection");

        let mut env = TestEnv::new(worktree.clone());
        env.stdin = serde_json::json!({
            "schema_version": 1,
            "operation": "workspace.update",
            "params": {
                "agent_session": "session-1",
                "purpose": "Headless browser check",
                "current_focus": "Headless server open for user browser check",
            }
        })
        .to_string();

        let code = crate::cli::env::dispatch(&mut env, &["gwtd".to_string()]);

        assert_ne!(
            code,
            0,
            "transient activity purpose must be rejected: {}",
            String::from_utf8_lossy(&env.stderr)
        );
        let stderr = String::from_utf8_lossy(&env.stderr);
        assert!(stderr.contains("transient activity"), "{stderr}");

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
            Some("UI surface audit"),
            "existing purpose must be preserved when a transient activity write is rejected"
        );
    }

    #[test]
    fn workspace_update_does_not_run_split_state_repair() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );

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
                progress_summary: None,
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
            None,
            "mutation must not import identity from the obsolete split projection"
        );
        assert_eq!(
            agent.current_focus.as_deref(),
            Some("Continue from canonical Project State")
        );
        let untouched_split = load_workspace_projection(&worktree)
            .expect("load untouched split projection")
            .expect("split projection remains present");
        assert_eq!(
            untouched_split
                .latest_agent_for_session("session-1")
                .and_then(|agent| agent.title_summary.as_deref()),
            Some("Split root title"),
            "workspace.update must not mutate or repair the obsolete split projection"
        );
    }

    #[test]
    fn workspace_update_rejects_invalid_status_before_strict_transaction_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        write_session_with_project_state_root("session-1", &worktree, &project_root);
        let _session = crate::cli::test_support::ScopedEnvVar::set(
            gwt_agent::session::GWT_SESSION_ID_ENV,
            "session-1",
        );

        let mut canonical = WorkspaceProjection::default_for_project(&project_root);
        let canonical_agent =
            assigned_agent_with_window("session-1", "project::agent-1", &worktree);
        let canonical_updated_at = canonical_agent.updated_at;
        canonical.agents.push(canonical_agent);
        save_workspace_projection(&project_root, &canonical).expect("save canonical projection");

        let mut split = WorkspaceProjection::default_for_project(&worktree);
        let mut split_agent =
            assigned_agent_with_window("session-1", "project::agent-1", &worktree);
        split_agent.title_summary = Some("must not be repaired".to_string());
        split_agent.updated_at = canonical_updated_at + chrono::Duration::seconds(1);
        split.agents.push(split_agent);
        save_workspace_projection(&worktree, &split).expect("save split projection");

        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&worktree),
            gwt_core::paths::gwt_workspace_journal_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let mut env = TestEnv::new(worktree);
        let mut out = String::new();
        let error = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: Some("invalid-status".to_string()),
                status_text: None,
                summary: None,
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut out,
        )
        .expect_err("invalid status must fail before the strict transaction");

        assert!(error.to_string().contains("unknown workspace status"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "invalid status must not mutate Work state");
    }

    #[test]
    fn workspace_join_assigns_agent_with_durable_claim_provenance() {
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

        let attached_by_after_join = load_workspace_work_items(&repo)
            .expect("load Work items after Join")
            .expect("Work items after Join")
            .work_items
            .into_iter()
            .find(|item| item.id == "workspace-existing")
            .and_then(|item| {
                item.agents
                    .into_iter()
                    .find(|agent| agent.session_id == "session-1")
            })
            .and_then(|agent| agent.attached_by);

        gwt_core::workspace_projection::rebuild_work_items_from_events_for_repo(&repo)
            .expect("refold Work items from events");
        let attached_by_after_refold = load_workspace_work_items(&repo)
            .expect("load refolded Work items")
            .expect("refolded Work items")
            .work_items
            .into_iter()
            .find(|item| item.id == "workspace-existing")
            .and_then(|item| {
                item.agents
                    .into_iter()
                    .find(|agent| agent.session_id == "session-1")
            })
            .and_then(|agent| agent.attached_by);

        assert_eq!(
            (attached_by_after_join, attached_by_after_refold),
            (Some(WorkEventKind::Claim), Some(WorkEventKind::Claim)),
            "direct Join must persist Claim provenance in the hot projection and event refold"
        );
    }

    #[test]
    fn workspace_join_rejects_terminal_work_without_persisting_claim_or_assignment() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");

        for (label, close_kind) in [
            ("done", WorkEventKind::Done),
            ("discarded", WorkEventKind::Discard),
        ] {
            let repo = temp.path().join(label);
            std::fs::create_dir_all(&repo).expect("repo");
            let mut env = TestEnv::new(repo.clone());
            let mut current = WorkspaceProjection::default_for_project(&repo);
            current.agents.push(unassigned_agent("session-1"));
            save_workspace_projection(&repo, &current).expect("save projection");
            let work_id = format!("work-{label}");
            let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, Utc::now());
            start.title = Some(format!("{label} Work"));
            start.status_category = Some(WorkspaceStatusCategory::Active);
            record_workspace_work_event(&repo, start).expect("record workspace");
            match close_kind {
                WorkEventKind::Done => {
                    gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
                        &repo,
                        &work_id,
                        Utc::now(),
                    )
                    .expect("complete Work");
                }
                WorkEventKind::Discard => {
                    gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
                        &repo,
                        &work_id,
                        Utc::now(),
                    )
                    .expect("discard Work");
                }
                _ => unreachable!(),
            }
            let current_before = load_workspace_projection(&repo).unwrap().unwrap();
            let works_before = load_workspace_work_items(&repo).unwrap().unwrap();
            let mut out = String::new();

            let result = run(
                &mut env,
                WorkspaceCommand::Join {
                    agent_session: "session-1".to_string(),
                    workspace_id: work_id,
                    current_focus: Some("Must not join terminal Work".to_string()),
                    title_summary: Some("Terminal Work".to_string()),
                },
                &mut out,
            );

            assert!(result.is_err(), "joining {label} Work must fail");
            assert!(out.is_empty());
            assert_eq!(
                load_workspace_projection(&repo).unwrap().unwrap(),
                current_before,
                "joining {label} Work must not assign the Agent"
            );
            assert_eq!(
                load_workspace_work_items(&repo).unwrap().unwrap(),
                works_before,
                "joining {label} Work must not append Claim"
            );
        }
    }

    #[test]
    fn workspace_join_recovers_durable_unprojected_terminal_event_before_claim() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");

        for (label, close_kind) in [
            ("done", WorkEventKind::Done),
            ("discarded", WorkEventKind::Discard),
        ] {
            let repo = temp.path().join(format!("partial-{label}"));
            std::fs::create_dir_all(&repo).expect("repo");
            let mut env = TestEnv::new(repo.clone());
            let mut current = WorkspaceProjection::default_for_project(&repo);
            current.agents.push(unassigned_agent("session-1"));
            save_workspace_projection(&repo, &current).expect("save projection");
            let work_id = format!("work-partial-{label}");
            let mut start = WorkEvent::new(WorkEventKind::Start, &work_id, Utc::now());
            start.title = Some(format!("Partial {label} Work"));
            start.status_category = Some(WorkspaceStatusCategory::Active);
            record_workspace_work_event(&repo, start).expect("record workspace");

            let mut close = WorkEvent::new(close_kind, &work_id, Utc::now());
            if close_kind == WorkEventKind::Done {
                close.status_category = Some(WorkspaceStatusCategory::Done);
            }
            let closed_events_path =
                gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&repo);
            gwt_core::workspace_projection::append_workspace_work_event_to_path(
                &closed_events_path,
                &close,
            )
            .expect("append durable close without projecting it");
            let current_before = load_workspace_projection(&repo).unwrap().unwrap();
            let works_before = load_workspace_work_items(&repo).unwrap().unwrap();
            assert!(
                !works_before
                    .work_items
                    .iter()
                    .find(|item| item.id == work_id)
                    .unwrap()
                    .is_terminal(),
                "fixture must leave works.json stale"
            );
            let shared_events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);
            let shared_before = std::fs::read(&shared_events_path).unwrap();
            let mut out = String::new();

            let result = run(
                &mut env,
                WorkspaceCommand::Join {
                    agent_session: "session-1".to_string(),
                    workspace_id: work_id,
                    current_focus: Some("Must observe durable close".to_string()),
                    title_summary: Some("Partial terminal Work".to_string()),
                },
                &mut out,
            );

            assert!(result.is_err(), "partial {label} close must block Join");
            assert!(out.is_empty());
            assert_eq!(
                load_workspace_projection(&repo).unwrap().unwrap(),
                current_before
            );
            assert_eq!(
                load_workspace_work_items(&repo).unwrap().unwrap(),
                works_before
            );
            assert_eq!(std::fs::read(&shared_events_path).unwrap(), shared_before);
            assert_eq!(
                std::fs::read_to_string(&closed_events_path)
                    .unwrap()
                    .lines()
                    .count(),
                1,
                "Join must not duplicate or remove the durable close event"
            );
        }
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

        let result = apply_legacy_workspace_ensure_transition_for_test(
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
        let (workspace_id, event) = create_workspace_for_agent(
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
        assert_eq!(event.work_item_id, expected);
    }

    #[test]
    fn workspace_ensure_joins_similar_incomplete_workspace() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkEvent::new(WorkEventKind::Start, "workspace-existing", Utc::now());
        event.title = Some("Workspace materialization".to_string());
        event.intent = Some("Ensure actionable Unassigned Agents join Workspace".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let result = apply_legacy_workspace_ensure_transition_for_test(
            &repo,
            WorkspaceEnsureInput {
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
        )
        .expect("ensure workspace");

        assert_eq!(result.workspace_id, "workspace-existing");
        assert_eq!(result.disposition, WorkspaceEnsureDisposition::Joined);
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
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");

        let result = apply_legacy_workspace_ensure_transition_for_test(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some("Create Workspace from actionable intent".to_string()),
                spec: Some(2359),
                issue: None,
                topic: Some("workspace-materialization".to_string()),
                boundary: None,
            },
        )
        .expect("ensure workspace");

        assert_eq!(result.disposition, WorkspaceEnsureDisposition::Created);
        assert!(result.workspace_id.starts_with("work-"));
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
    fn workspace_ensure_probe_and_execute_share_actual_input_facts() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let session_id = "session-ensure-input-parity";
        write_bound_projectionless_session(session_id, &repo, &repo, 3412);
        let paths = workspace_recovery_state_paths(&repo, &repo);
        let before = workspace_recovery_state_bytes(&paths);
        let input = WorkspaceEnsureInput {
            agent_session: session_id.to_string(),
            title_summary: "Input parity must reject foreign ownership".to_string(),
            current_focus: Some("Evaluate the exact ensure request".to_string()),
            spec: None,
            issue: Some(9999),
            topic: Some("input-parity".to_string()),
            boundary: Some("No synthesized ownership facts".to_string()),
        };

        let probe = probe_workspace_ensure(&repo, &input);
        assert_eq!(
            probe.state,
            crate::cli::governance::RecoveryProbeState::Unavailable
        );
        let error = ensure_workspace_for_agent(&repo, input)
            .expect_err("the same input must be refused by execution");
        let message = error.to_string();
        assert!(
            probe
                .reason
                .as_deref()
                .is_some_and(|reason| message.contains(reason)),
            "probe and execution must preserve the same refusal fact: probe={probe:?}, error={message}"
        );
        assert_eq!(
            workspace_recovery_state_bytes(&paths),
            before,
            "probe and refused execution must preserve Workspace bytes"
        );
    }

    #[test]
    fn workspace_ensure_bootstraps_missing_projection_agent_from_durable_session() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-projectionless",
            &worktree,
            &project_root,
            3412,
        );
        let mut foreign = WorkEvent::new(WorkEventKind::Start, "work-foreign-similar", Utc::now());
        foreign.title = Some("Launch Work projection bootstrap".to_string());
        foreign.intent = Some("Recover the missing launch projection".to_string());
        foreign.status_category = Some(WorkspaceStatusCategory::Active);
        foreign.owner = Some("Issue #3327".to_string());
        record_recovery_work_event(&project_root, &worktree, foreign);

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-projectionless".to_string(),
                title_summary: "Work projection bootstrap".to_string(),
                current_focus: Some("Recover the missing launch projection".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("durable Session should bootstrap the missing projection agent");

        assert_eq!(result.disposition, WorkspaceEnsureDisposition::Created);
        let expected_work_id = gwt_core::workspace_projection::canonical_work_id(
            &project_root,
            Some("work/20260601-0934"),
            Some(worktree.as_path()),
        )
        .expect("canonical Work id");
        let expected_worktree = dunce::canonicalize(&worktree).expect("canonical worktree");
        assert_eq!(result.workspace_id, expected_work_id);

        let projection = load_workspace_projection(&project_root)
            .expect("load canonical projection")
            .expect("canonical projection");
        let agent = projection
            .latest_agent_for_session("session-projectionless")
            .expect("bootstrapped projection agent");
        assert!(agent.is_assigned());
        assert_eq!(
            agent.workspace_id.as_deref(),
            Some(expected_work_id.as_str())
        );
        assert_eq!(agent.branch.as_deref(), Some("work/20260601-0934"));
        assert_eq!(
            agent.worktree_path.as_deref(),
            Some(expected_worktree.as_path())
        );
        assert_eq!(
            agent.title_summary.as_deref(),
            Some("Work projection bootstrap")
        );

        let work_items = load_workspace_work_items(&project_root)
            .expect("load WorkItems projection")
            .expect("WorkItems projection");
        let work = work_items
            .work_items
            .iter()
            .find(|item| item.id == expected_work_id)
            .expect("materialized Work");
        assert_eq!(work.owner.as_deref(), Some("Issue #3412"));
        assert!(work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-projectionless"));
        assert!(work.execution_containers.iter().any(|container| {
            container.branch.as_deref() == Some("work/20260601-0934")
                && container.worktree_path.as_deref() == Some(expected_worktree.as_path())
        }));
        assert!(
            load_workspace_projection(&worktree)
                .expect("load worktree projection")
                .is_none(),
            "recovery must not create a split current projection"
        );
        assert!(
            !gwt_core::paths::gwt_repo_local_work_events_path(&project_root).exists(),
            "recovery must keep tracked Work events in the linked worktree"
        );
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_retry = std::fs::read(&events_path).expect("recovery event log");
        let retry = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-projectionless".to_string(),
                title_summary: "Work projection bootstrap".to_string(),
                current_focus: Some("Recover the missing launch projection".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("recovery retry");
        assert_eq!(
            retry.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(retry.workspace_id, expected_work_id);
        assert_eq!(
            std::fs::read(events_path).expect("event log after retry"),
            before_retry,
            "response-loss retry must not duplicate recovery events"
        );
    }

    #[test]
    fn workspace_ensure_rebinds_continued_session_to_existing_canonical_work_once() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let current_session = "session-continued-current";
        write_bound_projectionless_session(current_session, &worktree, &project_root, 3412);
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-continued-predecessor",
            Some("Issue #3412"),
            "codex",
        );

        let input = WorkspaceEnsureInput {
            agent_session: current_session.to_string(),
            title_summary: "Continue exact canonical Work".to_string(),
            current_focus: Some("Rebind the continued Session".to_string()),
            spec: None,
            issue: None,
            topic: None,
            boundary: None,
        };
        let result = ensure_workspace_for_agent(&worktree, input.clone())
            .expect("continued Session should rebind to the exact canonical Work");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let projection = load_workspace_projection(&project_root)
            .expect("load canonical projection")
            .expect("canonical projection");
        let agent = projection
            .latest_agent_for_session(current_session)
            .expect("continued Session projection agent");
        assert!(agent.is_assigned());
        assert_eq!(agent.workspace_id.as_deref(), Some(work_id.as_str()));

        let work_items = load_workspace_work_items(&project_root)
            .expect("load WorkItems projection")
            .expect("WorkItems projection");
        let work = work_items
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("existing canonical Work");
        assert_eq!(
            work.agents
                .iter()
                .filter(|agent| agent.session_id == current_session)
                .count(),
            1,
            "continued Session must be attached exactly once"
        );
        assert!(work
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-continued-predecessor"));

        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_retry = std::fs::read(&events_path).expect("event log after rebind");
        let retry = ensure_workspace_for_agent(&worktree, input).expect("idempotent rebind retry");
        assert_eq!(
            retry.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(retry.workspace_id, work_id);
        assert_eq!(
            std::fs::read(events_path).expect("event log after retry"),
            before_retry,
            "continued Session retry must not duplicate the Claim event"
        );
    }

    #[test]
    fn workspace_ensure_bound_host_retry_accepts_powershell_worktree_aliases() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-powershell-worktree-alias";
        write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let canonical_worktree = dunce::canonicalize(&worktree).expect("canonical worktree");
        let powershell_alias = PathBuf::from(format!(
            r"Microsoft.PowerShell.Core\FileSystem::{}",
            canonical_worktree.display()
        ));

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = assigned_agent_with_window(session_id, "window-host", &worktree);
        agent.workspace_id = Some(work_id.clone());
        agent.worktree_path = Some(powershell_alias.clone());
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection)
            .expect("save aliased Host assignment");

        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let mut work_items =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&works_path)
                .expect("load exact WorkItems")
                .expect("exact WorkItems");
        let item = work_items
            .work_items
            .iter_mut()
            .find(|item| item.id == work_id)
            .expect("exact Work");
        item.execution_containers[0].worktree_path = Some(powershell_alias);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &works_path,
            &work_items,
        )
        .expect("save aliased WorkItems");

        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_events = std::fs::read(&events_path).expect("seeded Host event log");
        let retry = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Existing aliased Host Work".to_string(),
                current_focus: Some("Keep the existing assignment".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("equivalent PowerShell worktree aliases must retain durable authority");

        assert_eq!(
            retry.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(retry.workspace_id, work_id);
        assert_eq!(
            std::fs::read(events_path).expect("Host event log after retry"),
            before_events,
            "AlreadyAssigned retry must not append a Work event"
        );
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_work_agent_identity_once() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-legacy-codex";
        write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "Codex",
        );
        let future_alias_at = Utc::now() + chrono::Duration::days(7);
        let mut future_alias =
            WorkEvent::new(WorkEventKind::Update, work_id.clone(), future_alias_at);
        future_alias.agent_session_id = Some(session_id.to_string());
        future_alias.agent_id = Some("Codex".to_string());
        record_recovery_work_event(&project_root, &worktree, future_alias);
        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = assigned_agent_with_window(session_id, "window-host", &worktree);
        agent.agent_id = "codex".to_string();
        agent.workspace_id = Some(work_id.clone());
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection)
            .expect("save canonical Current with a legacy WorkAgentRef");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_events = std::fs::read_to_string(&events_path).expect("seeded event log");
        assert!(
            matches!(
                snapshot_workspace_update_bridge_authority(&worktree, session_id),
                WorkspaceUpdateBridgeAuthoritySnapshot::NeedsEnsure
            ),
            "legacy identity must be canonicalized only by workspace.ensure"
        );
        let state_paths = workspace_recovery_state_paths(&project_root, &worktree);
        let state_before_bridge = workspace_recovery_state_bytes(&state_paths);
        let probe = WorkspaceUpdateSuccessProbe::start(&work_id);
        {
            let _session = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_ID_ENV,
                session_id,
            );
            let _forward_url = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
                &probe.forward_url,
            );
            let _forward_token = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
                "fake-host-token",
            );
            let _runtime = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                worktree.join("managed-runtime.json"),
            );
            let mut env = TestEnv::new(worktree.clone());
            let mut output = String::new();
            let error = run(
                &mut env,
                WorkspaceCommand::Update {
                    title: None,
                    status: None,
                    status_text: None,
                    summary: Some("Must not bypass workspace.ensure".to_string()),
                    progress_summary: None,
                    next_action: None,
                    owner: None,
                    agent_session: None,
                    current_focus: None,
                    title_summary: None,
                },
                &mut output,
            )
            .expect_err("legacy identity must block managed update before Host contact");
            assert!(
                error.to_string().contains("workspace_ensure_required"),
                "{error}"
            );
            assert!(output.is_empty());
        }
        probe.assert_no_request();
        assert_eq!(
            workspace_recovery_state_bytes(&state_paths),
            state_before_bridge,
            "managed update preflight must preserve every legacy recovery surface"
        );

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize legacy Agent identity".to_string(),
                current_focus: Some("Retain exact durable authority".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("legacy builtin display name must resolve to the durable AgentId");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let saved = load_workspace_projection(&project_root)
            .expect("load canonical Current")
            .expect("canonical Current");
        assert_eq!(
            saved
                .latest_agent_for_session(session_id)
                .expect("canonical Session row")
                .agent_id,
            "codex"
        );
        let works = load_workspace_work_items(&project_root)
            .expect("load canonical WorkItems")
            .expect("canonical WorkItems");
        let work = works
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("canonical Work");
        assert_eq!(
            work.agents
                .iter()
                .find(|agent| agent.session_id == session_id)
                .and_then(|agent| agent.agent_id.as_deref()),
            Some("codex")
        );
        let after_first = std::fs::read_to_string(&events_path).expect("corrected event log");
        assert_eq!(
            after_first.lines().count(),
            before_events.lines().count() + 1
        );
        let correction = after_first
            .lines()
            .last()
            .and_then(|line| serde_json::from_str::<WorkEvent>(line).ok())
            .expect("corrective Work event");
        assert_eq!(correction.kind, WorkEventKind::Update);
        assert_eq!(correction.agent_session_id.as_deref(), Some(session_id));
        assert_eq!(correction.agent_id.as_deref(), Some("codex"));
        assert_eq!(correction.title, None);
        assert_eq!(correction.intent, None);
        assert_eq!(correction.summary, None);
        assert_eq!(correction.owner, None);
        assert_eq!(correction.execution_container, None);
        assert!(
            correction.updated_at > future_alias_at,
            "canonical correction must sort after every accepted legacy alias event"
        );
        let target =
            crate::agent_project_state::resolve_session_work_mutation_target(&worktree, session_id)
                .expect("canonicalized legacy authority must satisfy downstream strict resolution");
        assert_eq!(target.work_id, work_id);
        assert!(
            matches!(
                snapshot_workspace_update_bridge_authority(&worktree, session_id),
                WorkspaceUpdateBridgeAuthoritySnapshot::Exact(_)
            ),
            "canonical authority may be consumed after workspace.ensure"
        );

        let retry = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize legacy Agent identity".to_string(),
                current_focus: Some("Retain exact durable authority".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("canonical recovery retry");
        assert_eq!(
            retry.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(
            std::fs::read_to_string(events_path).expect("event log after retry"),
            after_first,
            "canonical recovery retry must not append another corrective event"
        );

        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        gwt_core::work_events_intake::rebuild_work_events_contents(
            &work_items_path,
            [after_first.as_str()],
            None,
        )
        .expect("deterministically refold the corrected legacy history");
        let refolded = load_workspace_work_items(&project_root)
            .expect("load refolded WorkItems")
            .expect("refolded WorkItems");
        assert_eq!(
            refolded
                .work_items
                .iter()
                .find(|item| item.id == work_id)
                .and_then(|item| {
                    item.agents
                        .iter()
                        .find(|agent| agent.session_id == session_id)
                })
                .and_then(|agent| agent.agent_id.as_deref()),
            Some("codex"),
            "deterministic refold must not restore the future-dated legacy alias"
        );

        let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        let session = gwt_agent::Session::load_and_migrate(&session_path)
            .expect("load canonicalized bound Session");
        let binding = session
            .execution_binding
            .clone()
            .expect("bound Session execution authority");
        let observation = crate::observe_agent_runtime(&worktree).expect("runtime observation");
        let update = crate::agent_project_state::apply_bound_authenticated_workspace_update(
            &project_root,
            session_id,
            &binding,
            crate::AgentWorkspaceUpdateRequest {
                schema_version: crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                claimed_session_id: session_id.to_string(),
                observation: observation.clone(),
                intent: crate::AgentWorkspaceUpdateIntent {
                    summary: Some("Canonical Agent identity accepted downstream".to_string()),
                    ..Default::default()
                },
            },
        )
        .expect("bound update must retain strict canonical authority");
        assert_eq!(update.work_id, work_id);
        let terminal = crate::agent_project_state::apply_bound_authenticated_work_terminalization(
            &project_root,
            session_id,
            &binding,
            crate::AgentWorkTerminalizationRequest {
                schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                claimed_session_id: session_id.to_string(),
                observation,
                terminal_kind: crate::AgentWorkTerminalKind::Done,
            },
        )
        .expect("bound terminalization must retain strict canonical authority");
        assert_eq!(
            terminal.outcome,
            crate::AgentWorkTerminalizationOutcome::Emitted
        );
    }

    #[test]
    fn managed_workspace_update_accepts_host_2xx_when_local_authority_is_unavailable() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("container-worktree");
        std::fs::create_dir_all(&repo).expect("container worktree");
        crate::cli::trusted_store::init_git_repo_with_origin(&repo);
        let session_id = "container-only-update-session";
        assert!(matches!(
            snapshot_workspace_update_bridge_authority(&repo, session_id),
            WorkspaceUpdateBridgeAuthoritySnapshot::Unavailable
        ));
        let probe = WorkspaceUpdateSuccessProbe::start("host-authoritative-work");
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _forward_url = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            &probe.forward_url,
        );
        let _forward_token = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "container-host-token",
        );
        let _runtime = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            repo.join("managed-runtime.json"),
        );
        let mut env = TestEnv::new(repo.clone());
        let mut output = String::new();

        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: Some("Authenticated Host owns container mutation".to_string()),
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut output,
        )
        .expect("container-local unavailable authority must retain Host 2xx compatibility");

        assert_eq!(code, 0);
        assert!(output.contains("workspace updated: fake-host-journal"));
        assert!(
            !gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo).exists(),
            "container client must not invent local Work authority after Host 2xx"
        );
        probe.expect_request();
    }

    #[test]
    fn managed_docker_workspace_update_keeps_host_bridge_authoritative_for_runtime_path() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let host_repo = temp.path().join("host-worktree");
        let runtime_repo = temp.path().join("container-worktree");
        let session_id = "docker-runtime-update-session";
        write_docker_session(session_id, &host_repo);
        std::fs::create_dir_all(&runtime_repo).expect("container worktree");
        crate::cli::trusted_store::init_git_repo_with_origin(&runtime_repo);

        let probe = WorkspaceUpdateSuccessProbe::start("host-authoritative-docker-work");
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _forward_url = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            &probe.forward_url,
        );
        let _forward_token = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "docker-host-token",
        );
        let _runtime = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_repo.join("managed-runtime.json"),
        );
        let mut env = TestEnv::new(runtime_repo.clone());
        let mut output = String::new();

        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: Some("Authenticated Host owns Docker mutation".to_string()),
                progress_summary: None,
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut output,
        )
        .expect("Docker runtime path must retain authenticated Host 2xx compatibility");

        assert_eq!(code, 0, "{output}");
        assert!(output.contains("workspace updated: fake-host-journal"));
        assert!(
            !gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&runtime_repo).exists(),
            "container client must not materialize local Work authority"
        );
        probe.expect_request();
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_current_without_corrective_work_event() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-legacy-current-only";
        write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = assigned_agent_with_window(session_id, "window-host", &worktree);
        agent.agent_id = "Codex".to_string();
        agent.workspace_id = Some(work_id.clone());
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection).expect("save legacy Current");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_events = std::fs::read(&events_path).expect("seeded event log");

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize legacy Current".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("typed legacy Current identity must be canonicalized");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let current = load_workspace_projection(&project_root)
            .expect("load canonical Current")
            .expect("canonical Current");
        assert_eq!(
            current
                .latest_agent_for_session(session_id)
                .expect("canonical Session row")
                .agent_id,
            "codex"
        );
        assert_eq!(
            std::fs::read(events_path).expect("event log after Current-only correction"),
            before_events,
            "a canonical WorkAgentRef must not receive a corrective Work event"
        );
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_work_agent_while_bootstrapping_current() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-legacy-work-only";
        write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "Codex",
        );
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before_events = std::fs::read_to_string(&events_path).expect("seeded event log");

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Bootstrap canonical Current".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("durable Session must authorize legacy builtin canonicalization");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let saved = load_workspace_projection(&project_root)
            .expect("load bootstrapped Current")
            .expect("bootstrapped Current");
        let agent = saved
            .latest_agent_for_session(session_id)
            .expect("bootstrapped Session row");
        assert!(agent.is_assigned());
        assert_eq!(agent.workspace_id.as_deref(), Some(work_id.as_str()));
        assert_eq!(agent.agent_id, "codex");
        let works = load_workspace_work_items(&project_root)
            .expect("load corrected WorkItems")
            .expect("corrected WorkItems");
        let work = works
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("corrected Work");
        assert_eq!(
            work.agents
                .iter()
                .find(|agent| agent.session_id == session_id)
                .and_then(|agent| agent.agent_id.as_deref()),
            Some("codex")
        );
        let after = std::fs::read_to_string(events_path).expect("corrected event log");
        assert_eq!(after.lines().count(), before_events.lines().count() + 1);
        let correction = after
            .lines()
            .last()
            .and_then(|line| serde_json::from_str::<WorkEvent>(line).ok())
            .expect("corrective attachment event");
        assert_eq!(correction.kind, WorkEventKind::Claim);
        assert_eq!(correction.agent_session_id.as_deref(), Some(session_id));
        assert_eq!(correction.agent_id.as_deref(), Some("codex"));
        assert_eq!(
            correction.status_category,
            Some(WorkspaceStatusCategory::Active)
        );
    }

    #[test]
    fn workspace_ensure_rejects_existing_canonical_work_owner_mismatch_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-foreign-canonical-owner",
            &worktree,
            &project_root,
            3412,
        );
        seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-foreign-canonical-owner",
            Some("Issue #9999"),
            "codex",
        );
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-foreign-canonical-owner".to_string(),
                title_summary: "Reject foreign canonical owner".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("a canonical Work id must not authorize owner reassignment");

        assert!(error.to_string().contains("owner mismatch"), "{error}");
        assert_eq!(
            workspace_recovery_state_bytes(&paths),
            before,
            "foreign canonical owner refusal must be zero-mutation"
        );
    }

    #[test]
    fn workspace_ensure_rejects_spec_owner_downgrade_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-spec-owner-downgrade";
        write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("SPEC-3412"),
            "codex",
        );
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Reject SPEC owner downgrade".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("an Issue execution must not downgrade an existing SPEC owner");

        assert!(error.to_string().contains("owner mismatch"), "{error}");
        assert_eq!(
            workspace_recovery_state_bytes(&paths),
            before,
            "SPEC owner downgrade refusal must be zero-mutation"
        );
    }

    #[test]
    fn workspace_ensure_owner_upgrade_requires_canonical_one_way_spelling() {
        assert!(workspace_ensure_can_upgrade_owner(
            Some("Issue #3412"),
            Some("SPEC-3412")
        ));
        for (stored, durable) in [
            (None, Some("SPEC-3412")),
            (Some(""), Some("SPEC-3412")),
            (Some("Issue #03412"), Some("SPEC-3412")),
            (Some("Issue #3412 "), Some("SPEC-3412")),
            (Some("Issue #3412"), Some("SPEC-03412")),
            (Some("Issue #3412"), Some("SPEC-9999")),
            (Some("SPEC-3412"), Some("Issue #3412")),
            (Some("Issue #3412"), None),
        ] {
            assert!(
                !workspace_ensure_can_upgrade_owner(stored, durable),
                "unexpected owner upgrade: stored={stored:?}, durable={durable:?}"
            );
        }
    }

    /// SPEC #3431 FR-070: heal Work items the knowledge-launch wizard stamped
    /// with the non-canonical `SPEC #<n>` spelling. No resolver emits that
    /// form, so without this bridge every such Work is permanently wedged at
    /// `workspace.ensure` and its agent can never persist a title-summary.
    #[test]
    fn workspace_ensure_owner_upgrade_heals_legacy_spec_hash_spelling() {
        assert!(workspace_ensure_can_upgrade_owner(
            Some("SPEC #3412"),
            Some("SPEC-3412")
        ));
        for (stored, durable) in [
            (Some("SPEC #03412"), Some("SPEC-3412")),
            (Some("SPEC #3412 "), Some("SPEC-3412")),
            (Some("SPEC#3412"), Some("SPEC-3412")),
            (Some("SPEC #3412"), Some("SPEC-9999")),
            (Some("SPEC #3412"), Some("Issue #3412")),
            (Some("SPEC-3412"), Some("SPEC #3412")),
            (Some("SPEC #3412"), None),
        ] {
            assert!(
                !workspace_ensure_can_upgrade_owner(stored, durable),
                "unexpected owner upgrade: stored={stored:?}, durable={durable:?}"
            );
        }
    }

    #[test]
    fn workspace_ensure_probe_advertises_owner_canonicalization_until_applied() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-probe-legacy-issue-owner";
        write_bound_projectionless_session_for_owner(
            session_id,
            &worktree,
            &project_root,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3412,
        );
        seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let input = WorkspaceEnsureInput {
            agent_session: session_id.to_string(),
            title_summary: "Canonicalize legacy Issue owner".to_string(),
            current_focus: Some("Use the durable SPEC owner".to_string()),
            spec: None,
            issue: None,
            topic: None,
            boundary: None,
        };

        assert_eq!(
            probe_workspace_ensure(&worktree, &input).state,
            crate::cli::governance::RecoveryProbeState::Available,
            "a required owner correction must be advertised as executable recovery"
        );

        ensure_workspace_for_agent(&worktree, input.clone())
            .expect("apply the advertised owner canonicalization");

        assert_eq!(
            probe_workspace_ensure(&worktree, &input).state,
            crate::cli::governance::RecoveryProbeState::Satisfied,
            "fully canonical authority must no longer advertise recovery"
        );
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_issue_owner_to_durable_spec_once() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-legacy-issue-owner";
        write_bound_projectionless_session_for_owner(
            session_id,
            &worktree,
            &project_root,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3412,
        );
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&worktree);
        let before = std::fs::read_to_string(&events_path).expect("legacy event log");

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize legacy Issue owner".to_string(),
                current_focus: Some("Use the durable SPEC owner".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("exact durable SPEC authority should upgrade the legacy Issue owner");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let current = load_workspace_projection(&project_root)
            .expect("load corrected Current")
            .expect("corrected Current");
        assert_eq!(current.owner.as_deref(), Some("SPEC-3412"));
        let items = load_workspace_work_items(&project_root)
            .expect("load corrected WorkItems")
            .expect("corrected WorkItems");
        let item = items
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("corrected Work");
        assert_eq!(item.owner.as_deref(), Some("SPEC-3412"));
        let after = std::fs::read_to_string(&events_path).expect("corrected event log");
        assert_eq!(after.lines().count(), before.lines().count() + 1);
        let correction = after
            .lines()
            .last()
            .and_then(|line| serde_json::from_str::<WorkEvent>(line).ok())
            .expect("owner correction event");
        assert_eq!(correction.kind, WorkEventKind::Claim);
        assert_eq!(correction.owner.as_deref(), Some("SPEC-3412"));
        assert_eq!(correction.agent_session_id.as_deref(), Some(session_id));
        assert_eq!(
            correction.status_category,
            Some(WorkspaceStatusCategory::Active)
        );

        let retry = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize legacy Issue owner".to_string(),
                current_focus: Some("Use the durable SPEC owner".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("canonical owner retry");
        assert_eq!(
            retry.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(
            std::fs::read_to_string(events_path).expect("event log after retry"),
            after,
            "owner canonicalization must be idempotent"
        );
    }

    #[test]
    fn workspace_ensure_rejects_projection_agent_id_mismatch_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-projection-agent-mismatch",
            &worktree,
            &project_root,
            3412,
        );
        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut foreign = unassigned_agent("session-projection-agent-mismatch");
        foreign.agent_id = "claude".to_string();
        foreign.branch = Some("work/20260601-0934".to_string());
        foreign.worktree_path =
            Some(dunce::canonicalize(&worktree).expect("canonical projection worktree"));
        projection.agents.push(foreign);
        save_workspace_projection(&project_root, &projection).expect("save foreign agent row");
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-projection-agent-mismatch".to_string(),
                title_summary: "Reject projection agent mismatch".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("durable agent identity must own recovery");

        assert!(
            error.to_string().contains("agent identity mismatch"),
            "{error}"
        );
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_work_agent_id_mismatch_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-work-agent-mismatch",
            &worktree,
            &project_root,
            3412,
        );
        seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-work-agent-mismatch",
            Some("Issue #3412"),
            "claude",
        );
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-work-agent-mismatch".to_string(),
                title_summary: "Reject Work agent mismatch".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("a foreign Work agent ref must not authorize recovery");

        assert!(
            error.to_string().contains("agent identity mismatch"),
            "{error}"
        );
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_unsafe_legacy_agent_identity_without_mutation() {
        let _guard = env_guard();
        for (label, durable_agent_id, stored_agent_id) in [
            (
                "custom-case",
                gwt_agent::AgentId::Custom("ReviewBot".to_string()),
                "reviewbot",
            ),
            ("foreign", gwt_agent::AgentId::Codex, "claude"),
            ("empty", gwt_agent::AgentId::Codex, ""),
            (
                "shell",
                gwt_agent::AgentId::Codex,
                gwt_core::workspace_projection::SHELL_WORK_AGENT_ID,
            ),
        ] {
            let gwt_home = tempfile::tempdir().expect("gwt home");
            let _home = ScopedHome::set(gwt_home.path());
            let temp = tempfile::tempdir().expect("tempdir");
            let project_root = temp.path().join(format!("workspace-home-{label}"));
            let worktree = project_root.join("work").join("issue-3412");
            let session_id = format!("session-unsafe-agent-{label}");
            write_bound_projectionless_session(&session_id, &worktree, &project_root, 3412);
            if durable_agent_id != gwt_agent::AgentId::Codex {
                let sessions_dir = gwt_core::paths::gwt_sessions_dir();
                let path = sessions_dir.join(format!("{session_id}.toml"));
                let mut session = gwt_agent::Session::load_and_migrate(&path)
                    .expect("load bound Session for custom Agent fixture");
                session.agent_id = durable_agent_id;
                session
                    .save(&sessions_dir)
                    .expect("save custom Agent Session fixture");
            }
            seed_exact_workspace_work(
                &project_root,
                &worktree,
                &session_id,
                Some("Issue #3412"),
                stored_agent_id,
            );
            let paths = workspace_recovery_state_paths(&project_root, &worktree);
            let before = workspace_recovery_state_bytes(&paths);

            let error = ensure_workspace_for_agent(
                &worktree,
                WorkspaceEnsureInput {
                    agent_session: session_id,
                    title_summary: "Reject unsafe legacy Agent identity".to_string(),
                    current_focus: None,
                    spec: None,
                    issue: None,
                    topic: None,
                    boundary: None,
                },
            )
            .expect_err("only a typed-equivalent builtin Agent identity may be canonicalized");

            assert!(
                error.to_string().contains("agent identity mismatch"),
                "{label}: {error}"
            );
            assert_eq!(
                workspace_recovery_state_bytes(&paths),
                before,
                "{label} refusal must preserve every Workspace recovery surface"
            );
        }
    }

    #[test]
    fn workspace_ensure_rejects_ambiguous_exact_work_refs_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-ambiguous-exact",
            &worktree,
            &project_root,
            3412,
        );
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-ambiguous-exact",
            Some("Issue #3412"),
            "codex",
        );
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let mut work_items =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&works_path)
                .expect("load exact WorkItems")
                .expect("exact WorkItems");
        let item = work_items
            .work_items
            .iter_mut()
            .find(|item| item.id == work_id)
            .expect("exact Work");
        item.agents.push(item.agents[0].clone());
        item.execution_containers
            .push(item.execution_containers[0].clone());
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &works_path,
            &work_items,
        )
        .expect("save ambiguous exact WorkItems");
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-ambiguous-exact".to_string(),
                title_summary: "Reject ambiguous exact Work".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("duplicate agent/container authority must fail closed");

        assert!(error.to_string().contains("ambiguous"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_duplicate_current_session_rows_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-duplicate-current",
            &worktree,
            &project_root,
            3412,
        );
        seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-duplicate-current",
            Some("Issue #3412"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = unassigned_agent("session-duplicate-current");
        agent.branch = Some("work/20260601-0934".to_string());
        agent.worktree_path = Some(dunce::canonicalize(&worktree).expect("canonical worktree"));
        projection.agents.push(agent.clone());
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection).expect("save duplicate current rows");
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-duplicate-current".to_string(),
                title_summary: "Reject duplicate current authority".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("duplicate current Session rows must fail closed");

        assert!(error.to_string().contains("ambiguous"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_terminal_canonical_work_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-terminal-canonical",
            &worktree,
            &project_root,
            3412,
        );
        let work_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            "session-terminal-canonical",
            Some("Issue #3412"),
            "codex",
        );
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let mut work_items =
            gwt_core::workspace_projection::load_workspace_work_items_from_path(&works_path)
                .expect("load exact WorkItems")
                .expect("exact WorkItems");
        let mut done = WorkEvent::new(WorkEventKind::Done, work_id, Utc::now());
        done.status_category = Some(WorkspaceStatusCategory::Done);
        work_items.apply_event(done.clone());
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &works_path,
            &work_items,
        )
        .expect("terminalize canonical WorkItems");
        gwt_core::workspace_projection::append_workspace_work_event_to_path(
            &gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            &done,
        )
        .expect("append terminal event");
        let paths = workspace_recovery_state_paths(&project_root, &worktree);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-terminal-canonical".to_string(),
                title_summary: "Reject terminal canonical Work".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("terminal canonical Work must not be resurrected");

        assert!(error.to_string().contains("terminal"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_noncanonical_durable_assignment_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session("session-wrong-work", &worktree, &project_root, 3412);

        let mut foreign = WorkEvent::new(WorkEventKind::Start, "work-foreign", Utc::now());
        foreign.title = Some("Launch Work projection recovery".to_string());
        foreign.status_category = Some(WorkspaceStatusCategory::Active);
        foreign.owner = Some("Issue #3412".to_string());
        foreign.agent_session_id = Some("session-wrong-work".to_string());
        record_recovery_work_event(&project_root, &worktree, foreign);

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut agent = unassigned_agent("session-wrong-work");
        agent.worktree_path = Some(worktree.clone());
        agent.branch = Some("work/20260601-0934".to_string());
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some("work-foreign".to_string());
        projection.agents.push(agent);
        save_workspace_projection(&project_root, &projection).expect("save wrong assignment");
        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            worktree.join(".gitattributes"),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-wrong-work".to_string(),
                title_summary: "Launch Work projection recovery".to_string(),
                current_focus: Some("Reject the noncanonical Work assignment".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("noncanonical assignment requires explicit operator repair");

        assert!(error
            .to_string()
            .contains("noncanonical Workspace assignment"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "noncanonical refusal must be zero-mutation");
    }

    #[test]
    fn workspace_ensure_rejects_noncurrent_execution_binding_before_workspace_write() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let owner = write_bound_projectionless_session(
            "session-stale-binding",
            &worktree,
            &project_root,
            3412,
        );
        assert!(matches!(
            crate::cli::execution_state::settle(
                &worktree,
                "session-stale-binding",
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "terminal test binding".to_string(),
                    missing_verification: Some("test evidence".to_string()),
                },
            )
            .expect("terminalize bound execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        assert_eq!(owner.number, 3412);
        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            worktree.join(".gitattributes"),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-stale-binding".to_string(),
                title_summary: "Stale projection recovery".to_string(),
                current_focus: Some("Reject terminal execution authority".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("terminal execution binding must not bootstrap a Work");

        assert!(
            error.to_string().contains("not current"),
            "unexpected error: {error}"
        );
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "authority refusal must be zero-mutation");
    }

    #[test]
    fn workspace_ensure_accepts_older_active_session_exact_binding() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-older-active";
        let owner = write_bound_projectionless_session(session_id, &worktree, &project_root, 3412);
        let older_binding = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
        )
        .expect("load older Session")
        .execution_binding
        .expect("older Session binding");
        let request = crate::cli::execution_state::SuccessorRequest {
            operation_id: "fresh-independent-session".to_string(),
            principal_id: "gwt-host-launch".to_string(),
            work_id: None,
            source: crate::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
            session_binding_id: "binding-independent-session".to_string(),
            initial_session_id: "session-newer-active".to_string(),
            entrypoint: "$gwt-execute #3412".to_string(),
            requested_at: Utc::now(),
        };
        crate::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
            &worktree, owner, &request,
        )
        .expect("prepare independent newer generation");
        crate::cli::execution_state::activate_successor(&worktree, owner, &request)
            .expect("activate independent newer generation");
        assert_ne!(
            crate::cli::execution_state::current_execution_binding(&worktree, owner)
                .expect("read latest binding")
                .expect("latest binding")
                .generation_id,
            older_binding.identity.generation_id
        );

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Recover the older independent Session".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("an older Active generation remains exact authority for its own Session");

        assert_eq!(result.disposition, WorkspaceEnsureDisposition::Created);
    }

    #[test]
    fn workspace_ensure_rejects_terminal_noncanonical_work_attachment() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-terminal-foreign",
            &worktree,
            &project_root,
            3412,
        );
        let mut start = WorkEvent::new(WorkEventKind::Start, "work-terminal-foreign", Utc::now());
        start.agent_session_id = Some("session-terminal-foreign".to_string());
        start.owner = Some("Issue #3412".to_string());
        record_recovery_work_event(&project_root, &worktree, start);
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-terminal-foreign", Utc::now());
        done.status_category = Some(WorkspaceStatusCategory::Done);
        record_recovery_work_event(&project_root, &worktree, done);
        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-terminal-foreign".to_string(),
                title_summary: "Reject historical reassignment".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("terminal foreign attachment must remain authoritative");

        assert!(error.to_string().contains("noncanonical Work"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "terminal attachment refusal must be exact");
    }

    #[test]
    fn workspace_ensure_bootstraps_canonical_work_despite_foreign_live_container() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-canonical-bootstrap",
            &worktree,
            &project_root,
            3412,
        );
        let mut start = WorkEvent::new(
            WorkEventKind::Start,
            "work-foreign-live-container",
            Utc::now(),
        );
        start.agent_session_id = Some("foreign-session".to_string());
        start.agent_id = Some("codex".to_string());
        start.owner = Some("Issue #9999".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(dunce::canonicalize(&worktree).expect("canonical shadow worktree")),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(&project_root, &worktree, start);

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-canonical-bootstrap".to_string(),
                title_summary: "Bootstrap independent canonical Work".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("a foreign live container must not block canonical Work bootstrap");

        assert_eq!(result.disposition, WorkspaceEnsureDisposition::Created);
        let items = load_workspace_work_items(&project_root)
            .expect("load WorkItems")
            .expect("WorkItems");
        assert!(items
            .work_items
            .iter()
            .any(|item| item.id == "work-foreign-live-container"));
        assert!(items
            .work_items
            .iter()
            .any(|item| item.id == result.workspace_id));
    }

    #[test]
    fn workspace_ensure_ignores_terminal_foreign_shadow_when_canonical_work_exists() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-canonical-with-terminal-shadow";
        write_bound_projectionless_session_for_owner(
            session_id,
            &worktree,
            &project_root,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3412,
        );
        let canonical_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let canonical_worktree = dunce::canonicalize(&worktree).expect("canonical worktree");
        let mut shadow = WorkEvent::new(
            WorkEventKind::Start,
            "work-terminal-foreign-shadow",
            Utc::now(),
        );
        shadow.agent_session_id = Some("historical-session".to_string());
        shadow.agent_id = Some("codex".to_string());
        shadow.owner = Some("Issue #9999".to_string());
        shadow.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(canonical_worktree),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(&project_root, &worktree, shadow);
        let mut done = WorkEvent::new(
            WorkEventKind::Done,
            "work-terminal-foreign-shadow",
            Utc::now(),
        );
        done.status_category = Some(WorkspaceStatusCategory::Done);
        record_recovery_work_event(&project_root, &worktree, done);

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Retain canonical Work authority".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("a terminal foreign shadow must not block an existing canonical Work");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, canonical_id);
        let items = load_workspace_work_items(&project_root)
            .expect("load WorkItems")
            .expect("WorkItems");
        let canonical = items
            .work_items
            .iter()
            .find(|item| item.id == canonical_id)
            .expect("canonical Work");
        assert_eq!(canonical.owner.as_deref(), Some("SPEC-3412"));
        assert!(
            items
                .work_items
                .iter()
                .find(|item| item.id == "work-terminal-foreign-shadow")
                .is_some_and(WorkItem::is_terminal),
            "historical shadow must remain terminal audit history"
        );
    }

    #[test]
    fn workspace_ensure_ignores_paused_foreign_shadow_when_canonical_work_exists() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        let session_id = "session-canonical-with-paused-shadow";
        write_bound_projectionless_session_for_owner(
            session_id,
            &worktree,
            &project_root,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3412,
        );
        let canonical_id = seed_exact_workspace_work(
            &project_root,
            &worktree,
            session_id,
            Some("Issue #3412"),
            "codex",
        );
        let canonical_worktree = dunce::canonicalize(&worktree).expect("canonical worktree");
        let mut shadow = WorkEvent::new(
            WorkEventKind::Start,
            "work-paused-foreign-shadow",
            Utc::now(),
        );
        shadow.agent_session_id = Some("historical-session".to_string());
        shadow.agent_id = Some("codex".to_string());
        shadow.owner = Some("Issue #9999".to_string());
        shadow.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260601-0934".to_string()),
            worktree_path: Some(canonical_worktree),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        record_recovery_work_event(&project_root, &worktree, shadow);
        let mut pause = WorkEvent::new(
            WorkEventKind::Pause,
            "work-paused-foreign-shadow",
            Utc::now(),
        );
        pause.agent_session_id = Some("historical-session".to_string());
        record_recovery_work_event(&project_root, &worktree, pause);

        let result = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Retain canonical Work authority".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("a paused foreign shadow must not block an existing canonical Work");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, canonical_id);
        let items = load_workspace_work_items(&project_root)
            .expect("load WorkItems")
            .expect("WorkItems");
        let paused = items
            .work_items
            .iter()
            .find(|item| item.id == "work-paused-foreign-shadow")
            .expect("paused historical Work");
        assert_eq!(paused.status_category, WorkspaceStatusCategory::Idle);
        assert!(!paused.is_terminal(), "Pause remains resumable history");
    }

    #[test]
    fn workspace_ensure_ignores_foreign_live_authorities_when_canonical_work_exists() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");

        for (label, status, owner_number) in [
            ("active", WorkspaceStatusCategory::Active, 3412),
            ("blocked", WorkspaceStatusCategory::Blocked, 3413),
            ("unknown", WorkspaceStatusCategory::Unknown, 3414),
        ] {
            let project_root = temp.path().join(label).join("workspace-home");
            let worktree = project_root
                .join("work")
                .join(format!("issue-{owner_number}"));
            let session_id = format!("session-{label}-canonical");
            write_bound_projectionless_session_for_owner(
                &session_id,
                &worktree,
                &project_root,
                crate::cli::execution_state::ExecutionOwnerKind::Spec,
                owner_number,
            );
            let canonical_id = seed_exact_workspace_work(
                &project_root,
                &worktree,
                &session_id,
                Some(&format!("Issue #{owner_number}")),
                "codex",
            );
            let shadow_id = format!("work-{label}-foreign-shadow");
            seed_workspace_container_shadow(
                &project_root,
                &worktree,
                &shadow_id,
                "foreign-session",
                status,
            );

            let result = ensure_workspace_for_agent(
                &worktree,
                WorkspaceEnsureInput {
                    agent_session: session_id,
                    title_summary: "Keep independent Session authority".to_string(),
                    current_focus: None,
                    spec: None,
                    issue: None,
                    topic: None,
                    boundary: None,
                },
            )
            .expect("a foreign authority on the same branch/worktree must not block ensure");

            assert_eq!(
                result.disposition,
                WorkspaceEnsureDisposition::AlreadyAssigned,
                "{label}"
            );
            assert_eq!(result.workspace_id, canonical_id, "{label}");
            let items = load_workspace_work_items(&project_root)
                .expect("load WorkItems")
                .expect("WorkItems");
            assert_eq!(
                items
                    .work_items
                    .iter()
                    .find(|item| item.id == shadow_id)
                    .expect("foreign authority remains independently recorded")
                    .status_category,
                status,
                "{label}"
            );
        }
    }

    #[test]
    fn workspace_ensure_rejects_projection_identity_mismatch_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session("session-stale-row", &worktree, &project_root, 3412);

        let mut projection = WorkspaceProjection::default_for_project(&project_root);
        let mut stale = unassigned_agent("session-stale-row");
        stale.branch = Some("work/foreign".to_string());
        stale.worktree_path = Some(project_root.join("work").join("foreign"));
        projection.agents.push(stale);
        let legacy_current = gwt_core::paths::gwt_project_dir_for_repo_path(&project_root)
            .join("workspace/current.json");
        std::fs::create_dir_all(legacy_current.parent().expect("legacy current parent"))
            .expect("create legacy current parent");
        std::fs::write(
            &legacy_current,
            serde_json::to_vec_pretty(&projection).expect("serialize stale projection row"),
        )
        .expect("save legacy stale projection row");

        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            legacy_current,
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            worktree.join(".gitattributes"),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-stale-row".to_string(),
                title_summary: "Reject stale projection identity".to_string(),
                current_focus: Some("Preserve durable Session authority".to_string()),
                spec: None,
                issue: Some(3412),
                topic: None,
                boundary: None,
            },
        )
        .expect_err("a stale projection row must not override durable Session identity");

        assert!(error.to_string().contains("projection identity mismatch"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "identity refusal must be zero-mutation");
    }

    #[test]
    fn workspace_ensure_does_not_bootstrap_without_durable_session() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let repo = tempfile::tempdir().expect("repo");
        let current_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo.path());
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo.path());
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(repo.path());

        let error = ensure_workspace_for_agent(
            repo.path(),
            WorkspaceEnsureInput {
                agent_session: "session-missing".to_string(),
                title_summary: "Work projection bootstrap".to_string(),
                current_focus: Some("Reject an unauthenticated bootstrap".to_string()),
                spec: None,
                issue: Some(3412),
                topic: None,
                boundary: None,
            },
        )
        .expect_err("a missing durable Session must fail closed");

        assert!(error.to_string().contains("Session ledger is missing"));
        assert!(
            !current_path.exists(),
            "current projection must stay absent"
        );
        assert!(!work_items_path.exists(), "WorkItems must stay absent");
        assert!(
            !events_path.exists(),
            "tracked Work events must stay absent"
        );
    }

    #[test]
    fn workspace_ensure_rejects_stale_projection_row_without_durable_session() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let repo = tempfile::tempdir().expect("repo");
        let mut projection = WorkspaceProjection::default_for_project(repo.path());
        projection
            .agents
            .push(unassigned_agent("session-stale-without-ledger"));
        save_workspace_projection(repo.path(), &projection).expect("save stale projection row");
        let current_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo.path());
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo.path());
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(repo.path());
        let before_current = std::fs::read(&current_path).expect("stale current projection");

        let error = ensure_workspace_for_agent(
            repo.path(),
            WorkspaceEnsureInput {
                agent_session: "session-stale-without-ledger".to_string(),
                title_summary: "Reject stale projection authority".to_string(),
                current_focus: None,
                spec: None,
                issue: Some(3412),
                topic: None,
                boundary: None,
            },
        )
        .expect_err("a projection row must not substitute for a durable Session");

        assert!(error.to_string().contains("Session ledger is missing"));
        assert_eq!(
            std::fs::read(&current_path).expect("current projection after refusal"),
            before_current,
            "stale projection refusal must not rewrite current state"
        );
        assert!(!work_items_path.exists(), "WorkItems must stay absent");
        assert!(
            !events_path.exists(),
            "tracked Work events must stay absent"
        );
    }

    #[test]
    fn workspace_ensure_preserves_docker_already_assigned_without_new_event() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_docker_session("session-docker-existing", &repo);

        let work_id = seed_exact_workspace_work(
            &repo,
            &repo,
            "session-docker-existing",
            Some("Issue #3412"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-existing", "window-docker", &repo);
        agent.workspace_id = Some(work_id.clone());
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save Docker assignment");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);
        let before_events = std::fs::read(&events_path).expect("Docker event log");

        let result = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-existing".to_string(),
                title_summary: "Existing Docker Work".to_string(),
                current_focus: Some("Keep the existing assignment".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("exact existing Docker assignment remains supported");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        assert_eq!(
            std::fs::read(events_path).expect("Docker event log after ensure"),
            before_events,
            "Docker AlreadyAssigned must not append a recovery event"
        );
    }

    #[test]
    fn workspace_ensure_docker_ignores_terminal_and_paused_foreign_history() {
        let _guard = env_guard();
        let temp = tempfile::tempdir().expect("tempdir");

        for (label, status, owner_number) in [
            ("terminal", WorkspaceStatusCategory::Done, 3412),
            ("paused", WorkspaceStatusCategory::Idle, 3412),
        ] {
            let gwt_home = tempfile::tempdir().expect("gwt home");
            let _home = ScopedHome::set(gwt_home.path());
            let repo = temp.path().join(label).join("repo");
            let session_id = format!("session-docker-{label}");
            write_docker_session_for_owner(
                &session_id,
                &repo,
                crate::cli::execution_state::ExecutionOwnerKind::Spec,
                owner_number,
            );
            let work_id = seed_exact_workspace_work(
                &repo,
                &repo,
                &session_id,
                Some(&format!("Issue #{owner_number}")),
                "codex",
            );
            let mut projection = WorkspaceProjection::default_for_project(&repo);
            projection.owner = Some(format!("Issue #{owner_number}"));
            let mut agent = assigned_agent_with_window(&session_id, "window-docker", &repo);
            agent.workspace_id = Some(work_id.clone());
            projection.agents.push(agent);
            save_workspace_projection(&repo, &projection).expect("save Docker assignment");
            let shadow_id = format!("work-docker-{label}-history");
            seed_workspace_container_shadow(&repo, &repo, &shadow_id, "historical-session", status);

            let result = ensure_workspace_for_agent(
                &repo,
                WorkspaceEnsureInput {
                    agent_session: session_id,
                    title_summary: "Retain exact Docker authority".to_string(),
                    current_focus: None,
                    spec: None,
                    issue: None,
                    topic: None,
                    boundary: None,
                },
            )
            .expect("historical shadow must not block exact Docker authority");

            assert_eq!(
                result.disposition,
                WorkspaceEnsureDisposition::AlreadyAssigned
            );
            assert_eq!(result.workspace_id, work_id);
            let items = load_workspace_work_items(&repo)
                .expect("load Docker WorkItems")
                .expect("Docker WorkItems");
            assert_eq!(
                items
                    .work_items
                    .iter()
                    .find(|item| item.id == shadow_id)
                    .expect("historical Docker shadow")
                    .status_category,
                status
            );
        }
    }

    #[test]
    fn workspace_ensure_docker_ignores_foreign_live_authorities() {
        let _guard = env_guard();
        let temp = tempfile::tempdir().expect("tempdir");

        for (label, status, owner_number) in [
            ("active", WorkspaceStatusCategory::Active, 3412),
            ("blocked", WorkspaceStatusCategory::Blocked, 3412),
            ("unknown", WorkspaceStatusCategory::Unknown, 3412),
        ] {
            let gwt_home = tempfile::tempdir().expect("gwt home");
            let _home = ScopedHome::set(gwt_home.path());
            let repo = temp.path().join(label).join("repo");
            let session_id = format!("session-docker-{label}");
            write_docker_session_for_owner(
                &session_id,
                &repo,
                crate::cli::execution_state::ExecutionOwnerKind::Spec,
                owner_number,
            );
            let work_id = seed_exact_workspace_work(
                &repo,
                &repo,
                &session_id,
                Some(&format!("Issue #{owner_number}")),
                "codex",
            );
            let mut projection = WorkspaceProjection::default_for_project(&repo);
            projection.owner = Some(format!("Issue #{owner_number}"));
            let mut agent = assigned_agent_with_window(&session_id, "window-docker", &repo);
            agent.workspace_id = Some(work_id);
            projection.agents.push(agent);
            save_workspace_projection(&repo, &projection).expect("save Docker assignment");
            let shadow_id = format!("work-docker-{label}-shadow");
            seed_workspace_container_shadow(&repo, &repo, &shadow_id, "foreign-session", status);

            let result = ensure_workspace_for_agent(
                &repo,
                WorkspaceEnsureInput {
                    agent_session: session_id,
                    title_summary: "Keep independent Docker Session authority".to_string(),
                    current_focus: None,
                    spec: None,
                    issue: None,
                    topic: None,
                    boundary: None,
                },
            )
            .expect("a foreign Docker authority must not block exact assignment");

            assert_eq!(
                result.disposition,
                WorkspaceEnsureDisposition::AlreadyAssigned,
                "{label}"
            );
        }
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_docker_agent_identity_once() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let session_id = "session-docker-legacy-codex";
        write_docker_session(session_id, &repo);
        let work_id =
            seed_exact_workspace_work(&repo, &repo, session_id, Some("Issue #3412"), "Codex");
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent = assigned_agent_with_window(session_id, "window-docker", &repo);
        agent.agent_id = "Codex".to_string();
        agent.workspace_id = Some(work_id.clone());
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save legacy Docker assignment");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);
        let before_events = std::fs::read_to_string(&events_path).expect("Docker event log");

        let result = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize Docker Agent identity".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("exact Docker authority may canonicalize a legacy builtin Agent identity");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let current = load_workspace_projection(&repo)
            .expect("load Docker Current")
            .expect("Docker Current");
        assert_eq!(
            current
                .latest_agent_for_session(session_id)
                .expect("Docker Session row")
                .agent_id,
            "codex"
        );
        let works = load_workspace_work_items(&repo)
            .expect("load Docker WorkItems")
            .expect("Docker WorkItems");
        let work = works
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("Docker Work");
        assert_eq!(
            work.agents
                .iter()
                .find(|agent| agent.session_id == session_id)
                .and_then(|agent| agent.agent_id.as_deref()),
            Some("codex")
        );
        let after_first = std::fs::read_to_string(&events_path).expect("corrected Docker log");
        assert_eq!(
            after_first.lines().count(),
            before_events.lines().count() + 1
        );

        ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize Docker Agent identity".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("canonical Docker retry");
        assert_eq!(
            std::fs::read_to_string(events_path).expect("Docker log after retry"),
            after_first,
            "canonical Docker retry must not append another corrective event"
        );
    }

    #[test]
    fn workspace_ensure_canonicalizes_legacy_docker_issue_owner_once() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let session_id = "session-docker-legacy-issue-owner";
        write_docker_session_for_owner(
            session_id,
            &repo,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3412,
        );
        let work_id =
            seed_exact_workspace_work(&repo, &repo, session_id, Some("Issue #3412"), "codex");
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.owner = Some("Issue #3412".to_string());
        let mut agent = assigned_agent_with_window(session_id, "window-docker", &repo);
        agent.workspace_id = Some(work_id.clone());
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save legacy Docker owner");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);
        let before_events = std::fs::read_to_string(&events_path).expect("Docker event log");

        let result = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize Docker Issue owner".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("exact durable Docker SPEC authority should upgrade the legacy Issue owner");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        assert_eq!(result.workspace_id, work_id);
        let current = load_workspace_projection(&repo)
            .expect("load corrected Docker Current")
            .expect("corrected Docker Current");
        assert_eq!(current.owner.as_deref(), Some("SPEC-3412"));
        let works = load_workspace_work_items(&repo)
            .expect("load corrected Docker WorkItems")
            .expect("corrected Docker WorkItems");
        assert_eq!(
            works
                .work_items
                .iter()
                .find(|item| item.id == work_id)
                .and_then(|item| item.owner.as_deref()),
            Some("SPEC-3412")
        );
        let after_events = std::fs::read_to_string(&events_path).expect("corrected Docker log");
        assert_eq!(
            after_events.lines().count(),
            before_events.lines().count() + 1
        );
        let correction = after_events
            .lines()
            .last()
            .and_then(|line| serde_json::from_str::<WorkEvent>(line).ok())
            .expect("Docker owner correction event");
        assert_eq!(correction.kind, WorkEventKind::Update);
        assert_eq!(correction.owner.as_deref(), Some("SPEC-3412"));
        let paths = workspace_recovery_state_paths(&repo, &repo);
        let after_first = workspace_recovery_state_bytes(&paths);

        ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: session_id.to_string(),
                title_summary: "Canonicalize Docker Issue owner".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("canonical Docker owner retry");
        assert_eq!(
            workspace_recovery_state_bytes(&paths),
            after_first,
            "Docker owner canonicalization must be byte-idempotent"
        );
    }

    #[test]
    fn workspace_ensure_rejects_unbound_docker_session_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_unbound_docker_session("session-docker-unbound", &repo);
        let work_id = seed_exact_workspace_work(
            &repo,
            &repo,
            "session-docker-unbound",
            Some("Issue #3412"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-unbound", "window-docker", &repo);
        agent.workspace_id = Some(work_id);
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save unbound Docker assignment");
        let paths = workspace_recovery_state_paths(&repo, &repo);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-unbound".to_string(),
                title_summary: "Reject unbound Docker authority".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("Docker recovery requires a durable execution binding");

        assert!(error.to_string().contains("execution binding"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_noncurrent_docker_binding_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_docker_session("session-docker-stale-binding", &repo);
        let work_id = seed_exact_workspace_work(
            &repo,
            &repo,
            "session-docker-stale-binding",
            Some("Issue #3412"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-stale-binding", "window-docker", &repo);
        agent.workspace_id = Some(work_id);
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save stale Docker assignment");
        assert!(matches!(
            crate::cli::execution_state::settle(
                &repo,
                "session-docker-stale-binding",
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "terminal Docker binding".to_string(),
                    missing_verification: Some("test evidence".to_string()),
                },
            )
            .expect("terminalize Docker execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let paths = workspace_recovery_state_paths(&repo, &repo);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-stale-binding".to_string(),
                title_summary: "Reject stale Docker authority".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("Docker recovery requires a current execution binding");

        assert!(error.to_string().contains("not current"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_docker_foreign_owner_without_mutation() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_docker_session("session-docker-owner-mismatch", &repo);
        let work_id = seed_exact_workspace_work(
            &repo,
            &repo,
            "session-docker-owner-mismatch",
            Some("Issue #9999"),
            "codex",
        );
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-owner-mismatch", "window-docker", &repo);
        agent.workspace_id = Some(work_id);
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save Docker assignment");
        let paths = workspace_recovery_state_paths(&repo, &repo);
        let before = workspace_recovery_state_bytes(&paths);

        let error = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-owner-mismatch".to_string(),
                title_summary: "Reject Docker foreign owner".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("Docker validation must enforce durable owner");

        assert!(error.to_string().contains("owner mismatch"), "{error}");
        assert_eq!(workspace_recovery_state_bytes(&paths), before);
    }

    #[test]
    fn workspace_ensure_rejects_docker_missing_work_without_recovery() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_docker_session("session-docker-missing", &repo);

        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-missing", "window-docker", &repo);
        agent.workspace_id = gwt_core::workspace_projection::canonical_work_id(
            &repo,
            Some("work/20260601-0934"),
            Some(repo.as_path()),
        );
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save Docker assignment");
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(&repo);

        let error = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-missing".to_string(),
                title_summary: "Missing Docker Work".to_string(),
                current_focus: Some("Reject local recovery".to_string()),
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("Docker recovery requires the Host bridge");

        assert!(error.to_string().contains("cannot recover a missing Work"));
        assert!(
            !events_path.exists(),
            "Docker refusal must not append Start"
        );
    }

    #[test]
    fn workspace_ensure_rejects_docker_legacy_missing_work_without_migration() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        write_docker_session("session-docker-legacy-missing", &repo);
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        let mut agent =
            assigned_agent_with_window("session-docker-legacy-missing", "window-docker", &repo);
        agent.workspace_id = gwt_core::workspace_projection::canonical_work_id(
            &repo,
            Some("work/20260601-0934"),
            Some(repo.as_path()),
        );
        projection.agents.push(agent);
        let legacy_current =
            gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("workspace/current.json");
        std::fs::create_dir_all(legacy_current.parent().expect("legacy current parent"))
            .expect("legacy current parent");
        let legacy_bytes =
            serde_json::to_vec_pretty(&projection).expect("serialize legacy current");
        std::fs::write(&legacy_current, &legacy_bytes).expect("write legacy current");
        let canonical_current = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
        let canonical_works = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
        let canonical_events = gwt_core::paths::gwt_repo_local_work_events_path(&repo);

        let error = ensure_workspace_for_agent(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-docker-legacy-missing".to_string(),
                title_summary: "Missing legacy Docker Work".to_string(),
                current_focus: None,
                spec: None,
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect_err("Docker legacy recovery must fail before migration");

        assert!(error.to_string().contains("cannot recover a missing Work"));
        assert_eq!(
            std::fs::read(&legacy_current).expect("legacy current after refusal"),
            legacy_bytes
        );
        assert!(!canonical_current.exists());
        assert!(!canonical_works.exists());
        assert!(!canonical_events.exists());
    }

    #[test]
    fn workspace_ensure_rejects_owner_mismatch_before_workspace_write() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_bound_projectionless_session(
            "session-owner-mismatch",
            &worktree,
            &project_root,
            3412,
        );
        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            worktree.join(".gitattributes"),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-owner-mismatch".to_string(),
                title_summary: "Reject owner mismatch".to_string(),
                current_focus: None,
                spec: None,
                issue: Some(9999),
                topic: None,
                boundary: None,
            },
        )
        .expect_err("explicit owner must match the durable Session");

        assert!(error.to_string().contains("owner mismatch"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "owner refusal must precede every write");
    }

    #[test]
    fn workspace_ensure_rejects_invalid_execution_binding_before_workspace_write() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("issue-3412");
        write_projectionless_session("session-invalid-binding", &worktree, &project_root, 3412);
        let session_path = gwt_core::paths::gwt_sessions_dir().join("session-invalid-binding.toml");
        let mut session = gwt_agent::Session::load(&session_path).expect("load Session fixture");
        session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: "wrong-repository".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3412,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-invalid".to_string(),
                binding_id: "binding-invalid".to_string(),
                ledger_head_hash: "head-invalid".to_string(),
            },
            capability_generation: 1,
        });
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save invalid binding fixture");
        let paths = [
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
            gwt_core::paths::gwt_repo_local_work_events_path(&worktree),
            worktree.join(".gitattributes"),
        ];
        let before = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();

        let error = ensure_workspace_for_agent(
            &worktree,
            WorkspaceEnsureInput {
                agent_session: "session-invalid-binding".to_string(),
                title_summary: "Reject invalid binding".to_string(),
                current_focus: None,
                spec: None,
                issue: Some(3412),
                topic: None,
                boundary: None,
            },
        )
        .expect_err("invalid binding must fail before Work mutation");

        assert!(error
            .to_string()
            .contains("invalid durable Session execution binding"));
        let after = paths
            .iter()
            .map(|path| std::fs::read(path).ok())
            .collect::<Vec<_>>();
        assert_eq!(after, before, "binding refusal must precede every write");
    }

    #[test]
    fn workspace_ensure_is_idempotent_for_already_assigned_agent() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
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

        let result = apply_legacy_workspace_ensure_transition_for_test(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace materialization".to_string(),
                current_focus: Some("Continue current Workspace".to_string()),
                spec: Some(2359),
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("ensure workspace");

        assert_eq!(result.workspace_id, "workspace-existing");
        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(items.work_items.len(), 1);
    }

    #[test]
    fn workspace_ensure_repairs_assigned_agent_when_work_creation_was_interrupted() {
        let _guard = env_guard();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut agent = unassigned_agent("session-interrupted");
        agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        agent.workspace_id = Some("workspace-interrupted".to_string());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.id = "workspace-interrupted".to_string();
        projection.agents.push(agent);
        save_workspace_projection(&repo, &projection).expect("save projection");

        let result = apply_legacy_workspace_ensure_transition_for_test(
            &repo,
            WorkspaceEnsureInput {
                agent_session: "session-interrupted".to_string(),
                title_summary: "Interrupted materialization".to_string(),
                current_focus: Some("Repair Work creation".to_string()),
                spec: Some(2359),
                issue: None,
                topic: None,
                boundary: None,
            },
        )
        .expect("repair ensure");

        assert_eq!(
            result.disposition,
            WorkspaceEnsureDisposition::AlreadyAssigned
        );
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        let repaired = items
            .work_items
            .iter()
            .find(|item| item.id == "workspace-interrupted")
            .expect("missing Work is recreated");
        assert_eq!(repaired.title, "Interrupted materialization");
        assert!(repaired
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-interrupted"));
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

    #[test]
    fn run_projection_prune_with_scan_root_dry_run_counts_empty_workspace_state_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("canvas-state-only");
        std::fs::create_dir_all(&project_dir).expect("project dir");
        crate::save_workspace_state(
            &project_dir.join("workspace.json"),
            &crate::empty_workspace_state(),
        )
        .expect("save empty workspace state");

        let mut out = String::new();
        let code = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            Utc::now(),
            true,
            &[],
            |_| false,
            &mut out,
        )
        .expect("prune dry-run");

        assert_eq!(code, 0);
        assert!(out.contains("DRY-RUN: archive=0 delete=1 skip=0"));
        assert!(
            project_dir.join("workspace.json").is_file(),
            "dry-run must not remove the legacy canvas state file"
        );
    }

    #[test]
    fn run_projection_prune_with_scan_root_counts_no_window_workspace_state_with_custom_viewport() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("viewport-only-canvas-state");
        std::fs::create_dir_all(&project_dir).expect("project dir");
        let mut state = crate::empty_workspace_state();
        state.viewport.x = 24.0;
        state.viewport.y = 36.0;
        state.viewport.zoom = 1.5;
        state.next_z_index = 8;
        crate::save_workspace_state(&project_dir.join("workspace.json"), &state)
            .expect("save viewport-only workspace state");

        let mut out = String::new();
        let code = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            Utc::now(),
            true,
            &[],
            |_| false,
            &mut out,
        )
        .expect("prune dry-run");

        assert_eq!(code, 0);
        assert!(out.contains("DRY-RUN: archive=0 delete=1 skip=0"));
    }

    #[test]
    fn run_projection_prune_with_scan_root_removes_empty_workspace_state_project_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("canvas-state-only");
        std::fs::create_dir_all(&project_dir).expect("project dir");
        crate::save_workspace_state(
            &project_dir.join("workspace.json"),
            &crate::empty_workspace_state(),
        )
        .expect("save empty workspace state");

        let mut out = String::new();
        let code = run_projection_prune_with_scan_root(
            tmp.path(),
            &WorkspaceRetentionConfig::default(),
            Utc::now(),
            false,
            &[],
            |_| false,
            &mut out,
        )
        .expect("prune apply");

        assert_eq!(code, 0);
        assert!(out.contains("APPLIED: archive=0 delete=1 skip=0"));
        assert!(
            !project_dir.exists(),
            "empty project dir should be removed after deleting its only legacy canvas state"
        );
    }
}
