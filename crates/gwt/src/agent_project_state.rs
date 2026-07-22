use std::path::{Path, PathBuf};

use chrono::Utc;
use gwt_agent::{LaunchRuntimeTarget, Session};
use gwt_core::{
    error::{GwtError, Result},
    paths::normalize_windows_child_process_path,
    workspace_projection::{
        load_workspace_projection, load_workspace_projection_from_path,
        load_workspace_work_items_from_path, mutate_existing_workspace_projection,
        update_workspace_projection_with_journal_for_resolved_work_target,
        SessionBoundWorkspaceMutationTarget, SessionBoundWorkspaceTerminalTarget,
        TrackedWorkEventPolicy, WorkspaceAgentSummary, WorkspaceProjectionUpdate,
    },
};
use serde::{Deserialize, Serialize};

pub const AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION: u32 = 1;
pub const AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRuntimeObservation {
    pub cwd: String,
    pub git_toplevel: String,
    pub repo_hash: String,
    pub branch: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkspaceUpdateIntent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_category: Option<gwt_core::workspace_projection::WorkspaceStatusCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkspaceUpdateRequest {
    pub schema_version: u32,
    pub claimed_session_id: String,
    pub observation: AgentRuntimeObservation,
    pub intent: AgentWorkspaceUpdateIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkspaceUpdateReceipt {
    pub schema_version: u32,
    pub work_id: String,
    pub journal_entry_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkTerminalKind {
    Done,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkTerminalizationRequest {
    pub schema_version: u32,
    pub claimed_session_id: String,
    pub observation: AgentRuntimeObservation,
    pub terminal_kind: AgentWorkTerminalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkTerminalizationOutcome {
    Emitted,
    AlreadyMatching,
    WrongTerminal,
    AmbiguousTerminal,
    AssignedWorkMissing,
    NoTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkTerminalizationReceipt {
    pub schema_version: u32,
    pub outcome: AgentWorkTerminalizationOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspaceUpdateErrorCode {
    InvalidRequest,
    RelaunchRequired,
    WorkspaceEnsureRequired,
    ProvenanceMismatch,
    IdentityConflict,
    TransactionConflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkspaceUpdateError {
    pub code: AgentWorkspaceUpdateErrorCode,
    pub message: String,
}

impl AgentWorkspaceUpdateError {
    fn new(code: AgentWorkspaceUpdateErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AgentWorkspaceUpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AgentWorkspaceUpdateError {}

pub fn observe_agent_runtime(
    invocation_cwd: &Path,
) -> std::result::Result<AgentRuntimeObservation, AgentWorkspaceUpdateError> {
    let cwd = canonicalize_mutation_path(invocation_cwd, "cwd").map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update runtime cwd is unavailable or non-canonical",
        )
    })?;
    let git_toplevel = git_toplevel(&cwd, "cwd").map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update must run at a readable Git worktree",
        )
    })?;
    let repo_hash = repo_hash_for_mutation(&git_toplevel, "repo hash").map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update runtime repository identity is unavailable",
        )
    })?;
    let branch = git_branch(&git_toplevel, "cwd").map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update runtime branch identity is unavailable",
        )
    })?;
    Ok(AgentRuntimeObservation {
        cwd: cwd.to_string_lossy().into_owned(),
        git_toplevel: git_toplevel.to_string_lossy().into_owned(),
        repo_hash,
        branch: canonical_branch_identity(&branch),
    })
}

pub fn apply_authenticated_workspace_update(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    request: AgentWorkspaceUpdateRequest,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    if request.schema_version != AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "unsupported workspace update bridge schema version",
        ));
    }
    validate_mutation_session_id(authenticated_session_id)?;
    if request.claimed_session_id != authenticated_session_id {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            "workspace.update Session claim does not match the authenticated launch",
        ));
    }

    let observation = request.observation.clone();
    let target = resolve_authenticated_session_work_mutation_target(
        authenticated_project_root,
        authenticated_session_id,
        &observation,
    )?;
    let tracked_event_policy = if crate::cli::execution_state::is_completed(&target.work_event_root)
    {
        TrackedWorkEventPolicy::SkipTracked
    } else {
        TrackedWorkEventPolicy::Persist
    };
    let opens_work_settlement = tracked_event_policy == TrackedWorkEventPolicy::Persist
        && request.intent.status_category
            == Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
    let persistence_target = target.persistence_target();
    let update = WorkspaceProjectionUpdate {
        title: request.intent.title,
        status_category: request.intent.status_category,
        status_text: request.intent.status_text,
        owner: request.intent.owner,
        next_action: request.intent.next_action,
        summary: request.intent.summary,
        progress_summary: request.intent.progress_summary,
        agent_session_id: Some(target.session_id.clone()),
        agent_current_focus: request.intent.current_focus,
        agent_title_summary: request.intent.title_summary,
    };
    let mut revalidation_error_code = None;
    let entry = update_workspace_projection_with_journal_for_resolved_work_target(
        &persistence_target,
        update,
        tracked_event_policy,
        |_, _| {
            let refreshed = resolve_authenticated_session_work_mutation_target(
                authenticated_project_root,
                authenticated_session_id,
                &observation,
            )
            .map_err(|error| {
                revalidation_error_code = Some(error.code);
                GwtError::Other(
                    "authenticated Session-bound workspace target revalidation failed".to_string(),
                )
            })?;
            if refreshed != target {
                return Err(GwtError::Other(
                    "authenticated Session-bound workspace target changed before commit"
                        .to_string(),
                ));
            }
            Ok(())
        },
    )
    .map_err(|error| {
        revalidation_error_code.map_or_else(
            || classify_workspace_transaction_error(&error),
            workspace_revalidation_error,
        )
    })?;
    if opens_work_settlement {
        crate::cli::verification_record::save_work_event_settlement_record(
            &target.work_event_root,
            &target.session_id,
            true,
        )
        .map_err(|_| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host persisted the terminal Work event but could not open its settlement obligation",
            )
        })?;
    }
    Ok(AgentWorkspaceUpdateReceipt {
        schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
        work_id: target.work_id,
        journal_entry_id: entry.id,
    })
}

pub fn apply_authenticated_work_terminalization(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    request: AgentWorkTerminalizationRequest,
) -> std::result::Result<AgentWorkTerminalizationReceipt, AgentWorkspaceUpdateError> {
    apply_authenticated_work_terminalization_inner(
        authenticated_project_root,
        authenticated_session_id,
        request,
        |_| {},
    )
}

fn apply_authenticated_work_terminalization_inner(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    request: AgentWorkTerminalizationRequest,
    after_resolve: impl FnOnce(&SessionBoundWorkspaceTerminalTarget),
) -> std::result::Result<AgentWorkTerminalizationReceipt, AgentWorkspaceUpdateError> {
    if request.schema_version != AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "unsupported Work terminalization bridge schema version",
        ));
    }
    validate_mutation_session_id(authenticated_session_id)?;
    if request.claimed_session_id != authenticated_session_id {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            "Work terminalization Session claim does not match the authenticated launch",
        ));
    }

    let target = resolve_authenticated_session_terminal_target(
        authenticated_project_root,
        authenticated_session_id,
        &request.observation,
    )?;
    after_resolve(&target);
    let close_kind = match request.terminal_kind {
        AgentWorkTerminalKind::Done => gwt_core::workspace_projection::WorkCloseKind::Done,
        AgentWorkTerminalKind::Discarded => {
            gwt_core::workspace_projection::WorkCloseKind::Discarded
        }
    };
    let observation = request.observation;
    let mut revalidation_error_code = None;
    let outcome =
        gwt_core::workspace_projection::emit_workspace_terminal_event_for_resolved_work_target(
            &target,
            close_kind,
            Utc::now(),
            |_, _| {
                let refreshed = resolve_authenticated_session_terminal_target(
                    authenticated_project_root,
                    authenticated_session_id,
                    &observation,
                )
                .map_err(|error| {
                    revalidation_error_code = Some(error.code);
                    GwtError::Other(
                        "authenticated Session-bound Work terminalization revalidation failed"
                            .to_string(),
                    )
                })?;
                if refreshed != target {
                    revalidation_error_code =
                        Some(AgentWorkspaceUpdateErrorCode::TransactionConflict);
                    return Err(GwtError::Other(
                    "authenticated Session-bound Work terminalization target changed before commit"
                        .to_string(),
                ));
                }
                Ok(())
            },
        )
        .map_err(|error| {
            revalidation_error_code.map_or_else(
                || classify_workspace_transaction_error(&error),
                workspace_revalidation_error,
            )
        })?;

    let outcome = match outcome {
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::Emitted => {
            AgentWorkTerminalizationOutcome::Emitted
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AlreadyMatching => {
            AgentWorkTerminalizationOutcome::AlreadyMatching
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::WrongTerminal => {
            AgentWorkTerminalizationOutcome::WrongTerminal
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AmbiguousTerminal => {
            AgentWorkTerminalizationOutcome::AmbiguousTerminal
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AssignedWorkMissing(_) => {
            AgentWorkTerminalizationOutcome::AssignedWorkMissing
        }
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::NoTarget => {
            AgentWorkTerminalizationOutcome::NoTarget
        }
    };
    Ok(AgentWorkTerminalizationReceipt {
        schema_version: AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
        outcome,
    })
}

fn resolve_authenticated_session_work_mutation_target(
    authenticated_project_root: &Path,
    session_id: &str,
    observation: &AgentRuntimeObservation,
) -> std::result::Result<SessionWorkMutationTarget, AgentWorkspaceUpdateError> {
    let authority = resolve_authenticated_session_terminal_target(
        authenticated_project_root,
        session_id,
        observation,
    )?;
    let current_path =
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&authority.project_state_root);
    let work_id = resolve_unique_existing_work_id(
        &current_path,
        &authority.work_event_root,
        session_id,
        &authority.branch_identity,
        &authority.worktree_identity,
        false,
    )
    .map_err(classify_target_error)?;
    Ok(SessionWorkMutationTarget {
        project_state_root: authority.project_state_root,
        work_event_root: authority.work_event_root,
        session_id: authority.session_id,
        branch_identity: authority.branch_identity,
        worktree_identity: authority.worktree_identity,
        work_id,
    })
}

fn resolve_authenticated_session_terminal_target(
    authenticated_project_root: &Path,
    session_id: &str,
    observation: &AgentRuntimeObservation,
) -> std::result::Result<SessionBoundWorkspaceTerminalTarget, AgentWorkspaceUpdateError> {
    let session = load_session_for_mutation(session_id).map_err(classify_target_error)?;
    if session.id != session_id {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            "workspace.update Session ledger does not match the authenticated launch",
        ));
    }

    let principal_root =
        canonicalize_mutation_path(authenticated_project_root, "authenticated project")
            .map_err(|_| relaunch_required_error())?;
    let configured_project_root =
        strict_project_state_root(&session).map_err(classify_target_error)?;
    let project_state_root =
        canonicalize_mutation_path(&configured_project_root, "canonical repository")
            .map_err(|_| relaunch_required_error())?;
    if principal_root != project_state_root {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            "workspace.update project does not match the authenticated launch",
        ));
    }

    let session_worktree = canonicalize_mutation_path(&session.worktree_path, "worktree")
        .map_err(|_| relaunch_required_error())?;
    let session_git_root =
        git_toplevel(&session_worktree, "worktree").map_err(|_| relaunch_required_error())?;
    if session_worktree != session_git_root {
        return Err(relaunch_required_error());
    }
    let declared_repo_hash = required_session_repo_hash(&session)
        .map_err(classify_target_error)?
        .to_string();
    let branch_identity = required_session_branch(&session).map_err(classify_target_error)?;
    validate_runtime_repo_and_branch(
        &session_git_root,
        &declared_repo_hash,
        &branch_identity,
        &session,
    )
    .map_err(|_| relaunch_required_error())?;
    validate_visible_project_state_root(&project_state_root, &declared_repo_hash, session_id)
        .map_err(|_| relaunch_required_error())?;

    if observation.repo_hash != declared_repo_hash
        || canonical_branch_identity(&observation.branch) != branch_identity
    {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            "workspace.update runtime repository or branch does not match the authenticated Session",
        ));
    }
    match session.runtime_target {
        LaunchRuntimeTarget::Docker => {
            validate_docker_runtime_observation(&session, observation, &project_state_root)?;
        }
        LaunchRuntimeTarget::Host => {
            let observed_cwd =
                canonicalize_mutation_path(Path::new(&observation.cwd), "observed cwd")
                    .map_err(|_| provenance_mismatch_error())?;
            let observed_git_root = canonicalize_mutation_path(
                Path::new(&observation.git_toplevel),
                "observed Git root",
            )
            .map_err(|_| provenance_mismatch_error())?;
            if observed_cwd != session_worktree || observed_git_root != session_worktree {
                return Err(provenance_mismatch_error());
            }
        }
    }

    Ok(SessionBoundWorkspaceTerminalTarget {
        project_state_root,
        work_event_root: session_worktree.clone(),
        session_id: session.id,
        branch_identity,
        worktree_identity: session_worktree,
    })
}

