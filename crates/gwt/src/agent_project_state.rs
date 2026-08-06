use std::path::{Path, PathBuf};

use chrono::Utc;
use gwt_agent::{ExecutionBindingIdentity, LaunchRuntimeTarget, Session, SessionExecutionBinding};
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
pub const AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION: u32 = 1;
pub const AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionContinuationRequest {
    pub schema_version: u32,
    pub operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionContinuationOutcome {
    ReboundCurrent,
    SuccessorCreated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionContinuationReceipt {
    pub schema_version: u32,
    pub operation_id: String,
    pub outcome: AgentExecutionContinuationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_generation_id: Option<String>,
    pub generation_id: String,
    pub execution_binding: ExecutionBindingIdentity,
    pub capability_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_execution_binding: Option<ExecutionBindingIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takeover_audit_id: Option<String>,
    pub validated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionBindingProbeRequest {
    pub schema_version: u32,
    pub operation_id: String,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionBindingProbeReceipt {
    pub schema_version: u32,
    pub operation_id: String,
    pub nonce: String,
    pub host_instance_id: String,
    pub execution_binding: ExecutionBindingIdentity,
    pub capability_generation: u64,
}

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
    ExecutionBindingMismatch,
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

pub fn probe_authenticated_execution_binding(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    host_instance_id: &str,
    request: AgentExecutionBindingProbeRequest,
) -> std::result::Result<AgentExecutionBindingProbeReceipt, AgentWorkspaceUpdateError> {
    if request.schema_version != AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "unsupported execution binding probe schema version",
        ));
    }
    validate_ephemeral_probe_identifier(&request.operation_id, "operation id")?;
    if request.operation_id.len() > 256 {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "execution continuation operation id exceeds 256 bytes",
        ));
    }
    validate_ephemeral_probe_identifier(&request.nonce, "nonce")?;
    validate_ephemeral_probe_identifier(host_instance_id, "Host instance id")?;
    let validated = validate_current_execution_binding_authority(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
    )?;

    Ok(AgentExecutionBindingProbeReceipt {
        schema_version: AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
        operation_id: request.operation_id,
        nonce: request.nonce,
        host_instance_id: host_instance_id.to_string(),
        execution_binding: validated.identity,
        capability_generation: validated.capability_generation,
    })
}

pub fn probe_authenticated_prepared_execution_binding(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    host_instance_id: &str,
    request: AgentExecutionBindingProbeRequest,
) -> std::result::Result<AgentExecutionBindingProbeReceipt, AgentWorkspaceUpdateError> {
    if request.schema_version != AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "unsupported execution binding probe schema version",
        ));
    }
    validate_ephemeral_probe_identifier(&request.operation_id, "operation id")?;
    validate_ephemeral_probe_identifier(&request.nonce, "nonce")?;
    validate_ephemeral_probe_identifier(host_instance_id, "Host instance id")?;
    let validated = validate_prepared_execution_binding_authority(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
    )?;

    Ok(AgentExecutionBindingProbeReceipt {
        schema_version: AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
        operation_id: request.operation_id,
        nonce: request.nonce,
        host_instance_id: host_instance_id.to_string(),
        execution_binding: validated.identity,
        capability_generation: validated.capability_generation,
    })
}

/// Establish producing execution authority for one capability-authenticated
/// Session. Request fields are correlation only; project, Session, owner, and
/// worktree authority are derived from the authenticated principal and
/// durable records.
/// SPEC-3393 FR-012 (AC-12): recover producing authority for a Resume or
/// Continue launch before spawn. Wraps [`continue_authenticated_execution`]
/// for the launch path: `None` means the durable session has no producing
/// linkage or the coordinator refused, and the launch stays observation-only
/// (the unlinked-session carve-out) instead of failing.
#[must_use]
pub fn prepare_resume_producing_authority(
    project_root: &Path,
    predecessor_session_id: &str,
) -> Option<(AgentExecutionContinuationReceipt, SessionExecutionBinding)> {
    let request = AgentExecutionContinuationRequest {
        schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
        operation_id: format!("resume-producing-{}", uuid::Uuid::new_v4()),
    };
    match continue_authenticated_execution(project_root, predecessor_session_id, request) {
        Ok(result) => {
            tracing::info!(
                session_id = predecessor_session_id,
                outcome = ?result.0.outcome,
                "resume launch recovered producing authority (SPEC-3393 FR-012)"
            );
            Some(result)
        }
        Err(error) => {
            tracing::warn!(
                session_id = predecessor_session_id,
                code = ?error.code,
                message = %error.message,
                "resume launch stays observation-only"
            );
            None
        }
    }
}

/// Recover the execution binding for an exact persisted-Session relaunch.
///
/// Unlike [`continue_authenticated_execution`], this operation is identity
/// recovery rather than an execution lifecycle transition. It never creates a
/// continuation attempt or successor generation, and it accepts terminal
/// lifecycle heads only when the durable Session is still the exact current
/// writer for that owner.
pub fn prepare_exact_relaunch_execution_authority(
    project_root: &Path,
    predecessor_session_id: &str,
) -> std::result::Result<SessionExecutionBinding, AgentWorkspaceUpdateError> {
    let authority =
        evaluate_authenticated_execution_continuation(project_root, predecessor_session_id)?;
    if authority.exact_unbound {
        return Err(execution_binding_error(
            "exact_relaunch_requires_existing_binding",
        ));
    }

    let expected_generation_id = authority.current_binding.generation_id.clone();
    let binding = rebind_session_to_current_execution(
        &authority.worktree,
        authority.owner,
        &authority.session,
        Some(&expected_generation_id),
    )?;

    let revalidated =
        evaluate_authenticated_execution_continuation(project_root, predecessor_session_id)?;
    if revalidated.exact_unbound
        || revalidated.session.execution_binding.as_ref() != Some(&binding)
        || revalidated.current_binding != binding.identity
    {
        return Err(execution_binding_error(
            "exact_relaunch_binding_readback_mismatch",
        ));
    }

    tracing::info!(
        session_id = predecessor_session_id,
        generation_id = %binding.identity.generation_id,
        status = ?revalidated.record.status,
        "exact relaunch recovered current execution authority"
    );
    Ok(binding)
}

struct ExecutionContinuationAuthority {
    session: Session,
    project_state_root: PathBuf,
    worktree: PathBuf,
    record: crate::cli::execution_state::ExecutionControlRecord,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    exact_unbound: bool,
    current_binding: ExecutionBindingIdentity,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionRecoveryContext {
    session: Session,
    project_state_root: PathBuf,
    worktree: PathBuf,
    exact_unbound_host: bool,
}

impl ExecutionRecoveryContext {
    pub(crate) fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn project_state_root(&self) -> &Path {
        &self.project_state_root
    }

    pub(crate) fn worktree(&self) -> &Path {
        &self.worktree
    }

    pub(crate) fn exact_unbound_host(&self) -> bool {
        self.exact_unbound_host
    }
}

pub(crate) fn session_requires_execution_continuation(session_id: &str) -> bool {
    if gwt_agent::validate_session_id_path_component(session_id).is_err() {
        return false;
    }
    load_session_for_mutation(session_id)
        .is_ok_and(|session| session.id == session_id && is_exact_unbound_host_session(&session))
}

fn is_exact_unbound_host_session(session: &Session) -> bool {
    session.linked_issue_number.is_none()
        && session.execution_binding.is_none()
        && session.runtime_target == LaunchRuntimeTarget::Host
        && session.docker_runtime_binding.is_none()
}

pub(crate) fn resolve_execution_recovery_context_if_session_exists(
    invocation_scope: &Path,
    session_id: &str,
) -> Option<Result<ExecutionRecoveryContext>> {
    if let Err(error) = gwt_agent::validate_session_id_path_component(session_id) {
        return Some(Err(mutation_error(format!(
            "invalid or unsafe Session id: {error}"
        ))));
    }
    let ledger_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    match ledger_path.try_exists() {
        Ok(true) => Some(resolve_execution_recovery_context(
            invocation_scope,
            session_id,
        )),
        Ok(false) => None,
        Err(error) => Some(Err(mutation_error(format!(
            "failed to inspect Session ledger for Session {session_id} at {}: {error}",
            ledger_path.display()
        )))),
    }
}

pub(crate) fn durable_session_runtime_target_if_session_exists(
    session_id: &str,
) -> Option<Result<LaunchRuntimeTarget>> {
    if let Err(error) = gwt_agent::validate_session_id_path_component(session_id) {
        return Some(Err(mutation_error(format!(
            "invalid or unsafe Session id: {error}"
        ))));
    }
    let ledger_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    match ledger_path.try_exists() {
        Ok(false) => None,
        Ok(true) => Some(load_session_for_mutation(session_id).and_then(|session| {
            if session.id != session_id {
                return Err(mutation_error(format!(
                    "Session ledger id mismatch: requested {session_id}, loaded {}",
                    session.id
                )));
            }
            Ok(session.runtime_target)
        })),
        Err(error) => Some(Err(mutation_error(format!(
            "failed to inspect Session ledger for Session {session_id} at {}: {error}",
            ledger_path.display()
        )))),
    }
}

/// Resolve the operation-local recovery scope without publishing or repairing authority.
///
/// A managed workspace may split repository-global Project State from the linked
/// Session worktree. Recovery diagnosis accepts the validated Project State root,
/// the exact linked worktree, or a real nested cwd inside that worktree. It never
/// accepts another worktree merely because it belongs to the same repository.
pub(crate) fn resolve_execution_recovery_context(
    invocation_scope: &Path,
    session_id: &str,
) -> Result<ExecutionRecoveryContext> {
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| mutation_error(format!("invalid or unsafe Session id: {error}")))?;
    let session = load_session_for_mutation(session_id)?;
    if session.id != session_id {
        return Err(mutation_error(format!(
            "Session identity mismatch for recovery: expected {session_id}, got {}",
            session.id
        )));
    }
    let worktree = canonicalize_mutation_path(&session.worktree_path, "recovery worktree")?;
    let worktree_git_root = git_toplevel(&worktree, "recovery worktree")?;
    if worktree_git_root != worktree {
        return Err(mutation_error(format!(
            "Session event root mismatch for Session {session_id}: worktree {} must be the exact Git toplevel {}",
            worktree.display(),
            worktree_git_root.display()
        )));
    }

    let project_state_root = validated_project_state_root_for_session_recovery(&session)?;
    let declared_repo_hash = required_session_repo_hash(&session)?;
    let branch_identity = required_session_branch(&session)?;
    validate_runtime_repo_and_branch(&worktree, declared_repo_hash, &branch_identity, &session)?;

    let invocation = canonicalize_mutation_path(invocation_scope, "recovery invocation scope")?;
    if invocation != project_state_root {
        if !invocation.starts_with(&worktree) {
            return Err(mutation_error(format!(
                "Session cwd mismatch for Session {session_id}: recovery must run from Project State root {} or within exact worktree {}, got {}",
                project_state_root.display(),
                worktree.display(),
                invocation.display()
            )));
        }
        let invocation_git_root = git_toplevel(&invocation, "recovery invocation scope")?;
        if invocation_git_root != worktree {
            return Err(mutation_error(format!(
                "Session worktree mismatch for Session {session_id}: recovery invocation resolves to {}, expected {}",
                invocation_git_root.display(),
                worktree.display()
            )));
        }
        validate_runtime_repo_and_branch(
            &invocation_git_root,
            declared_repo_hash,
            &branch_identity,
            &session,
        )?;
    }

    let exact_unbound_host = is_exact_unbound_host_session(&session);
    Ok(ExecutionRecoveryContext {
        session,
        project_state_root,
        worktree,
        exact_unbound_host,
    })
}

fn evaluate_authenticated_execution_continuation(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
) -> std::result::Result<ExecutionContinuationAuthority, AgentWorkspaceUpdateError> {
    let session = load_session_for_mutation(authenticated_session_id).map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::RelaunchRequired,
            "durable Session metadata is unavailable; relaunch before continuing",
        )
    })?;
    let principal_root =
        canonicalize_mutation_path(authenticated_project_root, "authenticated project")
            .map_err(|_| execution_binding_error("execution_continuation_project_invalid"))?;
    let project_state_root = strict_project_state_root(&session)
        .and_then(|root| canonicalize_mutation_path(&root, "canonical repository"))
        .map_err(|_| execution_binding_error("execution_continuation_project_invalid"))?;
    if session.id != authenticated_session_id || principal_root != project_state_root {
        return Err(execution_binding_error(
            "execution_continuation_principal_mismatch",
        ));
    }
    let worktree = canonicalize_mutation_path(&session.worktree_path, "worktree")
        .map_err(|_| execution_binding_error("execution_continuation_worktree_invalid"))?;
    if git_toplevel(&worktree, "worktree")
        .map_err(|_| execution_binding_error("execution_continuation_worktree_invalid"))?
        != worktree
    {
        return Err(execution_binding_error(
            "execution_continuation_worktree_mismatch",
        ));
    }
    let record = crate::cli::execution_state::load(&worktree)
        .map_err(|_| execution_binding_error("execution_continuation_record_unreadable"))?
        .ok_or_else(|| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::RelaunchRequired,
                "no linked execution exists; start the Work before continuing",
            )
        })?;
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let repo_hash = repo_hash_for_mutation(&worktree, "repo hash")
        .map_err(|_| execution_binding_error("execution_continuation_repo_hash_unavailable"))?;
    let exact_unbound = is_exact_unbound_host_session(&session);
    let activated_publication_repair = (!crate::cli::execution_state::integrity_ok(&record)
        && exact_unbound)
        .then(|| {
            crate::cli::execution_state::activated_continuation_binding_for_session(
                &worktree,
                owner,
                &session.id,
            )
        })
        .transpose()
        .map_err(|_| execution_binding_error("execution_continuation_record_integrity_failure"))?
        .flatten();
    if !crate::cli::execution_state::integrity_ok(&record) && activated_publication_repair.is_none()
    {
        return Err(execution_binding_error(
            "execution_continuation_record_integrity_failure",
        ));
    }
    if session.runtime_target != LaunchRuntimeTarget::Host
        || session.docker_runtime_binding.is_some()
        || (session.linked_issue_number != Some(owner.number) && !exact_unbound)
        || session.repo_hash.as_deref() != Some(repo_hash.as_str())
    {
        return Err(execution_binding_error(
            "execution_continuation_session_scope_mismatch",
        ));
    }
    let identity = validate_host_session_identity(&worktree, &session)
        .map_err(|_| execution_binding_error("execution_continuation_host_identity_mismatch"))?;
    if identity.project_state_root != project_state_root
        || identity.worktree_identity != worktree
        || identity.work_event_root != worktree
    {
        return Err(execution_binding_error(
            "execution_continuation_host_identity_mismatch",
        ));
    }
    let current_binding = match activated_publication_repair {
        Some(binding) => binding,
        None => match crate::cli::execution_state::current_execution_binding(&worktree, owner) {
            Ok(Some(binding)) => binding,
            Ok(None) => {
                return Err(execution_binding_error(
                    "execution_continuation_binding_missing",
                ))
            }
            Err(_) if exact_unbound => {
                crate::cli::execution_state::activated_continuation_binding_for_session(
                    &worktree,
                    owner,
                    &session.id,
                )
                .map_err(|_| execution_binding_error("execution_continuation_binding_unreadable"))?
                .ok_or_else(|| {
                    execution_binding_error("execution_continuation_binding_unreadable")
                })?
            }
            Err(_) => {
                return Err(execution_binding_error(
                    "execution_continuation_binding_unreadable",
                ))
            }
        },
    };
    if !exact_unbound {
        let exact_bound = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .map_err(|_| execution_binding_error("execution_continuation_binding_invalid"))?
            .is_some_and(|identity| {
                record.primary_session_id == session.id
                    && crate::cli::execution_state::session_binding_authorizes_current_lifecycle_descendant(
                        &worktree,
                        owner,
                        &session.id,
                        &identity.execution_binding.identity,
                    )
                    .unwrap_or(false)
            });
        if !exact_bound {
            return Err(execution_binding_error(
                "execution_continuation_binding_not_exact",
            ));
        }
    }
    Ok(ExecutionContinuationAuthority {
        session,
        project_state_root,
        worktree,
        record,
        owner,
        exact_unbound,
        current_binding,
    })
}

/// Side-effect-free prerequisite evaluator shared by diagnosis and execution.
pub(crate) fn probe_authenticated_execution_continuation(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{
        GovernanceCause, GovernanceEffect, GovernanceMetadata, RecoveryProbe,
    };
    match evaluate_authenticated_execution_continuation(
        authenticated_project_root,
        authenticated_session_id,
    ) {
        Ok(authority) => {
            let governance = GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                fingerprint: Some(format!(
                    "execution.continue:{}:{}:{}",
                    authority.owner.number,
                    authority.session.id,
                    authority.current_binding.generation_id
                )),
                retryable: Some(true),
                repository_target: authority.session.repo_hash.clone(),
                target_state: Some(format!("{:?}", authority.record.status).to_ascii_lowercase()),
                execution_generation: Some(authority.current_binding.generation_id.clone()),
                ..GovernanceMetadata::default()
            };
            if authority.record.status
                == crate::cli::execution_state::ExecutionControlStatus::Blocked
            {
                return RecoveryProbe::unavailable(
                    "execution.continue",
                    GovernanceMetadata {
                        cause: Some(GovernanceCause::NotReady),
                        ..governance
                    },
                    "blocked_execution_requires_reopen",
                );
            }
            let current_bound = authority.record.status
                == crate::cli::execution_state::ExecutionControlStatus::Active
                && authority.record.primary_session_id == authority.session.id
                && authority.session.linked_issue_number == Some(authority.owner.number)
                && authority
                    .session
                    .execution_binding
                    .as_ref()
                    .is_some_and(|binding| binding.identity == authority.current_binding);
            if current_bound {
                RecoveryProbe::satisfied("execution.continue", governance)
            } else {
                RecoveryProbe::available("execution.continue", governance)
            }
        }
        Err(error) => RecoveryProbe::unavailable(
            "execution.continue",
            GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                cause: Some(GovernanceCause::Authority),
                retryable: Some(false),
                ..GovernanceMetadata::default()
            },
            error.message,
        ),
    }
}

pub fn continue_authenticated_execution(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    request: AgentExecutionContinuationRequest,
) -> std::result::Result<
    (AgentExecutionContinuationReceipt, SessionExecutionBinding),
    AgentWorkspaceUpdateError,
