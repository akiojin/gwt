//! Launch authority finalization shared by GUI and project runtime owners.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::agent_capability::AgentCapabilityIssuer;
use crate::project_runtime::launch_authority::{
    install_agent_capability_env, issue_preflighted_agent_capability_env,
    preflight_agent_capability_endpoints, ActiveContinuationInstall,
    AgentCapabilityLaunchAuthority,
};
use crate::project_runtime::launch_payload::{
    AgentLaunchError, LaunchFeedbackContext, PendingContinueWork, PendingContinueWorkExecution,
    PendingFreshExecutionLaunch, WorkspaceResumeContext,
};
use crate::session_launch::durable_launch_recovery_exists;
use crate::session_launch::recovery::{
    clear_durable_launch_recovery, durable_launch_recovery_session_identity,
    persist_durable_launch_recovery, persist_durable_launch_recovery_with_identity,
    DurableLaunchRecoveryKind,
};

pub fn pending_execution_activation_status(pending: &PendingContinueWork) -> Option<bool> {
    match &pending.execution {
        PendingContinueWorkExecution::Successor(_) => {
            crate::cli::execution_state::continuation_attempt_for_operation(
                &pending.worktree_path,
                pending.owner,
                &pending.operation_id,
            )
            .ok()
            .map(|attempt| {
                attempt.is_some_and(|attempt| {
                    attempt.status
                        == crate::cli::execution_state::ContinuationAttemptStatus::Activated
                })
            })
        }
        PendingContinueWorkExecution::Takeover(_) => {
            crate::cli::execution_state::generation_takeover_attempt_for_operation(
                &pending.worktree_path,
                pending.owner,
                &pending.operation_id,
            )
            .ok()
            .map(|attempt| {
                attempt.is_some_and(|attempt| {
                    attempt.status
                        == crate::cli::execution_state::GenerationTakeoverAttemptStatus::Activated
                })
            })
        }
    }
}

pub fn pending_fresh_execution_activation_status(
    pending: &PendingFreshExecutionLaunch,
) -> Option<bool> {
    pending_fresh_execution_attempt_status(pending)
        .map(|status| status == crate::cli::execution_state::ContinuationAttemptStatus::Activated)
}

pub fn pending_fresh_execution_attempt_status(
    pending: &PendingFreshExecutionLaunch,
) -> Option<crate::cli::execution_state::ContinuationAttemptStatus> {
    crate::cli::execution_state::continuation_attempt_for_operation(
        &pending.worktree_path,
        pending.owner,
        &pending.operation_id,
    )
    .ok()
    .flatten()
    .map(|attempt| attempt.status)
}

pub fn pending_continue_work_session_identity(
    pending: &PendingContinueWork,
) -> Result<gwt_agent::SessionExecutionIdentity, String> {
    let mut session = gwt_agent::Session::new(
        &pending.worktree_path,
        pending.work_branch.clone(),
        pending.work_agent_id.clone(),
    );
    session.id = pending.binding.session_id.clone();
    session.project_state_root = Some(pending.project_root.clone());
    session.repo_hash = Some(pending.binding.repo_hash.clone());
    session.linked_issue_number = Some(pending.owner.number);
    gwt_agent::SessionExecutionIdentity::for_binding(&session, &pending.binding)
}

#[allow(clippy::too_many_arguments)]
pub fn abort_prepared_execution_and_remove_exact_session<F>(
    worktree: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    execution: &PendingContinueWorkExecution,
    reason: &str,
    sessions_dir: &Path,
    session_identity: &gwt_agent::SessionExecutionIdentity,
    after_abort: F,
) -> std::io::Result<bool>
where
    F: FnOnce() -> std::io::Result<()>,
{
    match execution {
        PendingContinueWorkExecution::Successor(request) => {
            crate::cli::execution_state::abort_successor_and_remove_exact_session(
                worktree,
                owner,
                request,
                reason,
                sessions_dir,
                session_identity,
                after_abort,
            )
        }
        PendingContinueWorkExecution::Takeover(request) => {
            crate::cli::execution_state::abort_generation_takeover_and_remove_exact_session(
                worktree,
                owner,
                request,
                reason,
                sessions_dir,
                session_identity,
                after_abort,
            )
        }
    }
}

/// Return false only for an irreversible Activated generation. Exact candidate
/// conflicts and unreadable authority retain their evidence for reconciliation.
pub fn rollback_worker_continue_work(
    sessions_dir: &Path,
    pending: &PendingContinueWork,
) -> Result<bool, String> {
    match pending_execution_activation_status(pending) {
        Some(true) => return Ok(false),
        Some(false) => {}
        None => return Err("continuation activation status is unreadable".into()),
    }
    let identity = pending_continue_work_session_identity(pending)?;
    let cleanup = abort_prepared_execution_and_remove_exact_session(
        &pending.worktree_path,
        pending.owner,
        &pending.execution,
        "continuation launch failed before SessionStart",
        sessions_dir,
        &identity,
        || {
            reject_continue_work_workspace_commit(
                &pending.project_root,
                &pending.worktree_path,
                &pending.operation_id,
            )
        },
    );
    match cleanup {
        Ok(true) => {}
        Ok(false) => {
            return Err("continuation candidate Session changed before exact rollback".into())
        }
        Err(_) if pending_execution_activation_status(pending) == Some(true) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    }
    clear_durable_launch_recovery(sessions_dir, &pending.binding.session_id)?;
    Ok(true)
}