fn validate_docker_runtime_observation(
    session: &Session,
    observation: &AgentRuntimeObservation,
    project_state_root: &Path,
) -> std::result::Result<(), AgentWorkspaceUpdateError> {
    let binding = session
        .docker_runtime_binding
        .as_ref()
        .ok_or_else(relaunch_required_error)?;
    let bound_runtime =
        canonical_posix_runtime_path(&binding.runtime_worktree_path.to_string_lossy())?;
    let observed_cwd = canonical_posix_runtime_path(&observation.cwd)?;
    let observed_git_root = canonical_posix_runtime_path(&observation.git_toplevel)?;
    if observed_cwd != bound_runtime || observed_git_root != bound_runtime {
        return Err(provenance_mismatch_error());
    }
    let expected_scope = gwt_core::paths::project_scope_hash(project_state_root);
    if binding.project_state_scope_hash != expected_scope.as_str() {
        return Err(relaunch_required_error());
    }
    Ok(())
}

fn canonical_posix_runtime_path(
    value: &str,
) -> std::result::Result<String, AgentWorkspaceUpdateError> {
    if value.is_empty()
        || value.trim() != value
        || !value.starts_with('/')
        || value.contains('\0')
        || value.contains('\\')
        || value.ends_with('/')
        || value
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update Docker runtime path must be an absolute canonical POSIX path",
        ));
    }
    Ok(value.to_string())
}

fn validate_mutation_session_id(
    session_id: &str,
) -> std::result::Result<(), AgentWorkspaceUpdateError> {
    gwt_agent::validate_session_id_path_component(session_id).map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "workspace.update Session id is invalid or unsafe",
        )
    })
}

fn classify_target_error(error: GwtError) -> AgentWorkspaceUpdateError {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("workspace.ensure") {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired,
            "Session-bound Work target is missing or ambiguous; run workspace.ensure for this Session before retrying workspace.update",
        )
    } else if message.contains("relaunch") || message.contains("ledger") {
        relaunch_required_error()
    } else {
        provenance_mismatch_error()
    }
}

fn provenance_mismatch_error() -> AgentWorkspaceUpdateError {
    AgentWorkspaceUpdateError::new(
        AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
        "workspace.update runtime provenance does not match the authenticated launch",
    )
}

fn relaunch_required_error() -> AgentWorkspaceUpdateError {
    AgentWorkspaceUpdateError::new(
        AgentWorkspaceUpdateErrorCode::RelaunchRequired,
        "workspace.update launch binding is missing or stale; relaunch the Session before retrying",
    )
}

fn workspace_revalidation_error(code: AgentWorkspaceUpdateErrorCode) -> AgentWorkspaceUpdateError {
    match code {
        AgentWorkspaceUpdateErrorCode::RelaunchRequired => relaunch_required_error(),
        AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => AgentWorkspaceUpdateError::new(
            code,
            "Session-bound Work target changed before commit; run workspace.ensure before retrying",
        ),
        AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
        | AgentWorkspaceUpdateErrorCode::InvalidRequest => provenance_mismatch_error(),
        AgentWorkspaceUpdateErrorCode::IdentityConflict
        | AgentWorkspaceUpdateErrorCode::TransactionConflict
        | AgentWorkspaceUpdateErrorCode::Internal => AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::TransactionConflict,
            "Host workspace authority changed before commit; retry after inspecting the Host gwt log",
        ),
    }
}

fn classify_workspace_transaction_error(error: &GwtError) -> AgentWorkspaceUpdateError {
    if error
        .to_string()
        .to_ascii_lowercase()
        .contains("owner claim conflicts")
    {
        return AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::IdentityConflict,
            "workspace.update owner claim conflicts with the Session-bound Work",
        );
    }
    AgentWorkspaceUpdateError::new(
        AgentWorkspaceUpdateErrorCode::TransactionConflict,
        "Host workspace transaction failed without committing; inspect the Host gwt log before retrying",
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionWorkMutationTarget {
    pub(crate) project_state_root: PathBuf,
    pub(crate) work_event_root: PathBuf,
    pub(crate) session_id: String,
    pub(crate) branch_identity: String,
    pub(crate) worktree_identity: PathBuf,
    pub(crate) work_id: String,
}

impl SessionWorkMutationTarget {
    pub(crate) fn persistence_target(&self) -> SessionBoundWorkspaceMutationTarget {
        SessionBoundWorkspaceMutationTarget {
            project_state_root: self.project_state_root.clone(),
            work_event_root: self.work_event_root.clone(),
            session_id: self.session_id.clone(),
            branch_identity: self.branch_identity.clone(),
            worktree_identity: self.worktree_identity.clone(),
            work_id: self.work_id.clone(),
        }
    }
}

pub(crate) fn resolve_session_work_mutation_target(
    invocation_cwd: &Path,
    session_id: &str,
) -> Result<SessionWorkMutationTarget> {
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| mutation_error(format!("invalid or unsafe Session id: {error}")))?;
    let session = load_session_for_mutation(session_id)?;
    if session.id != session_id {
        return Err(mutation_error(format!(
            "Session ledger id mismatch: requested {session_id}, loaded {}",
            session.id
        )));
    }

    if session.runtime_target == LaunchRuntimeTarget::Docker {
        return Err(mutation_error(format!(
            "Docker workspace.update for Session {} requires an authenticated Host bridge; relaunch the Session",
            session.id
        )));
    }
    resolve_host_session_work_mutation_target(invocation_cwd, session)
}

fn resolve_host_session_work_mutation_target(
    invocation_cwd: &Path,
    session: Session,
) -> Result<SessionWorkMutationTarget> {
    let session_id = session.id.as_str();
    let invocation_raw = canonicalize_mutation_path(invocation_cwd, "cwd")?;
    let session_worktree = canonicalize_mutation_path(&session.worktree_path, "worktree")?;
    if invocation_raw != session_worktree {
        return Err(mutation_error(format!(
            "Session cwd mismatch for Session {session_id}: expected {}, got {}",
            session_worktree.display(),
            invocation_raw.display()
        )));
    }
    let session_git_root = git_toplevel(&session_worktree, "worktree")?;
    let declared_repo_hash = required_session_repo_hash(&session)?;
    let observed = repo_hash_for_mutation(&session_git_root, "repo hash")?;
    if observed != declared_repo_hash {
        return Err(mutation_error(format!(
            "Session repo hash mismatch for Session {session_id}: ledger={declared_repo_hash}, worktree={observed}"
        )));
    }

    let configured_project_state_root = strict_project_state_root(&session)?;
    let project_state_root =
        canonicalize_mutation_path(&configured_project_state_root, "canonical repository")?;
    let project_anchor =
        validate_visible_project_state_root(&project_state_root, declared_repo_hash, session_id)?;

    let branch_identity = required_session_branch(&session)?;
    let session_branch = git_branch(&session_git_root, "worktree")?;
    if canonical_branch_identity(&session_branch) != branch_identity {
        return Err(mutation_error(format!(
            "Session branch mismatch for Session {session_id}: ledger={}, worktree={session_branch}",
            session.branch
        )));
    }
    let session_anchor = canonical_repository_anchor(&session_git_root).map_err(|error| {
        mutation_error(format!(
            "Session worktree mismatch for Session {session_id}: {error}"
        ))
    })?;
    if session_anchor != project_anchor {
        return Err(mutation_error(format!(
            "Session worktree mismatch for Session {session_id}: {} does not belong to canonical repository {}",
            session_git_root.display(),
            project_anchor.display()
        )));
    }

    let invocation_git_root = git_toplevel(&invocation_raw, "cwd")?;
    validate_runtime_repo_and_branch(
        &invocation_git_root,
        declared_repo_hash,
        &branch_identity,
        &session,
    )?;
    if session_worktree != session_git_root {
        return Err(mutation_error(format!(
            "Session event root mismatch for Session {session_id}: workspace.update must run at the validated Git toplevel"
        )));
    }

    let current_path =
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_state_root);
    let work_id = resolve_unique_existing_work_id(
        &current_path,
        &invocation_git_root,
        session_id,
        &branch_identity,
        &session_worktree,
        false,
    )?;

    Ok(SessionWorkMutationTarget {
        project_state_root,
        work_event_root: invocation_git_root,
        session_id: session.id,
        branch_identity,
        worktree_identity: session_worktree,
        work_id,
    })
}

fn required_session_repo_hash(session: &Session) -> Result<&str> {
    session
        .repo_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            mutation_error(format!(
                "Session repo hash is missing for Session {}; relaunch the Session",
                session.id
            ))
        })
}

fn required_session_branch(session: &Session) -> Result<String> {
    let branch_identity = canonical_branch_identity(&session.branch);
    if branch_identity.is_empty() {
        return Err(mutation_error(format!(
            "Session branch mismatch for Session {}: ledger branch is empty",
            session.id
        )));
    }
    Ok(branch_identity)
}

fn validate_runtime_repo_and_branch(
    git_root: &Path,
    declared_repo_hash: &str,
    branch_identity: &str,
    session: &Session,
) -> Result<()> {
    let observed_repo_hash = repo_hash_for_mutation(git_root, "repo hash")?;
    if observed_repo_hash != declared_repo_hash {
        return Err(mutation_error(format!(
            "Session repo hash mismatch for Session {}: ledger={declared_repo_hash}, runtime={observed_repo_hash}",
            session.id
        )));
    }
    let observed_branch = git_branch(git_root, "runtime")?;
    if canonical_branch_identity(&observed_branch) != branch_identity {
        return Err(mutation_error(format!(
            "Session branch mismatch for Session {}: ledger={}, runtime={observed_branch}",
            session.id, session.branch
        )));
    }
    Ok(())
}