> {
    if request.schema_version != AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            "unsupported execution continuation schema version",
        ));
    }
    validate_ephemeral_probe_identifier(&request.operation_id, "operation id")?;
    let authority = evaluate_authenticated_execution_continuation(
        authenticated_project_root,
        authenticated_session_id,
    )?;
    let ExecutionContinuationAuthority {
        session,
        project_state_root,
        worktree,
        record,
        owner,
        exact_unbound,
        current_binding,
    } = authority;

    if let Some(audit) = crate::cli::execution_state::continuation_validation_for_operation(
        &worktree,
        owner,
        &request.operation_id,
    )
    .map_err(|_| execution_binding_error("execution_continuation_validation_unreadable"))?
    {
        let binding = session.execution_binding.clone().ok_or_else(|| {
            execution_binding_error("execution_continuation_replay_binding_missing")
        })?;
        let current = crate::cli::execution_state::current_execution_binding(&worktree, owner)
            .map_err(|_| execution_binding_error("execution_continuation_binding_unreadable"))?
            .ok_or_else(|| execution_binding_error("execution_continuation_binding_missing"))?;
        if audit.session_id != session.id
            || audit.generation_id != current.generation_id
            || audit.execution_binding != current
            || audit.execution_binding != binding.identity
            || audit.capability_generation != binding.capability_generation
        {
            return Err(AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::IdentityConflict,
                "continuation operation receipt no longer matches current authority",
            ));
        }
        validate_continuation_binding(
            &project_state_root,
            &session.id,
            &request.operation_id,
            &binding,
        )?;
        return Ok((
            AgentExecutionContinuationReceipt {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: request.operation_id,
                outcome: AgentExecutionContinuationOutcome::ReboundCurrent,
                predecessor_generation_id: None,
                generation_id: audit.generation_id,
                execution_binding: audit.execution_binding,
                capability_generation: audit.capability_generation,
                superseded_execution_binding: None,
                takeover_audit_id: None,
                validated: true,
            },
            binding,
        ));
    }

    if let Some(existing) = crate::cli::execution_state::continuation_attempt_for_operation(
        &worktree,
        owner,
        &request.operation_id,
    )
    .map_err(|_| execution_binding_error("execution_continuation_attempt_unreadable"))?
    {
        if existing.request.source != "execution-continue"
            || existing.request.initial_session_id != session.id
        {
            return Err(AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::IdentityConflict,
                "continuation operation id is already bound to another authority",
            ));
        }
        if existing.status == crate::cli::execution_state::ContinuationAttemptStatus::Activated {
            let activated = existing.activated_generation.as_ref().ok_or_else(|| {
                execution_binding_error("execution_continuation_activation_receipt_missing")
            })?;
            let (_, binding) = crate::cli::execution_state::activate_successor_with_session_rebind(
                &worktree,
                owner,
                &existing.request,
                existing.predecessor_status,
                &current_binding,
                &gwt_core::paths::gwt_sessions_dir(),
                &session,
            )
            .map_err(|_| {
                execution_binding_error("execution_continuation_activation_repair_failed")
            })?
            .ok_or_else(|| {
                AgentWorkspaceUpdateError::new(
                    AgentWorkspaceUpdateErrorCode::IdentityConflict,
                    "Activated continuation no longer matches current authority",
                )
            })?;
            if binding.identity.generation_id != activated.generation_id {
                return Err(execution_binding_error(
                    "execution_continuation_activation_repair_mismatch",
                ));
            }
            validate_continuation_binding(
                &project_state_root,
                &session.id,
                &request.operation_id,
                &binding,
            )?;
            return Ok((
                AgentExecutionContinuationReceipt {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: request.operation_id,
                    outcome: AgentExecutionContinuationOutcome::SuccessorCreated,
                    predecessor_generation_id: Some(existing.predecessor.generation_id.clone()),
                    generation_id: binding.identity.generation_id.clone(),
                    execution_binding: binding.identity.clone(),
                    capability_generation: binding.capability_generation,
                    superseded_execution_binding: Some(ExecutionBindingIdentity {
                        generation_id: existing.predecessor.generation_id,
                        binding_id: existing.predecessor.session_binding_id,
                        ledger_head_hash: existing.predecessor_generation_content_hash,
                    }),
                    takeover_audit_id: Some(existing.request.operation_id),
                    validated: true,
                },
                binding,
            ));
        }
    }

    if !exact_unbound
        && record.status == crate::cli::execution_state::ExecutionControlStatus::Active
        && record.primary_session_id == session.id
    {
        let superseded = session
            .execution_binding
            .as_ref()
            .map(|binding| binding.identity.clone());
        let binding = rebind_session_to_current_execution(&worktree, owner, &session, None)?;
        validate_continuation_binding(
            &project_state_root,
            &session.id,
            &request.operation_id,
            &binding,
        )?;
        crate::cli::execution_state::record_rebound_continuation_validation(
            &worktree,
            owner,
            &request.operation_id,
            &session.id,
            &binding,
        )
        .map_err(|_| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::TransactionConflict,
                "rebound continuation validation could not be durably recorded",
            )
        })?;
        return Ok((
            AgentExecutionContinuationReceipt {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: request.operation_id,
                outcome: AgentExecutionContinuationOutcome::ReboundCurrent,
                predecessor_generation_id: None,
                generation_id: binding.identity.generation_id.clone(),
                execution_binding: binding.identity.clone(),
                capability_generation: binding.capability_generation,
                superseded_execution_binding: superseded
                    .filter(|identity| identity != &binding.identity),
                takeover_audit_id: None,
                validated: true,
            },
            binding,
        ));
    }
    if record.status == crate::cli::execution_state::ExecutionControlStatus::Blocked {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::RelaunchRequired,
            "Blocked execution must be recovered with execution.reopen before continuing",
        ));
    }

    let existing = crate::cli::execution_state::continuation_attempt_for_operation(
        &worktree,
        owner,
        &request.operation_id,
    )
    .map_err(|_| execution_binding_error("execution_continuation_attempt_unreadable"))?;
    let successor_request = existing.map_or_else(
        || crate::cli::execution_state::SuccessorRequest {
            operation_id: request.operation_id.clone(),
            principal_id: "gwt-host-continuation".to_string(),
            work_id: None,
            source: "execution-continue".to_string(),
            session_binding_id: format!("execution-continue-{}", request.operation_id),
            initial_session_id: session.id.clone(),
            entrypoint: "execution.continue".to_string(),
            requested_at: Utc::now(),
        },
        |attempt| attempt.request,
    );
    let predecessor_binding = current_binding;
    let predecessor_status = match record.status {
        crate::cli::execution_state::ExecutionControlStatus::Active => {
            crate::cli::execution_state::SuccessorPredecessorStatus::Active
        }
        crate::cli::execution_state::ExecutionControlStatus::Completed => {
            crate::cli::execution_state::SuccessorPredecessorStatus::Completed
        }
        crate::cli::execution_state::ExecutionControlStatus::Blocked => unreachable!(),
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (_, binding) = crate::cli::execution_state::activate_successor_with_session_rebind(
        &worktree,
        owner,
        &successor_request,
        predecessor_status,
        &predecessor_binding,
        &sessions_dir,
        &session,
    )
    .map_err(|_| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::TransactionConflict,
            "continuation successor activation lost a concurrent authority race",
        )
    })?
    .ok_or_else(|| {
        AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::TransactionConflict,
            "durable Session changed before continuation activation",
        )
    })?;
    validate_continuation_binding(
        &project_state_root,
        &session.id,
        &request.operation_id,
        &binding,
    )?;
    Ok((
        AgentExecutionContinuationReceipt {
            schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: request.operation_id,
            outcome: AgentExecutionContinuationOutcome::SuccessorCreated,
            predecessor_generation_id: Some(predecessor_binding.generation_id.clone()),
            generation_id: binding.identity.generation_id.clone(),
            execution_binding: binding.identity.clone(),
            capability_generation: binding.capability_generation,
            superseded_execution_binding: Some(predecessor_binding),
            takeover_audit_id: Some(successor_request.operation_id),
            validated: true,
        },
        binding,
    ))
}

fn rebind_session_to_current_execution(
    worktree: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    expected_session: &Session,
    expected_generation_id: Option<&str>,
) -> std::result::Result<SessionExecutionBinding, AgentWorkspaceUpdateError> {
    let identity = crate::cli::execution_state::current_execution_binding(worktree, owner)
        .map_err(|_| execution_binding_error("execution_continuation_binding_unreadable"))?
        .ok_or_else(|| execution_binding_error("execution_continuation_binding_missing"))?;
    if expected_generation_id.is_some_and(|expected| expected != identity.generation_id) {
        return Err(execution_binding_error(
            "execution_continuation_generation_mismatch",
        ));
    }
    let repo_hash = expected_session.repo_hash.clone().ok_or_else(|| {
        execution_binding_error("execution_continuation_session_repo_hash_missing")
    })?;
    let mut updated = expected_session.clone();
    let capability_generation = updated.execution_binding.as_ref().map_or(1, |binding| {
        if binding.identity == identity {
            binding.capability_generation
        } else {
            binding.capability_generation.saturating_add(1)
        }
    });
    let binding = SessionExecutionBinding {
        schema_version: SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: updated.id.clone(),
        repo_hash,
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation,
    };
    if updated.execution_binding.as_ref() != Some(&binding) {
        updated
            .set_execution_binding(Some(binding.clone()))
            .map_err(|_| execution_binding_error("execution_continuation_rebind_invalid"))?;
        if !updated
            .save_if_unchanged(&gwt_core::paths::gwt_sessions_dir(), expected_session)
            .map_err(|_| execution_binding_error("execution_continuation_rebind_failed"))?
        {
            return Err(AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::TransactionConflict,
                "durable Session changed before continuation rebind",
            ));
        }
    }
    Ok(binding)
}

fn validate_continuation_binding(
    project_state_root: &Path,
    session_id: &str,
    operation_id: &str,
    binding: &SessionExecutionBinding,
) -> std::result::Result<(), AgentWorkspaceUpdateError> {
    let receipt = probe_authenticated_execution_binding(
        project_state_root,
        session_id,
        binding,
        "execution-continuation",
        AgentExecutionBindingProbeRequest {
            schema_version: AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: operation_id.to_string(),
            nonce: operation_id.to_string(),
        },
    )?;
    if receipt.execution_binding != binding.identity
        || receipt.capability_generation != binding.capability_generation
    {
        return Err(execution_binding_error(
            "execution_continuation_readback_mismatch",
        ));
    }
    Ok(())
}

fn validate_ephemeral_probe_identifier(
    value: &str,
    identity: &str,
) -> std::result::Result<(), AgentWorkspaceUpdateError> {
    const MAX_EPHEMERAL_IDENTIFIER_BYTES: usize = 512;
    if value.trim().is_empty()
        || value.trim() != value
        || value.len() > MAX_EPHEMERAL_IDENTIFIER_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::InvalidRequest,
            format!("execution binding probe {identity} is invalid"),
        ));
    }
    Ok(())
}

fn validate_current_execution_binding_authority(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
) -> std::result::Result<SessionExecutionBinding, AgentWorkspaceUpdateError> {
    let (validated, worktree, owner) = validate_execution_binding_authority_structure(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
    )?;
    let current = crate::cli::execution_state::current_active_execution_binding_matches(
        &worktree,
        owner,
        authenticated_session_id,
        &validated.identity,
    )
    .map_err(|_| execution_binding_error("active_execution_state_unreadable"))?;
    if !current {
        return Err(execution_binding_error(
            "active_execution_binding_not_current",
        ));
    }
    Ok(validated)
}

fn validate_prepared_execution_binding_authority(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
) -> std::result::Result<SessionExecutionBinding, AgentWorkspaceUpdateError> {
    let (validated, worktree, owner) = validate_execution_binding_authority_structure(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
    )?;
    let prepared = crate::cli::execution_state::prepared_execution_binding_matches(
        &worktree,
        owner,
        authenticated_session_id,
        &validated.identity,
    )
    .map_err(|_| execution_binding_error("prepared_execution_state_unreadable"))?;
    if !prepared {
        return Err(execution_binding_error(
            "prepared_execution_binding_not_current",
        ));
    }
    Ok(validated)
}

fn validate_execution_binding_authority_structure(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
) -> std::result::Result<
    (
        SessionExecutionBinding,
        PathBuf,
        crate::cli::execution_state::ExecutionOwnerKey,
    ),
    AgentWorkspaceUpdateError,
> {
    validate_mutation_session_id(authenticated_session_id)?;
    let session = load_session_for_mutation(authenticated_session_id)
        .map_err(|_| execution_binding_error("session_ledger_unavailable"))?;
    if session.id != authenticated_session_id
        || authenticated_binding.schema_version != SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || authenticated_binding.session_id != authenticated_session_id
        || session.repo_hash.as_deref() != Some(authenticated_binding.repo_hash.as_str())
        || session.linked_issue_number != Some(authenticated_binding.owner_number)
        || session.execution_binding.as_ref() != Some(authenticated_binding)
    {
        return Err(execution_binding_error("session_binding_identity_mismatch"));
    }

    let mut structural_validation = session.clone();
    structural_validation.execution_binding = None;
    structural_validation
        .set_execution_binding(Some(authenticated_binding.clone()))
        .map_err(|_| execution_binding_error("session_binding_structure_invalid"))?;

    let principal_root =
        canonicalize_mutation_path(authenticated_project_root, "authenticated project")
            .map_err(|_| execution_binding_error("authenticated_project_root_invalid"))?;
    let project_state_root = strict_project_state_root(&session)
        .and_then(|root| canonicalize_mutation_path(&root, "canonical repository"))
        .map_err(|_| execution_binding_error("project_state_root_invalid"))?;
    if principal_root != project_state_root {
        return Err(execution_binding_error(
            "authenticated_project_root_mismatch",
        ));
    }

    let worktree = canonicalize_mutation_path(&session.worktree_path, "worktree")
        .map_err(|_| execution_binding_error("session_worktree_invalid"))?;
    let worktree_git_root = git_toplevel(&worktree, "worktree")
        .map_err(|_| execution_binding_error("session_worktree_not_git_toplevel"))?;
    if worktree != worktree_git_root
        || repo_hash_for_mutation(&worktree_git_root, "repo hash")
            .map_err(|_| execution_binding_error("session_worktree_repo_hash_unavailable"))?
            != authenticated_binding.repo_hash
    {
        return Err(execution_binding_error(
            "session_worktree_repository_mismatch",
        ));
    }
    validate_visible_project_state_root(
        &project_state_root,
        &authenticated_binding.repo_hash,
        authenticated_session_id,
    )
    .map_err(|_| execution_binding_error("project_state_anchor_invalid"))?;

    let owner_kind = match authenticated_binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err(execution_binding_error("execution_owner_kind_invalid")),
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: authenticated_binding.owner_number,
    };
    Ok((authenticated_binding.clone(), worktree, owner))
}

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
    apply_authenticated_workspace_update_inner(
        authenticated_project_root,
        authenticated_session_id,
        request,
        |worktree, session_id| {
            crate::cli::verification_record::save_work_event_settlement_record(
                worktree, session_id, true,
            )
            .map(|_| ())
        },
    )
}

pub fn apply_bound_authenticated_workspace_update(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    request: AgentWorkspaceUpdateRequest,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    apply_bound_authenticated_workspace_update_inner(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
        None,
        request,
        |_| {},
        |worktree, session_id| {
            crate::cli::verification_record::save_work_event_settlement_record(
                worktree, session_id, true,
            )
            .map(|_| ())
        },
    )
}

#[cfg(test)]
pub(crate) fn apply_bound_authenticated_workspace_update_for_exact_work(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    authenticated_work_id: &str,
    request: AgentWorkspaceUpdateRequest,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    apply_bound_authenticated_workspace_update_inner(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
        Some(authenticated_work_id),
        request,
        |_| {},
        |worktree, session_id| {
            crate::cli::verification_record::save_work_event_settlement_record(
                worktree, session_id, true,
            )
            .map(|_| ())
        },
    )
}

pub(crate) fn apply_bound_authenticated_workspace_update_for_exact_work_with_held_global_lease(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    authenticated_work_id: &str,
    settlement_trusted_dir: &Path,
    request: AgentWorkspaceUpdateRequest,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    apply_authenticated_workspace_update_with_binding(
        authenticated_project_root,
        authenticated_session_id,
        Some(authenticated_binding),
        Some(authenticated_work_id),
        request,
        |_| {},
        WorkspaceUpdateSettlementHooks {
            held_global_trusted_dir: Some(settlement_trusted_dir),
            refresh: skip_workspace_update_settlement_refresh,
        },
    )
}

fn skip_workspace_update_settlement_refresh(
    _worktree: &Path,
    _session_id: &str,
) -> std::io::Result<()> {
    Ok(())
}

fn apply_authenticated_workspace_update_inner(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    request: AgentWorkspaceUpdateRequest,
    refresh_settlement: impl FnOnce(&Path, &str) -> std::io::Result<()>,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    apply_authenticated_workspace_update_with_binding(
        authenticated_project_root,
        authenticated_session_id,
        None,
        None,
        request,
        |_| {},
        WorkspaceUpdateSettlementHooks {
            held_global_trusted_dir: None,
            refresh: refresh_settlement,
        },
    )
}

fn apply_bound_authenticated_workspace_update_inner(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    authenticated_work_id: Option<&str>,
    request: AgentWorkspaceUpdateRequest,
    after_resolve: impl FnOnce(&SessionWorkMutationTarget),
    refresh_settlement: impl FnOnce(&Path, &str) -> std::io::Result<()>,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError> {
    apply_authenticated_workspace_update_with_binding(
        authenticated_project_root,
        authenticated_session_id,
        Some(authenticated_binding),
        authenticated_work_id,
        request,
        after_resolve,
        WorkspaceUpdateSettlementHooks {
            held_global_trusted_dir: None,
            refresh: refresh_settlement,
        },
    )
}

struct WorkspaceUpdateSettlementHooks<'a, Refresh> {
    held_global_trusted_dir: Option<&'a Path>,
    refresh: Refresh,
}

fn apply_authenticated_workspace_update_with_binding<Refresh>(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: Option<&SessionExecutionBinding>,
    authenticated_work_id: Option<&str>,
    request: AgentWorkspaceUpdateRequest,
    after_resolve: impl FnOnce(&SessionWorkMutationTarget),
    settlement_hooks: WorkspaceUpdateSettlementHooks<'_, Refresh>,
) -> std::result::Result<AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateError>
where
    Refresh: FnOnce(&Path, &str) -> std::io::Result<()>,
{
    let WorkspaceUpdateSettlementHooks {
        held_global_trusted_dir,
        refresh: refresh_settlement,
    } = settlement_hooks;
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
    if let Some(binding) = authenticated_binding {
        validate_current_execution_binding_authority(
            authenticated_project_root,
            authenticated_session_id,
            binding,
        )?;
    }

    let observation = request.observation.clone();
    let target = resolve_authenticated_session_work_mutation_target(
        authenticated_project_root,
        authenticated_session_id,
        &observation,
        authenticated_work_id.is_some(),
    )?;
    if authenticated_work_id.is_some_and(|work_id| work_id != target.work_id) {
        return Err(AgentWorkspaceUpdateError::new(
            AgentWorkspaceUpdateErrorCode::IdentityConflict,
            "workspace.update canonical Work changed after the compatibility authority snapshot",
        ));
    }
    after_resolve(&target);
    let tracked_event_policy = if crate::cli::execution_state::is_completed(&target.work_event_root)
    {
        TrackedWorkEventPolicy::SkipTracked
    } else {
        TrackedWorkEventPolicy::Persist
    };
    let opens_work_settlement = tracked_event_policy == TrackedWorkEventPolicy::Persist
        && request.intent.status_category
            == Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
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
    let transaction = AuthenticatedWorkspaceUpdateTransaction {
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
        authenticated_work_id,
        observation: &observation,
        target: &target,
        tracked_event_policy,
        opens_work_settlement,
    };
    let persisted = if !opens_work_settlement {
        persist_authenticated_workspace_update(&transaction, update, None)?
    } else if let Some(trusted_dir) = held_global_trusted_dir {
        persist_authenticated_workspace_update(&transaction, update, Some(trusted_dir))?
    } else {
        let trusted_dir =
            crate::cli::trusted_store::trusted_dir_for_worktree(&target.work_event_root)
                .ok_or_else(|| {
                    AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host could not resolve the terminal Work event settlement store before mutation",
            )
                })?;
        let nested = crate::cli::trusted_store::with_write_lease_for_resolved_dir(
            &trusted_dir,
            || -> std::io::Result<_> {
                Ok(persist_authenticated_workspace_update(
                    &transaction,
                    update,
                    Some(&trusted_dir),
                ))
            },
        )
        .map_err(|_| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host could not acquire the terminal Work event settlement lease before mutation",
            )
        })?;
        nested?
    };
    if opens_work_settlement {
        if let Err(error) = refresh_settlement(&target.work_event_root, &target.session_id) {
            tracing::warn!(
                ?error,
                "terminal Work event persisted; retaining the write-ahead settlement receipt after refresh failure"
            );
        }
    }
    Ok(AgentWorkspaceUpdateReceipt {
        schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
        work_id: target.work_id,
        journal_entry_id: persisted.receipt_evidence_id,
    })
}

struct AuthenticatedWorkspaceUpdateTransaction<'a> {
    authenticated_project_root: &'a Path,
    authenticated_session_id: &'a str,
    authenticated_binding: Option<&'a SessionExecutionBinding>,
    authenticated_work_id: Option<&'a str>,
    observation: &'a AgentRuntimeObservation,
    target: &'a SessionWorkMutationTarget,
    tracked_event_policy: TrackedWorkEventPolicy,
    opens_work_settlement: bool,
}

struct PersistedAuthenticatedWorkspaceUpdate {
    receipt_evidence_id: String,
}