pub fn rollback_worker_fresh_execution(
    sessions_dir: &Path,
    pending: &PendingFreshExecutionLaunch,
) -> Result<bool, String> {
    match pending_fresh_execution_activation_status(pending) {
        Some(true) => return Ok(false),
        Some(false) => {}
        None => return Err("fresh launch activation status is unreadable".into()),
    }
    let cleanup = crate::cli::execution_state::abort_successor_and_remove_exact_session(
        &pending.worktree_path,
        pending.owner,
        &pending.request,
        "fresh linked-owner launch failed before SessionStart",
        sessions_dir,
        &pending.session_identity,
        || {
            reject_continue_work_workspace_commit(
                &pending.project_root,
                &pending.worktree_path,
                &pending.operation_id,
            )
        },
    );
    match cleanup {
        Ok(true) => {}
        Ok(false) => return Err("fresh candidate Session changed before exact rollback".into()),
        Err(_) if pending_fresh_execution_activation_status(pending) == Some(true) => {
            return Ok(false)
        }
        Err(error) => return Err(error.to_string()),
    }
    if !pending
        .launch_feedback_context
        .as_ref()
        .is_some_and(|context| {
            context
                .issue_monitor_launch_binding_issue_number()
                .is_some()
                && context.issue_monitor_delivery_id.is_some()
                && context.issue_monitor_autonomous_handoff.is_none()
        })
    {
        clear_durable_launch_recovery(sessions_dir, &pending.binding.session_id)?;
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
pub fn pending_fresh_execution_launch_from_session(
    sessions_dir: &Path,
    session_id: &str,
    worktree_path: &Path,
    agent_project_root: &str,
    linked_issue_number: Option<u64>,
    base_branch: Option<String>,
    resume_context: Option<WorkspaceResumeContext>,
    launch_feedback_context: Option<LaunchFeedbackContext>,
    readiness_nonce: &str,
    expected_agent_id: &gwt_agent::AgentId,
) -> Result<PendingFreshExecutionLaunch, String> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path).map_err(|error| {
        format!(
            "fresh linked-owner launch Session could not be read at {}: {error}",
            session_path.display()
        )
    })?;
    if session.id != session_id
        || &session.agent_id != expected_agent_id
        || !same_worktree_path(&session.worktree_path, worktree_path)
    {
        return Err(
            "fresh linked-owner launch Session does not match its materialized worktree"
                .to_string(),
        );
    }
    let binding = session.execution_binding.clone().ok_or_else(|| {
        "fresh linked-owner launch Session has no Prepared execution binding".to_string()
    })?;
    if binding.session_id != session_id
        || session.linked_issue_number != Some(binding.owner_number)
        || linked_issue_number != Some(binding.owner_number)
    {
        return Err(
            "fresh linked-owner launch owner does not match its persisted Session binding"
                .to_string(),
        );
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("fresh linked-owner launch owner kind is not canonical".to_string()),
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let ledger = crate::cli::execution_state::load_generation_ledger(worktree_path, owner)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "fresh linked-owner launch owner ledger is missing".to_string())?;
    let attempt = crate::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
        worktree_path,
        owner,
        session_id,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| {
        "fresh linked-owner launch has no unique Prepared successor attempt".to_string()
    })?;
    let expected_status = match attempt.predecessor_status {
        crate::cli::execution_state::SuccessorPredecessorStatus::Blocked => {
            crate::cli::execution_state::ExecutionControlStatus::Blocked
        }
        crate::cli::execution_state::SuccessorPredecessorStatus::Completed => {
            crate::cli::execution_state::ExecutionControlStatus::Completed
        }
        crate::cli::execution_state::SuccessorPredecessorStatus::Active => {
            crate::cli::execution_state::ExecutionControlStatus::Active
        }
    };
    if ledger.current_effective_status() != Some(expected_status) {
        return Err("fresh linked-owner launch predecessor status changed".to_string());
    }
    if !crate::cli::execution_state::prepared_execution_binding_matches(
        worktree_path,
        owner,
        session_id,
        &binding.identity,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(
            "fresh linked-owner launch Session does not match its Prepared successor".to_string(),
        );
    }
    let predecessor_binding =
        crate::cli::execution_state::current_execution_binding(worktree_path, owner)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "fresh linked-owner launch predecessor binding is missing".to_string()
            })?;
    if predecessor_binding == binding.identity {
        return Err(
            "fresh linked-owner launch candidate is already the current generation".to_string(),
        );
    }
    let project_root = session
        .project_state_root
        .clone()
        .or_else(|| {
            (!agent_project_root.trim().is_empty()).then(|| PathBuf::from(agent_project_root))
        })
        .ok_or_else(|| {
            "fresh linked-owner launch canonical Project State root is missing".to_string()
        })?;
    let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .map_err(|error| format!("fresh linked-owner Session identity is invalid: {error}"))?
        .ok_or_else(|| "fresh linked-owner Session identity is missing".to_string())?;
    if durable_launch_recovery_session_identity(sessions_dir, session_id)?
        != Some(session_identity.clone())
    {
        return Err(
            "fresh linked-owner Session no longer matches its durable launch identity".to_string(),
        );
    }
    Ok(PendingFreshExecutionLaunch {
        operation_id: attempt.request.operation_id.clone(),
        project_root,
        worktree_path: worktree_path.to_path_buf(),
        owner,
        request: attempt.request,
        binding,
        session_identity,
        readiness_nonce: readiness_nonce.to_string(),
        predecessor_binding,
        base_branch,
        linked_issue_number,
        resume_context,
        launch_feedback_context,
    })
}

pub fn rollback_materialized_fresh_execution_launch(
    sessions_dir: &Path,
    session_id: &str,
    worktree_path: &Path,
    reason: &str,
    expected_agent_id: &gwt_agent::AgentId,
) -> Result<(), String> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path).map_err(|error| error.to_string())?;
    let binding = session.execution_binding.clone().ok_or_else(|| {
        "fresh linked-owner rollback cannot prove the candidate Session binding".to_string()
    })?;
    let session_identity = durable_launch_recovery_session_identity(sessions_dir, session_id)?
        .ok_or_else(|| "fresh linked-owner rollback identity is missing".to_string())?;
    if session.id != session_id
        || &session.agent_id != expected_agent_id
        || binding.session_id != session_id
        || !same_worktree_path(&session.worktree_path, worktree_path)
    {
        return Err("fresh linked-owner rollback candidate identity no longer matches".to_string());
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => {
            return Err("fresh linked-owner rollback candidate owner is not canonical".to_string())
        }
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let attempt = crate::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
        worktree_path,
        owner,
        session_id,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "fresh linked-owner rollback found no unique Prepared candidate".to_string())?;
    if !crate::cli::execution_state::prepared_execution_binding_matches(
        worktree_path,
        owner,
        session_id,
        &binding.identity,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(
            "fresh linked-owner rollback candidate binding is no longer Prepared".to_string(),
        );
    }
    match crate::cli::execution_state::abort_successor_and_remove_exact_session(
        worktree_path,
        owner,
        &attempt.request,
        reason,
        sessions_dir,
        &session_identity,
        || Ok(()),
    ) {
        Ok(true) => clear_durable_launch_recovery(sessions_dir, session_id),
        Ok(false) => Err(
            "fresh linked-owner rollback retained a Session whose binding no longer matches"
                .to_string(),
        ),
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Clone)]
pub struct MaterializedGenesisLaunch {
    pub worktree_path: PathBuf,
    pub owner: crate::cli::execution_state::ExecutionOwnerKey,
    pub session_id: String,
    pub binding: gwt_agent::SessionExecutionBinding,
    pub session_identity: gwt_agent::SessionExecutionIdentity,
}