fn validate_visible_project_state_root(
    project_state_root: &Path,
    declared_repo_hash: &str,
    session_id: &str,
) -> Result<PathBuf> {
    let project_anchor = canonical_repository_anchor(project_state_root).map_err(|error| {
        mutation_error(format!(
            "canonical repository mismatch for Session {session_id}: {error}"
        ))
    })?;
    let project_repo_hash = repo_hash_for_mutation(&project_anchor, "canonical repository")
        .map_err(|error| {
            mutation_error(format!(
                "canonical repository mismatch for Session {session_id}: {error}"
            ))
        })?;
    if project_repo_hash != declared_repo_hash {
        return Err(mutation_error(format!(
            "canonical repository mismatch for Session {session_id}: expected repo hash {declared_repo_hash}, got {project_repo_hash}"
        )));
    }
    validate_project_state_anchor(project_state_root, &project_anchor, session_id)?;
    Ok(project_anchor)
}

fn mutation_error(message: impl Into<String>) -> GwtError {
    GwtError::Other(message.into())
}

fn load_session_for_mutation(session_id: &str) -> Result<Session> {
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    if !path.try_exists().map_err(|error| {
        mutation_error(format!(
            "failed to inspect Session ledger for Session {session_id} at {}: {error}",
            path.display()
        ))
    })? {
        return Err(mutation_error(format!(
            "Session ledger is missing for Session {session_id} at {}",
            path.display()
        )));
    }
    Session::load(&path).map_err(|_| {
        mutation_error(format!(
            "invalid, corrupt, or unreadable Session ledger for Session {session_id} at {}",
            path.display()
        ))
    })
}

fn normalize_mutation_path(path: &Path) -> PathBuf {
    let path = normalize_windows_child_process_path(path);
    let path = dunce::canonicalize(&path).unwrap_or(path);
    normalize_windows_child_process_path(&path)
}

fn canonicalize_mutation_path(path: &Path, identity: &str) -> Result<PathBuf> {
    let normalized = normalize_windows_child_process_path(path);
    let canonical = dunce::canonicalize(&normalized).map_err(|error| {
        mutation_error(format!(
            "Session {identity} mismatch: cannot canonicalize {}: {error}",
            normalized.display()
        ))
    })?;
    Ok(normalize_windows_child_process_path(&canonical))
}

fn git_toplevel(path: &Path, identity: &str) -> Result<PathBuf> {
    let output = gwt_core::process::run_git_logged(&["rev-parse", "--show-toplevel"], Some(path))
        .map_err(|error| {
        mutation_error(format!(
            "Session {identity} mismatch: git rev-parse failed at {}: {error}",
            path.display()
        ))
    })?;
    if !output.status.success() {
        return Err(mutation_error(format!(
            "Session {identity} mismatch: {} is not a Git worktree: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    canonicalize_mutation_path(&root, identity)
}

fn git_branch(path: &Path, identity: &str) -> Result<String> {
    let output = gwt_core::process::run_git_logged(
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        Some(path),
    )
    .map_err(|error| {
        mutation_error(format!(
            "Session branch mismatch: git symbolic-ref failed for {identity} {}: {error}",
            path.display()
        ))
    })?;
    if !output.status.success() {
        return Err(mutation_error(format!(
            "Session branch mismatch: {identity} {} has no attached branch",
            path.display()
        )));
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        return Err(mutation_error(format!(
            "Session branch mismatch: {identity} {} returned an empty branch",
            path.display()
        )));
    }
    Ok(branch)
}

fn repo_hash_for_mutation(path: &Path, identity: &str) -> Result<String> {
    gwt_core::repo_hash::detect_repo_hash(path)
        .map(|hash| hash.as_str().to_string())
        .ok_or_else(|| {
            mutation_error(format!(
                "Session {identity} mismatch: origin repo hash is unavailable at {}",
                path.display()
            ))
        })
}

fn strict_project_state_root(session: &Session) -> Result<PathBuf> {
    if let Some(root) = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
    {
        return Ok(root.to_path_buf());
    }
    derive_legacy_project_state_root(&session.worktree_path).ok_or_else(|| {
        mutation_error(format!(
            "canonical repository mismatch for Session {}: project_state_root is missing and the legacy root cannot be derived",
            session.id
        ))
    })
}

fn canonical_repository_anchor(path: &Path) -> Result<PathBuf> {
    let anchor = gwt_git::worktree::main_worktree_root(path)
        .map_err(|error| mutation_error(error.to_string()))?;
    canonicalize_mutation_path(&anchor, "canonical repository")
}

fn validate_project_state_anchor(
    project_state_root: &Path,
    project_anchor: &Path,
    session_id: &str,
) -> Result<()> {
    if project_state_root == project_anchor {
        return Ok(());
    }
    let is_workspace_home = is_bare_child_common_dir(project_anchor)
        && project_anchor.parent() == Some(project_state_root);
    if is_workspace_home {
        return Ok(());
    }
    Err(mutation_error(format!(
        "canonical repository mismatch for Session {session_id}: Project State root {} is neither the repository anchor nor the parent of its bare common-dir {}",
        project_state_root.display(),
        project_anchor.display()
    )))
}

fn canonical_branch_identity(branch: &str) -> String {
    let branch = branch.trim();
    let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
    let branch = branch.strip_prefix("refs/remotes/").unwrap_or(branch);
    branch.strip_prefix("origin/").unwrap_or(branch).to_string()
}

fn resolve_unique_existing_work_id(
    current_path: &Path,
    work_event_root: &Path,
    session_id: &str,
    branch_identity: &str,
    worktree_identity: &Path,
    docker: bool,
) -> Result<String> {
    let projection = load_workspace_projection_from_path(current_path)
        .map_err(|error| {
            workspace_ensure_error(
                session_id,
                &format!("canonical Session assignment cannot be read: {error}"),
            )
        })?
        .ok_or_else(|| {
            workspace_ensure_error(session_id, "canonical Session assignment is missing")
        })?;
    let agent = projection
        .latest_agent_for_session(session_id)
        .ok_or_else(|| {
            workspace_ensure_error(session_id, "canonical Session assignment is missing")
        })?;
    if !agent.is_assigned() {
        return Err(workspace_ensure_error(
            session_id,
            "latest canonical Session assignment is Unassigned",
        ));
    }
    let work_id = agent
        .workspace_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            workspace_ensure_error(
                session_id,
                "latest canonical Session assignment has no Work id",
            )
        })?
        .to_string();

    let assigned_branch = agent
        .branch
        .as_deref()
        .map(canonical_branch_identity)
        .filter(|branch| !branch.is_empty());
    let assigned_worktree = agent.worktree_path.as_deref().map(normalize_mutation_path);
    if assigned_branch.as_deref() != Some(branch_identity)
        || assigned_worktree.as_deref() != Some(worktree_identity)
    {
        return Err(workspace_ensure_error(
            session_id,
            "canonical Session assignment container does not match the validated branch/worktree",
        ));
    }

    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let work_items = load_workspace_work_items_from_path(&work_items_path)
        .map_err(|error| {
            workspace_ensure_error(
                session_id,
                &format!("assigned WorkItems projection cannot be read: {error}"),
            )
        })?
        .ok_or_else(|| {
            workspace_ensure_error(session_id, "assigned WorkItems projection is missing")
        })?;
    let matches = work_items
        .work_items
        .iter()
        .filter(|item| item.id == work_id)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} is missing"),
        ));
    }
    if matches.len() > 1 {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} is ambiguous"),
        ));
    }
    let item = matches[0];
    if item.is_terminal() {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} is terminal"),
        ));
    }
    let matching_containers = item
        .execution_containers
        .iter()
        .filter(|container| {
            mutation_container_matches(
                container,
                branch_identity,
                worktree_identity,
                work_event_root,
                docker,
            )
        })
        .count();
    if matching_containers == 0 {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} has no matching execution container"),
        ));
    }
    if matching_containers > 1 {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} has ambiguous matching execution containers"),
        ));
    }
    let competing_work = work_items.work_items.iter().find(|other| {
        other.id != work_id
            && !other.is_terminal()
            && other
                .agents
                .iter()
                .any(|agent| agent.session_id == session_id)
    });
    if let Some(competing_work) = competing_work {
        return Err(workspace_ensure_error(
            session_id,
            &format!(
                "assigned Work {work_id} is ambiguous with active Work {} for the same Session",
                competing_work.id
            ),
        ));
    }
    Ok(work_id)
}

fn mutation_container_matches(
    container: &gwt_core::workspace_projection::WorkspaceExecutionContainerRef,
    branch_identity: &str,
    worktree_identity: &Path,
    work_event_root: &Path,
    docker: bool,
) -> bool {
    let branch_matches = container
        .branch
        .as_deref()
        .map(canonical_branch_identity)
        .as_deref()
        == Some(branch_identity);
    let worktree_matches = container
        .worktree_path
        .as_deref()
        .map(normalize_mutation_path)
        .is_some_and(|path| path == worktree_identity || docker && path == work_event_root);
    branch_matches && worktree_matches
}

fn workspace_ensure_error(session_id: &str, reason: &str) -> GwtError {
    mutation_error(format!(
        "Session-bound Work target for Session {session_id} is invalid: {reason}; run workspace.ensure for this Session before retrying workspace.update"
    ))
}