fn persist_authenticated_workspace_update(
    transaction: &AuthenticatedWorkspaceUpdateTransaction<'_>,
    update: WorkspaceProjectionUpdate,
    settlement_trusted_dir: Option<&Path>,
) -> std::result::Result<PersistedAuthenticatedWorkspaceUpdate, AgentWorkspaceUpdateError> {
    let persistence_target = transaction.target.persistence_target();
    let mut revalidation_error_code = None;
    let mut settlement_prepare_failed = false;
    let mut target_was_current = false;
    let mut work_event_id = None;
    let journal_entry = update_workspace_projection_with_journal_for_resolved_work_target(
        &persistence_target,
        update,
        transaction.tracked_event_policy,
        |projection, _| {
            target_was_current = projection.id == transaction.target.work_id;
            if let Some(binding) = transaction.authenticated_binding {
                validate_current_execution_binding_authority(
                    transaction.authenticated_project_root,
                    transaction.authenticated_session_id,
                    binding,
                )
                .map_err(|error| {
                    revalidation_error_code = Some(error.code);
                    GwtError::Other(
                        "authenticated execution binding revalidation failed".to_string(),
                    )
                })?;
            }
            let refreshed = resolve_authenticated_session_work_mutation_target(
                transaction.authenticated_project_root,
                transaction.authenticated_session_id,
                transaction.observation,
                transaction.authenticated_work_id.is_some(),
            )
            .map_err(|error| {
                revalidation_error_code = Some(error.code);
                GwtError::Other(
                    "authenticated Session-bound workspace target revalidation failed".to_string(),
                )
            })?;
            if refreshed != *transaction.target {
                return Err(GwtError::Other(
                    "authenticated Session-bound workspace target changed before commit"
                        .to_string(),
                ));
            }
            Ok(())
        },
        |event, journal_entry| {
            work_event_id = Some(event.id.clone());
            if !transaction.opens_work_settlement {
                return Ok(());
            }
            let trusted_dir = settlement_trusted_dir.ok_or_else(|| {
                settlement_prepare_failed = true;
                GwtError::Other(
                    "Host terminal Work event settlement lease is missing".to_string(),
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
                settlement_prepare_failed = true;
                GwtError::Other(format!(
                    "Host could not reserve the terminal Work event settlement obligation: {error}"
                ))
            })
        },
    )
    .map_err(|error| {
        if settlement_prepare_failed {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host could not reserve the terminal Work event settlement obligation before mutation",
            )
        } else {
            revalidation_error_code.map_or_else(
                || classify_workspace_transaction_error(&error),
                workspace_revalidation_error,
            )
        }
    })?;
    let receipt_evidence_id = if target_was_current {
        journal_entry.id
    } else {
        work_event_id.ok_or_else(|| {
            AgentWorkspaceUpdateError::new(
                AgentWorkspaceUpdateErrorCode::Internal,
                "Host workspace transaction committed without durable Work event evidence",
            )
        })?
    };
    Ok(PersistedAuthenticatedWorkspaceUpdate {
        receipt_evidence_id,
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

pub fn apply_bound_authenticated_work_terminalization(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    request: AgentWorkTerminalizationRequest,
) -> std::result::Result<AgentWorkTerminalizationReceipt, AgentWorkspaceUpdateError> {
    apply_bound_authenticated_work_terminalization_inner(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
        None,
        request,
        |_| {},
    )
}

pub(crate) fn apply_bound_authenticated_work_terminalization_for_exact_work(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    expected_work_id: &str,
    policy: gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy,
    request: AgentWorkTerminalizationRequest,
) -> std::result::Result<AgentWorkTerminalizationReceipt, AgentWorkspaceUpdateError> {
    apply_bound_authenticated_work_terminalization_inner(
        authenticated_project_root,
        authenticated_session_id,
        authenticated_binding,
        Some((expected_work_id, policy)),
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
    apply_authenticated_work_terminalization_with_binding(
        authenticated_project_root,
        authenticated_session_id,
        None,
        None,
        request,
        after_resolve,
    )
}

fn apply_bound_authenticated_work_terminalization_inner(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: &SessionExecutionBinding,
    exact_work: Option<(
        &str,
        gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy,
    )>,
    request: AgentWorkTerminalizationRequest,
    after_resolve: impl FnOnce(&SessionBoundWorkspaceTerminalTarget),
) -> std::result::Result<AgentWorkTerminalizationReceipt, AgentWorkspaceUpdateError> {
    apply_authenticated_work_terminalization_with_binding(
        authenticated_project_root,
        authenticated_session_id,
        Some(authenticated_binding),
        exact_work,
        request,
        after_resolve,
    )
}

fn apply_authenticated_work_terminalization_with_binding(
    authenticated_project_root: &Path,
    authenticated_session_id: &str,
    authenticated_binding: Option<&SessionExecutionBinding>,
    exact_work: Option<(
        &str,
        gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy,
    )>,
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
    if let Some(binding) = authenticated_binding {
        validate_current_execution_binding_authority(
            authenticated_project_root,
            authenticated_session_id,
            binding,
        )?;
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
    let revalidate =
        |_: &gwt_core::workspace_projection::WorkspaceProjection,
         _: &gwt_core::workspace_projection::WorkItemsProjection| {
            if let Some(binding) = authenticated_binding {
                validate_current_execution_binding_authority(
                    authenticated_project_root,
                    authenticated_session_id,
                    binding,
                )
                .map_err(|error| {
                    revalidation_error_code = Some(error.code);
                    GwtError::Other(
                        "authenticated execution binding revalidation failed".to_string(),
                    )
                })?;
            }
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
                revalidation_error_code = Some(AgentWorkspaceUpdateErrorCode::TransactionConflict);
                return Err(GwtError::Other(
                    "authenticated Session-bound Work terminalization target changed before commit"
                        .to_string(),
                ));
            }
            Ok(())
        };
    let outcome = match exact_work {
        Some((expected_work_id, policy)) => gwt_core::workspace_projection::emit_workspace_terminal_event_for_exact_resolved_work_target(
            &target,
            expected_work_id,
            close_kind,
            policy,
            Utc::now(),
            revalidate,
        ),
        None => gwt_core::workspace_projection::emit_workspace_terminal_event_for_resolved_work_target(
            &target,
            close_kind,
            Utc::now(),
            revalidate,
        ),
    }
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
    require_single_session_assignment: bool,
) -> std::result::Result<SessionWorkMutationTarget, AgentWorkspaceUpdateError> {
    let authority = resolve_authenticated_session_terminal_target(
        authenticated_project_root,
        session_id,
        observation,
    )?;
    let work_id = resolve_unique_existing_work_id(
        &authority.project_state_root,
        &authority.work_event_root,
        session_id,
        &authority.branch_identity,
        &authority.worktree_identity,
        SessionWorkAuthorityExpectation {
            owner: &authority.owner,
            agent_id: &authority.agent_id,
            require_single_session_assignment,
            allow_terminal: false,
        },
    )
    .map_err(classify_target_error)?;
    Ok(SessionWorkMutationTarget {
        project_state_root: authority.project_state_root,
        work_event_root: authority.work_event_root,
        session_id: authority.session_id,
        branch_identity: authority.branch_identity,
        worktree_identity: authority.worktree_identity,
        work_id,
        owner: authority.owner,
        agent_id: authority.agent_id,
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
    let (owner, agent_id) =
        durable_session_work_authority(&session).map_err(classify_target_error)?;
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
        owner,
        agent_id,
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

fn execution_binding_error(reason: &'static str) -> AgentWorkspaceUpdateError {
    tracing::warn!(
        reason,
        "authenticated Host operation rejected an execution binding"
    );
    AgentWorkspaceUpdateError::new(
        AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
        "Execution binding is missing, stale, or no longer current; relaunch the Session before retrying",
    )
}

fn workspace_revalidation_error(code: AgentWorkspaceUpdateErrorCode) -> AgentWorkspaceUpdateError {
    match code {
        AgentWorkspaceUpdateErrorCode::RelaunchRequired => relaunch_required_error(),
        AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            execution_binding_error("workspace_revalidation_binding_mismatch")
        }
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
    pub(crate) owner: String,
    pub(crate) agent_id: String,
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
            owner: self.owner.clone(),
            agent_id: self.agent_id.clone(),
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

pub(crate) struct ValidatedWorkspaceRecoverySession {
    pub(crate) session: Session,
    pub(crate) project_state_root: PathBuf,
    pub(crate) work_event_root: PathBuf,
    pub(crate) branch_identity: String,
    pub(crate) worktree_identity: PathBuf,
}

pub(crate) enum ValidatedWorkspaceEnsureSession {
    Host(ValidatedWorkspaceRecoverySession),
    Docker(ValidatedWorkspaceRecoverySession),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundTerminalCompatibilityDisposition {
    EmitIfNeeded,
    ConfirmOnly,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BoundTerminalCompatibilityAuthority {
    identity: gwt_agent::SessionExecutionIdentity,
    project_state_root: PathBuf,
    work_id: String,
    requested_terminal: AgentWorkTerminalKind,
    disposition: BoundTerminalCompatibilityDisposition,
}

/// Snapshot the exact canonical Work authority before an authenticated Host
/// terminalization request. Only a nonterminal Work or the already-requested
/// terminal is eligible; opposite and ambiguous canonical terminals fail
/// before a rolling-version compatibility continuation can be considered.
pub(crate) fn snapshot_bound_terminal_compatibility_authority(
    invocation_cwd: &Path,
    session_id: &str,
    requested_terminal: AgentWorkTerminalKind,
) -> Result<Option<BoundTerminalCompatibilityAuthority>> {
    let Some(recovery) = validated_workspace_recovery_session(invocation_cwd, session_id)? else {
        return Ok(None);
    };
    let ValidatedWorkspaceEnsureSession::Host(recovery) = recovery else {
        return Ok(None);
    };
    if recovery.session.runtime_target != LaunchRuntimeTarget::Host
        || recovery.session.docker_runtime_binding.is_some()
    {
        return Err(workspace_ensure_error(
            session_id,
            "durable Host Session has a stale or foreign runtime binding",
        ));
    }
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&recovery.session)
        .map_err(mutation_error)?
        .ok_or_else(|| {
            mutation_error(format!(
                "durable Session {session_id} has no execution binding"
            ))
        })?;
    let (owner, agent_id) = durable_session_work_authority(&recovery.session)?;
    let resolved_work = resolve_unique_existing_work(
        &recovery.project_state_root,
        &recovery.work_event_root,
        session_id,
        &recovery.branch_identity,
        &recovery.worktree_identity,
        SessionWorkAuthorityExpectation {
            owner: &owner,
            agent_id: &agent_id,
            require_single_session_assignment: true,
            allow_terminal: true,
        },
    )?;
    if resolved_work.done && resolved_work.discarded {
        return Err(workspace_ensure_error(
            session_id,
            "canonical Work has ambiguous Done and Discarded terminal state",
        ));
    }
    let requested_matches = match requested_terminal {
        AgentWorkTerminalKind::Done => resolved_work.done && !resolved_work.discarded,
        AgentWorkTerminalKind::Discarded => resolved_work.discarded,
    };
    let disposition = if requested_matches {
        BoundTerminalCompatibilityDisposition::ConfirmOnly
    } else if resolved_work.is_terminal {
        return Err(workspace_ensure_error(
            session_id,
            "canonical Work has the opposite terminal state",
        ));
    } else {
        BoundTerminalCompatibilityDisposition::EmitIfNeeded
    };
    Ok(Some(BoundTerminalCompatibilityAuthority {
        identity,
        project_state_root: recovery.project_state_root,
        work_id: resolved_work.work_id,
        requested_terminal,
        disposition,
    }))
}

pub(crate) fn confirm_bound_terminal_compatibility_authority(
    authority: &BoundTerminalCompatibilityAuthority,
    request: AgentWorkTerminalizationRequest,
) -> Result<()> {
    let mut confirmation = authority.clone();
    confirmation.disposition = BoundTerminalCompatibilityDisposition::ConfirmOnly;
    let receipt =
        continue_bound_terminal_compatibility(&confirmation, request).map_err(mutation_error)?;
    if receipt.outcome != AgentWorkTerminalizationOutcome::AlreadyMatching {
        return Err(mutation_error(
            "canonical Work terminal readback does not match the pre-request authority",
        ));
    }
    Ok(())
}

pub(crate) fn continue_bound_terminal_compatibility(
    authority: &BoundTerminalCompatibilityAuthority,
    request: AgentWorkTerminalizationRequest,
) -> std::result::Result<AgentWorkTerminalizationReceipt, String> {
    if request.claimed_session_id != authority.identity.session_id
        || request.terminal_kind != authority.requested_terminal
    {
        return Err(
            "terminal compatibility continuation request does not match its authority snapshot"
                .to_string(),
        );
    }
    let expected_identity = authority.identity.clone();
    let project_state_root = authority.project_state_root.clone();
    let work_id = authority.work_id.clone();
    let policy = match authority.disposition {
        BoundTerminalCompatibilityDisposition::EmitIfNeeded => {
            gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy::EmitIfNeeded
        }
        BoundTerminalCompatibilityDisposition::ConfirmOnly => {
            gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy::ConfirmOnly
        }
    };
    let session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", expected_identity.session_id));
    let result =
        crate::cli::execution_state::with_current_active_session_execution_identity_global_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            &expected_identity,
            |_| -> std::result::Result<AgentWorkTerminalizationReceipt, String> {
                let current = Session::load(&session_path).map_err(|_| {
                    "terminal compatibility continuation could not reload the durable Session"
                        .to_string()
                })?;
                if current.runtime_target != LaunchRuntimeTarget::Host
                    || current.docker_runtime_binding.is_some()
                    || gwt_agent::SessionExecutionIdentity::from_session(&current)
                        .ok()
                        .flatten()
                        .as_ref()
                        != Some(&expected_identity)
                {
                    return Err(
                        "terminal compatibility continuation authority changed before commit"
                            .to_string(),
                    );
                }
                apply_bound_authenticated_work_terminalization_for_exact_work(
                    &project_state_root,
                    &current.id,
                    &expected_identity.execution_binding,
                    &work_id,
                    policy,
                    request,
                )
                .map_err(|error| {
                    format!("terminal compatibility continuation was refused: {error}")
                })
            },
        )
        .map_err(|_| {
            "terminal compatibility continuation could not validate the durable authority"
                .to_string()
        })?;
    let receipt = result.ok_or_else(|| {
        "terminal compatibility continuation authority changed before commit".to_string()
    })??;
    match receipt.outcome {
        AgentWorkTerminalizationOutcome::Emitted
        | AgentWorkTerminalizationOutcome::AlreadyMatching => Ok(receipt),
        _ => Err(
            "terminal compatibility continuation did not reach the requested canonical terminal"
                .to_string(),
        ),
    }
}

/// Load the exact durable Session identity used to recover a missing Work
/// projection registration. Recovery intentionally stops before resolving a
/// Work id: `workspace.ensure` is the operation that materializes that missing
/// assignment. All Session/repository/container checks remain identical to the
/// authenticated local `workspace.update` path.
pub(crate) fn validated_workspace_recovery_session(
    invocation_cwd: &Path,
    session_id: &str,
) -> Result<Option<ValidatedWorkspaceEnsureSession>> {
    gwt_agent::validate_session_id_path_component(session_id)
        .map_err(|error| mutation_error(format!("invalid or unsafe Session id: {error}")))?;
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    if !session_path.try_exists().map_err(|error| {
        mutation_error(format!(
            "failed to inspect Session ledger for Session {session_id} at {}: {error}",
            session_path.display()
        ))
    })? {
        return Ok(None);
    }
    let recovery_context = resolve_execution_recovery_context(invocation_cwd, session_id)?;
    let session = load_session_for_mutation(session_id)?;
    if session.id != session_id {
        return Err(mutation_error(format!(
            "Session ledger id mismatch: requested {session_id}, loaded {}",
            session.id
        )));
    }
    let _exact_session = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .map_err(|error| {
            mutation_error(format!(
                "invalid durable Session execution binding for Session {session_id}: {error}"
            ))
        })?
        .ok_or_else(|| {
            mutation_error(format!(
                "durable Session {session_id} has no execution binding"
            ))
        })?;
    let binding = session.execution_binding.as_ref().ok_or_else(|| {
        mutation_error(format!(
            "durable Session {session_id} has no execution binding"
        ))
    })?;
    let identity = validate_host_session_identity(recovery_context.worktree(), &session)?;
    validate_current_execution_binding_authority(&identity.project_state_root, session_id, binding)
        .map_err(|error| {
            mutation_error(format!(
            "durable Session execution binding is not current for Session {session_id}: {error}"
        ))
        })?;
    if session.runtime_target == LaunchRuntimeTarget::Docker {
        return Ok(Some(ValidatedWorkspaceEnsureSession::Docker(
            ValidatedWorkspaceRecoverySession {
                session,
                project_state_root: identity.project_state_root,
                work_event_root: identity.work_event_root,
                branch_identity: identity.branch_identity,
                worktree_identity: identity.worktree_identity,
            },
        )));
    }
    Ok(Some(ValidatedWorkspaceEnsureSession::Host(
        ValidatedWorkspaceRecoverySession {
            session,
            project_state_root: identity.project_state_root,
            work_event_root: identity.work_event_root,
            branch_identity: identity.branch_identity,
            worktree_identity: identity.worktree_identity,
        },
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceUpdateApplicabilityReason {
    InvalidSession,
    SessionUnavailable,
    HostBridgeRequired,
    CwdMismatch,
    RepositoryMismatch,
    BranchMismatch,
    WorkspaceEnsureRequired,
    AuthorityUnknown,
}

impl WorkspaceUpdateApplicabilityReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSession => "invalid_session",
            Self::SessionUnavailable => "session_unavailable",
            Self::HostBridgeRequired => "host_bridge_required",
            Self::CwdMismatch => "cwd_mismatch",
            Self::RepositoryMismatch => "repository_mismatch",
            Self::BranchMismatch => "branch_mismatch",
            Self::WorkspaceEnsureRequired => "workspace_ensure_required",
            Self::AuthorityUnknown => "authority_unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceUpdateApplicabilityFailure {
    pub(crate) reason: WorkspaceUpdateApplicabilityReason,
    pub(crate) message: String,
}

/// Read-only classification for the same target resolver used by
/// `workspace.update`. Stable reasons let `execution.status` distinguish
/// provenance failures without weakening the mutation authority check.
pub(crate) fn diagnose_session_work_mutation_target(
    invocation_cwd: &Path,
    session_id: &str,
) -> std::result::Result<(), WorkspaceUpdateApplicabilityFailure> {
    resolve_session_work_mutation_target(invocation_cwd, session_id)
        .map(|_| ())
        .map_err(|error| {
            let message = error.to_string();
            let normalized = message.to_ascii_lowercase();
            let reason = if normalized.contains("invalid or unsafe session id") {
                WorkspaceUpdateApplicabilityReason::InvalidSession
            } else if normalized.contains("session ledger")
                || normalized.contains("ledger id mismatch")
            {
                WorkspaceUpdateApplicabilityReason::SessionUnavailable
            } else if normalized.contains("authenticated host bridge") {
                WorkspaceUpdateApplicabilityReason::HostBridgeRequired
            } else if normalized.contains("session cwd mismatch") {
                WorkspaceUpdateApplicabilityReason::CwdMismatch
            } else if normalized.contains("repo hash mismatch")
                || normalized.contains("canonical repository mismatch")
                || normalized.contains("session worktree mismatch")
                || normalized.contains("event root")
                || normalized.contains("project state")
            {
                WorkspaceUpdateApplicabilityReason::RepositoryMismatch
            } else if normalized.contains("session branch mismatch") {
                WorkspaceUpdateApplicabilityReason::BranchMismatch
            } else if normalized.contains("workspace.ensure") || normalized.contains("work target")
            {
                WorkspaceUpdateApplicabilityReason::WorkspaceEnsureRequired
            } else {
                WorkspaceUpdateApplicabilityReason::AuthorityUnknown
            };
            WorkspaceUpdateApplicabilityFailure { reason, message }
        })
}

pub(crate) fn probe_session_work_mutation_target(
    invocation_cwd: &Path,
    session_id: &str,
) -> crate::cli::governance::RecoveryProbe {
    use crate::cli::governance::{
        GovernanceCause, GovernanceEffect, GovernanceMetadata, RecoveryProbe,
    };
    match diagnose_session_work_mutation_target(invocation_cwd, session_id) {
        Ok(()) => RecoveryProbe::available(
            "workspace.update",
            GovernanceMetadata {
                effect: Some(GovernanceEffect::Reversible),
                retryable: Some(true),
                target_state: Some("workspace_update".to_string()),
                ..GovernanceMetadata::default()
            },
        ),
        Err(failure) => {
            let cause = match failure.reason {
                WorkspaceUpdateApplicabilityReason::WorkspaceEnsureRequired => {
                    GovernanceCause::NotReady
                }
                WorkspaceUpdateApplicabilityReason::InvalidSession
                | WorkspaceUpdateApplicabilityReason::SessionUnavailable => {
                    GovernanceCause::ManagedIdentity
                }
                WorkspaceUpdateApplicabilityReason::HostBridgeRequired
                | WorkspaceUpdateApplicabilityReason::CwdMismatch
                | WorkspaceUpdateApplicabilityReason::RepositoryMismatch
                | WorkspaceUpdateApplicabilityReason::BranchMismatch
                | WorkspaceUpdateApplicabilityReason::AuthorityUnknown => {
                    GovernanceCause::Authority
                }
            };
            RecoveryProbe::unavailable(
                "workspace.update",
                GovernanceMetadata {
                    effect: Some(GovernanceEffect::Reversible),
                    cause: Some(cause),
                    retryable: Some(matches!(cause, GovernanceCause::NotReady)),
                    target_state: Some("workspace_update".to_string()),
                    ..GovernanceMetadata::default()
                },
                failure.message,
            )
        }
    }
}

fn resolve_host_session_work_mutation_target(
    invocation_cwd: &Path,
    session: Session,
) -> Result<SessionWorkMutationTarget> {
    let identity = validate_host_session_identity(invocation_cwd, &session)?;
    let session_id = session.id.as_str();
    let (owner, agent_id) = durable_session_work_authority(&session)?;
    let work_id = resolve_unique_existing_work_id(
        &identity.project_state_root,
        &identity.work_event_root,
        session_id,
        &identity.branch_identity,
        &identity.worktree_identity,
        SessionWorkAuthorityExpectation {
            owner: &owner,
            agent_id: &agent_id,
            require_single_session_assignment: false,
            allow_terminal: false,
        },
    )?;

    Ok(SessionWorkMutationTarget {
        project_state_root: identity.project_state_root,
        work_event_root: identity.work_event_root,
        session_id: session.id,
        branch_identity: identity.branch_identity,
        worktree_identity: identity.worktree_identity,
        work_id,
        owner,
        agent_id,
    })
}

struct ValidatedHostSessionIdentity {
    project_state_root: PathBuf,
    work_event_root: PathBuf,
    branch_identity: String,
    worktree_identity: PathBuf,
}

fn validate_host_session_identity(
    invocation_cwd: &Path,
    session: &Session,
) -> Result<ValidatedHostSessionIdentity> {
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
    let declared_repo_hash = required_session_repo_hash(session)?;
    let observed = repo_hash_for_mutation(&session_git_root, "repo hash")?;
    if observed != declared_repo_hash {
        return Err(mutation_error(format!(
            "Session repo hash mismatch for Session {session_id}: ledger={declared_repo_hash}, worktree={observed}"
        )));
    }

    let configured_project_state_root = strict_project_state_root(session)?;
    let project_state_root =
        canonicalize_mutation_path(&configured_project_state_root, "canonical repository")?;
    let project_anchor =
        validate_visible_project_state_root(&project_state_root, declared_repo_hash, session_id)?;

    let branch_identity = required_session_branch(session)?;
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
        session,
    )?;
    if session_worktree != session_git_root {
        return Err(mutation_error(format!(
            "Session event root mismatch for Session {session_id}: workspace.update must run at the validated Git toplevel"
        )));
    }

    Ok(ValidatedHostSessionIdentity {
        project_state_root,
        work_event_root: invocation_git_root,
        branch_identity,
        worktree_identity: session_worktree,
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

#[doc(hidden)]
pub fn validated_project_state_root_for_session_recovery(session: &Session) -> Result<PathBuf> {
    let declared_repo_hash = required_session_repo_hash(session)?;
    let worktree_root = git_toplevel(&session.worktree_path, "recovery worktree")?;
    let observed_repo_hash = repo_hash_for_mutation(&worktree_root, "recovery worktree")?;
    if observed_repo_hash != declared_repo_hash {
        return Err(mutation_error(format!(
            "Session repo hash mismatch for Session {}: ledger={declared_repo_hash}, runtime={observed_repo_hash}",
            session.id
        )));
    }

    let project_state_root = canonicalize_mutation_path(
        &strict_project_state_root(session)?,
        "recovery Project State root",
    )?;
    let project_anchor =
        validate_visible_project_state_root(&project_state_root, declared_repo_hash, &session.id)?;
    let worktree_anchor = canonical_repository_anchor(&worktree_root).map_err(|error| {
        mutation_error(format!(
            "canonical repository mismatch for Session {}: {error}",
            session.id
        ))
    })?;
    if project_anchor != worktree_anchor {
        return Err(mutation_error(format!(
            "canonical repository mismatch for Session {}: Project State anchor {} does not match worktree anchor {}",
            session.id,
            project_anchor.display(),
            worktree_anchor.display()
        )));
    }

    Ok(normalize_mutation_path(&project_state_root))
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

pub(crate) fn normalize_mutation_path(path: &Path) -> PathBuf {
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
    if git_toplevel(project_state_root, "Project State root")
        .is_ok_and(|worktree_root| worktree_root == project_state_root)
    {
        return Ok(());
    }
    Err(mutation_error(format!(
        "canonical repository mismatch for Session {session_id}: Project State root {} is neither the repository anchor, the parent of its bare common-dir {}, nor a Git worktree toplevel",
        project_state_root.display(),
        project_anchor.display()
    )))
}

pub(crate) fn canonical_branch_identity(branch: &str) -> String {
    let branch = branch.trim();
    let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
    let branch = branch.strip_prefix("refs/remotes/").unwrap_or(branch);
    branch.strip_prefix("origin/").unwrap_or(branch).to_string()
}

fn durable_session_work_authority(session: &Session) -> Result<(String, String)> {
    let owner = if let Some(binding) = session.execution_binding.as_ref() {
        match binding.owner_kind.as_str() {
            "spec" => format!("SPEC-{}", binding.owner_number),
            "issue" => format!("Issue #{}", binding.owner_number),
            _ => {
                return Err(workspace_ensure_error(
                    &session.id,
                    "durable execution owner kind is invalid",
                ))
            }
        }
    } else if let Some(number) = session.linked_issue_number {
        format!("Issue #{number}")
    } else {
        return Err(workspace_ensure_error(
            &session.id,
            "durable Work owner is missing",
        ));
    };
    Ok((owner, session.agent_id.command().to_string()))
}

struct SessionWorkAuthorityExpectation<'a> {
    owner: &'a str,
    agent_id: &'a str,
    require_single_session_assignment: bool,
    allow_terminal: bool,
}

struct ResolvedExistingWorkAuthority {
    work_id: String,
    is_terminal: bool,
    done: bool,
    discarded: bool,
}

fn resolve_unique_existing_work_id(
    work_items_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    branch_identity: &str,
    worktree_identity: &Path,
    expected: SessionWorkAuthorityExpectation<'_>,
) -> Result<String> {
    Ok(resolve_unique_existing_work(
        work_items_root,
        work_event_root,
        session_id,
        branch_identity,
        worktree_identity,
        expected,
    )?
    .work_id)
}

fn resolve_unique_existing_work(
    work_items_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    branch_identity: &str,
    worktree_identity: &Path,
    expected: SessionWorkAuthorityExpectation<'_>,
) -> Result<ResolvedExistingWorkAuthority> {
    let current_path =
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(work_items_root);
    let projection = load_workspace_projection_from_path(&current_path)
        .map_err(|error| {
            workspace_ensure_error(
                session_id,
                &format!("canonical Session assignment cannot be read: {error}"),
            )
        })?
        .ok_or_else(|| {
            workspace_ensure_error(session_id, "canonical Session assignment is missing")
        })?;
    let session_assignments = projection
        .agents
        .iter()
        .filter(|candidate| candidate.session_id == session_id)
        .collect::<Vec<_>>();
    if expected.require_single_session_assignment && session_assignments.len() != 1 {
        return Err(workspace_ensure_error(
            session_id,
            "canonical Session assignment authority is ambiguous",
        ));
    }
    let agent = projection
        .latest_agent_for_session(session_id)
        .ok_or_else(|| {
            workspace_ensure_error(session_id, "canonical Session assignment is missing")
        })?;
    for candidate in session_assignments {
        let same_authority = candidate.agent_id == agent.agent_id
            && candidate.affiliation_status == agent.affiliation_status
            && candidate.workspace_id == agent.workspace_id
            && candidate.branch.as_deref().map(canonical_branch_identity)
                == agent.branch.as_deref().map(canonical_branch_identity)
            && candidate
                .worktree_path
                .as_deref()
                .map(normalize_mutation_path)
                == agent.worktree_path.as_deref().map(normalize_mutation_path);
        if !same_authority {
            return Err(workspace_ensure_error(
                session_id,
                "canonical Session assignment authority is ambiguous",
            ));
        }
    }
    if !agent.is_assigned() {
        return Err(workspace_ensure_error(
            session_id,
            "latest canonical Session assignment is Unassigned",
        ));
    }
    if agent.agent_id != expected.agent_id {
        return Err(workspace_ensure_error(
            session_id,
            "canonical Session agent identity does not match the durable Session",
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
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_items_root);
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
    if item.is_terminal() && !expected.allow_terminal {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} is terminal"),
        ));
    }
    if item.owner.as_deref() != Some(expected.owner) {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} owner does not match durable authority"),
        ));
    }
    let session_refs = item
        .agents
        .iter()
        .filter(|agent| agent.session_id == session_id)
        .collect::<Vec<_>>();
    if session_refs.len() != 1 || session_refs[0].agent_id.as_deref() != Some(expected.agent_id) {
        return Err(workspace_ensure_error(
            session_id,
            &format!("assigned Work {work_id} agent identity is missing, foreign, or ambiguous"),
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
                false,
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
    for other in work_items
        .work_items
        .iter()
        .filter(|other| other.id != work_id)
    {
        if other
            .agents
            .iter()
            .any(|agent| agent.session_id == session_id)
        {
            return Err(workspace_ensure_error(
                session_id,
                &format!(
                    "assigned Work {work_id} is ambiguous with Work {} for the same Session",
                    other.id
                ),
            ));
        }
        if other.is_terminal()
            || other.status_category
                == gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
        {
            continue;
        }
        if other.execution_containers.iter().any(|container| {
            mutation_container_matches(
                container,
                branch_identity,
                worktree_identity,
                work_event_root,
                false,
            )
        }) {
            return Err(workspace_ensure_error(
                session_id,
                &format!(
                    "assigned Work {work_id} execution container is ambiguous with Work {}",
                    other.id
                ),
            ));
        }
    }
    Ok(ResolvedExistingWorkAuthority {
        work_id,
        is_terminal: item.is_terminal(),
        done: item.status_category == gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        discarded: item.discarded,
    })
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
        session.linked_issue_number = Some(2359);
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
        event.owner = session
            .execution_binding
            .as_ref()
            .map(|binding| match binding.owner_kind.as_str() {
                "spec" => format!("SPEC-{}", binding.owner_number),
                _ => format!("Issue #{}", binding.owner_number),
            })
            .or_else(|| {
                session
                    .linked_issue_number
                    .map(|number| format!("Issue #{number}"))
            });
        event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        event.agent_session_id = Some(session.id.clone());
        event.agent_id = Some(session.agent_id.command().to_string());
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
        work_items_root: &Path,
        projection: &gwt_core::workspace_projection::WorkItemsProjection,
    ) {
        let path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_items_root);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &path, projection,
        )
        .expect("save WorkItems projection");
    }

    fn save_mutation_work_items_with_tracked_events(
        work_items_root: &Path,
        work_event_root: &Path,
        projection: &gwt_core::workspace_projection::WorkItemsProjection,
    ) {
        save_mutation_work_items(work_items_root, projection);
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
            project_state_root,
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
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(
                        project_state_root,
                    ),
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

    #[derive(Debug, PartialEq, Eq)]
    struct ExecutionBindingAuthoritySnapshot {
        session: Option<Vec<u8>>,
        execution_control_mirror: Option<Vec<u8>>,
        generation_pointer_mirror: Option<Vec<u8>>,
        trusted_files: Vec<(PathBuf, Vec<u8>)>,
        work: WorkMutationSnapshot,
    }

    impl ExecutionBindingAuthoritySnapshot {
        fn capture(project_state_root: &Path, work_event_root: &Path, session_id: &str) -> Self {
            let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(work_event_root)
                .expect("bound fixture has a trusted-store directory");
            let trusted_root = trusted_dir
                .parent()
                .expect("worktree trusted directory has a repository trusted root");
            Self {
                session: std::fs::read(
                    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
                )
                .ok(),
                execution_control_mirror: std::fs::read(crate::cli::execution_state::state_path(
                    work_event_root,
                ))
                .ok(),
                generation_pointer_mirror: std::fs::read(work_event_root.join(
                    crate::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE,
                ))
                .ok(),
                trusted_files: snapshot_regular_files(trusted_root),
                work: WorkMutationSnapshot::capture(project_state_root, work_event_root),
            }
        }
    }

    fn snapshot_regular_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn visit(root: &Path, current: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries = std::fs::read_dir(current)
                .unwrap_or_else(|error| {
                    panic!("read snapshot directory {}: {error}", current.display())
                })
                .collect::<std::io::Result<Vec<_>>>()
                .expect("read snapshot directory entries");
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                let file_type = entry.file_type().expect("read snapshot file type");
                if file_type.is_dir() {
                    visit(root, &path, files);
                } else if file_type.is_file() {
                    files.push((
                        path.strip_prefix(root)
                            .expect("snapshot path belongs to root")
                            .to_path_buf(),
                        std::fs::read(&path).expect("read trusted-store snapshot file"),
                    ));
                }
            }
        }

        let mut files = Vec::new();
        visit(root, root, &mut files);
        files
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

    fn with_split_root_exact_unbound_fixture(
        test: impl FnOnce(&Path, &Path, &Path, &Path, &Session),
    ) {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace-home");
        let bare_repo = workspace_home.join("gwt.git");
        std::fs::create_dir_all(&workspace_home).expect("workspace home");
        run_git(
            &[
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ],
            temp.path(),
        );
        run_git(
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/split-root-recovery.git",
            ],
            &bare_repo,
        );

        let bootstrap = temp.path().join("bootstrap");
        run_git(
            &["init", bootstrap.to_str().expect("bootstrap path")],
            temp.path(),
        );
        run_git(&["config", "user.email", "test@example.com"], &bootstrap);
        run_git(&["config", "user.name", "Test User"], &bootstrap);
        run_git(&["checkout", "-b", "develop"], &bootstrap);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &bootstrap);
        run_git(
            &[
                "remote",
                "add",
                "origin",
                bare_repo.to_str().expect("bare repo path"),
            ],
            &bootstrap,
        );
        run_git(&["push", "origin", "develop"], &bootstrap);

        let worktree = workspace_home.join("work").join("issue-3415");
        let sibling = workspace_home.join("work").join("issue-sibling");
        std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
            .expect("worktree parent");
        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "work/issue-3415",
                worktree.to_str().expect("worktree path"),
                "develop",
            ],
            &bare_repo,
        );
        run_git(
            &[
                "worktree",
                "add",
                "-b",
                "work/issue-sibling",
                sibling.to_str().expect("sibling path"),
                "develop",
            ],
            &bare_repo,
        );
        let workspace_home = dunce::canonicalize(workspace_home).expect("workspace home");
        let worktree = dunce::canonicalize(worktree).expect("linked worktree");
        let sibling = dunce::canonicalize(sibling).expect("sibling worktree");
        let nested = worktree.join("nested").join("cwd");
        std::fs::create_dir_all(&nested).expect("nested cwd");

        let mut session = Session::new(&worktree, "work/issue-3415", gwt_agent::AgentId::Codex);
        session.id = "split-root-exact-unbound".to_string();
        session.project_state_root = Some(workspace_home.clone());
        session.linked_issue_number = None;
        session.execution_binding = None;
        session.runtime_target = LaunchRuntimeTarget::Host;
        session.docker_runtime_binding = None;
        save_session_fixture(&session);

        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3393,
        };
        crate::cli::execution_state::materialize_at_launch(
            &worktree,
            owner.kind,
            owner.number,
            "split-root-foreign-predecessor",
            "gwt-execute",
            false,
        )
        .expect("materialize split-root predecessor");
        crate::cli::execution_state::ensure_generation_ledger(
            &worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize split-root generation ledger");
        seed_work_mutation_surfaces(&workspace_home, &worktree);
        save_mutation_work_items(
            &workspace_home,
            &gwt_core::workspace_projection::WorkItemsProjection::empty(Utc::now()),
        );

        test(&workspace_home, &worktree, &nested, &sibling, &session);
    }

    fn bind_session_to_current_execution(
        repo: &Path,
        session: &Session,
    ) -> (Session, gwt_agent::SessionExecutionBinding) {
        const OWNER_NUMBER: u64 = 2359;
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: OWNER_NUMBER,
        };
        let mut bound_session = session.clone();
        bound_session.linked_issue_number = Some(OWNER_NUMBER);
        save_session_fixture(&bound_session);
        crate::cli::execution_state::materialize_at_launch(
            repo,
            owner.kind,
            owner.number,
            &bound_session.id,
            "gwt-execute",
            false,
        )
        .expect("materialize execution control record");
        crate::cli::execution_state::ensure_generation_ledger(
            repo,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize owner generation ledger");
        let identity = crate::cli::execution_state::current_execution_binding(repo, owner)
            .expect("read current owner binding")
            .expect("current owner binding");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: bound_session.id.clone(),
            repo_hash: bound_session
                .repo_hash
                .clone()
                .expect("strict fixture repository identity"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        bound_session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind durable Session to current execution");
        save_session_fixture(&bound_session);
        (bound_session, binding)
    }

    fn execution_binding_probe_request(
        operation_id: &str,
        nonce: &str,
    ) -> AgentExecutionBindingProbeRequest {
        AgentExecutionBindingProbeRequest {
            schema_version: AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: operation_id.to_string(),
            nonce: nonce.to_string(),
        }
    }

    fn bound_workspace_update_request(session: &Session) -> AgentWorkspaceUpdateRequest {
        AgentWorkspaceUpdateRequest {
            schema_version: AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: session.id.clone(),
            observation: observe_agent_runtime(&session.worktree_path)
                .expect("runtime observation"),
            intent: AgentWorkspaceUpdateIntent {
                summary: Some("bound Host mutation".to_string()),
                current_focus: Some("current execution binding".to_string()),
                ..AgentWorkspaceUpdateIntent::default()
            },
        }
    }

    fn bound_work_terminalization_request(session: &Session) -> AgentWorkTerminalizationRequest {
        AgentWorkTerminalizationRequest {
            schema_version: AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: session.id.clone(),
            observation: observe_agent_runtime(&session.worktree_path)
                .expect("runtime observation"),
            terminal_kind: AgentWorkTerminalKind::Done,
        }
    }

    fn assert_execution_binding_denial(error: &AgentWorkspaceUpdateError) {
        assert_eq!(
            error.code,
            AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch
        );
        assert_eq!(
            error.message,
            "Execution binding is missing, stale, or no longer current; relaunch the Session before retrying"
        );
    }

    #[test]
    fn execution_binding_probe_request_cannot_select_host_or_execution_authority() {
        let request = serde_json::json!({
            "schema_version": AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-probe",
            "nonce": "nonce-probe"
        });
        serde_json::from_value::<AgentExecutionBindingProbeRequest>(request.clone())
            .expect("minimal non-secret probe request");

        for (field, value) in [
            ("host_instance_id", serde_json::json!("caller-host")),
            ("session_id", serde_json::json!("caller-session")),
            (
                "execution_binding",
                serde_json::json!({
                    "generation_id": "caller-generation",
                    "binding_id": "caller-binding",
                    "ledger_head_hash": "caller-head"
                }),
            ),
            ("capability_generation", serde_json::json!(99)),
            ("owner_kind", serde_json::json!("issue")),
            ("owner_number", serde_json::json!(2359)),
        ] {
            let mut forbidden = request.clone();
            forbidden
                .as_object_mut()
                .expect("probe request object")
                .insert(field.to_string(), value);
            serde_json::from_value::<AgentExecutionBindingProbeRequest>(forbidden)
                .expect_err("probe request must not accept caller-selected Host authority");
        }
    }

    #[test]
    fn execution_binding_probe_is_byte_equivalent_and_returns_exact_secret_free_receipt() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-probe");
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let receipt = probe_authenticated_execution_binding(
                repo,
                &session.id,
                &binding,
                "host-instance-probe",
                execution_binding_probe_request("operation-probe", "nonce-probe"),
            )
            .expect("current execution binding probe");

            assert_eq!(
                serde_json::to_value(&receipt).expect("serialize probe receipt"),
                serde_json::json!({
                    "schema_version": AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                    "operation_id": "operation-probe",
                    "nonce": "nonce-probe",
                    "host_instance_id": "host-instance-probe",
                    "execution_binding": binding.identity,
                    "capability_generation": binding.capability_generation
                }),
                "the probe receipt must contain only correlation and non-secret authority identity"
            );
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "a probe must preserve Session, owner ledger, pointer, ECR, and Work bytes"
            );
        });
    }

    #[test]
    fn execution_continuation_rebinds_current_generation_after_host_restart() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);

            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-rebound-current".to_string(),
            };
            let (receipt, rebound) =
                continue_authenticated_execution(repo, &session.id, request.clone())
                    .expect("rebind current execution");
            let replay = continue_authenticated_execution(repo, &session.id, request)
                .expect("replay rebound execution");

            assert_eq!(
                receipt.outcome,
                AgentExecutionContinuationOutcome::ReboundCurrent
            );
            assert_eq!(replay, (receipt.clone(), rebound.clone()));
            assert_eq!(receipt.execution_binding, binding.identity);
            assert_eq!(rebound, binding);
            assert!(receipt.validated);
            let diagnosis = crate::cli::execution_state::diagnose(repo, Some(session.id.as_str()));
            let continuation = diagnosis
                .continuation
                .expect("durable rebound continuation diagnosis");
            assert_eq!(continuation.outcome.as_deref(), Some("rebound_current"));
            assert_eq!(continuation.generation_id, binding.identity.generation_id);
            assert!(!continuation.predecessor_stale);
            assert_eq!(continuation.from_session_id, None);
            assert_eq!(
                continuation.current_writer.as_deref(),
                Some(session.id.as_str())
            );
            assert!(continuation.validated);
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 2359,
            };
            assert_eq!(
                crate::cli::execution_state::load_generation_ledger(repo, owner)
                    .unwrap()
                    .unwrap()
                    .continuation_validations
                    .len(),
                1,
                "replay must reuse one durable validation audit"
            );
        });
    }

    #[test]
    fn split_root_exact_unbound_status_matches_host_continuation_from_all_invocation_shapes() {
        with_split_root_exact_unbound_fixture(
            |project_state_root, worktree, nested, _sibling, session| {
                let owner = crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 3393,
                };
                let predecessor =
                    crate::cli::execution_state::current_execution_binding(worktree, owner)
                        .expect("read split-root predecessor")
                        .expect("split-root predecessor binding");
                let before = ExecutionBindingAuthoritySnapshot::capture(
                    project_state_root,
                    worktree,
                    &session.id,
                );
                let _session_env = gwt_core::test_support::ScopedEnvVar::set(
                    gwt_agent::GWT_SESSION_ID_ENV,
                    &session.id,
                );
                let _forward_url = gwt_core::test_support::ScopedEnvVar::unset(
                    gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
                );
                let _forward_token = gwt_core::test_support::ScopedEnvVar::unset(
                    gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
                );
                let _runtime_path = gwt_core::test_support::ScopedEnvVar::unset(
                    gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                );

                for invocation in [project_state_root, worktree, nested] {
                    let diagnosis = crate::cli::execution_state::diagnose(
                        invocation,
                        Some(session.id.as_str()),
                    );
                    assert_eq!(
                        diagnosis.ecr_status,
                        crate::cli::execution_state::ExecutionDiagnosisState::Active,
                        "{} must resolve the linked Session worktree",
                        invocation.display()
                    );
                    assert_eq!(diagnosis.owner_number, Some(owner.number));
                    assert_eq!(
                        diagnosis.generation_id.as_deref(),
                        Some(predecessor.generation_id.as_str())
                    );
                    let continuation = diagnosis
                        .recovery_probes
                        .iter()
                        .find(|probe| probe.operation == "execution.continue")
                        .expect("continuation probe");
                    assert_eq!(
                        continuation.state,
                        crate::cli::governance::RecoveryProbeState::Available,
                        "{}: {:?}",
                        invocation.display(),
                        continuation.reason
                    );
                    assert!(diagnosis
                        .available_recoveries
                        .contains(&"execution.continue".to_string()));
                    assert!(!diagnosis
                        .available_recoveries
                        .contains(&"execution.adopt".to_string()));

                    let mut env = crate::cli::TestEnv::new(invocation.to_path_buf());
                    let (code, out) = crate::cli::run_collect(
                        &mut env,
                        crate::cli::CliCommand::Execution(
                            crate::cli::execution_state::ExecutionCommand::Status,
                        ),
                    )
                    .expect("run execution.status");
                    assert_eq!(code, 0, "{out}");
                    let status: serde_json::Value =
                        serde_json::from_str(&out).expect("parse execution.status output");
                    assert_eq!(status["ecr_status"], "active");
                    assert_eq!(status["owner_number"], owner.number);
                    assert_eq!(status["generation_id"], predecessor.generation_id.as_str());
                    assert!(status["available_recoveries"]
                        .as_array()
                        .expect("available recoveries")
                        .contains(&serde_json::json!("execution.continue")));
                    assert!(!status["available_recoveries"]
                        .as_array()
                        .expect("available recoveries")
                        .contains(&serde_json::json!("execution.adopt")));
                }

                assert_eq!(
                    ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        &session.id,
                    ),
                    before,
                    "status and operation-local probes must be completely read-only"
                );

                let (receipt, _binding) = continue_authenticated_execution(
                    project_state_root,
                    &session.id,
                    AgentExecutionContinuationRequest {
                        schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                        operation_id: "split-root-status-host-parity".to_string(),
                    },
                )
                .expect("the Host evaluator must execute the continuation advertised by status");
                assert_eq!(
                    receipt.outcome,
                    AgentExecutionContinuationOutcome::SuccessorCreated
                );

                let after_continue =
                    crate::cli::execution_state::diagnose(nested, Some(session.id.as_str()));
                assert!(after_continue
                    .available_recoveries
                    .contains(&"workspace.ensure".to_string()));
                assert!(!after_continue
                    .available_recoveries
                    .contains(&"execution.adopt".to_string()));

                let mut ensure_env = crate::cli::TestEnv::new(nested.to_path_buf());
                let (ensure_code, ensure_out) = crate::cli::run_collect(
                    &mut ensure_env,
                    crate::cli::CliCommand::Workspace(crate::cli::WorkspaceCommand::Ensure {
                        agent_session: session.id.clone(),
                        title_summary: "Split-root recovery".to_string(),
                        current_focus: Some(
                            "Continue exact authority into Workspace mutation".to_string(),
                        ),
                        spec: None,
                        issue: Some(owner.number),
                        topic: Some("recovery".to_string()),
                        boundary: Some("split-root continuation".to_string()),
                    }),
                )
                .expect("execute advertised split-root workspace.ensure");
                assert_eq!(ensure_code, 0, "{ensure_out}");
                let ensured_work_id = ensure_out
                    .strip_prefix("workspace ensured: ")
                    .and_then(|value| value.split_once(" (").map(|(work_id, _)| work_id))
                    .expect("workspace.ensure output contains Work id");
                let after_ensure =
                    crate::cli::execution_state::diagnose(nested, Some(session.id.as_str()));
                assert!(after_ensure
                    .available_recoveries
                    .contains(&"workspace.update".to_string()));
                assert!(!after_ensure
                    .available_recoveries
                    .contains(&"workspace.ensure".to_string()));

                let mut update_env = crate::cli::TestEnv::new(project_state_root.to_path_buf());
                let (update_code, update_out) = crate::cli::run_collect(
                    &mut update_env,
                    crate::cli::CliCommand::Workspace(crate::cli::WorkspaceCommand::Update {
                        title: None,
                        status: None,
                        status_text: None,
                        summary: Some("Split-root recovery reached Workspace update".to_string()),
                        progress_summary: None,
                        next_action: None,
                        owner: None,
                        agent_session: Some(session.id.clone()),
                        current_focus: None,
                        title_summary: None,
                    }),
                )
                .expect("execute advertised split-root workspace.update");
                assert_eq!(update_code, 0, "{update_out}");
                let projection =
                    gwt_core::workspace_projection::load_workspace_projection(project_state_root)
                        .expect("load split-root Workspace projection")
                        .expect("split-root Workspace projection exists");
                assert_eq!(
                    projection.summary.as_deref(),
                    Some("Split-root recovery reached Workspace update")
                );
                assert!(projection.agents.iter().any(|agent| {
                    agent.session_id == session.id
                        && agent.workspace_id.as_deref() == Some(ensured_work_id)
                }));
            },
        );
    }

    #[test]
    fn split_root_exact_unbound_adopt_is_rejected_byte_identically() {
        with_split_root_exact_unbound_fixture(
            |project_state_root, worktree, nested, _sibling, session| {
                let before = ExecutionBindingAuthoritySnapshot::capture(
                    project_state_root,
                    worktree,
                    &session.id,
                );
                let diagnosis =
                    crate::cli::execution_state::diagnose(nested, Some(session.id.as_str()));
                assert_eq!(
                    diagnosis
                        .recovery_probes
                        .iter()
                        .find(|probe| probe.operation == "execution.adopt")
                        .and_then(|probe| probe.reason.as_deref()),
                    Some("exact_unbound_session_requires_execution_continue")
                );
                let _session_env = gwt_core::test_support::ScopedEnvVar::set(
                    gwt_agent::GWT_SESSION_ID_ENV,
                    &session.id,
                );
                let mut env = crate::cli::TestEnv::new(nested.to_path_buf());
                let (code, out) = crate::cli::run_collect(
                    &mut env,
                    crate::cli::CliCommand::Execution(
                        crate::cli::execution_state::ExecutionCommand::Adopt {
                            reason: "must use canonical continuation".to_string(),
                        },
                    ),
                )
                .expect("run execution.adopt");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains("execution.continue"), "{out}");
                assert_eq!(
                    ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        &session.id,
                    ),
                    before,
                    "rejected adoption must preserve Session, ECR, pointer, ledger, capability, and Work bytes"
                );
            },
        );
    }

    #[test]
    fn split_root_recovery_scope_drift_fails_closed_byte_identically() {
        with_split_root_exact_unbound_fixture(
            |project_state_root, worktree, _nested, sibling, session| {
                let assert_read_only_refusal = |invocation: &Path, label: &str| {
                    let before = ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        &session.id,
                    );
                    resolve_execution_recovery_context(invocation, &session.id)
                        .expect_err(&format!("{label}: recovery scope must be rejected"));
                    let diagnosis = crate::cli::execution_state::diagnose(
                        invocation,
                        Some(session.id.as_str()),
                    );
                    for operation in ["execution.continue", "execution.adopt"] {
                        let probe = diagnosis
                            .recovery_probes
                            .iter()
                            .find(|probe| probe.operation == operation)
                            .unwrap_or_else(|| panic!("{label}: missing {operation} probe"));
                        assert_eq!(
                            probe.state,
                            crate::cli::governance::RecoveryProbeState::Unavailable,
                            "{label}: {operation} must fail closed"
                        );
                    }
                    assert_eq!(
                        ExecutionBindingAuthoritySnapshot::capture(
                            project_state_root,
                            worktree,
                            &session.id,
                        ),
                        before,
                        "{label}: failed diagnosis must preserve every authority byte"
                    );
                };

                assert_read_only_refusal(sibling, "sibling-worktree");

                let foreign_temp = tempfile::tempdir().expect("foreign tempdir");
                let foreign = init_git_repo(
                    foreign_temp.path(),
                    "foreign",
                    "https://example.invalid/acme/foreign-recovery.git",
                    "work/issue-3415",
                );
                assert_read_only_refusal(&foreign, "foreign-repository");

                #[cfg(unix)]
                {
                    let escape = worktree.join("symlink-escape");
                    std::os::unix::fs::symlink(&foreign, &escape).expect("symlink escape");
                    assert_read_only_refusal(&escape, "symlink-escape");
                }

                let mut wrong_branch = session.clone();
                wrong_branch.branch = "work/wrong-branch".to_string();
                save_session_fixture(&wrong_branch);
                assert_read_only_refusal(worktree, "wrong-branch");
                save_session_fixture(session);

                let mut wrong_repo_hash = session.clone();
                wrong_repo_hash.repo_hash = Some("foreign-repository-hash".to_string());
                save_session_fixture(&wrong_repo_hash);
                assert_read_only_refusal(worktree, "repo-hash-mismatch");
                save_session_fixture(session);

                let before = ExecutionBindingAuthoritySnapshot::capture(
                    project_state_root,
                    worktree,
                    &session.id,
                );
                let _session_env = gwt_core::test_support::ScopedEnvVar::set(
                    gwt_agent::GWT_SESSION_ID_ENV,
                    &session.id,
                );
                let mut env = crate::cli::TestEnv::new(sibling.to_path_buf());
                let (code, out) = crate::cli::run_collect(
                    &mut env,
                    crate::cli::CliCommand::Execution(
                        crate::cli::execution_state::ExecutionCommand::Continue {
                            operation_id: "reject-sibling-before-bridge".to_string(),
                        },
                    ),
                )
                .expect("local continuation preflight");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains("recovery scope is invalid"), "{out}");
                assert_eq!(
                    ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        &session.id,
                    ),
                    before,
                    "CLI continuation scope rejection must happen before authority mutation"
                );
            },
        );
    }

    #[test]
    fn protected_recovery_negative_matrix_keeps_probe_execute_parity() {
        with_split_root_exact_unbound_fixture(
            |project_state_root, worktree, nested, sibling, session| {
                let assert_refused = |label: &str, invocation: &Path, caller: Option<&str>| {
                    let tracked_session_id = caller.unwrap_or(session.id.as_str());
                    let before = ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        tracked_session_id,
                    );
                    let diagnosis = crate::cli::execution_state::diagnose(invocation, caller);
                    for operation in [
                        "execution.adopt",
                        "execution.repair",
                        "execution.reopen",
                        "execution.continue",
                        "workspace.update",
                        "workspace.ensure",
                    ] {
                        let probe = diagnosis
                            .recovery_probes
                            .iter()
                            .find(|probe| probe.operation == operation)
                            .unwrap_or_else(|| panic!("{label}: missing {operation} probe"));
                        assert_eq!(
                            probe.state,
                            crate::cli::governance::RecoveryProbeState::Unavailable,
                            "{label}: {operation} probe must fail closed"
                        );
                        assert_eq!(
                            probe.reason.as_deref(),
                            Some("execution_recovery_scope_invalid"),
                            "{label}: {operation} must use the canonical scope refusal"
                        );
                    }

                    let _ambient = caller.map_or_else(
                        || {
                            gwt_core::test_support::ScopedEnvVar::unset(
                                gwt_agent::GWT_SESSION_ID_ENV,
                            )
                        },
                        |caller| {
                            gwt_core::test_support::ScopedEnvVar::set(
                                gwt_agent::GWT_SESSION_ID_ENV,
                                caller,
                            )
                        },
                    );
                    let _forward_url = gwt_core::test_support::ScopedEnvVar::unset(
                        gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
                    );
                    let _forward_token = gwt_core::test_support::ScopedEnvVar::unset(
                        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
                    );
                    for command in [
                        crate::cli::execution_state::ExecutionCommand::Adopt {
                            reason: format!("reject {label}"),
                        },
                        crate::cli::execution_state::ExecutionCommand::Repair {
                            reason: format!("reject {label}"),
                        },
                        crate::cli::execution_state::ExecutionCommand::Reopen {
                            reason: format!("reject {label}"),
                        },
                        crate::cli::execution_state::ExecutionCommand::Continue {
                            operation_id: format!("reject-{label}"),
                        },
                    ] {
                        let mut env = crate::cli::TestEnv::new(invocation.to_path_buf());
                        let (code, out) = crate::cli::run_collect(
                            &mut env,
                            crate::cli::CliCommand::Execution(command),
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "{label}: protected execution must return typed refusal: {error}"
                            )
                        });
                        assert_eq!(code, 2, "{label}: {out}");
                        assert!(out.contains("recovery scope is invalid"), "{label}: {out}");
                    }

                    // workspace.update and workspace.ensure can contact a
                    // Host bridge on valid input. Exercise their shared
                    // canonical execute evaluators directly so every
                    // negative row remains pure and cannot escape the
                    // local authority fence.
                    let caller = caller.unwrap_or("");
                    resolve_session_work_mutation_target(invocation, caller)
                        .expect_err(&format!("{label}: workspace.update must refuse"));
                    assert!(
                        !matches!(
                            validated_workspace_recovery_session(invocation, caller),
                            Ok(Some(_))
                        ),
                        "{label}: workspace.ensure must refuse"
                    );
                    assert_eq!(
                        ExecutionBindingAuthoritySnapshot::capture(
                            project_state_root,
                            worktree,
                            tracked_session_id,
                        ),
                        before,
                        "{label}: probes and refused execution must preserve all authority bytes"
                    );
                };

                assert_refused("no-ambient", nested, None);
                assert_refused(
                    "missing-session",
                    nested,
                    Some("protected-matrix-missing-session"),
                );

                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
                let session_bytes = std::fs::read(&session_path).expect("read durable Session");
                std::fs::write(&session_path, "broken = [").expect("corrupt durable Session");
                assert_refused("corrupt-session", nested, Some(session.id.as_str()));
                std::fs::write(&session_path, &session_bytes).expect("restore durable Session");

                assert_refused("sibling-worktree", sibling, Some(session.id.as_str()));

                let foreign_temp = tempfile::tempdir().expect("foreign tempdir");
                let foreign = init_git_repo(
                    foreign_temp.path(),
                    "foreign-matrix",
                    "https://example.invalid/acme/foreign-matrix.git",
                    "work/issue-3415",
                );
                assert_refused("foreign-repository", &foreign, Some(session.id.as_str()));

                let mut wrong_branch = session.clone();
                wrong_branch.branch = "work/foreign-branch".to_string();
                save_session_fixture(&wrong_branch);
                assert_refused("branch-mismatch", worktree, Some(session.id.as_str()));
                save_session_fixture(session);

                #[cfg(unix)]
                {
                    let escape = worktree.join("protected-matrix-symlink");
                    std::os::unix::fs::symlink(&foreign, &escape)
                        .expect("create protected recovery symlink escape");
                    assert_refused("symlink-escape", &escape, Some(session.id.as_str()));
                }
            },
        );
    }

    #[test]
    fn recovery_context_rejects_unsafe_session_id_before_path_lookup() {
        with_strict_target_fixture(|repo, _session| {
            let unsafe_id = "../escaped-session";
            let error = resolve_execution_recovery_context_if_session_exists(repo, unsafe_id)
                .expect("invalid Session ids must fail closed even when no escaped file exists")
                .expect_err("path traversal must be rejected");
            assert!(
                error.to_string().contains("invalid or unsafe Session id"),
                "{error}"
            );
            let direct = resolve_execution_recovery_context(repo, unsafe_id)
                .expect_err("direct resolver must reject traversal before loading a ledger");
            assert!(
                direct.to_string().contains("invalid or unsafe Session id"),
                "{direct}"
            );
            assert!(!session_requires_execution_continuation(unsafe_id));
            let diagnosis = crate::cli::execution_state::diagnose(repo, Some(unsafe_id));
            assert_eq!(
                diagnosis
                    .recovery_probes
                    .iter()
                    .find(|probe| probe.operation == "execution.continue")
                    .and_then(|probe| probe.reason.as_deref()),
                Some("execution_recovery_scope_invalid")
            );
        });
    }

    #[test]
    fn legacy_missing_session_nested_diagnosis_keeps_worktree_ecr_visibility() {
        with_strict_target_fixture(|repo, _session| {
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 3393,
            };
            crate::cli::execution_state::materialize_at_launch(
                repo,
                owner.kind,
                owner.number,
                "legacy-predecessor",
                "gwt-execute",
                false,
            )
            .expect("materialize legacy predecessor");
            crate::cli::execution_state::ensure_generation_ledger(
                repo,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize legacy ledger");
            let expected = crate::cli::execution_state::current_execution_binding(repo, owner)
                .expect("read legacy binding")
                .expect("legacy binding");
            let nested = repo.join("nested").join("legacy-cwd");
            std::fs::create_dir_all(&nested).expect("legacy nested cwd");
            seed_work_mutation_surfaces(repo, repo);

            let diagnosis = crate::cli::execution_state::diagnose(
                &nested,
                Some("legacy-session-without-ledger"),
            );
            assert_eq!(
                diagnosis.ecr_status,
                crate::cli::execution_state::ExecutionDiagnosisState::Active
            );
            assert_eq!(diagnosis.owner_number, Some(owner.number));
            assert_eq!(
                diagnosis.generation_id.as_deref(),
                Some(expected.generation_id.as_str())
            );
            for operation in [
                "execution.adopt",
                "execution.repair",
                "execution.reopen",
                "execution.continue",
                "workspace.update",
                "workspace.ensure",
            ] {
                assert_eq!(
                    diagnosis
                        .recovery_probes
                        .iter()
                        .find(|probe| probe.operation == operation)
                        .and_then(|probe| probe.reason.as_deref()),
                    Some("execution_recovery_scope_invalid"),
                    "{operation} must fail closed without a durable Session"
                );
            }

            let _session_env = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_ID_ENV,
                "legacy-session-without-ledger",
            );
            let commands = [
                crate::cli::execution_state::ExecutionCommand::Adopt {
                    reason: "must not recover without durable Session".to_string(),
                },
                crate::cli::execution_state::ExecutionCommand::Repair {
                    reason: "must not recover without durable Session".to_string(),
                },
                crate::cli::execution_state::ExecutionCommand::Reopen {
                    reason: "must not recover without durable Session".to_string(),
                },
                crate::cli::execution_state::ExecutionCommand::Continue {
                    operation_id: "must-not-recover-without-durable-session".to_string(),
                },
            ];
            for command in commands {
                let before = ExecutionBindingAuthoritySnapshot::capture(
                    repo,
                    repo,
                    "legacy-session-without-ledger",
                );
                let mut env = crate::cli::TestEnv::new(nested.clone());
                let (code, out) =
                    crate::cli::run_collect(&mut env, crate::cli::CliCommand::Execution(command))
                        .expect("missing-Session recovery must return a typed refusal");
                assert_eq!(code, 2, "{out}");
                assert!(out.contains("recovery scope is invalid"), "{out}");
                assert_eq!(
                    ExecutionBindingAuthoritySnapshot::capture(
                        repo,
                        repo,
                        "legacy-session-without-ledger",
                    ),
                    before,
                    "missing-Session recovery refusal must preserve every authority byte"
                );
            }
        });
    }

    #[test]
    fn corrupt_durable_session_blocks_all_recovery_commands_byte_identically() {
        with_split_root_exact_unbound_fixture(
            |project_state_root, worktree, nested, _sibling, session| {
                let ledger_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
                std::fs::write(&ledger_path, "broken = [").expect("corrupt durable Session ledger");
                let _session_env = gwt_core::test_support::ScopedEnvVar::set(
                    gwt_agent::GWT_SESSION_ID_ENV,
                    &session.id,
                );
                let diagnosis =
                    crate::cli::execution_state::diagnose(nested, Some(session.id.as_str()));
                for operation in [
                    "execution.adopt",
                    "execution.repair",
                    "execution.reopen",
                    "execution.continue",
                    "workspace.update",
                    "workspace.ensure",
                ] {
                    assert_eq!(
                        diagnosis
                            .recovery_probes
                            .iter()
                            .find(|probe| probe.operation == operation)
                            .and_then(|probe| probe.reason.as_deref()),
                        Some("execution_recovery_scope_invalid"),
                        "{operation} diagnosis must match its mutation refusal"
                    );
                }
                let commands = [
                    crate::cli::execution_state::ExecutionCommand::Adopt {
                        reason: "must not bypass corrupt Session".to_string(),
                    },
                    crate::cli::execution_state::ExecutionCommand::Repair {
                        reason: "must not bypass corrupt Session".to_string(),
                    },
                    crate::cli::execution_state::ExecutionCommand::Reopen {
                        reason: "must not bypass corrupt Session".to_string(),
                    },
                    crate::cli::execution_state::ExecutionCommand::Continue {
                        operation_id: "must-not-bypass-corrupt-session".to_string(),
                    },
                ];
                for command in commands {
                    let before = ExecutionBindingAuthoritySnapshot::capture(
                        project_state_root,
                        worktree,
                        &session.id,
                    );
                    let mut env = crate::cli::TestEnv::new(nested.to_path_buf());
                    let (code, out) = crate::cli::run_collect(
                        &mut env,
                        crate::cli::CliCommand::Execution(command),
                    )
                    .expect("recovery command must return a typed refusal");
                    assert_eq!(code, 2, "{out}");
                    assert!(out.contains("recovery scope is invalid"), "{out}");
                    assert_eq!(
                        ExecutionBindingAuthoritySnapshot::capture(
                            project_state_root,
                            worktree,
                            &session.id,
                        ),
                        before,
                        "corrupt durable Session refusal must preserve every authority byte"
                    );
                }
            },
        );
    }

    fn assert_continuation_refused_byte_identically(
        repo: &Path,
        baseline: &Session,
        label: &str,
        mutate: impl FnOnce(&mut Session),
    ) {
        let mut candidate = baseline.clone();
        mutate(&mut candidate);
        save_session_fixture(&candidate);
        let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &candidate.id);
        let probe = probe_authenticated_execution_continuation(repo, &candidate.id);
        assert_eq!(
            probe.state,
            crate::cli::governance::RecoveryProbeState::Unavailable,
            "{label}: invalid continuation must be unavailable"
        );
        let request = AgentExecutionContinuationRequest {
            schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: format!("reject-{label}"),
        };
        continue_authenticated_execution(repo, &candidate.id, request).expect_err(&format!(
            "{label}: invalid continuation unexpectedly executed"
        ));
        assert_eq!(
            ExecutionBindingAuthoritySnapshot::capture(repo, repo, &candidate.id),
            before,
            "{label}: rejection must preserve Session, ECR, pointer, ledger, capability, and Work bytes"
        );
    }

    #[test]
    fn execution_continuation_bootstraps_only_exact_unbound_host_identity() {
        with_strict_target_fixture(|repo, session| {
            let (bound, _) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &bound, "work-continuation-rejection");

            assert_continuation_refused_byte_identically(repo, &bound, "owner-only", |session| {
                session.execution_binding = None;
            });
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "legacy-owner-only",
                |session| {
                    session.schema_version = Session::CURRENT_SCHEMA_VERSION - 1;
                    session.execution_binding = None;
                },
            );
            assert_continuation_refused_byte_identically(repo, &bound, "binding-only", |session| {
                session.linked_issue_number = None;
            });
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "foreign-binding",
                |session| {
                    session.execution_binding.as_mut().unwrap().owner_number += 1;
                },
            );
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "legacy-foreign-binding",
                |session| {
                    session.schema_version = Session::CURRENT_SCHEMA_VERSION - 1;
                    session.execution_binding.as_mut().unwrap().owner_number += 1;
                },
            );
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "stale-binding",
                |session| {
                    session
                        .execution_binding
                        .as_mut()
                        .unwrap()
                        .identity
                        .generation_id = "generation-stale".to_string();
                },
            );
            assert_continuation_refused_byte_identically(repo, &bound, "wrong-repo", |session| {
                session.repo_hash = Some("foreign-repository".to_string());
            });
            assert_continuation_refused_byte_identically(repo, &bound, "wrong-root", |session| {
                session.project_state_root = Some(repo.join("foreign-root"));
            });
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "wrong-worktree",
                |session| {
                    session.worktree_path = repo.join("foreign-worktree");
                },
            );
            assert_continuation_refused_byte_identically(repo, &bound, "wrong-branch", |session| {
                session.branch = "work/foreign".to_string();
            });
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "wrong-session",
                |session| {
                    session.execution_binding.as_mut().unwrap().session_id =
                        "foreign-session".to_string();
                },
            );
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "docker-target",
                |session| {
                    session.runtime_target = LaunchRuntimeTarget::Docker;
                },
            );
            assert_continuation_refused_byte_identically(
                repo,
                &bound,
                "docker-binding",
                |session| {
                    session.docker_runtime_binding = Some(gwt_agent::DockerRuntimeBinding {
                        runtime_worktree_path: PathBuf::from("/workspace"),
                        project_state_scope_hash: "foreign-scope".to_string(),
                    });
                },
            );
        });
    }

    #[test]
    fn execution_continuation_refuses_a_takeover_suffix_byte_identically() {
        with_strict_target_fixture(|repo, session| {
            let (bound, _) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &bound, "work-takeover-suffix");
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 2359,
            };
            let takeover = crate::cli::execution_state::GenerationTakeoverRequest {
                operation_id: "takeover-suffix-continuation".to_string(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some("work-takeover-suffix-continuation".to_string()),
                source: Some("continue-work:handoff".to_string()),
                from_session_id: bound.id.clone(),
                to_session_id: "session-adopting".to_string(),
                reason: "explicit handoff".to_string(),
                requested_at: Utc::now(),
            };
            crate::cli::execution_state::prepare_generation_takeover(repo, owner, &takeover)
                .expect("prepare takeover suffix");
            crate::cli::execution_state::activate_generation_takeover(repo, owner, &takeover)
                .expect("activate takeover suffix");
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &bound.id);

            let probe = probe_authenticated_execution_continuation(repo, &bound.id);
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Unavailable
            );
            let error = continue_authenticated_execution(
                repo,
                &bound.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "continue-after-takeover-suffix".to_string(),
                },
            )
            .expect_err("the predecessor Session must not continue after a takeover suffix");
            assert_eq!(probe.reason.as_deref(), Some(error.message.as_str()));
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &bound.id),
                before,
                "takeover-suffix refusal must preserve Session, ECR, pointer, ledger, capability, and Work bytes"
            );
        });
    }

    fn seed_foreign_active_exact_unbound(
        repo: &Path,
        session: &Session,
    ) -> (
        Session,
        crate::cli::execution_state::ExecutionOwnerKey,
        gwt_agent::ExecutionBindingIdentity,
    ) {
        let mut exact_unbound = session.clone();
        exact_unbound.linked_issue_number = None;
        exact_unbound.execution_binding = None;
        exact_unbound.runtime_target = LaunchRuntimeTarget::Host;
        exact_unbound.docker_runtime_binding = None;
        save_session_fixture(&exact_unbound);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        crate::cli::execution_state::materialize_at_launch(
            repo,
            owner.kind,
            owner.number,
            "foreign-predecessor",
            "gwt-execute",
            false,
        )
        .expect("materialize foreign predecessor");
        crate::cli::execution_state::ensure_generation_ledger(
            repo,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize foreign predecessor ledger");
        let predecessor = crate::cli::execution_state::current_execution_binding(repo, owner)
            .expect("read predecessor")
            .expect("predecessor binding");
        seed_work_mutation_surfaces(repo, repo);
        seed_unique_mutation_target(repo, repo, &exact_unbound, "work-exact-unbound-race");
        (exact_unbound, owner, predecessor)
    }

    #[test]
    fn execution_continuation_prepare_failure_preserves_all_authority_bytes() {
        with_strict_target_fixture(|repo, session| {
            let (session, _, _) = seed_foreign_active_exact_unbound(repo, session);
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);
            crate::cli::execution_state::set_continuation_rebind_failure_before_prepare_commit();

            continue_authenticated_execution(
                repo,
                &session.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "continue-fail-before-prepare-commit".to_string(),
                },
            )
            .expect_err("injected prepare failure");

            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "prepare failure must not publish Session, ECR, pointer, ledger, capability, or Work bytes"
            );
        });
    }

    #[test]
    fn execution_continuation_activation_failure_rolls_back_prepared_authority() {
        with_strict_target_fixture(|repo, session| {
            let (session, _, _) = seed_foreign_active_exact_unbound(repo, session);
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);
            crate::cli::execution_state::set_continuation_rebind_failure_before_activation_commit();

            continue_authenticated_execution(
                repo,
                &session.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "continue-fail-before-activation-commit".to_string(),
                },
            )
            .expect_err("injected activation failure");

            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "pre-commit activation failure must roll back Prepared and Session authority bytes"
            );
        });
    }

    #[test]
    fn execution_continuation_post_ledger_failure_is_retryable_without_ghost_session_authority() {
        with_strict_target_fixture(|repo, session| {
            let (session, owner, predecessor) = seed_foreign_active_exact_unbound(repo, session);
            crate::cli::execution_state::set_generation_write_failure_after_ledger();
            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-post-ledger-retry".to_string(),
            };

            continue_authenticated_execution(repo, &session.id, request.clone())
                .expect_err("injected post-ledger response loss");
            let after_failure = Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
            )
            .expect("load Session after response loss");
            assert_eq!(after_failure.linked_issue_number, None);
            assert_eq!(after_failure.execution_binding, None);
            let committed = crate::cli::execution_state::load_owner_generation_ledger(repo, owner)
                .unwrap()
                .unwrap();
            assert_ne!(committed.current_generation_id, predecessor.generation_id);
            assert_eq!(
                committed
                    .continuation_attempts
                    .last()
                    .map(|attempt| attempt.status),
                Some(crate::cli::execution_state::ContinuationAttemptStatus::Activated)
            );

            let (receipt, binding) = continue_authenticated_execution(repo, &session.id, request)
                .expect("exact retry repairs publication and Session binding");
            assert_eq!(
                receipt.outcome,
                AgentExecutionContinuationOutcome::SuccessorCreated
            );
            assert_eq!(receipt.execution_binding, binding.identity);
            assert!(
                crate::cli::execution_state::current_active_execution_binding_matches(
                    repo,
                    owner,
                    &session.id,
                    &binding.identity,
                )
                .unwrap()
            );
        });
    }

    #[test]
    fn activated_response_loss_retry_refuses_a_legal_takeover_suffix_byte_identically() {
        with_strict_target_fixture(|repo, session| {
            let (session, owner, _) = seed_foreign_active_exact_unbound(repo, session);
            crate::cli::execution_state::set_generation_write_failure_after_ledger();
            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-post-ledger-takeover-suffix".to_string(),
            };

            continue_authenticated_execution(repo, &session.id, request.clone())
                .expect_err("inject response loss after the Activated ledger commit");
            let attempt = crate::cli::execution_state::continuation_attempt_for_operation(
                repo,
                owner,
                &request.operation_id,
            )
            .expect("read Activated response-loss attempt")
            .expect("Activated response-loss attempt");
            assert_eq!(
                attempt.status,
                crate::cli::execution_state::ContinuationAttemptStatus::Activated
            );
            crate::cli::execution_state::activate_successor(repo, owner, &attempt.request)
                .expect("canonical Activated retry republishes only projection and pointer");

            let takeover = crate::cli::execution_state::GenerationTakeoverRequest {
                operation_id: "takeover-post-ledger-successor".to_string(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some("work-post-ledger-takeover".to_string()),
                source: Some("continue-work:handoff".to_string()),
                from_session_id: session.id.clone(),
                to_session_id: "post-ledger-takeover-winner".to_string(),
                reason: "legal handoff after response-loss publication".to_string(),
                requested_at: Utc::now(),
            };
            crate::cli::execution_state::prepare_generation_takeover(repo, owner, &takeover)
                .expect("prepare legal takeover suffix");
            let planned =
                crate::cli::execution_state::prepared_generation_takeover_execution_binding(
                    repo, owner, &takeover,
                )
                .expect("planned takeover binding");
            let mut winner = session.clone();
            winner.id.clone_from(&takeover.to_session_id);
            winner.linked_issue_number = Some(owner.number);
            winner
                .set_execution_binding(Some(SessionExecutionBinding {
                    schema_version: SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: winner.id.clone(),
                    repo_hash: winner.repo_hash.clone().expect("winner repo hash"),
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity: planned.clone(),
                    capability_generation: 1,
                }))
                .expect("project Prepared takeover capability into winner Session");
            save_session_fixture(&winner);
            let winner_identity = gwt_agent::SessionExecutionIdentity::from_session(&winner)
                .expect("validate winner Session")
                .expect("winner execution identity");
            let activated =
                crate::cli::execution_state::with_prepared_generation_takeover_exact_session_activation(
                    repo,
                    owner,
                    &takeover,
                    &gwt_core::paths::gwt_sessions_dir(),
                    &winner_identity,
                    |activate| activate(),
                )
                .expect("activate legal takeover transaction")
                .expect("winner Session retains exact Prepared identity");
            assert_eq!(activated, planned);

            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);
            let winner_before = std::fs::read(
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", winner.id)),
            )
            .expect("read winner Session bytes");
            let error = continue_authenticated_execution(repo, &session.id, request)
                .expect_err("historical Activated operation must not repair across a takeover");

            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "historical retry must preserve old Session, ECR, pointer, ledger, capability, and Work bytes"
            );
            assert_eq!(
                std::fs::read(
                    gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", winner.id)),
                )
                .expect("read winner Session after historical retry"),
                winner_before,
                "historical retry must preserve winner Session bytes"
            );
            assert!(
                matches!(
                    error.code,
                    AgentWorkspaceUpdateErrorCode::TransactionConflict
                        | AgentWorkspaceUpdateErrorCode::IdentityConflict
                ),
                "unexpected historical retry refusal: {error:?}"
            );
        });
    }

    #[test]
    fn stale_exact_unbound_loser_preserves_winner_authority_bytes() {
        with_strict_target_fixture(|repo, session| {
            let (stale_unbound, owner, predecessor) =
                seed_foreign_active_exact_unbound(repo, session);
            continue_authenticated_execution(
                repo,
                &stale_unbound.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "continue-byte-winner".to_string(),
                },
            )
            .expect("commit winner");
            let winner = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &stale_unbound.id);
            let loser = crate::cli::execution_state::SuccessorRequest {
                operation_id: "continue-byte-loser".to_string(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: None,
                source: "execution-continue".to_string(),
                session_binding_id: "execution-continue-byte-loser".to_string(),
                initial_session_id: stale_unbound.id.clone(),
                entrypoint: "execution.continue".to_string(),
                requested_at: Utc::now(),
            };

            assert!(
                crate::cli::execution_state::activate_successor_with_session_rebind(
                    repo,
                    owner,
                    &loser,
                    crate::cli::execution_state::SuccessorPredecessorStatus::Active,
                    &predecessor,
                    &gwt_core::paths::gwt_sessions_dir(),
                    &stale_unbound,
                )
                .expect("stale predecessor is a transaction miss")
                .is_none(),
                "stale loser must lose the Session CAS before ledger publication"
            );

            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(
                    repo,
                    repo,
                    &stale_unbound.id,
                ),
                winner,
                "loser must preserve winner Session, ECR, pointer, ledger, capability, and Work bytes"
            );
        });
    }

    #[test]
    fn distinct_stale_exact_unbound_session_loses_predecessor_cas_byte_identically() {
        with_strict_target_fixture(|repo, session| {
            let (session_a, owner, predecessor) = seed_foreign_active_exact_unbound(repo, session);
            let mut session_b = session_a.clone();
            session_b.id = "exact-unbound-session-b".to_string();
            save_session_fixture(&session_b);

            let observed_a = evaluate_authenticated_execution_continuation(repo, &session_a.id)
                .expect("Session A observes predecessor P");
            let observed_b = evaluate_authenticated_execution_continuation(repo, &session_b.id)
                .expect("Session B observes predecessor P");
            assert_eq!(observed_a.current_binding, predecessor);
            assert_eq!(observed_b.current_binding, predecessor);

            let winner_request = crate::cli::execution_state::SuccessorRequest {
                operation_id: "continue-distinct-session-winner".to_string(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: None,
                source: "execution-continue".to_string(),
                session_binding_id: "execution-continue-distinct-session-winner".to_string(),
                initial_session_id: session_b.id.clone(),
                entrypoint: "execution.continue".to_string(),
                requested_at: Utc::now(),
            };
            crate::cli::execution_state::activate_successor_with_session_rebind(
                repo,
                owner,
                &winner_request,
                crate::cli::execution_state::SuccessorPredecessorStatus::Active,
                &observed_b.current_binding,
                &gwt_core::paths::gwt_sessions_dir(),
                &observed_b.session,
            )
            .expect("winner activation transaction")
            .expect("Session B must commit its observed predecessor");
            let winner_a = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session_a.id);
            let winner_b = std::fs::read(
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session_b.id)),
            )
            .expect("read winner Session B bytes");

            let loser_request = crate::cli::execution_state::SuccessorRequest {
                operation_id: "continue-distinct-session-loser".to_string(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: None,
                source: "execution-continue".to_string(),
                session_binding_id: "execution-continue-distinct-session-loser".to_string(),
                initial_session_id: session_a.id.clone(),
                entrypoint: "execution.continue".to_string(),
                requested_at: Utc::now(),
            };
            let loser = crate::cli::execution_state::activate_successor_with_session_rebind(
                repo,
                owner,
                &loser_request,
                crate::cli::execution_state::SuccessorPredecessorStatus::Active,
                &observed_a.current_binding,
                &gwt_core::paths::gwt_sessions_dir(),
                &observed_a.session,
            )
            .expect("stale predecessor is a transaction miss");
            assert!(
                loser.is_none(),
                "Session A must not activate from a predecessor superseded by Session B"
            );
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session_a.id),
                winner_a,
                "stale Session A must preserve its Session, ECR, pointer, ledger, capability, and Work bytes"
            );
            assert_eq!(
                std::fs::read(
                    gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session_b.id)),
                )
                .expect("read Session B after stale loss"),
                winner_b,
                "stale Session A must preserve winner Session B bytes"
            );
        });
    }

    #[test]
    fn legacy_exact_unbound_session_migrates_and_binds_in_one_continuation() {
        with_strict_target_fixture(|repo, session| {
            let (mut legacy, owner, predecessor) = seed_foreign_active_exact_unbound(repo, session);
            legacy.schema_version = Session::CURRENT_SCHEMA_VERSION - 1;
            save_session_fixture(&legacy);

            let (receipt, binding) = continue_authenticated_execution(
                repo,
                &legacy.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "continue-legacy-exact-unbound".to_string(),
                },
            )
            .expect("legacy exact-unbound Session must migrate and activate atomically");

            assert_eq!(
                receipt.outcome,
                AgentExecutionContinuationOutcome::SuccessorCreated
            );
            assert_eq!(
                receipt.predecessor_generation_id.as_deref(),
                Some(predecessor.generation_id.as_str())
            );
            let durable = Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", legacy.id)),
            )
            .expect("read migrated successor Session");
            assert_eq!(durable.schema_version, Session::CURRENT_SCHEMA_VERSION);
            assert_eq!(durable.linked_issue_number, Some(owner.number));
            assert_eq!(durable.execution_binding.as_ref(), Some(&binding));
            assert_eq!(binding.capability_generation, 1);
            assert_eq!(receipt.execution_binding, binding.identity);
            assert_eq!(
                crate::cli::execution_state::current_execution_binding(repo, owner)
                    .expect("read current successor binding")
                    .as_ref(),
                Some(&binding.identity)
            );
            assert!(
                crate::cli::execution_state::current_active_execution_binding_matches(
                    repo,
                    owner,
                    &legacy.id,
                    &binding.identity,
                )
                .expect("validate migrated active binding readback")
            );
        });
    }

    #[test]
    fn unbound_session_continuation_creates_exact_active_successor() {
        with_strict_target_fixture(|repo, session| {
            let mut session = session.clone();
            session.linked_issue_number = None;
            session.execution_binding = None;
            save_session_fixture(&session);
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 2359,
            };
            crate::cli::execution_state::materialize_at_launch(
                repo,
                owner.kind,
                owner.number,
                &session.id,
                "gwt-execute",
                false,
            )
            .expect("materialize live predecessor");
            crate::cli::execution_state::ensure_generation_ledger(
                repo,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize valid predecessor ledger");
            let predecessor = crate::cli::execution_state::current_execution_binding(repo, owner)
                .expect("read predecessor binding")
                .expect("predecessor binding");
            let session_path =
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
            let session_before_probe = std::fs::read(&session_path).expect("read Session bytes");
            let ledger_before_probe =
                crate::cli::execution_state::load_generation_ledger(repo, owner)
                    .expect("read ledger before probe");
            let probe = probe_authenticated_execution_continuation(repo, &session.id);
            assert_eq!(
                probe.state,
                crate::cli::governance::RecoveryProbeState::Available
            );
            assert_eq!(probe.operation, "execution.continue");
            assert_eq!(
                std::fs::read(&session_path).expect("read Session bytes after probe"),
                session_before_probe,
                "the recovery probe must not mutate Session bytes"
            );
            assert_eq!(
                crate::cli::execution_state::load_generation_ledger(repo, owner)
                    .expect("read ledger after probe"),
                ledger_before_probe,
                "the recovery probe must not mutate execution authority"
            );
            let before = crate::cli::execution_state::diagnose(repo, Some(session.id.as_str()));
            assert!(before
                .available_recoveries
                .contains(&"execution.continue".to_string()));
            assert!(!before
                .available_recoveries
                .contains(&"workspace.ensure".to_string()));
            assert!(!before
                .available_recoveries
                .contains(&"execution.repair".to_string()));

            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-exact-unbound-successor".to_string(),
            };
            let (receipt, binding) = continue_authenticated_execution(repo, &session.id, request)
                .expect("exact unbound Host Session must create one successor");

            assert_eq!(
                receipt.outcome,
                AgentExecutionContinuationOutcome::SuccessorCreated
            );
            assert_eq!(
                receipt.predecessor_generation_id.as_deref(),
                Some(predecessor.generation_id.as_str())
            );
            assert_ne!(receipt.execution_binding, predecessor);
            assert_eq!(receipt.execution_binding, binding.identity);
            assert_eq!(binding.owner_kind, owner.kind.as_str());
            assert_eq!(binding.owner_number, owner.number);
            assert_eq!(binding.session_id, session.id);
            let durable = Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
            )
            .expect("read successor-bound Session");
            assert_eq!(durable.linked_issue_number, Some(owner.number));
            assert_eq!(durable.execution_binding.as_ref(), Some(&binding));
            assert_eq!(
                crate::cli::execution_state::current_execution_binding(repo, owner)
                    .expect("read successor authority")
                    .as_ref(),
                Some(&binding.identity)
            );
            let after = crate::cli::execution_state::diagnose(repo, Some(session.id.as_str()));
            assert!(after
                .available_recoveries
                .contains(&"workspace.ensure".to_string()));
            assert!(!after
                .available_recoveries
                .contains(&"execution.repair".to_string()));
        });
    }

    #[test]
    fn resume_producing_helper_recovers_authority_for_linked_session() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            let (receipt, rebound) = prepare_resume_producing_authority(repo, &session.id)
                .expect("linked durable session must recover producing authority");
            assert_eq!(
                receipt.outcome,
                AgentExecutionContinuationOutcome::ReboundCurrent
            );
            assert_eq!(rebound, binding);
        });
    }

    #[test]
    fn resume_producing_helper_returns_none_without_durable_linkage() {
        with_strict_target_fixture(|repo, _session| {
            assert!(
                prepare_resume_producing_authority(repo, "session-unknown").is_none(),
                "unknown durable session must stay observation-only"
            );
        });
    }

    #[test]
    fn exact_relaunch_recovers_current_binding_without_changing_execution_history() {
        for (lifecycle, settlement, expected_status) in [
            (
                "active",
                None,
                crate::cli::execution_state::ExecutionControlStatus::Active,
            ),
            (
                "blocked",
                Some(crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "exact relaunch terminal fixture".to_string(),
                    missing_verification: Some(
                        "exact relaunch must preserve recovery requirements".to_string(),
                    ),
                }),
                crate::cli::execution_state::ExecutionControlStatus::Blocked,
            ),
            (
                "completed",
                Some(crate::cli::execution_state::ExecutionSettlement::Completed),
                crate::cli::execution_state::ExecutionControlStatus::Completed,
            ),
        ] {
            with_strict_target_fixture(|repo, session| {
                let (session, launch_binding) = bind_session_to_current_execution(repo, session);
                if let Some(settlement) = settlement {
                    assert!(matches!(
                        crate::cli::execution_state::settle(repo, &session.id, settlement)
                            .expect("settle exact relaunch fixture"),
                        crate::cli::execution_state::SettleResult::Settled(_)
                    ));
                }

                let owner = crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: launch_binding.owner_number,
                };
                let expected_identity =
                    crate::cli::execution_state::current_execution_binding(repo, owner)
                        .expect("read current exact relaunch binding")
                        .expect("exact relaunch fixture has a current binding");
                let record_before = crate::cli::execution_state::load(repo)
                    .expect("read execution record before exact relaunch")
                    .expect("exact relaunch fixture has an execution record");
                let ledger_before =
                    crate::cli::execution_state::load_generation_ledger(repo, owner)
                        .expect("read generation ledger before exact relaunch")
                        .expect("exact relaunch fixture has a generation ledger");
                let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(repo)
                    .expect("exact relaunch fixture has a trusted-store directory");
                let trusted_root = trusted_dir
                    .parent()
                    .expect("worktree trusted directory has a repository trusted root");
                let trusted_files_before = snapshot_regular_files(trusted_root);

                let recovered = prepare_exact_relaunch_execution_authority(repo, &session.id)
                    .unwrap_or_else(|error| {
                        panic!("{lifecycle} exact relaunch must recover authority: {error}")
                    });

                assert_eq!(recovered.session_id, session.id);
                assert_eq!(recovered.repo_hash, launch_binding.repo_hash);
                assert_eq!(recovered.owner_kind, owner.kind.as_str());
                assert_eq!(recovered.owner_number, owner.number);
                assert_eq!(
                    recovered.identity, expected_identity,
                    "{lifecycle} exact relaunch must reuse the current generation and binding"
                );

                let record_after = crate::cli::execution_state::load(repo)
                    .expect("read execution record after exact relaunch")
                    .expect("execution record must remain present");
                let ledger_after = crate::cli::execution_state::load_generation_ledger(repo, owner)
                    .expect("read generation ledger after exact relaunch")
                    .expect("generation ledger must remain present");
                assert_eq!(record_after.status, expected_status);
                assert_eq!(record_after, record_before, "{lifecycle} ECR changed");
                assert_eq!(
                    ledger_after.continuation_attempts, ledger_before.continuation_attempts,
                    "{lifecycle} exact relaunch must not append a continuation attempt"
                );
                assert_eq!(
                    ledger_after.takeover_attempts, ledger_before.takeover_attempts,
                    "{lifecycle} exact relaunch must not append a takeover attempt"
                );
                assert_eq!(
                    ledger_after.takeovers, ledger_before.takeovers,
                    "{lifecycle} exact relaunch must not append a transfer audit"
                );
                assert_eq!(
                    ledger_after.lifecycle_events, ledger_before.lifecycle_events,
                    "{lifecycle} exact relaunch must not change generation lifecycle"
                );
                assert_eq!(
                    ledger_after.continuation_validations, ledger_before.continuation_validations,
                    "{lifecycle} exact relaunch must not masquerade as continuation validation"
                );
                assert_eq!(
                    ledger_after, ledger_before,
                    "{lifecycle} exact relaunch must leave the generation ledger byte-equivalent"
                );
                assert_eq!(
                    snapshot_regular_files(trusted_root),
                    trusted_files_before,
                    "{lifecycle} exact relaunch must not mutate trusted execution history"
                );
            });
        }
    }

    #[test]
    fn exact_relaunch_rejects_unbound_and_stale_bindings_without_side_effects() {
        for authority_case in [
            "linked-unbound",
            "legacy-unbound",
            "foreign-generation",
            "owner-mismatch",
            "tampered-ledger-head",
            "repo-mismatch",
        ] {
            with_strict_target_fixture(|repo, session| {
                let (mut session, binding) = bind_session_to_current_execution(repo, session);
                match authority_case {
                    "linked-unbound" => session
                        .set_execution_binding(None)
                        .expect("clear exact relaunch binding"),
                    "legacy-unbound" => {
                        session.linked_issue_number = None;
                        session
                            .set_execution_binding(None)
                            .expect("clear legacy exact relaunch binding");
                    }
                    "foreign-generation" => {
                        let mut stale = binding.clone();
                        stale.identity.generation_id =
                            "foreign-exact-relaunch-generation".to_string();
                        stale.capability_generation = stale.capability_generation.saturating_add(1);
                        session
                            .set_execution_binding(Some(stale))
                            .expect("install well-formed stale exact relaunch binding");
                    }
                    "owner-mismatch" => {
                        let mut foreign = binding.clone();
                        foreign.owner_number = binding.owner_number + 1;
                        foreign.capability_generation =
                            foreign.capability_generation.saturating_add(1);
                        session.linked_issue_number = Some(foreign.owner_number);
                        session
                            .set_execution_binding(Some(foreign))
                            .expect("install foreign-owner exact relaunch binding");
                    }
                    "tampered-ledger-head" => {
                        let mut tampered = binding.clone();
                        tampered.identity.ledger_head_hash =
                            "tampered-exact-relaunch-ledger-head".to_string();
                        tampered.capability_generation =
                            tampered.capability_generation.saturating_add(1);
                        session
                            .set_execution_binding(Some(tampered))
                            .expect("install tampered-head exact relaunch binding");
                    }
                    "repo-mismatch" => {
                        let mut foreign = binding.clone();
                        foreign.repo_hash = "foreign-exact-relaunch-repo".to_string();
                        foreign.capability_generation =
                            foreign.capability_generation.saturating_add(1);
                        session.repo_hash = Some(foreign.repo_hash.clone());
                        session
                            .set_execution_binding(Some(foreign))
                            .expect("install foreign-repository exact relaunch binding");
                    }
                    _ => unreachable!("covered exact relaunch authority case"),
                }
                save_session_fixture(&session);

                let owner = crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: binding.owner_number,
                };
                let session_path =
                    gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
                let session_before =
                    std::fs::read(&session_path).expect("read rejected exact relaunch Session");
                let record_before = crate::cli::execution_state::load(repo)
                    .expect("read rejected exact relaunch ECR")
                    .expect("rejected exact relaunch fixture has an ECR");
                let ledger_before =
                    crate::cli::execution_state::load_generation_ledger(repo, owner)
                        .expect("read rejected exact relaunch generation ledger")
                        .expect("rejected exact relaunch fixture has a generation ledger");
                let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(repo)
                    .expect("rejected exact relaunch fixture has a trusted-store directory");
                let trusted_root = trusted_dir
                    .parent()
                    .expect("worktree trusted directory has a repository trusted root");
                let trusted_files_before = snapshot_regular_files(trusted_root);

                let error = prepare_exact_relaunch_execution_authority(repo, &session.id)
                    .expect_err("unbound or stale exact relaunch authority must fail closed");
                assert!(
                    matches!(
                        error.code,
                        AgentWorkspaceUpdateErrorCode::RelaunchRequired
                            | AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch
                            | AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
                            | AgentWorkspaceUpdateErrorCode::IdentityConflict
                    ),
                    "{authority_case} must fail as an authority error, got {error:?}"
                );
                assert_eq!(
                    std::fs::read(&session_path).expect("reread rejected exact relaunch Session"),
                    session_before,
                    "{authority_case} rejection must not synthesize or repair a Session binding"
                );
                assert_eq!(
                    crate::cli::execution_state::load(repo)
                        .expect("reread rejected exact relaunch ECR")
                        .expect("rejected exact relaunch ECR remains present"),
                    record_before,
                    "{authority_case} rejection must not mutate lifecycle or transfers"
                );
                assert_eq!(
                    crate::cli::execution_state::load_generation_ledger(repo, owner)
                        .expect("reread rejected exact relaunch generation ledger")
                        .expect("rejected exact relaunch generation ledger remains present"),
                    ledger_before,
                    "{authority_case} rejection must not append attempts, transfers, or lifecycle"
                );
                assert_eq!(
                    snapshot_regular_files(trusted_root),
                    trusted_files_before,
                    "{authority_case} rejection must not mutate trusted execution history"
                );
            });
        }
    }

    #[test]
    fn blocked_exact_relaunch_does_not_turn_explicit_continue_into_a_successor() {
        with_strict_target_fixture(|repo, session| {
            let (session, _) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            assert!(matches!(
                crate::cli::execution_state::settle(
                    repo,
                    &session.id,
                    crate::cli::execution_state::ExecutionSettlement::Blocked {
                        reason: "exact relaunch requires recovery".to_string(),
                        missing_verification: Some("verify.run".to_string()),
                    },
                )
                .expect("settle blocked exact relaunch fixture"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
            prepare_exact_relaunch_execution_authority(repo, &session.id)
                .expect("recover blocked exact relaunch identity");
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let error = continue_authenticated_execution(
                repo,
                &session.id,
                AgentExecutionContinuationRequest {
                    schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                    operation_id: "blocked-exact-relaunch-explicit-continue".to_string(),
                },
            )
            .expect_err("Blocked exact relaunch must require reopen before explicit Continue");

            assert_eq!(error.code, AgentWorkspaceUpdateErrorCode::RelaunchRequired);
            assert!(error.message.contains("execution.reopen"), "{error:?}");
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "refused explicit Continue must preserve exact relaunch authority"
            );
        });
    }

    #[test]
    fn execution_continuation_retries_validation_write_without_double_rebind() {
        with_strict_target_fixture(|repo, session| {
            let (session, _) = bind_session_to_current_execution(repo, session);
            crate::cli::execution_state::set_continuation_validation_write_failure();
            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-validation-retry".to_string(),
            };

            let first = continue_authenticated_execution(repo, &session.id, request.clone())
                .expect_err("injected validation write failure");
            assert_eq!(
                first.code,
                AgentWorkspaceUpdateErrorCode::TransactionConflict
            );
            let rebound_once = Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
            )
            .expect("read rebound Session")
            .execution_binding
            .expect("first attempt rebound Session before audit write");
            let (receipt, rebound_retry) =
                continue_authenticated_execution(repo, &session.id, request)
                    .expect("retry validation write");

            assert_eq!(receipt.capability_generation, 1);
            assert_eq!(rebound_retry, rebound_once);
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 2359,
            };
            assert_eq!(
                crate::cli::execution_state::load_generation_ledger(repo, owner)
                    .unwrap()
                    .unwrap()
                    .continuation_validations
                    .len(),
                1
            );
        });
    }

    #[test]
    fn execution_continuation_creates_and_replays_completed_successor() {
        with_strict_target_fixture(|repo, session| {
            let (session, predecessor) = bind_session_to_current_execution(repo, session);
            assert!(matches!(
                crate::cli::execution_state::settle(
                    repo,
                    &session.id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("settle predecessor"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
            let request = AgentExecutionContinuationRequest {
                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                operation_id: "continue-successor".to_string(),
            };

            let (first, first_binding) =
                continue_authenticated_execution(repo, &session.id, request.clone())
                    .expect("activate successor");
            let (replay, replay_binding) =
                continue_authenticated_execution(repo, &session.id, request)
                    .expect("replay activated successor");

            assert_eq!(
                first.outcome,
                AgentExecutionContinuationOutcome::SuccessorCreated
            );
            assert_eq!(replay, first);
            assert_eq!(replay_binding, first_binding);
            assert_ne!(first.execution_binding, predecessor.identity);
            let superseded = first
                .superseded_execution_binding
                .as_ref()
                .expect("successor names superseded authority");
            assert_eq!(superseded.generation_id, predecessor.identity.generation_id);
            assert_eq!(superseded.binding_id, predecessor.identity.binding_id);
            assert_eq!(
                crate::cli::execution_state::load(repo)
                    .unwrap()
                    .unwrap()
                    .primary_session_id,
                session.id
            );
            let diagnosis = crate::cli::execution_state::diagnose(repo, Some(session.id.as_str()));
            assert!(diagnosis
                .warnings
                .iter()
                .any(|warning| { warning.starts_with("latest_continuation_attempt:activated:") }));
            assert_eq!(
                diagnosis
                    .continuation
                    .as_ref()
                    .and_then(|continuation| continuation.outcome.as_deref()),
                Some("successor_created")
            );
            assert_eq!(
                diagnosis
                    .continuation
                    .as_ref()
                    .and_then(|continuation| continuation.takeover_audit_id.as_deref()),
                None,
                "an unrelated takeover must never be attached to a Completed successor"
            );
            let continuation = diagnosis
                .continuation
                .expect("durable Completed successor diagnosis");
            assert!(continuation.predecessor_stale);
            assert_eq!(
                continuation.from_session_id.as_deref(),
                Some(session.id.as_str())
            );
            assert_eq!(
                continuation.current_writer.as_deref(),
                Some(session.id.as_str())
            );
        });
    }

    #[test]
    fn execution_continuation_foreign_active_generation_has_one_concurrent_successor() {
        with_strict_target_fixture(|repo, session| {
            let (predecessor, predecessor_binding) =
                bind_session_to_current_execution(repo, session);
            let mut successor_session =
                session_fixture("foreign-active-successor", repo, &session.branch);
            successor_session.linked_issue_number = None;
            successor_session.execution_binding = None;
            save_session_fixture(&successor_session);
            let repo = repo.to_path_buf();
            let session_id = successor_session.id.clone();
            let requests = ["continue-race-a", "continue-race-b"];

            let results = std::thread::scope(|scope| {
                let handles = requests.map(|operation_id| {
                    let repo = repo.clone();
                    let session_id = session_id.clone();
                    scope.spawn(move || {
                        continue_authenticated_execution(
                            &repo,
                            &session_id,
                            AgentExecutionContinuationRequest {
                                schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
                                operation_id: operation_id.to_string(),
                            },
                        )
                    })
                });
                handles.map(|handle| handle.join().expect("continuation thread"))
            });

            let successes = results
                .iter()
                .filter_map(|result| result.as_ref().ok())
                .collect::<Vec<_>>();
            assert_eq!(successes.len(), 1, "race results: {results:?}");
            assert_eq!(
                successes[0].0.outcome,
                AgentExecutionContinuationOutcome::SuccessorCreated
            );
            assert_ne!(
                successes[0].0.execution_binding,
                predecessor_binding.identity
            );
            assert!(
                crate::cli::execution_state::current_active_execution_binding_matches(
                    &repo,
                    crate::cli::execution_state::ExecutionOwnerKey {
                        kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                        number: 2359,
                    },
                    &session_id,
                    &successes[0].1.identity,
                )
                .unwrap()
            );
            let durable_successor = Session::load(
                &gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")),
            )
            .expect("read successor Session");
            assert_eq!(
                durable_successor.execution_binding.as_ref(),
                Some(&successes[0].1)
            );
            assert_eq!(
                durable_successor
                    .execution_binding
                    .as_ref()
                    .map(|binding| binding.capability_generation),
                Some(1),
                "the losing exact-unbound continuation must not rotate capability authority",
            );
            let ledger = crate::cli::execution_state::load_generation_ledger(
                &repo,
                crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 2359,
                },
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                ledger.continuation_attempts.len(),
                2,
                "the losing continuation must not publish a Prepared ledger suffix: {:?}",
                ledger.continuation_attempts
            );
            assert_eq!(
                ledger.continuation_attempts[0].status,
                crate::cli::execution_state::ContinuationAttemptStatus::Prepared
            );
            assert_eq!(
                ledger.continuation_attempts[1].status,
                crate::cli::execution_state::ContinuationAttemptStatus::Activated
            );
            assert!(ledger
                .continuation_attempts
                .iter()
                .all(|attempt| { attempt.request.operation_id == successes[0].0.operation_id }));
            seed_work_mutation_surfaces(&repo, &repo);
            seed_unique_mutation_target(
                &repo,
                &repo,
                &durable_successor,
                "work-continuation-current",
            );
            apply_bound_authenticated_workspace_update(
                &repo,
                &session_id,
                &successes[0].1,
                bound_workspace_update_request(&durable_successor),
            )
            .expect("current continuation binding authorizes workspace mutation");
            assert_eq!(
                crate::cli::execution_state::load(&repo)
                    .unwrap()
                    .unwrap()
                    .primary_session_id,
                session_id
            );
            let diagnosis = crate::cli::execution_state::diagnose(&repo, Some(session_id.as_str()));
            assert_eq!(
                diagnosis
                    .continuation
                    .as_ref()
                    .and_then(|continuation| continuation.outcome.as_deref()),
                Some("successor_created")
            );
            assert!(diagnosis
                .continuation
                .as_ref()
                .and_then(|continuation| continuation.takeover_audit_id.as_deref())
                .is_some());
            let continuation = diagnosis
                .continuation
                .expect("durable foreign-owner successor diagnosis");
            assert!(continuation.predecessor_stale);
            assert_eq!(
                continuation.from_session_id.as_deref(),
                Some(predecessor.id.as_str())
            );
            assert_eq!(
                continuation.current_writer.as_deref(),
                Some(session_id.as_str())
            );
            assert!(matches!(
                crate::cli::execution_state::settle(
                    &repo,
                    &session_id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("current continuation binding authorizes settlement"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
        });
    }

    #[test]
    fn prepared_execution_binding_probe_is_observation_only_until_exact_activation() {
        with_strict_target_fixture(|repo, session| {
            let (mut session, predecessor_binding) =
                bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-prepared-probe");
            assert!(matches!(
                crate::cli::execution_state::settle(
                    repo,
                    &session.id,
                    crate::cli::execution_state::ExecutionSettlement::Completed,
                )
                .expect("complete predecessor"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: predecessor_binding.owner_number,
            };
            let request = crate::cli::execution_state::SuccessorRequest {
                operation_id: "continue-work-prepared-probe".to_string(),
                principal_id: "host-instance-prepared-probe".to_string(),
                work_id: Some("work-prepared-probe".to_string()),
                source: "continue-work".to_string(),
                requested_at: Utc::now(),
                session_binding_id: "binding-prepared-probe".to_string(),
                initial_session_id: session.id.clone(),
                entrypoint: "continue-work".to_string(),
            };
            crate::cli::execution_state::prepare_successor(repo, owner, &request)
                .expect("prepare successor");
            let planned = crate::cli::execution_state::prepared_successor_execution_binding(
                repo, owner, &request,
            )
            .expect("planned successor binding");
            let prepared_binding = gwt_agent::SessionExecutionBinding {
                identity: planned.clone(),
                capability_generation: predecessor_binding.capability_generation + 1,
                ..predecessor_binding
            };
            session
                .set_execution_binding(Some(prepared_binding.clone()))
                .expect("project Prepared binding into durable Session");
            save_session_fixture(&session);
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let receipt = probe_authenticated_prepared_execution_binding(
                repo,
                &session.id,
                &prepared_binding,
                "host-instance-prepared-probe",
                execution_binding_probe_request("operation-prepared-probe", "nonce-prepared-probe"),
            )
            .expect("Prepared exact binding probe");
            assert_eq!(receipt.execution_binding, planned);
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "Prepared probe must be byte-equivalent and side-effect free"
            );
            assert_execution_binding_denial(
                &probe_authenticated_execution_binding(
                    repo,
                    &session.id,
                    &prepared_binding,
                    "host-instance-active-probe",
                    execution_binding_probe_request(
                        "operation-active-before-cas",
                        "nonce-active-before-cas",
                    ),
                )
                .expect_err("Prepared authority must not pass the Active probe"),
            );

            crate::cli::execution_state::activate_successor(repo, owner, &request)
                .expect("activate exact successor");
            assert_execution_binding_denial(
                &probe_authenticated_prepared_execution_binding(
                    repo,
                    &session.id,
                    &prepared_binding,
                    "host-instance-prepared-after-cas",
                    execution_binding_probe_request(
                        "operation-prepared-after-cas",
                        "nonce-prepared-after-cas",
                    ),
                )
                .expect_err("Activated authority is no longer Prepared"),
            );
            probe_authenticated_execution_binding(
                repo,
                &session.id,
                &prepared_binding,
                "host-instance-active-after-cas",
                execution_binding_probe_request(
                    "operation-active-after-cas",
                    "nonce-active-after-cas",
                ),
            )
            .expect("same exact binding becomes Active only after CAS");
        });
    }

    #[test]
    fn execution_binding_probe_rejects_exact_identity_and_capability_mismatch_matrix() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-matrix");
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let mut generation = binding.clone();
            generation.identity.generation_id = "foreign-generation".to_string();
            let mut binding_id = binding.clone();
            binding_id.identity.binding_id = "foreign-binding".to_string();
            let mut head = binding.clone();
            head.identity.ledger_head_hash = "foreign-head".to_string();
            let mut capability = binding.clone();
            capability.capability_generation += 1;
            let mut repository = binding.clone();
            repository.repo_hash = "foreign-repository".to_string();
            let mut owner_kind = binding.clone();
            owner_kind.owner_kind = "spec".to_string();
            let mut owner_number = binding.clone();
            owner_number.owner_number += 1;
            let mut session_id = binding.clone();
            session_id.session_id = "foreign-session".to_string();

            for (label, candidate) in [
                ("generation", generation),
                ("binding", binding_id),
                ("head", head),
                ("capability", capability),
                ("repository", repository),
                ("owner-kind", owner_kind),
                ("owner-number", owner_number),
                ("session", session_id),
            ] {
                let operation_id = format!("secret-operation-{label}");
                let nonce = format!("secret-nonce-{label}");
                let host_instance_id = format!("secret-host-{label}");
                let error = probe_authenticated_execution_binding(
                    repo,
                    &session.id,
                    &candidate,
                    &host_instance_id,
                    execution_binding_probe_request(&operation_id, &nonce),
                )
                .expect_err("mismatched binding identity must be denied");
                assert_execution_binding_denial(&error);
                for secret in [&operation_id, &nonce, &host_instance_id] {
                    assert!(
                        !error.message.contains(secret),
                        "binding diagnostics must not echo correlation or Host identifiers"
                    );
                }
            }

            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "rejected probes must preserve every durable authority and Work surface"
            );
        });
    }

    #[test]
    fn execution_binding_probe_denies_unbound_and_corrupt_sessions_without_side_effects() {
        with_strict_target_fixture(|repo, session| {
            let (mut session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-unbound");
            session
                .set_execution_binding(None)
                .expect("clear durable binding");
            save_session_fixture(&session);
            let before_unbound =
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let error = probe_authenticated_execution_binding(
                repo,
                &session.id,
                &binding,
                "host-instance-unbound",
                execution_binding_probe_request("operation-unbound", "nonce-unbound"),
            )
            .expect_err("Inspection/unbound Session must not gain execution authority");
            assert_execution_binding_denial(&error);
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before_unbound
            );

            let session_path =
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
            std::fs::write(&session_path, b"corrupt = [")
                .expect("replace Session with corrupt durable state");
            let before_corrupt =
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);
            let error = probe_authenticated_execution_binding(
                repo,
                &session.id,
                &binding,
                "host-instance-corrupt",
                execution_binding_probe_request("operation-corrupt", "nonce-corrupt"),
            )
            .expect_err("corrupt Session authority must fail closed");
            assert_execution_binding_denial(&error);
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before_corrupt
            );
        });
    }

    #[test]
    fn execution_binding_probe_denies_corrupt_owner_ledger_without_side_effects() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-corrupt-ledger");
            let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(repo)
                .expect("bound fixture trusted directory");
            let trusted_root = trusted_dir.parent().expect("repository trusted root");
            let ledger_relative_path = snapshot_regular_files(trusted_root)
                .into_iter()
                .map(|(path, _)| path)
                .find(|path| {
                    path.file_name()
                        .is_some_and(|name| name == "generation-ledger.json")
                })
                .expect("owner generation ledger path");
            std::fs::write(
                trusted_root.join(ledger_relative_path),
                b"corrupt owner ledger",
            )
            .expect("corrupt owner generation ledger");
            let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

            let error = probe_authenticated_execution_binding(
                repo,
                &session.id,
                &binding,
                "host-instance-corrupt-ledger",
                execution_binding_probe_request("operation-corrupt-ledger", "nonce-corrupt-ledger"),
            )
            .expect_err("corrupt owner ledger must fail closed");
            assert_execution_binding_denial(&error);
            assert_eq!(
                ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                before,
                "a rejected corrupt-ledger probe must not repair or mutate authority"
            );
        });
    }

    #[test]
    fn execution_binding_bound_mutations_accept_only_the_current_binding() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            let work_id = "work-bound-current";
            seed_unique_mutation_target(repo, repo, &session, work_id);

            let update = apply_bound_authenticated_workspace_update(
                repo,
                &session.id,
                &binding,
                bound_workspace_update_request(&session),
            )
            .expect("current binding authorizes workspace update");
            assert_eq!(update.work_id, work_id);

            let terminal = apply_bound_authenticated_work_terminalization(
                repo,
                &session.id,
                &binding,
                bound_work_terminalization_request(&session),
            )
            .expect("current binding authorizes Work terminalization");
            assert_eq!(terminal.outcome, AgentWorkTerminalizationOutcome::Emitted);
        });
    }

    #[test]
    fn bound_work_terminalization_rejects_foreign_owner_and_agent_identity_without_mutation() {
        for mismatch in ["owner", "agent"] {
            with_strict_target_fixture(|repo, session| {
                let (session, binding) = bind_session_to_current_execution(repo, session);
                seed_work_mutation_surfaces(repo, repo);
                let work_id = format!("work-terminal-{mismatch}-mismatch");
                seed_unique_mutation_target(repo, repo, &session, &work_id);
                let path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo);
                let mut work_items =
                    gwt_core::workspace_projection::load_workspace_work_items_from_path(&path)
                        .expect("load terminal WorkItems")
                        .expect("terminal WorkItems");
                let work = work_items
                    .work_items
                    .iter_mut()
                    .find(|item| item.id == work_id)
                    .expect("terminal Work");
                match mismatch {
                    "owner" => work.owner = Some("Issue #9999".to_string()),
                    "agent" => work.agents[0].agent_id = Some("claude".to_string()),
                    _ => unreachable!(),
                }
                save_mutation_work_items(repo, &work_items);
                let before = WorkMutationSnapshot::capture(repo, repo);

                let error = apply_bound_authenticated_work_terminalization(
                    repo,
                    &session.id,
                    &binding,
                    bound_work_terminalization_request(&session),
                )
                .expect_err("foreign terminal Work authority must fail closed");

                assert!(matches!(
                    error.code,
                    AgentWorkspaceUpdateErrorCode::IdentityConflict
                        | AgentWorkspaceUpdateErrorCode::TransactionConflict
                ));
                assert_eq!(WorkMutationSnapshot::capture(repo, repo), before);
            });
        }
    }

    #[test]
    fn execution_binding_predecessor_and_superseded_authority_cannot_mutate_work() {
        with_strict_target_fixture(|repo, session| {
            let (mut session, predecessor_binding) =
                bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-stale");

            let settlement = crate::cli::execution_state::settle(
                repo,
                &session.id,
                crate::cli::execution_state::ExecutionSettlement::Completed,
            )
            .expect("settle producing execution");
            assert!(matches!(
                settlement,
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
            let before_predecessor = WorkMutationSnapshot::capture(repo, repo);
            let error = apply_bound_authenticated_workspace_update(
                repo,
                &session.id,
                &predecessor_binding,
                bound_workspace_update_request(&session),
            )
            .expect_err("pre-settlement ledger head must be stale");
            assert_execution_binding_denial(&error);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before_predecessor,
                "a predecessor ledger head must not mutate Work"
            );

            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: predecessor_binding.owner_number,
            };
            let mut current_binding = predecessor_binding.clone();
            current_binding.identity =
                crate::cli::execution_state::current_execution_binding(repo, owner)
                    .expect("read settled current binding")
                    .expect("settled current binding");
            current_binding.capability_generation += 1;
            session
                .set_execution_binding(Some(current_binding.clone()))
                .expect("advance durable binding to current ledger head");
            save_session_fixture(&session);
            gwt_agent::rotate_session_execution_capability(
                &gwt_core::paths::gwt_sessions_dir(),
                &session.id,
            )
            .expect("supersede Host capability generation");

            let before_superseded = WorkMutationSnapshot::capture(repo, repo);
            let error = apply_bound_authenticated_work_terminalization(
                repo,
                &session.id,
                &current_binding,
                bound_work_terminalization_request(&session),
            )
            .expect_err("superseded capability generation must be stale");
            assert_execution_binding_denial(&error);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before_superseded,
                "a superseded capability generation must not mutate Work"
            );
        });
    }

    #[test]
    fn execution_binding_terminal_generation_cannot_authorize_probe_or_work_mutation() {
        for (terminal_label, completed) in [("completed", true), ("blocked", false)] {
            with_strict_target_fixture(|repo, session| {
                let (session, _) = bind_session_to_current_execution(repo, session);
                seed_work_mutation_surfaces(repo, repo);
                seed_unique_mutation_target(
                    repo,
                    repo,
                    &session,
                    &format!("work-binding-{terminal_label}"),
                );

                let settlement = if completed {
                    crate::cli::execution_state::ExecutionSettlement::Completed
                } else {
                    crate::cli::execution_state::ExecutionSettlement::Blocked {
                        reason: "terminal authority test".to_string(),
                        missing_verification: Some("verification remains blocked".to_string()),
                    }
                };
                assert!(matches!(
                    crate::cli::execution_state::settle(repo, &session.id, settlement)
                        .expect("settle producing generation"),
                    crate::cli::execution_state::SettleResult::Settled(_)
                ));
                let terminal_binding =
                    prepare_exact_relaunch_execution_authority(repo, &session.id)
                        .expect("exact relaunch recovers terminal generation identity");
                let before = ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id);

                let probe_error = probe_authenticated_execution_binding(
                    repo,
                    &session.id,
                    &terminal_binding,
                    "host-instance-terminal",
                    execution_binding_probe_request("operation-terminal", "nonce-terminal"),
                )
                .expect_err("terminal generation must not authorize a Host probe");
                assert_execution_binding_denial(&probe_error);

                let update_error = apply_bound_authenticated_workspace_update(
                    repo,
                    &session.id,
                    &terminal_binding,
                    bound_workspace_update_request(&session),
                )
                .expect_err("terminal generation must not authorize workspace mutation");
                assert_execution_binding_denial(&update_error);

                let terminalization_error = apply_bound_authenticated_work_terminalization(
                    repo,
                    &session.id,
                    &terminal_binding,
                    bound_work_terminalization_request(&session),
                )
                .expect_err("terminal generation must not authorize Work terminalization");
                assert_execution_binding_denial(&terminalization_error);

                assert_eq!(
                    ExecutionBindingAuthoritySnapshot::capture(repo, repo, &session.id),
                    before,
                    "{terminal_label} generation denial must preserve authority and Work bytes"
                );
            });
        }
    }

    #[test]
    fn execution_binding_workspace_update_revalidates_capability_inside_commit() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-update-race");
            let before = WorkMutationSnapshot::capture(repo, repo);
            let sessions_dir = gwt_core::paths::gwt_sessions_dir();
            let session_id = session.id.clone();

            let error = apply_bound_authenticated_workspace_update_inner(
                repo,
                &session.id,
                &binding,
                None,
                bound_workspace_update_request(&session),
                |_| {
                    gwt_agent::rotate_session_execution_capability(&sessions_dir, &session_id)
                        .expect("rotate capability between resolve and commit");
                },
                |_, _| panic!("non-terminal update must not refresh settlement"),
            )
            .expect_err("commit must reject a capability rotated after target resolution");
            assert_execution_binding_denial(&error);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "commit-time binding failure must leave Work byte-equivalent"
            );
        });
    }

    #[test]
    fn execution_binding_exact_work_update_rejects_a_different_target_without_mutation() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-exact-target");
            let before = WorkMutationSnapshot::capture(repo, repo);

            let error = apply_bound_authenticated_workspace_update_for_exact_work(
                repo,
                &session.id,
                &binding,
                "work-binding-foreign-target",
                bound_workspace_update_request(&session),
            )
            .expect_err("a compatibility continuation must stay on its snapshotted Work");

            assert_eq!(error.code, AgentWorkspaceUpdateErrorCode::IdentityConflict);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "an exact-Work mismatch must preserve every Work mutation surface"
            );
        });
    }

    #[test]
    fn execution_binding_exact_work_update_rejects_duplicate_identical_current_assignment_without_mutation(
    ) {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            let work_id = "work-binding-duplicate-current-assignment";
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, work_id);

            let current_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(repo);
            let mut current = load_workspace_projection_from_path(&current_path)
                .expect("load current projection")
                .expect("current projection");
            let duplicate = current
                .agents
                .iter()
                .find(|agent| agent.session_id == session.id)
                .expect("current Session assignment")
                .clone();
            current.agents.push(duplicate);
            gwt_core::workspace_projection::save_workspace_projection_to_path(
                &current_path,
                &current,
            )
            .expect("save duplicate identical current Session assignment");
            let before = WorkMutationSnapshot::capture(repo, repo);

            let error = apply_bound_authenticated_workspace_update_for_exact_work(
                repo,
                &session.id,
                &binding,
                work_id,
                bound_workspace_update_request(&session),
            )
            .expect_err("duplicate identical current assignments must be rejected as ambiguous");

            assert_eq!(
                error.code,
                AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
            );
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "ambiguous duplicate current assignments must preserve every Work mutation surface"
            );
        });
    }

    #[test]
    fn execution_binding_work_terminalization_revalidates_capability_inside_commit() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-binding-terminal-race");
            let before = WorkMutationSnapshot::capture(repo, repo);
            let sessions_dir = gwt_core::paths::gwt_sessions_dir();
            let session_id = session.id.clone();

            let error = apply_bound_authenticated_work_terminalization_inner(
                repo,
                &session.id,
                &binding,
                None,
                bound_work_terminalization_request(&session),
                |_| {
                    gwt_agent::rotate_session_execution_capability(&sessions_dir, &session_id)
                        .expect("rotate capability between resolve and commit");
                },
            )
            .expect_err("commit must reject a capability rotated after target resolution");
            assert_execution_binding_denial(&error);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "commit-time binding failure must leave Work byte-equivalent"
            );
        });
    }

    #[test]
    fn exact_work_terminalization_rejects_changed_work_without_mutation() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(repo, repo, &session, "work-terminal-exact-current");
            let before = WorkMutationSnapshot::capture(repo, repo);

            let error = apply_bound_authenticated_work_terminalization_for_exact_work(
                repo,
                &session.id,
                &binding,
                "work-terminal-exact-predecessor",
                gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy::EmitIfNeeded,
                bound_work_terminalization_request(&session),
            )
            .expect_err("an exact terminal continuation must not follow a changed assignment");

            assert_eq!(
                error.code,
                AgentWorkspaceUpdateErrorCode::TransactionConflict
            );
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "exact Work mismatch must preserve every canonical Work surface"
            );
        });
    }

    #[test]
    fn exact_work_terminal_confirmation_never_emits_for_an_active_work() {
        with_strict_target_fixture(|repo, session| {
            let (session, binding) = bind_session_to_current_execution(repo, session);
            seed_work_mutation_surfaces(repo, repo);
            let work_id = "work-terminal-confirm-active";
            seed_unique_mutation_target(repo, repo, &session, work_id);
            let before = WorkMutationSnapshot::capture(repo, repo);

            let error = apply_bound_authenticated_work_terminalization_for_exact_work(
                repo,
                &session.id,
                &binding,
                work_id,
                gwt_core::workspace_projection::ExactWorkspaceTerminalPolicy::ConfirmOnly,
                bound_work_terminalization_request(&session),
            )
            .expect_err("confirm-only must not create a missing canonical terminal");

            assert_eq!(
                error.code,
                AgentWorkspaceUpdateErrorCode::TransactionConflict
            );
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "confirm-only refusal must preserve every canonical Work surface"
            );
        });
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
    fn authenticated_terminal_workspace_update_refuses_before_mutation_when_settlement_store_is_unwritable(
    ) {
        with_strict_target_fixture(|repo, session| {
            seed_work_mutation_surfaces(repo, repo);
            seed_unique_mutation_target(
                repo,
                repo,
                session,
                "work-authenticated-terminal-store-failure",
            );
            let before = WorkMutationSnapshot::capture(repo, repo);
            let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(repo)
                .expect("strict fixture has a trusted-store path");
            std::fs::create_dir_all(trusted_dir.parent().expect("trusted-store parent"))
                .expect("create trusted-store parent");
            std::fs::write(&trusted_dir, b"block trusted-store directory creation")
                .expect("make trusted-store path unwritable");

            let error = apply_authenticated_workspace_update(
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
                        summary: Some("must not persist without settlement authority".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
            )
            .expect_err("unwritable settlement store must reject the terminal update");

            assert_eq!(error.code, AgentWorkspaceUpdateErrorCode::Internal);
            assert_eq!(
                WorkMutationSnapshot::capture(repo, repo),
                before,
                "settlement authority must be reserved before any terminal Work surface mutates"
            );
        });
    }

    #[test]
    fn authenticated_terminal_workspace_update_succeeds_when_post_persist_refresh_fails() {
        with_strict_target_fixture(|repo, session| {
            seed_unique_mutation_target(
                repo,
                repo,
                session,
                "work-authenticated-terminal-refresh-failure",
            );

            let receipt = apply_authenticated_workspace_update_inner(
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
                        summary: Some("terminal mutation is already durable".to_string()),
                        ..AgentWorkspaceUpdateIntent::default()
                    },
                },
                |_, _| {
                    Err(std::io::Error::other(
                        "synthetic post-persist refresh failure",
                    ))
                },
            )
            .expect("a prepared receipt makes post-persist refresh best-effort");

            assert_eq!(
                receipt.work_id,
                "work-authenticated-terminal-refresh-failure"
            );
            let record = crate::cli::verification_record::load_work_event_settlement_record(repo)
                .expect("load write-ahead settlement receipt")
                .expect("write-ahead settlement receipt exists");
            assert!(record.obligation_open);
            assert!(matches!(
                record.status,
                crate::cli::verification_record::WorkEventSettlementStatus::PendingMutation { .. }
            ));
            assert_eq!(
                crate::cli::verification_record::work_event_settlement_refusal(repo),
                None,
                "an unreachable environment must warn without blocking a later Stop"
            );
            let refreshed =
                crate::cli::verification_record::load_work_event_settlement_record(repo)
                    .expect("load refreshed settlement receipt")
                    .expect("refreshed settlement receipt exists");
            assert!(refreshed.obligation_open);
            assert!(matches!(
                refreshed.status,
                crate::cli::verification_record::WorkEventSettlementStatus::Blocked(
                    crate::cli::verification_record::WorkEventSettlementBlocker::PathDirtyInUnreachableEnvironment {
                        ..
                    }
                )
            ));
            assert_eq!(
                refreshed.status.severity(),
                crate::cli::verification_record::WorkEventSettlementSeverity::Warning,
                "a later Stop refresh must retain the obligation as a non-blocking offline warning"
            );
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
    fn strict_session_work_mutation_target_rejects_foreign_owner_and_agent_identity() {
        for mismatch in ["owner", "agent"] {
            with_strict_target_fixture(|repo, session| {
                let (session, _) = bind_session_to_current_execution(repo, session);
                let work_id = format!("work-strict-{mismatch}-mismatch");
                seed_unique_mutation_target(repo, repo, &session, &work_id);
                let path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(repo);
                let mut work_items =
                    gwt_core::workspace_projection::load_workspace_work_items_from_path(&path)
                        .expect("load strict WorkItems")
                        .expect("strict WorkItems");
                let work = work_items
                    .work_items
                    .iter_mut()
                    .find(|item| item.id == work_id)
                    .expect("strict Work");
                match mismatch {
                    "owner" => work.owner = Some("Issue #9999".to_string()),
                    "agent" => work.agents[0].agent_id = Some("claude".to_string()),
                    _ => unreachable!(),
                }
                save_mutation_work_items(repo, &work_items);

                let error = resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("foreign Work authority must fail closed");
                assert!(
                    error.to_string().to_ascii_lowercase().contains(mismatch),
                    "{mismatch}: {error}"
                );
            });
        }
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
                    .expect_err("superseded conflicting authority must remain ambiguous"),
                "ambiguous",
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
            assert_workspace_ensure_error(
                resolve_session_work_mutation_target(repo, &session.id)
                    .expect_err("terminal Session shadow must remain ambiguous"),
                "ambiguous",
            );

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
    fn strict_session_work_mutation_target_ignores_terminal_and_paused_foreign_history() {
        for (label, status) in [
            (
                "terminal",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
            ),
            (
                "paused",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
            ),
        ] {
            with_strict_target_fixture(|repo, session| {
                let work_id = "work-current";
                seed_unique_mutation_target(repo, repo, session, work_id);

                let mut work_items = mutation_work_items(repo, session, work_id);
                let mut history = work_items.work_items[0].clone();
                history.id = format!("work-{label}-history");
                history.status_category = status;
                history.completed_at = (status
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Done)
                    .then(Utc::now);
                history.agents[0].session_id = "historical-session".to_string();
                work_items.work_items.push(history);
                save_mutation_work_items(repo, &work_items);

                let target = resolve_session_work_mutation_target(repo, &session.id)
                    .expect("historical foreign Work must not shadow exact current authority");
                assert_eq!(target.work_id, work_id);
            });
        }
    }

    #[test]
    fn strict_session_work_mutation_target_rejects_nonhistorical_or_current_paused_shadow() {
        for (label, status, shadow_is_current) in [
            (
                "blocked",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked,
                false,
            ),
            (
                "unknown",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown,
                false,
            ),
            (
                "paused-current",
                gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
                true,
            ),
        ] {
            with_strict_target_fixture(|repo, session| {
                let work_id = "work-current";
                seed_unique_mutation_target(repo, repo, session, work_id);

                let mut work_items = mutation_work_items(repo, session, work_id);
                let mut shadow = work_items.work_items[0].clone();
                shadow.id = format!("work-{label}-shadow");
                shadow.status_category = status;
                shadow.completed_at = None;
                if !shadow_is_current {
                    shadow.agents[0].session_id = "historical-session".to_string();
                }
                work_items.work_items.push(shadow);
                save_mutation_work_items(repo, &work_items);

                assert_workspace_ensure_error(
                    resolve_session_work_mutation_target(repo, &session.id)
                        .expect_err("unsafe container shadow must fail closed"),
                    "ambiguous",
                );
            });
        }
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
            (
                "repo hash",
                repo_hash,
                repo.clone(),
                WorkspaceUpdateApplicabilityReason::RepositoryMismatch,
            ),
            (
                "canonical repository",
                canonical_repository,
                repo.clone(),
                WorkspaceUpdateApplicabilityReason::RepositoryMismatch,
            ),
            (
                "branch",
                branch_mismatch,
                repo.clone(),
                WorkspaceUpdateApplicabilityReason::BranchMismatch,
            ),
            (
                "worktree",
                worktree,
                repo.clone(),
                WorkspaceUpdateApplicabilityReason::CwdMismatch,
            ),
            (
                "cwd",
                cwd,
                sibling,
                WorkspaceUpdateApplicabilityReason::CwdMismatch,
            ),
            (
                "event root",
                event_root,
                nested_event_root,
                WorkspaceUpdateApplicabilityReason::RepositoryMismatch,
            ),
        ];
        let mut failures = Vec::new();

        for (expected_mismatch, session, invocation_cwd, expected_reason) in cases {
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
                    assert_eq!(
                        diagnose_session_work_mutation_target(&invocation_cwd, &session.id)
                            .expect_err("typed applicability reason")
                            .reason,
                        expected_reason,
                        "typed applicability must distinguish {expected_mismatch}"
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
        let mut provider_neutral_work = mutation_work_items(&repo, &provider_present, work_id);
        let mut second_session_claim = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Claim,
            work_id,
            Utc::now(),
        );
        second_session_claim.agent_session_id = Some(provider_absent.id.clone());
        second_session_claim.agent_id = Some(provider_absent.agent_id.command().to_string());
        provider_neutral_work.apply_event(second_session_claim);
        save_mutation_work_items_with_tracked_events(&repo, &repo, &provider_neutral_work);

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
        run_git(
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/gwt/recovery-workspace.git",
            ],
            &bare_repo,
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
        assert_eq!(
            validated_project_state_root_for_session_recovery(&session)
                .expect("validate derived recovery Project State root"),
            dunce::canonicalize(&workspace_home).expect("canonical workspace home")
        );
    }

    #[test]
    fn explicit_linked_worktree_root_is_a_valid_project_state_anchor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace-home");
        let bare_repo = workspace_home.join("gwt.git");
        std::fs::create_dir_all(&workspace_home).expect("workspace home");
        run_git(
            &["init", "--bare", bare_repo.to_str().unwrap()],
            temp.path(),
        );
        run_git(
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/gwt.git",
            ],
            &bare_repo,
        );

        let bootstrap = temp.path().join("bootstrap");
        run_git(&["init", bootstrap.to_str().unwrap()], temp.path());
        run_git(&["config", "user.email", "test@example.com"], &bootstrap);
        run_git(&["config", "user.name", "Test User"], &bootstrap);
        run_git(&["checkout", "-b", "develop"], &bootstrap);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &bootstrap);
        run_git(
            &[
                "remote",
                "add",
                "origin",
                bare_repo.to_str().expect("bare repo path"),
            ],
            &bootstrap,
        );
        run_git(&["push", "origin", "develop"], &bootstrap);

        let worktree = workspace_home.join("work").join("issue-3393");
        std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
            .expect("worktree parent");
        run_git(
            &["worktree", "add", worktree.to_str().unwrap(), "develop"],
            &bare_repo,
        );

        let mut session = Session::new(&worktree, "develop", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(worktree.clone());
        assert_eq!(
            validated_project_state_root_for_session_recovery(&session)
                .expect("linked worktree root is a visible Project State root"),
            dunce::canonicalize(&worktree).expect("canonical linked worktree")
        );

        let nested = worktree.join("nested");
        std::fs::create_dir_all(&nested).expect("nested worktree directory");
        session.project_state_root = Some(nested);
        validated_project_state_root_for_session_recovery(&session)
            .expect_err("a nested cwd is not a Project State anchor");
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

    #[test]
    fn execution_continuation_receipt_has_truthful_typed_outcomes() {
        let rebound = AgentExecutionContinuationReceipt {
            schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: "operation-rebound".to_string(),
            outcome: AgentExecutionContinuationOutcome::ReboundCurrent,
            predecessor_generation_id: None,
            generation_id: "generation-current".to_string(),
            execution_binding: ExecutionBindingIdentity {
                generation_id: "generation-current".to_string(),
                binding_id: "binding-current".to_string(),
                ledger_head_hash: "head-current".to_string(),
            },
            capability_generation: 2,
            superseded_execution_binding: None,
            takeover_audit_id: None,
            validated: true,
        };
        let successor = AgentExecutionContinuationReceipt {
            schema_version: AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: "operation-successor".to_string(),
            outcome: AgentExecutionContinuationOutcome::SuccessorCreated,
            predecessor_generation_id: Some("generation-current".to_string()),
            generation_id: "generation-successor".to_string(),
            execution_binding: ExecutionBindingIdentity {
                generation_id: "generation-successor".to_string(),
                binding_id: "binding-successor".to_string(),
                ledger_head_hash: "head-successor".to_string(),
            },
            capability_generation: 3,
            superseded_execution_binding: Some(ExecutionBindingIdentity {
                generation_id: "generation-current".to_string(),
                binding_id: "binding-current".to_string(),
                ledger_head_hash: "head-current".to_string(),
            }),
            takeover_audit_id: Some("operation-successor".to_string()),
            validated: true,
        };

        assert_eq!(
            serde_json::to_value(rebound).unwrap()["outcome"],
            "rebound_current"
        );
        assert_eq!(
            serde_json::to_value(successor).unwrap()["outcome"],
            "successor_created"
        );
    }
}