pub fn materialized_genesis_launch_from_session(
    sessions_dir: &Path,
    session_id: &str,
    worktree_path: &Path,
    project_state_root: &Path,
    branch: &str,
    linked_issue_number: Option<u64>,
    expected_agent_id: &gwt_agent::AgentId,
) -> Result<Option<MaterializedGenesisLaunch>, String> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    match std::fs::symlink_metadata(&session_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let session = gwt_agent::Session::load(&session_path).map_err(|error| {
        format!(
            "materialized genesis Session could not be read at {}: {error}",
            session_path.display()
        )
    })?;
    let Some(binding) = session.execution_binding.clone() else {
        return Ok(None);
    };
    if session.id != session_id
        || &session.agent_id != expected_agent_id
        || binding.session_id != session_id
        || !same_worktree_path(&session.worktree_path, worktree_path)
        || session.linked_issue_number != Some(binding.owner_number)
        || linked_issue_number != Some(binding.owner_number)
    {
        return Err(
            "materialized genesis Session no longer matches its launch identity".to_string(),
        );
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("materialized genesis owner kind is not canonical".to_string()),
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    if crate::cli::execution_state::current_execution_binding(worktree_path, owner)
        .map_err(|error| error.to_string())?
        != Some(binding.identity.clone())
    {
        return Err(
            "materialized genesis Session is not the current execution generation".to_string(),
        );
    }
    let ledger = crate::cli::execution_state::load_generation_ledger(worktree_path, owner)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "materialized genesis generation ledger is missing".to_string())?;
    if ledger.generations.len() != 1
        || ledger.current_effective_status()
            != Some(crate::cli::execution_state::ExecutionControlStatus::Active)
    {
        return Ok(None);
    }
    let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .map_err(|error| format!("materialized genesis identity is invalid: {error}"))?
        .ok_or_else(|| "materialized genesis identity is missing".to_string())?;
    let mut expected_session =
        gwt_agent::Session::new(worktree_path, branch, expected_agent_id.clone());
    expected_session.id = session_id.to_string();
    expected_session.project_state_root = Some(project_state_root.to_path_buf());
    expected_session.repo_hash = Some(binding.repo_hash.clone());
    expected_session.linked_issue_number = linked_issue_number;
    let expected_session_identity =
        gwt_agent::SessionExecutionIdentity::for_binding(&expected_session, &binding)?;
    if session_identity != expected_session_identity {
        return Err("materialized genesis durable Session identity changed".to_string());
    }
    if let Ok(Some(receipt_identity)) =
        durable_launch_recovery_session_identity(sessions_dir, session_id)
    {
        if receipt_identity != expected_session_identity {
            return Err("materialized genesis recovery identity changed".to_string());
        }
    }
    Ok(Some(MaterializedGenesisLaunch {
        worktree_path: worktree_path.to_path_buf(),
        owner,
        session_id: session_id.to_string(),
        binding,
        session_identity,
    }))
}

pub fn terminalize_materialized_genesis_launch(
    sessions_dir: &Path,
    genesis: Option<&MaterializedGenesisLaunch>,
    reason: &str,
) -> Result<(), String> {
    let Some(genesis) = genesis else {
        return Ok(());
    };
    let removed = crate::cli::execution_state::block_genesis_and_remove_exact_session(
        &genesis.worktree_path,
        genesis.owner,
        &genesis.session_id,
        &genesis.binding.identity,
        reason,
        sessions_dir,
        &genesis.session_identity,
        || Ok(()),
    )
    .map_err(|error| error.to_string())?;
    if !removed {
        return Err("terminal genesis Session identity changed before exact cleanup".to_string());
    }
    clear_durable_launch_recovery(sessions_dir, &genesis.session_id)
}

pub fn terminalized_genesis_failure_detail(
    sessions_dir: &Path,
    genesis: Option<&MaterializedGenesisLaunch>,
    reason: &str,
    detail: impl Into<String>,
) -> String {
    let detail = detail.into();
    let Some(genesis) = genesis else {
        return detail;
    };
    match terminalize_materialized_genesis_launch(sessions_dir, Some(genesis), reason) {
        Ok(()) => format!("{detail}; genesis authority was terminalized"),
        Err(error) => format!(
            "{detail}; failed genesis terminalization retained exact evidence for retry: {error}"
        ),
    }
}

pub fn persist_genesis_session_binding_with<F>(
    sessions_dir: &Path,
    session: &mut gwt_agent::Session,
    genesis: &MaterializedGenesisLaunch,
    persist: F,
) -> Result<(), String>
where
    F: FnOnce(&gwt_agent::Session) -> std::io::Result<bool>,
{
    match persist(session) {
        Ok(true) => Ok(()),
        Ok(false) => {
            let _ = session.set_execution_binding(None);
            Err(terminalized_genesis_failure_detail(
                sessions_dir,
                Some(genesis),
                "genesis Session binding persistence conflict",
                "failed to persist the genesis Session binding: same-id Session changed",
            ))
        }
        Err(error) => {
            let _ = session.set_execution_binding(None);
            Err(terminalized_genesis_failure_detail(
                sessions_dir,
                Some(genesis),
                "genesis Session binding persistence failed",
                format!("failed to persist the genesis Session binding: {error}"),
            ))
        }
    }
}

/// Issue #3759: best-effort republication of a lost worktree pointer/projection
/// pair from the committed owner ledger, run before a strict authority read.
///
/// The repair is idempotent and never touches the owner ledger. Any failure
/// is logged only: the strict read that follows reports the real state, so a
/// heal error can never mask or replace the authoritative refusal.
pub fn heal_lost_generation_publication_best_effort(
    worktree: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
) {
    use crate::cli::execution_state::GenerationPublicationHeal;
    match crate::cli::execution_state::heal_missing_generation_publication(worktree, owner) {
        Ok(GenerationPublicationHeal::Republished) => tracing::warn!(
            owner_kind = owner.kind.as_str(),
            owner_number = owner.number,
            worktree = %worktree.display(),
            "execution generation pointer/projection were missing after ledger ownership; republished from the owner ledger"
        ),
        Ok(GenerationPublicationHeal::ForeignWorktree) => tracing::warn!(
            owner_kind = owner.kind.as_str(),
            owner_number = owner.number,
            worktree = %worktree.display(),
            "execution generation pointer is missing and the current generation belongs to another worktree; left fail-closed"
        ),
        Ok(
            GenerationPublicationHeal::NoLedger
            | GenerationPublicationHeal::AlreadyPublished
            | GenerationPublicationHeal::NotApplicable,
        ) => {}
        Err(error) => tracing::warn!(
            owner_kind = owner.kind.as_str(),
            owner_number = owner.number,
            worktree = %worktree.display(),
            %error,
            "execution generation publication heal failed; the strict authority read decides"
        ),
    }
}

pub struct FinalizedAgentCapabilityLaunch<'a> {
    pub issuer: Option<&'a AgentCapabilityIssuer>,
    pub sessions_dir: &'a Path,
    pub session: &'a mut gwt_agent::Session,
    pub project_root: &'a Path,
    pub worktree: &'a Path,
    pub producing_owner: Option<crate::cli::execution_state::ExecutionOwnerKey>,
    pub prepared_continuation: Option<&'a gwt_agent::SessionExecutionBinding>,
    /// Issue #3423: a `ReboundCurrent` continuation re-validated the current
    /// generation for the predecessor Session. It carries no Prepared attempt,
    /// so it installs as Active authority instead of the Prepared path.
    pub rebound_continuation: Option<&'a gwt_agent::SessionExecutionBinding>,
    pub execution_entrypoint: &'a str,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub container_runtime: Option<&'a gwt_docker::detect::ResolvedContainerRuntime>,
    /// Issue #4543 AC-5: the Permission Mode Decision this launch was
    /// materialized under, recorded onto the Execution Control Record this
    /// install creates. `None` for the test installs that stand a record up
    /// without a `LaunchConfig`.
    pub permission_decision: Option<&'a gwt_agent::PermissionModeDecision>,
}