#[allow(dead_code)] // Legacy non-mutation callers may still use fail-open root lookup.
pub(crate) fn project_state_root_for_agent_session_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> PathBuf {
    load_session(session_id)
        .map(|session| canonical_project_state_root_for_session(&session, fallback_repo_path))
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

#[allow(dead_code)] // Legacy non-mutation callers may still use fail-open root lookup.
pub(crate) fn work_event_root_for_agent_session_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> PathBuf {
    load_session(session_id)
        .map(|session| normalize_project_state_root(&session.worktree_path))
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

pub(crate) fn agent_session_roots_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> std::io::Result<(PathBuf, PathBuf)> {
    let Some(session) = try_load_session(session_id)? else {
        let fallback = normalize_project_state_root(fallback_repo_path);
        return Ok((fallback.clone(), fallback));
    };
    Ok((
        canonical_project_state_root_for_session(&session, fallback_repo_path),
        normalize_project_state_root(&session.worktree_path),
    ))
}

pub(crate) fn canonical_project_state_root_for_session(
    session: &Session,
    fallback_repo_path: &Path,
) -> PathBuf {
    if let Some(root) = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
    {
        return normalize_project_state_root(root);
    }

    derive_legacy_project_state_root(&session.worktree_path)
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

pub(crate) fn repair_split_agent_state_if_needed(
    canonical_root: &Path,
    split_root: &Path,
    session_id: &str,
) -> Result<bool> {
    let canonical_root = normalize_project_state_root(canonical_root);
    let split_root = normalize_project_state_root(split_root);
    if canonical_root == split_root {
        return Ok(false);
    }

    let Some(split_projection) = load_workspace_projection(&split_root)? else {
        return Ok(false);
    };
    let Some(split_agent) = split_projection
        .latest_agent_for_session(session_id)
        .cloned()
    else {
        return Ok(false);
    };

    mutate_existing_workspace_projection(&canonical_root, |canonical_projection| {
        let projection_updated_at = canonical_projection.updated_at;
        let Some(canonical_agent) = canonical_projection.latest_agent_for_session_mut(session_id)
        else {
            return Ok(false);
        };
        let agent_updated_at = canonical_agent.updated_at;
        let changed = repair_agent_from_split(canonical_agent, &split_agent);
        if changed {
            let repaired_floor = Utc::now()
                .max(projection_updated_at)
                .max(agent_updated_at)
                .max(split_agent.updated_at);
            let repaired_at = repaired_floor
                .checked_add_signed(chrono::Duration::nanoseconds(1))
                .ok_or_else(|| {
                    gwt_core::GwtError::Other(
                        "split Agent repair timestamp exceeds the supported range".to_string(),
                    )
                })?;
            canonical_agent.updated_at = repaired_at;
            canonical_projection.updated_at = repaired_at;
        }
        Ok(changed)
    })
    .map(Option::unwrap_or_default)
}

#[allow(dead_code)] // Shared by the retained legacy fail-open root helpers.
fn load_session(session_id: &str) -> Option<Session> {
    match try_load_session(session_id) {
        Ok(session) => session,
        Err(error) => {
            let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
            tracing::debug!(
                error = %error,
                session_id,
                path = %path.display(),
                "failed to load agent session for Project State root resolution"
            );
            None
        }
    }
}

fn try_load_session(session_id: &str) -> std::io::Result<Option<Session>> {
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    if !path.try_exists()? {
        return Ok(None);
    }
    Session::load(&path).map(Some)
}

fn derive_legacy_project_state_root(worktree_path: &Path) -> Option<PathBuf> {
    let worktree_path = normalize_project_state_root(worktree_path);
    let main_root = gwt_git::worktree::main_worktree_root(&worktree_path).ok()?;
    let main_root = normalize_project_state_root(&main_root);

    if is_bare_child_common_dir(&main_root) {
        if let Some(parent) = main_root.parent() {
            let parent = normalize_project_state_root(parent);
            if worktree_path.starts_with(&parent) {
                return Some(parent);
            }
        }
    }

    Some(main_root)
}

fn is_bare_child_common_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name != ".git" && name.ends_with(".git"))
}

fn normalize_project_state_root(path: &Path) -> PathBuf {
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_windows_child_process_path(&path)
}

fn repair_agent_from_split(
    canonical: &mut WorkspaceAgentSummary,
    split: &WorkspaceAgentSummary,
) -> bool {
    let mut changed = false;
    let split_is_newer = split.updated_at > canonical.updated_at;
    changed |= fill_option_text_if_missing_or_newer(
        &mut canonical.title_summary,
        split.title_summary.as_deref(),
        split_is_newer,
    );
    changed |= fill_option_text_if_missing_or_newer(
        &mut canonical.current_focus,
        split.current_focus.as_deref(),
        split_is_newer,
    );
    changed |= fill_option_path(&mut canonical.worktree_path, split.worktree_path.as_deref());
    changed |= fill_option_text(&mut canonical.window_id, split.window_id.as_deref());
    changed |= fill_option_text(&mut canonical.branch, split.branch.as_deref());

    if canonical.agent_id.trim().is_empty() && !split.agent_id.trim().is_empty() {
        canonical.agent_id = split.agent_id.clone();
        changed = true;
    }
    if canonical.display_name.trim().is_empty() && !split.display_name.trim().is_empty() {
        canonical.display_name = split.display_name.clone();
        changed = true;
    }
    changed
}

fn fill_option_text_if_missing_or_newer(
    target: &mut Option<String>,
    source: Option<&str>,
    source_is_newer: bool,
) -> bool {
    let Some(source) = source.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let target_has_value = target
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if target_has_value && !source_is_newer {
        return false;
    }
    if target.as_deref().map(str::trim) == Some(source) {
        return false;
    }
    *target = Some(source.to_string());
    true
}

fn fill_option_text(target: &mut Option<String>, source: Option<&str>) -> bool {
    if target
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let Some(source) = source.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    *target = Some(source.to_string());
    true
}

fn fill_option_path(target: &mut Option<PathBuf>, source: Option<&Path>) -> bool {
    if target
        .as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty())
    {
        return false;
    }
    let Some(source) = source.filter(|path| !path.as_os_str().is_empty()) else {
        return false;
    };
    *target = Some(source.to_path_buf());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_summary(
        session_id: &str,
        title_summary: Option<&str>,
        current_focus: Option<&str>,
        updated_at: chrono::DateTime<Utc>,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: Some("project::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: current_focus.map(str::to_string),
            title_summary: title_summary.map(str::to_string),
            worktree_path: Some(PathBuf::from("/tmp/worktree")),
            branch: Some("work/title".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at,
        }
    }

    fn run_git(args: &[&str], cwd: &Path) {
        let output = gwt_core::process::run_git_logged(args, Some(cwd)).expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repo(root: &Path, name: &str, remote: &str, branch: &str) -> PathBuf {
        let repo = root.join(name);
        std::fs::create_dir_all(&repo).expect("create git fixture");
        run_git(&["init"], &repo);
        run_git(&["config", "user.email", "test@example.com"], &repo);
        run_git(&["config", "user.name", "Test User"], &repo);
        run_git(&["checkout", "-b", branch], &repo);
        run_git(&["remote", "add", "origin", remote], &repo);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &repo);
        dunce::canonicalize(repo).expect("canonical git fixture")
    }

    fn session_fixture(id: &str, repo: &Path, branch: &str) -> Session {
        let mut session = Session::new(repo, branch, gwt_agent::AgentId::Codex);
        session.id = id.to_string();
        session.project_state_root = Some(repo.to_path_buf());
        session
    }

    fn save_session_fixture(session: &Session) {
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Session ledger fixture");
    }

    fn assigned_session_agent(
        session: &Session,
        work_id: &str,
        updated_at: chrono::DateTime<Utc>,
    ) -> WorkspaceAgentSummary {
        let mut agent = agent_summary(&session.id, None, None, updated_at);
        agent.worktree_path = Some(session.worktree_path.clone());
        agent.branch = Some(session.branch.clone());
        agent.workspace_id = Some(work_id.to_string());
        agent
    }

    fn save_project_assignments(project_state_root: &Path, agents: Vec<WorkspaceAgentSummary>) {
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                project_state_root,
            );
        projection.agents = agents;
        gwt_core::workspace_projection::save_workspace_projection(project_state_root, &projection)
            .expect("save canonical Session assignments");
    }

    fn mutation_work_items(
        work_event_root: &Path,
        session: &Session,
        work_id: &str,
    ) -> gwt_core::workspace_projection::WorkItemsProjection {
        let now = Utc::now();
        let mut projection = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id,
            now,
        );
        event.title = Some("Session-bound Work".to_string());
        event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        event.agent_session_id = Some(session.id.clone());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(session.branch.clone()),
                worktree_path: Some(work_event_root.to_path_buf()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        projection.apply_event(event);
        projection
    }

    fn save_mutation_work_items(
        work_event_root: &Path,
        projection: &gwt_core::workspace_projection::WorkItemsProjection,
    ) {
        let path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &path, projection,
        )
        .expect("save WorkItems projection");
    }

    fn save_mutation_work_items_with_tracked_events(
        work_event_root: &Path,
        projection: &gwt_core::workspace_projection::WorkItemsProjection,
    ) {
        save_mutation_work_items(work_event_root, projection);
        let events_path = gwt_core::paths::gwt_repo_local_work_events_path(work_event_root);
        for event in projection
            .work_items
            .iter()
            .flat_map(|item| item.events.iter())
        {
            gwt_core::workspace_projection::append_workspace_work_event_to_path(
                &events_path,
                event,
            )
            .expect("seed tracked Work event");
        }
    }

    fn seed_unique_mutation_target(
        project_state_root: &Path,
        work_event_root: &Path,
        session: &Session,
        work_id: &str,
    ) {
        save_project_assignments(
            project_state_root,
            vec![assigned_session_agent(session, work_id, Utc::now())],
        );
        save_mutation_work_items_with_tracked_events(
            work_event_root,
            &mutation_work_items(work_event_root, session, work_id),
        );
    }

    fn save_completed_execution_fixture(worktree: &Path, session_id: &str) {
        let now = Utc::now();
        crate::cli::execution_state::save(
            worktree,
            &crate::cli::execution_state::ExecutionControlRecord {
                owner_kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                owner_number: 3278,
                primary_session_id: session_id.to_string(),
                entrypoint: "launch".to_string(),
                bundled_required_owners: Vec::new(),
                status: crate::cli::execution_state::ExecutionControlStatus::Completed,
                blocked_reason: None,
                missing_verification: None,
                launched_at: now,
                settled_at: Some(now),
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
            },
        )
        .expect("save completed execution fixture");
    }

    #[derive(Debug)]
    enum SessionLedgerFixture {
        Missing { session_id: String },
        Corrupt { session_id: String },
        Persisted(Box<Session>),
    }

    impl SessionLedgerFixture {
        fn session_id(&self) -> &str {
            match self {
                Self::Missing { session_id } | Self::Corrupt { session_id } => session_id,
                Self::Persisted(session) => &session.id,
            }
        }

        fn install(&self) {
            match self {
                Self::Missing { session_id } => {
                    let ledger_path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    assert!(
                        !ledger_path.exists(),
                        "missing-ledger fixture unexpectedly exists: {}",
                        ledger_path.display()
                    );
                }
                Self::Corrupt { session_id } => {
                    let ledger_path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    std::fs::create_dir_all(ledger_path.parent().expect("Session ledger parent"))
                        .expect("create sessions dir");
                    std::fs::write(&ledger_path, "broken = [")
                        .expect("write corrupt ledger fixture");
                }
                Self::Persisted(session) => save_session_fixture(session),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct WorkMutationSnapshot {
        current: Vec<u8>,
        journal: Vec<u8>,
        works: Vec<u8>,
        tracked_events: Vec<u8>,
    }

    impl WorkMutationSnapshot {
        fn capture(project_state_root: &Path, work_event_root: &Path) -> Self {
            Self {
                current: std::fs::read(
                    gwt_core::paths::gwt_workspace_projection_path_for_repo_path(
                        project_state_root,
                    ),
                )
                .expect("read current projection snapshot"),
                journal: std::fs::read(gwt_core::paths::gwt_workspace_journal_path_for_repo_path(
                    project_state_root,
                ))
                .expect("read journal snapshot"),
                works: std::fs::read(
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root),
                )
                .expect("read Work projection snapshot"),
                tracked_events: std::fs::read(gwt_core::paths::gwt_repo_local_work_events_path(
                    work_event_root,
                ))
                .expect("read tracked Work events snapshot"),
            }
        }

        fn changed_surfaces(&self, after: &Self) -> Vec<&'static str> {
            let mut changed = Vec::new();
            if self.current != after.current {
                changed.push("current");
            }
            if self.journal != after.journal {
                changed.push("journal");
            }
            if self.works != after.works {
                changed.push("works");
            }
            if self.tracked_events != after.tracked_events {
                changed.push("tracked events");
            }
            changed
        }
    }

    fn seed_work_mutation_surfaces(project_state_root: &Path, work_event_root: &Path) {
        gwt_core::workspace_projection::update_workspace_projection_with_journal_for_work_event_root(
            project_state_root,
            work_event_root,
            gwt_core::workspace_projection::WorkspaceProjectionUpdate {
                title: Some("Baseline Work".to_string()),
                status_category: Some(
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
                status_text: None,
                owner: Some("baseline-owner".to_string()),
                next_action: None,
                summary: Some("baseline state".to_string()),
                progress_summary: None,
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
            gwt_core::workspace_projection::TrackedWorkEventPolicy::Persist,
        )
        .expect("seed Work mutation surfaces");
    }

    struct RejectedWorkspaceMutationCase {
        label: &'static str,
        expected_error: &'static str,
        ledger: SessionLedgerFixture,
        invocation_cwd: PathBuf,
        project_state_root: PathBuf,
        work_event_root: PathBuf,
    }

    fn init_case_repo(root: &Path, label: &str, branch: &str) -> (PathBuf, String) {
        let remote = format!("https://example.invalid/acme/session-bound-{label}.git");
        let repo = init_git_repo(root, &format!("{label}-repo"), &remote, branch);
        (repo, remote)
    }

    fn json_value_contains(value: &serde_json::Value, needle: &str) -> bool {
        match value {
            serde_json::Value::String(value) => value.contains(needle),
            serde_json::Value::Array(values) => values
                .iter()
                .any(|value| json_value_contains(value, needle)),
            serde_json::Value::Object(values) => values
                .iter()
                .any(|(key, value)| key.contains(needle) || json_value_contains(value, needle)),
            _ => false,
        }
    }

    fn assert_workspace_ensure_error(error: gwt_core::GwtError, expected: &str) {
        let message = error.to_string();
        assert!(
            message.contains("workspace.ensure"),
            "target-resolution error must provide the recovery operation: {message}"
        );
        assert!(
            message.to_ascii_lowercase().contains(expected),
            "target-resolution error must identify {expected}: {message}"
        );
    }

    fn with_strict_target_fixture(test: impl FnOnce(&Path, &Session)) {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-target";
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/strict-target.git",
            branch,
        );
        let session = session_fixture("strict-target-session", &repo, branch);
        save_session_fixture(&session);
        test(&repo, &session);
    }

    #[test]
    fn authenticated_workspace_update_uses_principal_and_returns_target_receipt() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-authenticated-target";
            seed_unique_mutation_target(repo, repo, session, work_id);
            let current_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo);
            let mut current = load_workspace_projection_from_path(&current_path)
                .expect("load current")
                .expect("current projection");
            current.id = work_id.to_string();
            gwt_core::workspace_projection::save_workspace_projection_to_path(
                &current_path,
                &current,
            )
            .expect("align current Work identity");

            let receipt = apply_authenticated_workspace_update(
                repo,
                &session.id,
                AgentWorkspaceUpdateRequest {
                    schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                    claimed_session_id: session.id.clone(),
                    observation: observe_agent_runtime(repo).expect("runtime observation"),
                    intent: AgentWorkspaceUpdateIntent {
                        summary: Some("authenticated host mutation".to_string()),
                        current_focus: Some("host authority".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
            )
            .expect("authenticated update");

            assert_eq!(receipt.work_id, work_id);
            assert!(!receipt.journal_entry_id.is_empty());
            let saved = load_workspace_projection_from_path(&current_path)
                .expect("load updated current")
                .expect("updated current");
            assert_eq!(
                saved.summary.as_deref(),
                Some("authenticated host mutation")
            );
            assert_eq!(
                saved
                    .latest_agent_for_session(&session.id)
                    .and_then(|agent| agent.current_focus.as_deref()),
                Some("host authority")
            );
        });
    }

    #[test]
    fn authenticated_terminal_workspace_update_opens_host_settlement_obligation() {
        with_strict_target_fixture(|repo, session| {
            seed_unique_mutation_target(repo, repo, session, "work-authenticated-terminal");

            apply_authenticated_workspace_update(
                repo,
                &session.id,
                AgentWorkspaceUpdateRequest {
                    schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                    claimed_session_id: session.id.clone(),
                    observation: observe_agent_runtime(repo).expect("runtime observation"),
                    intent: AgentWorkspaceUpdateIntent {
                        status_category: Some(
                            gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
                        ),
                        summary: Some("final Host-authoritative summary".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
            )
            .expect("authenticated terminal update");

            let record = crate::cli::verification_record::load_work_event_settlement_record(repo)
                .expect("load Host settlement obligation")
                .expect("Host terminal update must create a settlement obligation");
            assert!(record.obligation_open);
            assert_eq!(record.session_id, session.id);
        });
    }

    #[test]
    fn authenticated_post_completion_update_skips_tracked_event_and_settlement() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-authenticated-completed";
            seed_unique_mutation_target(repo, repo, session, work_id);
            let events_path = gwt_core::paths::gwt_repo_local_work_events_path(repo);
            let events_before = std::fs::read(&events_path).expect("seeded tracked Work events");
            save_completed_execution_fixture(repo, &session.id);

            apply_authenticated_workspace_update(
                repo,
                &session.id,
                AgentWorkspaceUpdateRequest {
                    schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                    claimed_session_id: session.id.clone(),
                    observation: observe_agent_runtime(repo).expect("runtime observation"),
                    intent: AgentWorkspaceUpdateIntent {
                        status_category: Some(
                            gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
                        ),
                        current_focus: Some("post-completion coordination".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
            )
            .expect("authenticated post-completion update");

            assert_eq!(
                std::fs::read(&events_path).expect("tracked Work events after update"),
                events_before,
                "Host bridge must preserve tracked events byte-for-byte after execution completion"
            );
            let current = gwt_core::workspace_projection::load_workspace_projection(repo)
                .expect("load current projection")
                .expect("current projection");
            assert_eq!(
                current
                    .latest_agent_for_session(&session.id)
                    .and_then(|agent| agent.current_focus.as_deref()),
                Some("post-completion coordination")
            );
            assert!(
                crate::cli::verification_record::load_work_event_settlement_record(repo)
                    .expect("load settlement record")
                    .is_none(),
                "a skipped tracked event must not reopen the settlement obligation"
            );
        });
    }

    #[test]
    fn authenticated_work_terminalization_is_idempotent_without_tracked_settlement() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-authenticated-terminalization";
            seed_unique_mutation_target(repo, repo, session, work_id);
            let observation = observe_agent_runtime(repo).expect("runtime observation");
            let request = || AgentWorkTerminalizationRequest {
                schema_version: AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                claimed_session_id: session.id.clone(),
                observation: observation.clone(),
                terminal_kind: AgentWorkTerminalKind::Done,
            };

            let first = apply_authenticated_work_terminalization(repo, &session.id, request())
                .expect("first authenticated terminalization");
            assert_eq!(first.outcome, AgentWorkTerminalizationOutcome::Emitted);

            let events_path =
                gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(repo);
            let events_after_first = std::fs::read(&events_path).expect("terminal event log");
            let retry = apply_authenticated_work_terminalization(repo, &session.id, request())
                .expect("idempotent authenticated terminalization retry");
            assert_eq!(
                retry.outcome,
                AgentWorkTerminalizationOutcome::AlreadyMatching
            );
            assert_eq!(
                std::fs::read(&events_path).expect("terminal event log after retry"),
                events_after_first,
                "an idempotent bridge retry must not append another terminal event"
            );

            let wrong = apply_authenticated_work_terminalization(
                repo,
                &session.id,
                AgentWorkTerminalizationRequest {
                    terminal_kind: AgentWorkTerminalKind::Discarded,
                    ..request()
                },
            )
            .expect("wrong terminal is an explicit domain outcome");
            assert_eq!(
                wrong.outcome,
                AgentWorkTerminalizationOutcome::WrongTerminal
            );
            assert_eq!(
                std::fs::read(&events_path).expect("terminal event log after wrong retry"),
                events_after_first,
                "wrong-terminal retry must not mutate tracked Work history"
            );

            assert!(
                crate::cli::verification_record::load_work_event_settlement_record(repo)
                    .expect("inspect tracked Work settlement")
                    .is_none(),
                "machine-local close events must not create tracked events.jsonl obligations"
            );
            let works = gwt_core::workspace_projection::load_workspace_work_items(repo)
                .expect("load terminalized WorkItems")
                .expect("terminalized WorkItems");
            let work = works
                .work_items
                .iter()
                .find(|item| item.id == work_id)
                .expect("terminalized target Work");
            assert_eq!(
                work.status_category,
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done
            );
            assert!(!work.discarded);
        });
    }

    #[test]
    fn authenticated_work_terminalization_returns_locked_assignment_outcomes() {
        with_strict_target_fixture(|repo, session| {
            let request = || AgentWorkTerminalizationRequest {
                schema_version: AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                claimed_session_id: session.id.clone(),
                observation: observe_agent_runtime(repo).expect("runtime observation"),
                terminal_kind: AgentWorkTerminalKind::Done,
            };
            let close_events_path =
                gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(repo);

            let mut unassigned = assigned_session_agent(session, "work-unassigned", Utc::now());
            unassigned.affiliation_status =
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
            unassigned.workspace_id = None;
            save_project_assignments(repo, vec![unassigned]);
            save_mutation_work_items(repo, &mutation_work_items(repo, session, "work-unassigned"));
            let works_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo);
            let works_before = std::fs::read(&works_path).expect("unassigned WorkItems before");
            let unassigned_receipt =
                apply_authenticated_work_terminalization(repo, &session.id, request())
                    .expect("latest Unassigned is an idempotent no-target outcome");
            assert_eq!(
                unassigned_receipt.outcome,
                AgentWorkTerminalizationOutcome::NoTarget
            );
            assert_eq!(
                std::fs::read(&works_path).expect("unassigned WorkItems after"),
                works_before
            );
            assert!(!close_events_path.exists());

            save_project_assignments(
                repo,
                vec![assigned_session_agent(
                    session,
                    "work-assigned-but-missing",
                    Utc::now(),
                )],
            );
            save_mutation_work_items(repo, &mutation_work_items(repo, session, "work-different"));
            let works_before = std::fs::read(&works_path).expect("missing WorkItems before");
            let missing_receipt =
                apply_authenticated_work_terminalization(repo, &session.id, request())
                    .expect("assigned missing Work is a typed terminalization outcome");
            assert_eq!(
                missing_receipt.outcome,
                AgentWorkTerminalizationOutcome::AssignedWorkMissing
            );
            assert_eq!(
                std::fs::read(&works_path).expect("missing WorkItems after"),
                works_before
            );
            assert!(!close_events_path.exists());
        });
    }

    #[test]
    fn authenticated_work_terminalization_revalidates_session_after_lock_wait() {
        use fs2::FileExt;

        with_strict_target_fixture(|repo, session| {
            let work_id = "work-terminalization-revalidation";
            seed_unique_mutation_target(repo, repo, session, work_id);
            let works_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo);
            let close_events_path =
                gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(repo);
            let works_before = std::fs::read(&works_path).expect("WorkItems before race");
            assert!(!close_events_path.exists());

            let lock_path = works_path.with_extension("lock");
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)
                .expect("open WorkItems lock");
            FileExt::lock_exclusive(&lock).expect("hold WorkItems lock");

            let request = AgentWorkTerminalizationRequest {
                schema_version: AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                claimed_session_id: session.id.clone(),
                observation: observe_agent_runtime(repo).expect("runtime observation"),
                terminal_kind: AgentWorkTerminalKind::Done,
            };
            let project_root = repo.to_path_buf();
            let session_id = session.id.clone();
            let (resolved_tx, resolved_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                apply_authenticated_work_terminalization_inner(
                    &project_root,
                    &session_id,
                    request,
                    |_| resolved_tx.send(()).expect("signal resolved target"),
                )
            });
            resolved_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("terminalization must resolve before waiting on the dual lock");

            let mut changed_session = session.clone();
            changed_session.branch = "work/reassigned-while-waiting".to_string();
            save_session_fixture(&changed_session);
            FileExt::unlock(&lock).expect("release WorkItems lock");

            let error = worker
                .join()
                .expect("terminalization worker")
                .expect_err("changed Host Session authority must reject terminalization");
            assert!(
                matches!(
                    error.code,
                    AgentWorkspaceUpdateErrorCode::RelaunchRequired
                        | AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
                        | AgentWorkspaceUpdateErrorCode::TransactionConflict
                ),
                "unexpected revalidation error: {error:?}"
            );
            assert_eq!(
                std::fs::read(&works_path).expect("WorkItems after rejected race"),
                works_before,
                "Session authority races must leave WorkItems byte-equivalent"
            );
            assert!(
                !close_events_path.exists(),
                "Session authority races must not append a terminal close event"
            );
        });
    }

    #[test]
    fn work_terminalization_bridge_request_rejects_caller_authority_fields() {
        for forbidden in [
            ("work_id", serde_json::json!("foreign-work")),
            ("project_root", serde_json::json!("/foreign/project")),
            ("owner", serde_json::json!("foreign-owner")),
            (
                "execution_container",
                serde_json::json!({"branch": "work/foreign"}),
            ),
        ] {
            let mut request = serde_json::json!({
                "schema_version": 1,
                "claimed_session_id": "session-1",
                "observation": {
                    "cwd": "/workspace/repo",
                    "git_toplevel": "/workspace/repo",
                    "repo_hash": "repo-hash",
                    "branch": "work/bridge"
                },
                "terminal_kind": "done"
            });
            request
                .as_object_mut()
                .expect("terminal request object")
                .insert(forbidden.0.to_string(), forbidden.1);
            serde_json::from_value::<AgentWorkTerminalizationRequest>(request)
                .expect_err("terminal request must not accept caller-selected routing authority");
        }

        let invalid_kind = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "repo-hash",
                "branch": "work/bridge"
            },
            "terminal_kind": "active"
        });
        serde_json::from_value::<AgentWorkTerminalizationRequest>(invalid_kind)
            .expect_err("terminal bridge accepts only Done or Discarded");
    }

    #[test]
    fn authenticated_workspace_update_rejects_claim_mismatch_before_mutation() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-authenticated-target";
            seed_unique_mutation_target(repo, repo, session, work_id);
            let current_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo);
            let mut current = load_workspace_projection_from_path(&current_path)
                .expect("load current")
                .expect("current projection");
            current.id = work_id.to_string();
            gwt_core::workspace_projection::save_workspace_projection_to_path(
                &current_path,
                &current,
            )
            .expect("align current Work identity");
            seed_work_mutation_surfaces(repo, repo);
            let before = WorkMutationSnapshot::capture(repo, repo);

            let error = apply_authenticated_workspace_update(
                repo,
                &session.id,
                AgentWorkspaceUpdateRequest {
                    schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
                    claimed_session_id: "foreign-session".to_string(),
                    observation: observe_agent_runtime(repo).expect("runtime observation"),
                    intent: AgentWorkspaceUpdateIntent {
                        summary: Some("must not persist".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
            )
            .expect_err("foreign claim must fail before mutation");

            assert_eq!(
                error.code,
                AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
            );
            assert_eq!(WorkMutationSnapshot::capture(repo, repo), before);
        });
    }

    #[test]
    fn workspace_update_bridge_request_rejects_authority_fields() {
        let request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "repo-hash",
                "branch": "work/bridge"
            },
            "intent": {"summary": "update"},
            "work_id": "foreign-work"
        });

        serde_json::from_value::<AgentWorkspaceUpdateRequest>(request)
            .expect_err("request must not accept a caller-selected Work target");
    }

    #[test]
    fn strict_session_work_mutation_target_resolves_canonical_path_aliases() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-strict-target";
            seed_unique_mutation_target(repo, repo, session, work_id);

            let target = resolve_session_work_mutation_target(repo, &session.id)
                .expect("resolve unique Session-bound Work target");
            assert_eq!(target.project_state_root, repo);
            assert_eq!(target.work_event_root, repo);
            assert_eq!(target.session_id, session.id);
            assert_eq!(target.branch_identity, session.branch);
            assert_eq!(target.worktree_identity, session.worktree_path);
            assert_eq!(target.work_id, work_id);

            let provider_path = PathBuf::from(format!(
                "Microsoft.PowerShell.Core\\FileSystem::{}",
                repo.display()
            ));
            resolve_session_work_mutation_target(&provider_path, &session.id)
                .expect("PowerShell provider path must resolve to the canonical Git root");

            #[cfg(unix)]
            {
                let symlink = repo.parent().expect("repo parent").join("repo-link");
                std::os::unix::fs::symlink(repo, &symlink).expect("create worktree symlink");
                resolve_session_work_mutation_target(&symlink, &session.id)
                    .expect("symlink must resolve to the canonical Git root");
            }
        });
    }

    #[test]
    fn strict_session_work_mutation_target_requires_latest_assignment_and_unique_active_work() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-required";
            let empty =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
            gwt_core::workspace_projection::save_workspace_projection(repo, &empty)
                .expect("save empty assignment projection");
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("missing assignment"),
                "missing",
            );

            let older = Utc::now();
            let mut unassigned =
                assigned_session_agent(session, work_id, older + chrono::Duration::seconds(1));
            unassigned.affiliation_status =
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
            unassigned.workspace_id = None;
            save_project_assignments(
                repo,
                vec![assigned_session_agent(session, work_id, older), unassigned],
            );
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("latest Unassigned state"),
                "unassigned",
            );

            save_project_assignments(
                repo,
                vec![assigned_session_agent(session, work_id, Utc::now())],
            );
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("missing WorkItems projection"),
                "missing",
            );

            save_mutation_work_items(repo, &mutation_work_items(repo, session, "work-other"));
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("missing assigned Work"),
                "missing",
            );

            let mut terminal = mutation_work_items(repo, session, work_id);
            terminal.work_items[0].status_category =
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
            save_mutation_work_items(repo, &terminal);
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("terminal assigned Work"),
                "terminal",
            );

            let mut no_container = mutation_work_items(repo, session, work_id);
            no_container.work_items[0].execution_containers.clear();
            save_mutation_work_items(repo, &no_container);
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("missing execution container"),
                "container",
            );

            let mut duplicate_containers = mutation_work_items(repo, session, work_id);
            let duplicate_container =
                duplicate_containers.work_items[0].execution_containers[0].clone();
            duplicate_containers.work_items[0]
                .execution_containers
                .push(duplicate_container);
            save_mutation_work_items(repo, &duplicate_containers);
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("multiple matching execution containers must be ambiguous"),
                "ambiguous",
            );

            let mut foreign_active = mutation_work_items(repo, session, work_id);
            let mut foreign_item = foreign_active.work_items[0].clone();
            foreign_item.id = "work-foreign".to_string();
            foreign_active.work_items.push(foreign_item.clone());
            save_mutation_work_items(repo, &foreign_active);
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id).expect_err(
                    "another active Work attached to the same Session/container must be ambiguous",
                ),
                "ambiguous",
            );

            foreign_item.status_category =
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
            foreign_active.work_items[1] = foreign_item;
            save_mutation_work_items(repo, &foreign_active);
            resolve_session_work_mutation_target(repo, &session.id)
                .expect("terminal historical Work must not make the active target ambiguous");

            let mut duplicate = mutation_work_items(repo, session, work_id);
            let mut terminal_duplicate = duplicate.work_items[0].clone();
            terminal_duplicate.status_category =
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
            duplicate.work_items.push(terminal_duplicate);
            save_mutation_work_items(repo, &duplicate);
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("duplicate Work id must be ambiguous before terminal filtering"),
                "ambiguous",
            );
        });
    }

    #[test]
    fn strict_session_work_mutation_target_rejects_active_cross_container_session_ambiguity() {
        with_strict_target_fixture(|repo, session| {
            let work_id = "work-required";
            seed_unique_mutation_target(repo, repo, session, work_id);

            let mut work_items = mutation_work_items(repo, session, work_id);
            let mut competing = work_items.work_items[0].clone();
            competing.id = "work-competing-container".to_string();
            competing.execution_containers = vec![
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some("work/other-container".to_string()),
                    worktree_path: Some(repo.join("other-container")),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            ];
            work_items.work_items.push(competing);
            save_mutation_work_items(repo, &work_items);

            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id).expect_err(
                    "one Session attached to multiple active Works must be ambiguous even when their execution containers differ",
                ),
                "ambiguous",
            );
        });
    }

    #[test]
    fn strict_session_work_mutation_target_rejects_unsafe_session_id_before_ledger_lookup() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/unsafe-session.git",
            "work/unsafe-session",
        );
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions_dir).expect("create sessions directory");
        let mut escaped = session_fixture("../escaped-session", &repo, "work/unsafe-session");
        escaped.id = "../escaped-session".to_string();
        let escaped_path = sessions_dir
            .parent()
            .expect("sessions parent")
            .join("escaped-session.toml");
        std::fs::write(
            &escaped_path,
            toml::to_string_pretty(&escaped).expect("serialize escaped Session fixture"),
        )
        .expect("seed escaped Session ledger outside sessions directory");

        let error = resolve_session_work_mutation_target(&repo, &escaped.id)
            .expect_err("unsafe Session id must be rejected before ledger lookup");
        let message = error.to_string().to_ascii_lowercase();
        assert!(
            message.contains("session id")
                && (message.contains("unsafe") || message.contains("invalid")),
            "unsafe Session id must fail at the path-component boundary, got: {message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_reject_missing_ledger_without_fallback() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            "work/strict-session",
        );

        let error = resolve_session_work_mutation_target(&repo, "missing-session-ledger")
            .expect_err("missing Session ledger must fail closed instead of using cwd fallback");
        let message = error.to_string();
        assert!(message.contains("missing-session-ledger"), "{message}");
        assert!(
            message.to_ascii_lowercase().contains("session"),
            "{message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_reject_corrupt_ledger_with_actionable_error() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            "work/strict-session",
        );
        let session_id = "corrupt-session-ledger";
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let ledger_path = sessions_dir.join(format!("{session_id}.toml"));
        std::fs::write(&ledger_path, "broken = [").expect("write corrupt Session ledger");

        let error = resolve_session_work_mutation_target(&repo, session_id)
            .expect_err("corrupt Session ledger must fail closed");
        let message = error.to_string();
        assert!(
            message.contains(session_id),
            "corrupt Session ledger error must identify the Session: {message}"
        );
        assert!(
            message.contains(&ledger_path.display().to_string()),
            "corrupt Session ledger error must identify the full ledger path: {message}"
        );
        let lowercase_message = message.to_ascii_lowercase();
        assert!(
            lowercase_message.contains("session ledger"),
            "corrupt Session ledger error must identify its context: {message}"
        );
        assert!(
            lowercase_message.contains("invalid") || lowercase_message.contains("corrupt"),
            "corrupt Session ledger error must describe the failure: {message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_redact_corrupt_ledger_source_from_diagnostics() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-private-sentinel-811";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            "work/strict-session",
        );
        let session_id = "diagnostic-redaction-session";
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let ledger_path = sessions_dir.join(format!("{session_id}.toml"));
        std::fs::write(
            &ledger_path,
            format!("agent_session_id = \"{RAW_PROVIDER_ACTOR_ID}"),
        )
        .expect("write malformed Session ledger with private provider identifier");

        let error = resolve_session_work_mutation_target(&repo, session_id)
            .expect_err("malformed Session ledger must fail closed");
        let message = error.to_string();
        assert!(
            message.contains(session_id),
            "diagnostic must identify the canonical Session: {message}"
        );
        assert!(
            message.contains(&ledger_path.display().to_string()),
            "diagnostic must identify the corrupt ledger path: {message}"
        );
        assert!(
            !message.contains(RAW_PROVIDER_ACTOR_ID),
            "diagnostic must not echo private provider identifiers from TOML source: {message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_reject_provenance_mismatch_matrix() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let shared_remote = "https://example.invalid/acme/session-bound.git";
        let repo = init_git_repo(temp.path(), "repo", shared_remote, branch);
        let sibling = init_git_repo(temp.path(), "sibling", shared_remote, branch);
        let foreign = init_git_repo(
            temp.path(),
            "foreign",
            "https://example.invalid/foreign/project.git",
            branch,
        );
        let nested_event_root = repo.join("nested-event-root");
        std::fs::create_dir_all(&nested_event_root).expect("nested event root");

        let mut base = session_fixture("base", &repo, branch);
        base.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());

        let mut repo_hash = base.clone();
        repo_hash.id = "mismatch-repo-hash".to_string();
        repo_hash.repo_hash = Some("foreign-repo-hash".to_string());

        let mut canonical_repository = base.clone();
        canonical_repository.id = "mismatch-canonical-repository".to_string();
        canonical_repository.project_state_root = Some(foreign);

        let mut branch_mismatch = base.clone();
        branch_mismatch.id = "mismatch-branch".to_string();
        branch_mismatch.branch = "work/foreign-branch".to_string();

        let mut worktree = base.clone();
        worktree.id = "mismatch-worktree".to_string();
        worktree.worktree_path = sibling.clone();

        let mut cwd = base.clone();
        cwd.id = "mismatch-cwd".to_string();

        let mut event_root = base;
        event_root.id = "mismatch-event-root".to_string();
        event_root.worktree_path = nested_event_root.clone();

        let cases = [
            ("repo hash", repo_hash, repo.clone()),
            ("canonical repository", canonical_repository, repo.clone()),
            ("branch", branch_mismatch, repo.clone()),
            ("worktree", worktree, repo.clone()),
            ("cwd", cwd, sibling),
            ("event root", event_root, nested_event_root),
        ];
        let mut failures = Vec::new();

        for (expected_mismatch, session, invocation_cwd) in cases {
            save_session_fixture(&session);
            match resolve_session_work_mutation_target(&invocation_cwd, &session.id) {
                Ok(target) => failures.push(format!(
                    "{expected_mismatch}: unexpectedly resolved project={} event={}",
                    target.project_state_root.display(),
                    target.work_event_root.display()
                )),
                Err(error) => {
                    let message = error.to_string();
                    if !message.to_ascii_lowercase().contains(expected_mismatch) {
                        failures.push(format!(
                            "{expected_mismatch}: error was not actionable: {message}"
                        ));
                    }
                    assert!(
                        !message.contains(RAW_PROVIDER_ACTOR_ID),
                        "provider actor id leaked through {expected_mismatch} diagnostic: {message}"
                    );
                }
            }
        }

        assert!(
            failures.is_empty(),
            "Session provenance mismatches must fail closed:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn workspace_update_dispatch_rejects_invalid_session_provenance_without_mutation() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _forward_url =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _runtime_path =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let _provider_actor =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_THREAD_ID", RAW_PROVIDER_ACTOR_ID);
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let mut cases = Vec::new();

        let (missing_repo, _) = init_case_repo(temp.path(), "missing", branch);
        cases.push(RejectedWorkspaceMutationCase {
            label: "missing ledger",
            expected_error: "session ledger",
            ledger: SessionLedgerFixture::Missing {
                session_id: "dispatch-missing-ledger".to_string(),
            },
            invocation_cwd: missing_repo.clone(),
            project_state_root: missing_repo.clone(),
            work_event_root: missing_repo,
        });

        let (corrupt_repo, _) = init_case_repo(temp.path(), "corrupt", branch);
        cases.push(RejectedWorkspaceMutationCase {
            label: "corrupt ledger",
            expected_error: "session ledger",
            ledger: SessionLedgerFixture::Corrupt {
                session_id: "dispatch-corrupt-ledger".to_string(),
            },
            invocation_cwd: corrupt_repo.clone(),
            project_state_root: corrupt_repo.clone(),
            work_event_root: corrupt_repo,
        });

        let (repo_hash_repo, _) = init_case_repo(temp.path(), "repo-hash", branch);
        let mut repo_hash_session =
            session_fixture("dispatch-mismatch-repo-hash", &repo_hash_repo, branch);
        repo_hash_session.repo_hash = Some("foreign-repo-hash".to_string());
        repo_hash_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "repo hash mismatch",
            expected_error: "repo hash",
            ledger: SessionLedgerFixture::Persisted(Box::new(repo_hash_session)),
            invocation_cwd: repo_hash_repo.clone(),
            project_state_root: repo_hash_repo.clone(),
            work_event_root: repo_hash_repo,
        });

        let (canonical_repo, _) = init_case_repo(temp.path(), "canonical-repository", branch);
        let canonical_foreign = init_git_repo(
            temp.path(),
            "canonical-repository-foreign",
            "https://example.invalid/foreign/canonical-repository.git",
            branch,
        );
        let mut canonical_session = session_fixture(
            "dispatch-mismatch-canonical-repository",
            &canonical_repo,
            branch,
        );
        canonical_session.project_state_root = Some(canonical_foreign.clone());
        canonical_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "canonical repository mismatch",
            expected_error: "canonical repository",
            ledger: SessionLedgerFixture::Persisted(Box::new(canonical_session)),
            invocation_cwd: canonical_repo.clone(),
            project_state_root: canonical_foreign,
            work_event_root: canonical_repo,
        });

        let (branch_repo, _) = init_case_repo(temp.path(), "branch", branch);
        let mut branch_session = session_fixture("dispatch-mismatch-branch", &branch_repo, branch);
        branch_session.branch = "work/foreign-branch".to_string();
        branch_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "branch mismatch",
            expected_error: "branch",
            ledger: SessionLedgerFixture::Persisted(Box::new(branch_session)),
            invocation_cwd: branch_repo.clone(),
            project_state_root: branch_repo.clone(),
            work_event_root: branch_repo,
        });

        let (worktree_repo, worktree_remote) = init_case_repo(temp.path(), "worktree", branch);
        let worktree_sibling =
            init_git_repo(temp.path(), "worktree-sibling", &worktree_remote, branch);
        let mut worktree_session =
            session_fixture("dispatch-mismatch-worktree", &worktree_repo, branch);
        worktree_session.worktree_path = worktree_sibling.clone();
        worktree_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "worktree mismatch",
            expected_error: "worktree",
            ledger: SessionLedgerFixture::Persisted(Box::new(worktree_session)),
            invocation_cwd: worktree_repo.clone(),
            project_state_root: worktree_repo,
            work_event_root: worktree_sibling,
        });

        let (cwd_repo, cwd_remote) = init_case_repo(temp.path(), "cwd", branch);
        let cwd_sibling = init_git_repo(temp.path(), "cwd-sibling", &cwd_remote, branch);
        let mut cwd_session = session_fixture("dispatch-mismatch-cwd", &cwd_repo, branch);
        cwd_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "cwd mismatch",
            expected_error: "cwd",
            ledger: SessionLedgerFixture::Persisted(Box::new(cwd_session)),
            invocation_cwd: cwd_sibling,
            project_state_root: cwd_repo.clone(),
            work_event_root: cwd_repo,
        });

        let (event_repo, _) = init_case_repo(temp.path(), "event-root", branch);
        let nested_event_root = event_repo.join("nested-event-root");
        std::fs::create_dir_all(&nested_event_root).expect("nested event root");
        let mut event_session =
            session_fixture("dispatch-mismatch-event-root", &event_repo, branch);
        event_session.worktree_path = nested_event_root.clone();
        event_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "event root mismatch",
            expected_error: "event root",
            ledger: SessionLedgerFixture::Persisted(Box::new(event_session)),
            invocation_cwd: nested_event_root.clone(),
            project_state_root: event_repo,
            work_event_root: nested_event_root,
        });

        for (label, binding) in [
            ("missing Docker binding", None),
            (
                "incomplete Docker binding",
                Some(gwt_agent::session::DockerRuntimeBinding {
                    runtime_worktree_path: PathBuf::new(),
                    project_state_scope_hash: "0123456789abcdef".to_string(),
                }),
            ),
            (
                "invalid Docker scope",
                Some(gwt_agent::session::DockerRuntimeBinding {
                    runtime_worktree_path: PathBuf::from("/runtime/not-used-by-legacy-resolver"),
                    project_state_scope_hash: "../invalid-scope".to_string(),
                }),
            ),
        ] {
            let case_name = label.replace(' ', "-");
            let (docker_repo, _) = init_case_repo(temp.path(), &case_name, branch);
            std::fs::write(
                docker_repo.join("docker-compose.yml"),
                format!(
                    "services:\n  app:\n    image: test\n    working_dir: '{}'\n    volumes:\n      - '{}:{}'\n",
                    docker_repo.display(),
                    docker_repo.display(),
                    docker_repo.display()
                ),
            )
            .expect("write legacy Docker resolver fixture");
            let mut docker_session =
                session_fixture(&format!("dispatch-{case_name}"), &docker_repo, branch);
            docker_session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
            docker_session.docker_service = Some("app".to_string());
            docker_session.docker_runtime_binding = binding;
            docker_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
            cases.push(RejectedWorkspaceMutationCase {
                label,
                expected_error: "relaunch",
                ledger: SessionLedgerFixture::Persisted(Box::new(docker_session)),
                invocation_cwd: docker_repo.clone(),
                project_state_root: docker_repo.clone(),
                work_event_root: docker_repo,
            });
        }

        let mut failures = Vec::new();
        for case in cases {
            seed_work_mutation_surfaces(&case.project_state_root, &case.work_event_root);
            let before =
                WorkMutationSnapshot::capture(&case.project_state_root, &case.work_event_root);
            case.ledger.install();

            let _ambient = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::session::GWT_SESSION_ID_ENV,
                case.ledger.session_id(),
            );
            let mut env = crate::cli::TestEnv::new(case.invocation_cwd);
            env.stdin = serde_json::json!({
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "summary": "must be rejected without mutation",
                },
            })
            .to_string();

            let code = crate::cli::dispatch(&mut env, &["gwtd".to_string()]);
            let stderr = String::from_utf8_lossy(&env.stderr);
            if code == 0 {
                failures.push(format!("{}: unexpectedly accepted", case.label));
            } else if !stderr.to_ascii_lowercase().contains(case.expected_error) {
                failures.push(format!(
                    "{}: rejection was not actionable: {stderr}",
                    case.label
                ));
            }
            if stderr.contains(RAW_PROVIDER_ACTOR_ID) {
                failures.push(format!(
                    "{}: provider actor id leaked in diagnostic: {stderr}",
                    case.label
                ));
            }

            let after =
                WorkMutationSnapshot::capture(&case.project_state_root, &case.work_event_root);
            let changed_surfaces = before.changed_surfaces(&after);
            if !changed_surfaces.is_empty() {
                failures.push(format!(
                    "{}: rejection was not byte-equivalent for {}",
                    case.label,
                    changed_surfaces.join(", ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "workspace.update must reject invalid gwt Session provenance before persistence:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn provider_actor_id_is_not_authorization_or_tracked_provenance() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _forward_url =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _runtime_path =
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            branch,
        );

        let mut provider_present = session_fixture("provider-present", &repo, branch);
        provider_present.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        let provider_absent = session_fixture("provider-absent", &repo, branch);
        let work_id = "work-provider-neutral";
        save_project_assignments(
            &repo,
            vec![
                assigned_session_agent(&provider_present, work_id, Utc::now()),
                assigned_session_agent(&provider_absent, work_id, Utc::now()),
            ],
        );
        save_mutation_work_items_with_tracked_events(
            &repo,
            &mutation_work_items(&repo, &provider_present, work_id),
        );

        for session in [&provider_present, &provider_absent] {
            save_session_fixture(session);
            let _ambient = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::session::GWT_SESSION_ID_ENV,
                &session.id,
            );
            let mut env = crate::cli::TestEnv::new(repo.clone());
            env.stdin = serde_json::json!({
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "summary": "provider-neutral mutation",
                },
            })
            .to_string();

            let code = crate::cli::dispatch(&mut env, &["gwtd".to_string()]);
            assert_eq!(
                code,
                0,
                "workspace.update must accept valid gwt Session provenance: {}",
                String::from_utf8_lossy(&env.stderr)
            );
        }

        let tracked_events =
            std::fs::read_to_string(gwt_core::paths::gwt_repo_local_work_events_path(&repo))
                .expect("read tracked Work events");
        let events: Vec<serde_json::Value> = tracked_events
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("parse tracked Work event JSONL"))
            .collect();
        for session in [&provider_present, &provider_absent] {
            let event = events
                .iter()
                .find(|event| event["agent_session_id"].as_str() == Some(session.id.as_str()))
                .unwrap_or_else(|| panic!("tracked Work event missing Session {}", session.id));
            assert_eq!(
                event["agent_session_id"].as_str(),
                Some(session.id.as_str()),
                "tracked provenance must remain the immutable gwt Session id"
            );
        }
        assert!(
            !events
                .iter()
                .any(|event| json_value_contains(event, RAW_PROVIDER_ACTOR_ID)),
            "raw provider actor id must never enter any tracked Work event JSON value: {events:?}"
        );
    }

    #[test]
    fn legacy_bare_child_worktree_derives_workspace_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace-home");
        let bare_repo = workspace_home.join("gwt.git");
        std::fs::create_dir_all(&workspace_home).expect("workspace home");
        run_git(
            &["init", "--bare", bare_repo.to_str().unwrap()],
            temp.path(),
        );

        let bootstrap = temp.path().join("bootstrap");
        run_git(
            &[
                "clone",
                bare_repo.to_str().unwrap(),
                bootstrap.to_str().unwrap(),
            ],
            temp.path(),
        );
        run_git(&["config", "user.email", "test@example.com"], &bootstrap);
        run_git(&["config", "user.name", "Test User"], &bootstrap);
        run_git(&["checkout", "-b", "develop"], &bootstrap);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &bootstrap);
        run_git(&["push", "origin", "develop"], &bootstrap);

        let worktree = workspace_home.join("work").join("20260601-0934");
        std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
            .expect("worktree parent");
        run_git(
            &["worktree", "add", worktree.to_str().unwrap(), "develop"],
            &bare_repo,
        );

        let session = Session::new(&worktree, "work/20260601-0934", gwt_agent::AgentId::Codex);
        assert_eq!(
            canonical_project_state_root_for_session(&session, &worktree),
            dunce::canonicalize(&workspace_home).expect("canonical workspace home")
        );
    }

    #[test]
    fn repair_agent_from_split_prefers_newer_title_and_focus() {
        let older = Utc::now();
        let newer = older + chrono::Duration::seconds(1);
        let mut canonical = agent_summary(
            "session-1",
            Some("Old canonical title"),
            Some("Old canonical focus"),
            older,
        );
        let split = agent_summary(
            "session-1",
            Some("New split title"),
            Some("New split focus"),
            newer,
        );

        assert!(repair_agent_from_split(&mut canonical, &split));
        assert_eq!(canonical.title_summary.as_deref(), Some("New split title"));
        assert_eq!(canonical.current_focus.as_deref(), Some("New split focus"));
    }

    #[test]
    fn repair_agent_from_split_keeps_newer_canonical_title_and_focus() {
        let older = Utc::now();
        let newer = older + chrono::Duration::seconds(1);
        let mut canonical = agent_summary(
            "session-1",
            Some("New canonical title"),
            Some("New canonical focus"),
            newer,
        );
        let split = agent_summary(
            "session-1",
            Some("Old split title"),
            Some("Old split focus"),
            older,
        );

        assert!(!repair_agent_from_split(&mut canonical, &split));
        assert_eq!(
            canonical.title_summary.as_deref(),
            Some("New canonical title")
        );
        assert_eq!(
            canonical.current_focus.as_deref(),
            Some("New canonical focus")
        );
    }

    #[test]
    fn split_repair_updates_only_latest_duplicate_session_rows() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical_root = temp.path().join("canonical");
        let split_root = temp.path().join("split");
        std::fs::create_dir_all(&canonical_root).expect("create canonical root");
        std::fs::create_dir_all(&split_root).expect("create split root");
        let base = Utc::now();

        let canonical_stale = agent_summary(
            "duplicate-session",
            Some("Stale canonical title"),
            Some("Stale canonical focus"),
            base,
        );
        let canonical_current = agent_summary(
            "duplicate-session",
            Some("Current canonical title"),
            Some("Current canonical focus"),
            base + chrono::Duration::seconds(1),
        );
        let split_stale = agent_summary(
            "duplicate-session",
            Some("Stale split title"),
            Some("Stale split focus"),
            base + chrono::Duration::seconds(2),
        );
        let split_current = agent_summary(
            "duplicate-session",
            Some("Latest split title"),
            Some("Latest split focus"),
            base + chrono::Duration::seconds(3),
        );

        let mut canonical_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical_projection.agents = vec![canonical_stale.clone(), canonical_current];
        gwt_core::workspace_projection::save_workspace_projection(
            &canonical_root,
            &canonical_projection,
        )
        .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split_stale, split_current];
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        let saved_canonical =
            gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
                .expect("load canonical precondition")
                .expect("canonical precondition exists");
        assert_eq!(
            saved_canonical
                .latest_agent_for_session("duplicate-session")
                .and_then(|agent| agent.title_summary.as_deref()),
            Some("Current canonical title")
        );
        let saved_split = gwt_core::workspace_projection::load_workspace_projection(&split_root)
            .expect("load split precondition")
            .expect("split precondition exists");
        assert_eq!(
            saved_split
                .latest_agent_for_session("duplicate-session")
                .and_then(|agent| agent.title_summary.as_deref()),
            Some("Latest split title")
        );

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load canonical projection")
            .expect("canonical projection exists");
        assert_eq!(repaired.agents[0], canonical_stale);
        assert_eq!(
            repaired.agents[1].title_summary.as_deref(),
            Some("Latest split title")
        );
        assert_eq!(
            repaired.agents[1].current_focus.as_deref(),
            Some("Latest split focus")
        );
    }

    #[test]
    fn split_repair_keeps_future_timestamps_monotonic_and_repaired_row_latest() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical_root = temp.path().join("canonical");
        let split_root = temp.path().join("split");
        std::fs::create_dir_all(&canonical_root).expect("create canonical root");
        std::fs::create_dir_all(&split_root).expect("create split root");

        let future = Utc::now() + chrono::Duration::days(1);
        let competing_at = future + chrono::Duration::hours(1);
        let canonical_at = future + chrono::Duration::hours(2);
        let split_at = future + chrono::Duration::hours(3);
        let projection_at = future + chrono::Duration::hours(4);
        let competing = agent_summary(
            "duplicate-session",
            Some("Competing canonical title"),
            Some("Competing canonical focus"),
            competing_at,
        );
        let canonical = agent_summary(
            "duplicate-session",
            Some("Canonical title"),
            Some("Canonical focus"),
            canonical_at,
        );
        let split = agent_summary(
            "duplicate-session",
            Some("Repaired split title"),
            Some("Repaired split focus"),
            split_at,
        );

        let mut canonical_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical_projection.agents = vec![competing, canonical];
        canonical_projection.updated_at = projection_at;
        gwt_core::workspace_projection::save_workspace_projection(
            &canonical_root,
            &canonical_projection,
        )
        .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split];
        split_projection.updated_at = split_at;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load canonical projection")
            .expect("canonical projection exists");
        let repaired_agent = repaired
            .agents
            .iter()
            .find(|agent| agent.title_summary.as_deref() == Some("Repaired split title"))
            .expect("repaired agent row");
        assert!(
            repaired_agent.updated_at >= projection_at,
            "repaired Agent timestamp must not regress below Agent/projection inputs"
        );
        assert!(
            repaired.updated_at >= projection_at,
            "projection timestamp must not regress during split repair"
        );
        assert_eq!(
            repaired.latest_agent_for_session("duplicate-session"),
            Some(repaired_agent),
            "the repaired row must remain the latest Session row"
        );
    }

    #[test]
    fn split_repair_makes_repaired_duplicate_strictly_latest_when_timestamps_tie() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical_root = temp.path().join("canonical");
        let split_root = temp.path().join("split");
        std::fs::create_dir_all(&canonical_root).expect("create canonical root");
        std::fs::create_dir_all(&split_root).expect("create split root");
        let tied_at = Utc::now() + chrono::Duration::days(1);

        let competing = agent_summary(
            "duplicate-session",
            Some("Competing title"),
            Some("z competing focus"),
            tied_at,
        );
        let repair_target = agent_summary(
            "duplicate-session",
            Some("Repair target title"),
            None,
            tied_at,
        );
        let split = agent_summary(
            "duplicate-session",
            Some("Split title"),
            Some("a repaired focus"),
            tied_at,
        );

        let mut canonical =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical.agents = vec![competing, repair_target];
        canonical.updated_at = tied_at;
        gwt_core::workspace_projection::save_workspace_projection(&canonical_root, &canonical)
            .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split];
        split_projection.updated_at = tied_at;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load repaired projection")
            .expect("repaired projection");
        let latest = repaired
            .latest_agent_for_session("duplicate-session")
            .expect("latest repaired Agent");
        assert_eq!(latest.title_summary.as_deref(), Some("Repair target title"));
        assert_eq!(latest.current_focus.as_deref(), Some("a repaired focus"));
        assert!(latest.updated_at > tied_at);
    }

    #[test]
    fn split_repair_timestamp_overflow_does_not_persist_partial_update() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical_root = temp.path().join("canonical");
        let split_root = temp.path().join("split");
        std::fs::create_dir_all(&canonical_root).expect("create canonical root");
        std::fs::create_dir_all(&split_root).expect("create split root");
        let max = chrono::DateTime::<Utc>::MAX_UTC;

        let mut canonical =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical.agents = vec![agent_summary(
            "overflow-session",
            Some("Canonical title"),
            None,
            max,
        )];
        canonical.updated_at = max;
        gwt_core::workspace_projection::save_workspace_projection(&canonical_root, &canonical)
            .expect("save canonical projection");

        let mut split =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split.agents = vec![agent_summary(
            "overflow-session",
            Some("Split title"),
            Some("Split focus"),
            max,
        )];
        split.updated_at = max;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split)
            .expect("save split projection");

        let canonical_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&canonical_root);
        let before = std::fs::read(&canonical_path).expect("read canonical before repair");
        let error =
            repair_split_agent_state_if_needed(&canonical_root, &split_root, "overflow-session")
                .expect_err("timestamp overflow must fail closed");

        assert!(error.to_string().contains("timestamp exceeds"));
        assert_eq!(
            std::fs::read(&canonical_path).expect("read canonical after repair"),
            before,
            "failed repair must not persist partially copied Agent fields"
        );
    }
}