impl FinalizedAgentCapabilityLaunch<'_> {
    pub fn install(
        self,
        env: &mut HashMap<String, String>,
    ) -> Result<Option<gwt_agent::SessionActiveLaunchHandshake>, String> {
        self.install_with_prepared_claim(env, None)
            .map_err(|error| error.to_string())
    }

    pub fn install_with_prepared_claim(
        self,
        env: &mut HashMap<String, String>,
        prepared_claim: Option<&gwt_agent::SessionActiveLaunchHandshake>,
    ) -> Result<Option<gwt_agent::SessionActiveLaunchHandshake>, AgentLaunchError> {
        let Self {
            issuer,
            sessions_dir,
            session,
            project_root,
            worktree,
            producing_owner,
            prepared_continuation,
            rebound_continuation,
            execution_entrypoint,
            runtime_target,
            container_runtime,
            permission_decision,
        } = self;
        if let Some(binding) = prepared_continuation {
            if producing_owner.is_some() {
                return Err(
                    "Prepared continuation cannot enter the genesis execution launch path".into(),
                );
            }
            let issuer = issuer.ok_or_else(|| {
                "Prepared continuation is missing its Host capability issuer".to_string()
            })?;
            if session.execution_binding.as_ref() != Some(binding) {
                return Err(
                    "Prepared continuation Session binding changed before capability issuance"
                        .into(),
                );
            }
            let owner_kind = match binding.owner_kind.as_str() {
                "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
                "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
                _ => return Err("Prepared continuation owner kind is not canonical".into()),
            };
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: owner_kind,
                number: binding.owner_number,
            };
            if !crate::cli::execution_state::prepared_execution_binding_matches(
                worktree,
                owner,
                &session.id,
                &binding.identity,
            )
            .map_err(|error| error.to_string())?
            {
                // Issue #4207 AC-2: a binding that is already the current
                // generation is not a stale Prepared attempt — it is Active
                // authority that was activated before it got here. Recover it
                // as such instead of refusing, or the owner is parked for as
                // long as the label disagrees with the ledger. Anything that
                // is neither Prepared nor current is a genuine mismatch and
                // still fails closed.
                //
                // A launch that already holds an Active claim for this exact
                // Session is excluded: this arm would have to take a second
                // handshake over the one the caller is holding, and the caller
                // owns finishing that one. Only the manual successor path
                // arrives with a claim, and it carries a real Prepared attempt.
                if prepared_claim.is_none()
                    && crate::cli::execution_state::current_execution_binding(worktree, owner)
                        .map_err(|error| error.to_string())?
                        .as_ref()
                        == Some(&binding.identity)
                {
                    return ActiveContinuationInstall {
                        issuer: Some(issuer),
                        sessions_dir,
                        session,
                        project_root,
                        worktree,
                        owner,
                        binding,
                        runtime_target,
                        container_runtime,
                        label: "Prepared continuation",
                    }
                    .install(env);
                }
                return Err(
                    "Prepared continuation no longer matches its owner generation attempt".into(),
                );
            }
            let expected = gwt_agent::SessionExecutionIdentity::for_binding(session, binding)?;
            if let Some(claim) = prepared_claim {
                if claim.execution_identity != expected {
                    return Err(
                        "Prepared continuation launch claim does not match the exact Session"
                            .into(),
                    );
                }
                if !session
                    .save_if_execution_identity_matches(sessions_dir, &expected)
                    .map_err(|error| error.to_string())?
                {
                    return Err(
                        "Prepared continuation Session claim changed before capability issuance"
                            .into(),
                    );
                }
            } else {
                match session.save_if_absent(sessions_dir) {
                Ok(true) => {}
                Ok(false) => {
                    return Err("Prepared continuation Session already exists; reconcile the operation before retrying".into())
                }
                Err(error) => {
                    return Err(format!(
                        "failed to create the Prepared continuation Session binding before capability issuance: {error}"
                    ).into())
                }
                }
            }
            let endpoints = preflight_agent_capability_endpoints(
                issuer,
                project_root,
                &session.id,
                runtime_target,
                container_runtime,
            )?;
            issue_preflighted_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                AgentCapabilityLaunchAuthority::Prepared(binding),
                endpoints,
            )?;
            return Ok(prepared_claim.cloned());
        }
        if let Some(binding) = rebound_continuation {
            // Issue #3423: the continuation coordinator re-validated and
            // persisted this binding for the predecessor Session
            // (`ReboundCurrent`). There is no Prepared attempt to activate —
            // re-check the binding against the live owner ledger and issue
            // Active authority for the in-place relaunch.
            if producing_owner.is_some() {
                return Err(
                    "Rebound continuation cannot enter the genesis execution launch path".into(),
                );
            }
            let owner_kind = match binding.owner_kind.as_str() {
                "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
                "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
                _ => return Err("Rebound continuation owner kind is not canonical".into()),
            };
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: owner_kind,
                number: binding.owner_number,
            };
            return ActiveContinuationInstall {
                issuer,
                sessions_dir,
                session,
                project_root,
                worktree,
                owner,
                binding,
                runtime_target,
                container_runtime,
                label: "Rebound continuation",
            }
            .install(env);
        }
        let Some(owner) = producing_owner else {
            install_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                runtime_target,
                container_runtime,
            )?;
            return Ok(None);
        };
        let issuer = issuer.ok_or_else(|| {
            "producing launch is missing its Host capability issuer; no execution was materialized"
                .to_string()
        })?;
        let endpoints = preflight_agent_capability_endpoints(
            issuer,
            project_root,
            &session.id,
            runtime_target,
            container_runtime,
        )?;

        // Issue #3759: the owner ledger commits before the worktree
        // pointer/projection pair, and trusted-store GC of a removed
        // worktree drops that pair while the ledger stays. Rebuild a lost
        // publication from the ledger before the strict authority read
        // below; otherwise every relaunch on this worktree dies before the
        // PTY and burns the Issue Monitor retry budget. A heal failure is
        // logged and the strict read reports the real state.
        heal_lost_generation_publication_best_effort(worktree, owner);
        // Issue #6118: retain a legacy Active owner in the generation ledger
        // before refusing a Monitor duplicate. This grants no new producer:
        // Live preserves the holder and Active status, so owner diagnostics
        // and the existing operator stop recovery can see the same authority.
        if session.launch_route == gwt_agent::LaunchRoute::Autonomous
            && crate::cli::execution_state::load_owner_generation_ledger(worktree, owner)
                .map_err(|error| error.to_string())?
                .is_none()
            && crate::cli::execution_state::load(worktree)
                .map_err(|error| error.to_string())?
                .is_some_and(|record| {
                    record.owner_kind == owner.kind
                        && record.owner_number == owner.number
                        && record.status
                            == crate::cli::execution_state::ExecutionControlStatus::Active
                })
        {
            crate::cli::execution_state::ensure_generation_ledger(
                worktree,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .map_err(|error| error.to_string())?;
        }
        // Issue #5288 / #5840: reconcile exited children and stopped holders
        // before the duplicate gate, including an audited reaper commit whose
        // publication was interrupted. The scanner and reaper validate exact
        // authority under their leases; arbitrary invalid authority stays closed.
        if session.launch_route == gwt_agent::LaunchRoute::Autonomous
            && crate::cli::execution_state::load(worktree)
                .map_err(|error| error.to_string())?
                .is_some_and(|record| {
                    record.owner_kind == owner.kind
                        && record.owner_number == owner.number
                        && record.status
                            == crate::cli::execution_state::ExecutionControlStatus::Active
                })
        {
            let scan = crate::cli::execution_state::inspect_startup_active_generation_ledgers(&[
                worktree.to_path_buf(),
            ]);
            for candidate in scan
                .candidates
                .into_iter()
                .filter(|candidate| candidate.owner == owner)
            {
                if candidate.replay_operation_id.is_some() {
                    crate::cli::execution_state::repair_startup_defunct_active_generation(
                        &candidate,
                    )
                    .map_err(|error| error.to_string())?;
                    continue;
                }
                let Some(holder) = crate::cli::execution_state::startup_candidate_holder_identity(
                    sessions_dir,
                    &candidate,
                )
                .map_err(|error| error.to_string())?
                else {
                    continue;
                };
                if matches!(
                    crate::cli::execution_state::classify_exact_session_runtime(
                        sessions_dir,
                        &holder
                    )
                    .map_err(|error| error.to_string())?,
                    crate::cli::execution_state::ExactSessionRuntimeDisposition::ChildExited
                        | crate::cli::execution_state::ExactSessionRuntimeDisposition::Terminal(_)
                ) {
                    crate::cli::execution_state::reap_startup_defunct_active_generation(
                        &candidate,
                        sessions_dir,
                        &holder,
                        &[],
                    )
                    .map_err(|error| error.to_string())?;
                }
            }
        }
        // Issue #5478 AC-3: Monitor must not materialize another producer over
        // an Active owner. Manual concurrent launches retain SPEC #3590, and
        // the projection read also covers generation-less legacy executions.
        if session.launch_route == gwt_agent::LaunchRoute::Autonomous
            && crate::cli::execution_state::load(worktree)
                .map_err(|error| error.to_string())?
                .is_some_and(|record| {
                    record.owner_kind == owner.kind
                        && record.owner_number == owner.number
                        && record.status
                            == crate::cli::execution_state::ExecutionControlStatus::Active
                })
        {
            return Err(format!(
                "Issue Monitor owner {} #{} already has an Active execution; inspect execution.status and continue the existing holder; if stale, use its recommended recovery before requeueing",
                owner.kind.as_str(),
                owner.number,
            ).into());
        }
        reconcile_unindexed_prepared_fresh_launches(sessions_dir, worktree, owner)?;
        let mut current_binding =
            crate::cli::execution_state::current_execution_binding(worktree, owner)
                .map_err(|error| error.to_string())?;
        if current_binding.is_none()
            && crate::cli::execution_state::load(worktree)
                .map_err(|error| error.to_string())?
                .is_some_and(|legacy| {
                    legacy.owner_kind == owner.kind
                        && legacy.owner_number == owner.number
                        && legacy.status
                            == crate::cli::execution_state::ExecutionControlStatus::Blocked
                })
        {
            // A pre-generation terminal ECR is still authoritative audit
            // evidence. Import its exact bytes before any launch writer can
            // materialize a new Active record over it; the ordinary Blocked
            // successor transaction below then owns readiness/activation.
            crate::cli::execution_state::ensure_generation_ledger(
                worktree,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Unknown,
            )
            .map_err(|error| error.to_string())?;
            current_binding =
                crate::cli::execution_state::current_execution_binding(worktree, owner)
                    .map_err(|error| error.to_string())?;
        }
        if let Some(current) = current_binding {
            let ledger = crate::cli::execution_state::load_generation_ledger(worktree, owner)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "the current execution binding has no integrity-valid owner ledger".to_string()
                })?;
            // Issue #3457 / Issue #3472: only an Active generation whose
            // holder can still return legitimately blocks a fresh launch.
            // Blocked and Completed predecessors are terminal — they can never
            // produce again — so they always release to a successor, and an
            // Active holder with decisive dead-runtime evidence is superseded
            // through the exact terminal proof transaction instead of
            // refusing forever.
            enum FreshSuccessorRoute {
                Blocked,
                Completed,
                DeadHolder(
                    Box<gwt_agent::SessionExecutionIdentity>,
                    gwt_agent::ManualLaunchRuntimeEvidence,
                ),
                /// SPEC #3590 FR-001: the holder is alive and keeps producing.
                Concurrent,
            }
            let route = match ledger.current_effective_status() {
                Some(crate::cli::execution_state::ExecutionControlStatus::Blocked) => {
                    FreshSuccessorRoute::Blocked
                }
                Some(crate::cli::execution_state::ExecutionControlStatus::Completed) => {
                    FreshSuccessorRoute::Completed
                }
                _ => {
                    match crate::cli::execution_state::unreachable_current_generation_holder(
                        sessions_dir,
                        worktree,
                        owner,
                    )
                    .map_err(|error| error.to_string())?
                    {
                        Some((holder, evidence)) => {
                            FreshSuccessorRoute::DeadHolder(Box::new(holder), evidence)
                        }
                        // SPEC #3590 FR-001 / FR-002: a live Active holder is no
                        // longer a reason to refuse. The launch starts its own
                        // generation beside the holder's, and both Sessions settle
                        // the generation they own (FR-009).
                        //
                        // The one case that is not concurrency is this exact
                        // Session relaunching itself: its own generation is mid
                        // transaction, so a second generation for the same Session
                        // would strand the first. That is a same-lifetime resume,
                        // and Continue work owns it.
                        None => {
                            if crate::cli::execution_state::load(worktree)
                                .map_err(|error| error.to_string())?
                                .is_some_and(|record| record.primary_session_id == session.id)
                            {
                                return Err(format!(
                                "this Session already holds an unreconciled active execution generation for {} #{}; use Continue work to resume it, or run the execution.status JSON operation for the exact recovery route",
                                owner.kind.as_str(),
                                owner.number,
                            ).into());
                            }
                            // Issue #4200 AC-2: concurrency is for a holder that is
                            // actually producing. A launch that died before its
                            // agent ever authenticated — a directory-trust prompt is
                            // how it happens in production — leaves a holder that
                            // every liveness reading here is structurally unable to
                            // tell apart from a live one, so its generation would
                            // otherwise stay Active forever and every later launch
                            // would file itself beside a producer that does not
                            // exist. The holder's own lifecycle record settles it: a
                            // Session that never delivered a hook never ran a turn,
                            // and terminalizing it interrupts nothing. Anything less
                            // certain than that — including an agent still inside the
                            // start-up grace — falls through to concurrency.
                            match crate::cli::execution_state::release_unstarted_launch_generation(
                            worktree,
                            owner,
                            sessions_dir,
                            "the launch holding this generation never started an agent",
                        ) {
                            Ok(crate::cli::execution_state::LaunchGenerationRelease::Released {
                                generation_id,
                                holder_session_id,
                            }) => {
                                tracing::info!(
                                    owner = owner.number,
                                    %generation_id,
                                    %holder_session_id,
                                    "released an execution generation whose launch never started an agent"
                                );
                                FreshSuccessorRoute::Blocked
                            }
                            _ => FreshSuccessorRoute::Concurrent,
                        }
                        }
                    }
                }
            };

            let repo_hash = session
                .repo_hash
                .clone()
                .filter(|repo_hash| !repo_hash.trim().is_empty())
                .ok_or_else(|| {
                    "producing Session is missing its canonical repository hash".to_string()
                })?;
            let source = match &route {
                FreshSuccessorRoute::Completed => {
                    crate::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
                }
                FreshSuccessorRoute::Blocked | FreshSuccessorRoute::DeadHolder(..) => {
                    crate::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE
                }
                FreshSuccessorRoute::Concurrent => {
                    crate::cli::execution_state::CONCURRENT_LINKED_OWNER_LAUNCH_SOURCE
                }
            };
            let request = crate::cli::execution_state::SuccessorRequest {
                operation_id: format!("fresh-launch-{}", uuid::Uuid::new_v4()),
                principal_id: "gwt-host-launch".to_string(),
                work_id: None,
                source: source.to_string(),
                session_binding_id: uuid::Uuid::new_v4().to_string(),
                initial_session_id: session.id.clone(),
                entrypoint: execution_entrypoint.to_string(),
                requested_at: chrono::Utc::now(),
            };
            persist_durable_launch_recovery(
                sessions_dir,
                DurableLaunchRecoveryKind::FreshSuccessor {
                    operation_id: request.operation_id.clone(),
                },
                &session.id,
                project_root,
                worktree,
                owner,
                None,
                None,
            )?;
            let prepared = match &route {
                FreshSuccessorRoute::DeadHolder(holder, evidence) => {
                    crate::cli::execution_state::prepare_exact_terminal_active_successor(
                        worktree,
                        owner,
                        &request,
                        sessions_dir,
                        holder,
                        *evidence,
                        "holder Host exited without settling its execution generation",
                    )
                    .map(|_| ())
                }
                // A settled Completed predecessor needs no liveness proof: the
                // holder already handed its authority back, so the transaction
                // only has to re-prove that the exact generation it settled is
                // still the current one.
                FreshSuccessorRoute::Completed => {
                    crate::cli::execution_state::prepare_exact_manual_launch_successor(
                        worktree,
                        owner,
                        &request,
                        crate::cli::execution_state::ExactManualLaunchPredecessor {
                            sessions_dir,
                            session: None,
                            runtime: None,
                            binding: &current,
                            status:
                                crate::cli::execution_state::SuccessorPredecessorStatus::Completed,
                            terminal_reason:
                                "predecessor generation settled Completed before this launch",
                        },
                    )
                    .map(|_| ())
                }
                FreshSuccessorRoute::Blocked => {
                    crate::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
                        worktree, owner, &request,
                    )
                    .map(|_| ())
                }
                // The live predecessor is left untouched on purpose: no
                // lifecycle event, no fence, no binding change.
                FreshSuccessorRoute::Concurrent => {
                    crate::cli::execution_state::prepare_concurrent_linked_owner_launch_successor(
                        worktree, owner, &request,
                    )
                    .map(|_| ())
                }
            };
            if let Err(error) = prepared {
                if crate::cli::execution_state::continuation_attempt_for_operation(
                    worktree,
                    owner,
                    &request.operation_id,
                )
                .is_ok_and(|attempt| attempt.is_none())
                {
                    let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                }
                return Err(error.to_string().into());
            }
            let abort_prepared = |reason: &str| {
                crate::cli::execution_state::abort_successor(worktree, owner, &request, reason)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            };
            let identity = match crate::cli::execution_state::prepared_successor_execution_binding(
                worktree, owner, &request,
            ) {
                Ok(identity) => identity,
                Err(error) => {
                    let abort = abort_prepared("fresh launch binding derivation failed");
                    if abort.is_ok() {
                        let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                    }
                    return Err((match abort {
                        Ok(()) => error.to_string(),
                        Err(abort_error) => format!(
                            "{error}; failed to abort fresh launch successor: {abort_error}"
                        ),
                    })
                    .into());
                }
            };
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash,
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            let session_identity =
                gwt_agent::SessionExecutionIdentity::for_binding(session, &binding)?;
            let unbound_session_snapshot = session.clone();
            if let Err(error) = persist_durable_launch_recovery_with_identity(
                sessions_dir,
                DurableLaunchRecoveryKind::FreshSuccessor {
                    operation_id: request.operation_id.clone(),
                },
                &session.id,
                project_root,
                worktree,
                owner,
                Some(&binding),
                Some(&session.agent_id),
                Some(&session_identity),
            ) {
                let abort = abort_prepared("fresh launch recovery binding persistence failed");
                if abort.is_ok() {
                    let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                }
                return Err((match abort {
                    Ok(()) => format!(
                        "failed to persist exact fresh launch recovery binding: {error}"
                    ),
                    Err(abort_error) => format!(
                        "failed to persist exact fresh launch recovery binding: {error}; failed to abort fresh launch successor: {abort_error}"
                    ),
                }).into());
            }
            if let Err(error) = session.set_execution_binding(Some(binding.clone())) {
                let abort = abort_prepared("fresh launch Session binding failed");
                if abort.is_ok() {
                    let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                }
                return Err((match abort {
                    Ok(()) => error,
                    Err(abort_error) => {
                        format!("{error}; failed to abort fresh launch successor: {abort_error}")
                    }
                })
                .into());
            }
            let save_result = session
                .save_if_absent_or_unchanged(sessions_dir, &unbound_session_snapshot)
                .and_then(|saved| {
                    if saved {
                        Ok(())
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "same-id Session already exists",
                        ))
                    }
                });
            if let Err(error) = save_result {
                let _ = session.set_execution_binding(None);
                let cleanup = crate::cli::execution_state::abort_successor_and_remove_exact_session(
                    worktree,
                    owner,
                    &request,
                    "fresh launch Session persistence failed",
                    sessions_dir,
                    &session_identity,
                    || Ok(()),
                );
                if cleanup.as_ref().is_ok_and(|removed| *removed) {
                    let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                }
                return Err(format!(
                    "failed to persist fresh launch Session binding: {error}; exact rollback: {}",
                    cleanup
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "retained replacement".to_string())
                )
                .into());
            }
            if let Err(error) = issue_preflighted_agent_capability_env(
                env,
                issuer,
                project_root,
                &session.id,
                AgentCapabilityLaunchAuthority::Prepared(&binding),
                endpoints,
            ) {
                let cleanup = crate::cli::execution_state::abort_successor_and_remove_exact_session(
                    worktree,
                    owner,
                    &request,
                    "fresh launch capability issuance failed",
                    sessions_dir,
                    &session_identity,
                    || Ok(()),
                );
                return Err((match cleanup {
                    Ok(true) => {
                        let _ = clear_durable_launch_recovery(sessions_dir, &session.id);
                        error
                    }
                    Ok(false) => {
                        format!("{error}; fresh launch rollback retained a replacement Session")
                    }
                    Err(cleanup_error) => format!(
                        "{error}; fresh launch rollback retained exact evidence: {cleanup_error}"
                    ),
                })
                .into());
            }
            env.insert(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                uuid::Uuid::new_v4().to_string(),
            );
            return Ok(None);
        }

        let repo_hash = session
            .repo_hash
            .clone()
            .filter(|repo_hash| !repo_hash.trim().is_empty())
            .ok_or_else(|| {
                "producing Session is missing its canonical repository hash".to_string()
            })?;
        persist_durable_launch_recovery(
            sessions_dir,
            DurableLaunchRecoveryKind::Genesis,
            &session.id,
            project_root,
            worktree,
            owner,
            None,
            None,
        )?;
        crate::cli::execution_state::materialize_at_launch_with_permission_decision(
            worktree,
            owner.kind,
            owner.number,
            &session.id,
            execution_entrypoint,
            false,
            permission_decision,
        )
        .map_err(|error| error.to_string())?;
        crate::cli::execution_state::ensure_generation_ledger(
            worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .map_err(|error| error.to_string())?;
        let identity = crate::cli::execution_state::current_execution_binding(worktree, owner)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "execution generation materialization did not publish a current binding".to_string()
            })?;
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash,
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        let session_identity = gwt_agent::SessionExecutionIdentity::for_binding(session, &binding)?;
        let unbound_session_snapshot = session.clone();
        let genesis_session_id = session.id.clone();
        let terminalize_genesis = |reason: &str| {
            crate::cli::execution_state::block_uncommitted_genesis_launch(
                worktree,
                owner,
                &genesis_session_id,
                &binding.identity,
                reason,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        };
        if let Err(error) = persist_durable_launch_recovery_with_identity(
            sessions_dir,
            DurableLaunchRecoveryKind::Genesis,
            &session.id,
            project_root,
            worktree,
            owner,
            Some(&binding),
            Some(&session.agent_id),
            Some(&session_identity),
        ) {
            let terminalized = terminalize_genesis("genesis recovery binding persistence failed");
            if terminalized.is_ok() {
                let _ = clear_durable_launch_recovery(sessions_dir, &genesis_session_id);
            }
            return Err((match terminalized {
                Ok(()) => {
                    format!("failed to persist exact genesis recovery binding: {error}")
                }
                Err(terminal_error) => format!(
                    "failed to persist exact genesis recovery binding: {error}; failed to terminalize the generation: {terminal_error}"
                ),
            }).into());
        }
        if let Err(error) = session.set_execution_binding(Some(binding.clone())) {
            let terminalized = terminalize_genesis("genesis Session binding validation failed");
            if terminalized.is_ok() {
                let _ = clear_durable_launch_recovery(sessions_dir, &genesis_session_id);
            }
            return Err((match terminalized {
                Ok(()) => format!(
                    "genesis Session binding validation failed and the generation was terminally Blocked: {error}"
                ),
                Err(terminal_error) => format!(
                    "genesis Session binding validation failed: {error}; failed to terminalize the generation: {terminal_error}"
                ),
            }).into());
        }
        let genesis = MaterializedGenesisLaunch {
            worktree_path: worktree.to_path_buf(),
            owner,
            session_id: genesis_session_id.clone(),
            binding: binding.clone(),
            session_identity: session_identity.clone(),
        };
        persist_genesis_session_binding_with(sessions_dir, session, &genesis, |session| {
            session.save_if_absent_or_unchanged(sessions_dir, &unbound_session_snapshot)
        })?;

        match issue_preflighted_agent_capability_env(
            env,
            issuer,
            project_root,
            &session.id,
            AgentCapabilityLaunchAuthority::Active(&binding),
            endpoints,
        ) {
            Ok(()) => Ok(None),
            Err(error) => {
                let cleanup = crate::cli::execution_state::block_genesis_and_remove_exact_session(
                    worktree,
                    owner,
                    &genesis_session_id,
                    &binding.identity,
                    "genesis capability issuance failed",
                    sessions_dir,
                    &session_identity,
                    || Ok(()),
                );
                if cleanup.as_ref().is_ok_and(|removed| *removed) {
                    let _ = session.set_execution_binding(None);
                }
                match cleanup {
                    Ok(true) => {
                        let _ = clear_durable_launch_recovery(sessions_dir, &genesis_session_id);
                        Err(error.into())
                    }
                    Ok(false) => Err(format!(
                        "{error}; genesis failure retained a replacement Session"
                    )
                    .into()),
                    Err(cleanup_error) => Err(format!(
                        "{error}; genesis failure retained exact recovery evidence: {cleanup_error}"
                    )
                    .into()),
                }
            }
        }
    }
}

pub fn reject_continue_work_workspace_commit(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
) -> std::io::Result<()> {
    match resolve_split_workspace_state_external_commit(
        project_root,
        work_event_root,
        operation_id,
        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
    ) {
        Ok(
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected
            | gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing,
        ) => Ok(()),
        Ok(resolution) => Err(std::io::Error::other(format!(
            "continuation Work rejection returned {resolution:?}"
        ))),
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}

pub fn resolve_split_workspace_state_external_commit(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
    decision: gwt_core::workspace_projection::ExternalWorkspaceCommitDecision,
) -> gwt_core::error::Result<gwt_core::workspace_projection::ExternalWorkspaceCommitResolution> {
    resolve_split_workspace_state_external_commit_with_retry_hint(
        project_root,
        work_event_root,
        operation_id,
        decision,
    )
    .map(|(resolution, _)| resolution)
}

pub fn resolve_split_workspace_state_external_commit_with_retry_hint(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
    decision: gwt_core::workspace_projection::ExternalWorkspaceCommitDecision,
) -> gwt_core::error::Result<(
    gwt_core::workspace_projection::ExternalWorkspaceCommitResolution,
    Option<String>,
)> {
    use gwt_core::workspace_projection::ExternalWorkspaceCommitResolution;

    let current_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(project_root);
    let canonical_work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(project_root);
    let legacy_work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let canonical = gwt_core::workspace_projection::resolve_workspace_state_external_commit_at(
        &current_path,
        &canonical_work_items_path,
        operation_id,
        decision,
    );
    let resolve_legacy = legacy_work_items_path != canonical_work_items_path
        && match &canonical {
            Ok(ExternalWorkspaceCommitResolution::Missing) => true,
            Err(gwt_core::error::GwtError::ExternalWorkspacePathPairMismatch {
                operation_id: bound_operation_id,
            }) => bound_operation_id == operation_id,
            _ => false,
        };
    let (resolution, lock_work_items_path) = if resolve_legacy {
        (
            resolve_legacy_split_workspace_state_external_commit(
                project_root,
                work_event_root,
                operation_id,
                decision,
            )?,
            &legacy_work_items_path,
        )
    } else {
        (canonical?, &canonical_work_items_path)
    };
    let retry_hint = (resolution == ExternalWorkspaceCommitResolution::Busy).then(|| {
        gwt_core::workspace_projection::external_workspace_operation_retry_hint_at(
            &current_path,
            lock_work_items_path,
            operation_id,
        )
    });
    Ok((resolution, retry_hint))
}

fn resolve_legacy_split_workspace_state_external_commit(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
    decision: gwt_core::workspace_projection::ExternalWorkspaceCommitDecision,
) -> gwt_core::error::Result<gwt_core::workspace_projection::ExternalWorkspaceCommitResolution> {
    gwt_core::workspace_projection::resolve_legacy_workspace_state_external_commit_for_work_event_root(
        project_root,
        work_event_root,
        operation_id,
        decision,
    )
}

#[derive(Debug)]
pub struct DurableFreshExecutionCandidate {
    pub project_root: PathBuf,
    pub worktree_path: PathBuf,
    pub owner: crate::cli::execution_state::ExecutionOwnerKey,
    pub attempt: crate::cli::execution_state::ContinuationAttempt,
    pub binding: gwt_agent::SessionExecutionBinding,
    pub session_identity: gwt_agent::SessionExecutionIdentity,
    pub agent_id: gwt_agent::AgentId,
}

pub fn durable_fresh_execution_candidate(
    session: &gwt_agent::Session,
) -> Result<Option<DurableFreshExecutionCandidate>, String> {
    let Some(binding) = session.execution_binding.clone() else {
        return Ok(None);
    };
    if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
        || binding.session_id != session.id
        || session.linked_issue_number != Some(binding.owner_number)
        || session.repo_hash.as_deref() != Some(binding.repo_hash.as_str())
    {
        return Err("persisted fresh-launch Session binding is not canonical".to_string());
    }
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("persisted fresh-launch owner kind is not canonical".to_string()),
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let project_root = crate::validated_project_state_root_for_session_recovery(session)
        .map_err(|error| error.to_string())?;
    let Some(attempt) = crate::cli::execution_state::fresh_linked_owner_launch_for_session(
        &session.worktree_path,
        owner,
        &session.id,
    )
    .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if binding.identity.generation_id != attempt.candidate_generation_id
        || binding.identity.binding_id != attempt.request.session_binding_id
    {
        return Err(
            "persisted fresh-launch Session does not match its durable candidate generation"
                .to_string(),
        );
    }
    Ok(Some(DurableFreshExecutionCandidate {
        project_root,
        worktree_path: session.worktree_path.clone(),
        owner,
        attempt,
        binding,
        session_identity: gwt_agent::SessionExecutionIdentity::from_session(session)
            .map_err(|error| {
                format!("persisted fresh-launch Session identity is invalid: {error}")
            })?
            .ok_or_else(|| "persisted fresh-launch Session identity is missing".to_string())?,
        agent_id: session.agent_id.clone(),
    }))
}

/// Inspect only this owner's current Prepared attempts, not Session history.
/// Receipt-backed recovery stays with the existing startup reconciler.
pub fn reconcile_unindexed_prepared_fresh_launches(
    sessions_dir: &Path,
    worktree: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
) -> Result<(), String> {
    let Some(ledger) = crate::cli::execution_state::load_generation_ledger(worktree, owner)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let mut seen = std::collections::HashSet::new();
    for attempt in ledger.continuation_attempts.iter().rev() {
        if !seen.insert(&attempt.request.operation_id)
            || attempt.status != crate::cli::execution_state::ContinuationAttemptStatus::Prepared
            || attempt.predecessor.generation_id != ledger.current_generation_id
            || !crate::cli::execution_state::is_owner_launch_successor_attempt(attempt)
            || durable_launch_recovery_exists(sessions_dir, &attempt.request.initial_session_id)
        {
            continue;
        }
        let path = sessions_dir.join(format!("{}.toml", attempt.request.initial_session_id));
        let session = match gwt_agent::inspect_session_path(&path) {
            gwt_agent::SessionPathState::Present(session) => session,
            gwt_agent::SessionPathState::Missing => {
                tracing::warn!(owner = owner.number, operation_id = %attempt.request.operation_id,
                    session_id = %attempt.request.initial_session_id,
                    "retained Prepared launch without receipt or Session: abandonment cannot be proven");
                continue;
            }
            gwt_agent::SessionPathState::Error(error) => {
                tracing::warn!(owner = owner.number, operation_id = %attempt.request.operation_id,
                    session_id = %attempt.request.initial_session_id, %error,
                    "retained Prepared launch without receipt: Session evidence is unreadable");
                continue;
            }
        };
        let Some(candidate) = durable_fresh_execution_candidate(&session)? else {
            continue;
        };
        if candidate.owner != owner
            || !path_matches(&candidate.worktree_path, worktree)
            || candidate.attempt.request != attempt.request
        {
            continue;
        }
        let Some(receipt_project_root) = candidate.session_identity.project_state_root.as_deref()
        else {
            continue;
        };
        let cleaned = crate::cli::execution_state::abort_abandoned_prepared_successor_with(
            worktree,
            owner,
            &attempt.request,
            sessions_dir,
            &candidate.session_identity,
            || {
                persist_durable_launch_recovery_with_identity(
                    sessions_dir,
                    DurableLaunchRecoveryKind::FreshSuccessor {
                        operation_id: attempt.request.operation_id.clone(),
                    },
                    &session.id,
                    receipt_project_root,
                    &candidate.worktree_path,
                    owner,
                    Some(&candidate.binding),
                    Some(&candidate.agent_id),
                    Some(&candidate.session_identity),
                )
                .map_err(std::io::Error::other)
            },
            || {
                reject_continue_work_workspace_commit(
                    &candidate.project_root,
                    worktree,
                    &attempt.request.operation_id,
                )
            },
        )
        .map_err(|error| error.to_string())?;
        if cleaned {
            clear_durable_launch_recovery(sessions_dir, &session.id)?;
        }
    }
    Ok(())
}

pub fn path_matches(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            gwt_core::paths::normalize_windows_child_process_path(&left)
                == gwt_core::paths::normalize_windows_child_process_path(&right)
        }
        _ => false,
    }
}

fn same_worktree_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
