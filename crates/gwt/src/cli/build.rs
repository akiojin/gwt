//! `build.*` JSON lifecycle operations.
//!
//! Exit CLI for the `gwt-build-spec` skill (SPEC-1935 FR-014r). Writes
//! `.gwt/skill-state/build-spec.json` via [`gwt_core::skill_state`].

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

pub const SKILL_NAME: &str = "build-spec";
pub const SKILL_DISPLAY: &str = "gwt-build-spec";
pub const VERB: &str = "build";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if matches!(
        &action,
        SkillStateAction::Start { .. } | SkillStateAction::Phase { .. }
    ) {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        // Canonical successor activation takes this same lease. Keep both
        // ownership admission and persistence inside it so a stale caller
        // cannot overwrite a successor that became live in between.
        return super::trusted_store::with_write_lease(&worktree, || {
            Ok(run_action(env, action, out))
        })
        .map_err(|error| {
            SpecOpsError::from(gwt_github::client::ApiError::Unexpected(error.to_string()))
        })?;
    }
    // Finalization already owns its write lease; it must not nest this one.
    run_action(env, action, out)
}

fn run_action<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if let SkillStateAction::Phase { spec, .. } = &action {
        if let Some(code) = admit_build_phase(env, *spec, out) {
            return Ok(code);
        }
    }
    if let SkillStateAction::Start { spec } = &action {
        let mut start_evidence = None;
        if !managed_build_start_preflight(env, *spec, out, &mut start_evidence) {
            return Ok(2);
        }
        return skill_state_runtime::run_with_start_evidence(
            env,
            action,
            SKILL_NAME,
            SKILL_DISPLAY,
            VERB,
            out,
            start_evidence,
        );
    }
    let mut completion_verification_hash = None;
    let mut completion_session_id = None;
    if let SkillStateAction::Complete { spec, .. } | SkillStateAction::Abort { spec, .. } = &action
    {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
        let build_state = gwt_core::skill_state::load(&worktree, SKILL_NAME).map_err(|error| {
            gwt_github::SpecOpsError::from(gwt_github::client::ApiError::Unexpected(
                error.to_string(),
            ))
        })?;
        // Confirm an ended lifecycle without revalidating or rewriting its delivery.
        if build_state.as_ref().is_some_and(|state| {
            !state.active
                && state.owner_spec == Some(*spec)
                && !session_id.trim().is_empty()
                && state.session_id.trim() == session_id.trim()
        }) {
            out.push_str(&format!(
                "{VERB}: {SKILL_DISPLAY} lifecycle already finalized for SPEC-{spec}\n"
            ));
            return Ok(0);
        }
        let SkillStateAction::Complete { pr, .. } = &action else {
            return run_after_quarantine_precheck(env, action, out, None, None);
        };
        let owning_active_build = build_state.is_some_and(|state| {
            state.active
                && state.session_id.trim() == session_id.trim()
                && state.owner_spec.is_none_or(|owner| owner == *spec)
        });
        match if owning_active_build {
            super::completion_pr::admit(env, session_id.trim(), Some(*spec), *pr, out)
        } else {
            Ok(None)
        } {
            Ok(Some(hash)) => {
                completion_verification_hash = Some(hash);
                completion_session_id = Some(session_id.trim().to_string());
            }
            Ok(None) => {}
            Err(reason) => {
                out.push_str(&format!("{VERB}: completion refused — {reason}\n"));
                return Ok(2);
            }
        }
        // SPEC #3590 FR-026: an unsettled Work event log is a warning only.
        if let Some(warning) =
            crate::cli::verification_record::work_event_settlement_refusal(&worktree)
        {
            out.push_str(&format!("{VERB}: warning — {warning}\n"));
        }
        let verification = crate::cli::verification_record::load(&worktree).map_err(|error| {
            gwt_github::SpecOpsError::from(gwt_github::client::ApiError::Unexpected(format!(
                "failed to load verification evidence: {error}"
            )))
        })?;
        if verification
            .as_ref()
            .is_some_and(|record| !record.quarantined_failures.is_empty())
        {
            let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                out.push_str(&format!(
                    "{VERB}: completion refused — typed quarantine requires the current owning Session\n"
                ));
                return Ok(2);
            };
            let expected_owner = match &action {
                SkillStateAction::Complete { spec, .. } => Some(*spec),
                _ => None,
            };
            let evidence = crate::cli::verification_record::evaluate_evidence(
                &worktree,
                &session_id,
                expected_owner,
            );
            if evidence != crate::cli::verification_record::EvidenceStatus::FreshWithQuarantine {
                out.push_str(&format!(
                    "{VERB}: completion refused — {}\n",
                    evidence.describe()
                ));
                return Ok(2);
            }
            if let Err(refusal) =
                crate::cli::verification_record::validate_current_quarantine_references(
                    env,
                    verification.as_ref().expect("typed record checked above"),
                    None,
                )
            {
                out.push_str(&format!(
                    "{VERB}: completion refused — typed quarantine evidence is not current: {refusal}\n"
                ));
                return Ok(2);
            }
            if completion_verification_hash.as_ref().is_some_and(|hash| {
                verification
                    .as_ref()
                    .is_none_or(|record| &record.content_hash != hash)
            }) {
                out.push_str("build: completion refused — verification evidence changed after PR validation\n");
                return Ok(2);
            }
            completion_verification_hash = verification.map(|record| record.content_hash.clone());
            completion_session_id = Some(session_id);
        }
    }
    if let (Some(expected_hash), Some(session_id)) = (
        completion_verification_hash.as_deref(),
        completion_session_id.as_deref(),
    ) {
        let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
        if let Some(refusal) =
            crate::cli::action_obligation::open_obligation_refusal(&worktree, session_id, &[])
        {
            out.push_str(&format!("{VERB}: completion refused — {refusal}\n"));
            return Ok(2);
        }
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(&worktree)
            .ok_or_else(|| {
                gwt_github::SpecOpsError::from(gwt_github::client::ApiError::Unexpected(
                    "verified build completion requires canonical trusted storage".to_string(),
                ))
            })?;
        let result = crate::cli::trusted_store::with_write_lease_for_resolved_dir(
            &trusted_dir,
            || {
            let current = crate::cli::verification_record::load(&worktree)?;
            if current
                .as_ref()
                .is_none_or(|record| record.content_hash != expected_hash)
            {
                out.push_str(
                    "build: completion refused — verification evidence changed after live PR validation\n",
                );
                return Ok(Ok(2));
            }
            let expected_owner = match &action {
                SkillStateAction::Complete { spec, .. } => Some(*spec),
                _ => None,
            };
            let evidence = crate::cli::verification_record::evaluate_evidence(
                &worktree,
                session_id,
                expected_owner,
            );
            if !matches!(evidence, crate::cli::verification_record::EvidenceStatus::Fresh | crate::cli::verification_record::EvidenceStatus::FreshWithQuarantine) {
                out.push_str(&format!(
                    "build: completion refused — {}\n",
                    evidence.describe()
                ));
                return Ok(Ok(2));
            }
            Ok(run_after_quarantine_precheck(
                env,
                action,
                out,
                Some(expected_hash),
                Some(&trusted_dir),
            ))
            },
        )
        .map_err(|error| {
            gwt_github::SpecOpsError::from(gwt_github::client::ApiError::Unexpected(
                crate::cli::trusted_store::store_health_error(
                    "holding typed quarantine build completion lease",
                    &error,
                ),
            ))
        })?;
        return result;
    }
    run_after_quarantine_precheck(env, action, out, None, None)
}

/// `build.phase` uses the same ownership judgment as `build.start`: a live
/// foreign holder is refused, and a gone holder's lifecycle is inherited.
fn admit_build_phase<E: CliEnv>(env: &E, spec: u64, out: &mut String) -> Option<i32> {
    let current_session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let current_session_id = current_session_id.trim();
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let state = gwt_core::skill_state::load(&worktree, SKILL_NAME).ok()??;
    let Some(ownership) = admit_build_lifecycle(&worktree, &state, spec, current_session_id, out)
    else {
        return Some(2);
    };
    match ownership {
        BuildLifecycleOwnership::Inherited => {
            let inherited = gwt_core::skill_state::SkillState {
                session_id: current_session_id.to_string(),
                ..state
            };
            if let Err(error) = gwt_core::skill_state::save(&worktree, SKILL_NAME, &inherited) {
                out.push_str(&format!("{VERB}: phase failed: {error}\n"));
                return Some(1);
            }
            None
        }
        BuildLifecycleOwnership::Current | BuildLifecycleOwnership::HeldByLiveSession(_) => None,
    }
}

/// Validate the same current Execution identity before start or inheritance.
/// Returns whether this is a managed build; a refusal writes its typed error.
fn build_execution_owner_preflight(
    worktree: &std::path::Path,
    spec: u64,
    current_session_id: &str,
    out: &mut String,
) -> Option<bool> {
    let execution = match crate::cli::execution_state::load(worktree) {
        Ok(Some(record)) if crate::cli::execution_state::integrity_ok(&record) => Some(record),
        Ok(None) => None,
        Ok(Some(_)) | Err(_) => {
            push_start_preflight_refusal(
                out,
                "execution_state_invalid",
                "current Execution authority is unreadable or failed integrity validation",
                "execution.status",
            );
            return None;
        }
    };
    let managed = managed_build_environment_present() || execution.is_some();
    if !managed {
        return Some(false);
    }
    if current_session_id.is_empty() {
        push_start_preflight_refusal(
            out,
            "relaunch_required",
            "managed build.start is missing its Session identity; inspect execution.status, then relaunch through Start Work / Launch",
            "execution.status",
        );
        return None;
    }
    if let Some(record) = execution {
        if record.primary_session_id != current_session_id {
            push_start_preflight_refusal(
                out,
                "execution_session_mismatch",
                "current Session does not own the worktree Execution",
                "execution.status",
            );
            return None;
        }
        if record.owner_number != spec {
            push_start_preflight_refusal(
                out,
                "execution_owner_mismatch",
                "requested build owner does not match the current Execution owner",
                "build.start",
            );
            return None;
        }
    }
    Some(managed)
}

fn managed_build_start_preflight<E: CliEnv>(
    env: &E,
    spec: u64,
    out: &mut String,
    start_evidence: &mut Option<gwt_core::skill_state::SkillStartEvidence>,
) -> bool {
    let current_session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let mut inherited = false;
    if let Ok(Some(state)) = gwt_core::skill_state::load(&worktree, SKILL_NAME) {
        match admit_build_lifecycle(&worktree, &state, spec, &current_session_id, out) {
            None => return false,
            Some(ownership) => {
                inherited = matches!(ownership, BuildLifecycleOwnership::Inherited);
            }
        }
    }
    match gwt_core::skill_state::load(&worktree, SKILL_NAME) {
        Ok(Some(state))
            if state.active && state.owner_spec.is_none_or(|owner_spec| owner_spec != spec) =>
        {
            push_start_preflight_refusal(
                out,
                "build_state_owner_mismatch",
                "an active build lifecycle belongs to another or unknown owner",
                "build.abort",
            );
            return false;
        }
        Ok(Some(state)) if state.active && !inherited => {
            // An idempotent restart of the same lifecycle must not erase a
            // previously recorded compatibility path after the Host upgrades.
            *start_evidence = state.start_evidence;
        }
        Ok(Some(_)) | Ok(None) => {}
        Err(_) => {
            push_start_preflight_refusal(
                out,
                "build_state_invalid",
                "the existing build lifecycle state is unreadable",
                "execution.status",
            );
            return false;
        }
    }
    let Some(managed) = build_execution_owner_preflight(&worktree, spec, &current_session_id, out)
    else {
        return false;
    };
    if !managed {
        return true;
    }
    let target = match crate::daemon_runtime::HookForwardTarget::from_env_strict() {
        Ok(Some(target)) => target,
        Ok(None) => {
            push_start_preflight_refusal(
                out,
                "relaunch_required",
                "managed build.start is missing its Host bridge capability; inspect execution.status, then relaunch through Start Work / Launch",
                "execution.status",
            );
            return false;
        }
        Err(_) => {
            push_start_preflight_refusal(
                out,
                "relaunch_required",
                "managed build.start has an incomplete or invalid Host bridge capability; inspect execution.status, then relaunch through Start Work / Launch",
                "execution.status",
            );
            return false;
        }
    };
    let observation = match crate::observe_agent_runtime(env.repo_path()) {
        Ok(observation) => observation,
        Err(error) => {
            push_start_preflight_refusal(
                out,
                workspace_error_code(error.code),
                preflight_rejection_reason(error.code),
                recovery_operation(error.code),
            );
            return false;
        }
    };
    let request = crate::AgentWorkMaterializationProbeRequest {
        schema_version: crate::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION,
        claimed_session_id: current_session_id,
        owner_number: spec,
        observation,
    };
    match crate::daemon_runtime::send_work_materialization_probe_via_agent_bridge(&target, &request)
    {
        Ok(_) => true,
        Err(crate::daemon_runtime::AgentBridgeRequestError::OperationUnavailable) => {
            // Only a Host runtime can read the same durable authority as its
            // older Host. Docker must not infer authority from container files.
            let recovery = crate::agent_project_state::validated_workspace_recovery_session(
                env.repo_path(),
                &request.claimed_session_id,
            );
            let receipt = match recovery {
                Ok(Some(crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(
                    recovery,
                ))) if recovery.session.docker_runtime_binding.is_none() => recovery
                    .session
                    .execution_binding
                    .as_ref()
                    .and_then(|binding| {
                        crate::probe_bound_authenticated_work_materialization(
                            &recovery.project_state_root,
                            &request.claimed_session_id,
                            binding,
                            request.clone(),
                        )
                        .ok()
                    }),
                _ => None,
            };
            let Some(receipt) = receipt else {
                push_start_preflight_refusal(
                    out,
                    "host_probe_unavailable_unverified_work",
                    "Host probe returned HTTP 404 and exact local Host Work authority could not be verified; inspect execution.status for workspace.ensure or Host update/relaunch recovery",
                    "execution.status",
                );
                return false;
            };
            *start_evidence = Some(gwt_core::skill_state::SkillStartEvidence {
                reason: "host_probe_unavailable_404".to_string(),
                work_id: receipt.work_id,
            });
            out.push_str("build: host_probe_unavailable_404; Host probe was not used; exact local Host Work verified\n");
            true
        }
        Err(crate::daemon_runtime::AgentBridgeRequestError::Rejected(error)) => {
            push_start_preflight_refusal(
                out,
                workspace_error_code(error.code),
                preflight_rejection_reason(error.code),
                recovery_operation(error.code),
            );
            false
        }
        Err(error) => {
            push_start_preflight_refusal(
                out,
                "host_outcome_unknown",
                &format!("{error}; inspect execution.status; if the Host lacks this operation, update the Host and relaunch through Start Work / Launch"),
                "execution.status",
            );
            false
        }
    }
}

fn push_start_preflight_refusal(
    out: &mut String,
    error_code: &str,
    reason: &str,
    recovery_operation: &str,
) {
    out.push_str(&format!(
        "{}\n",
        serde_json::json!({
            "ok": false,
            "error_code": error_code,
            "reason": reason,
            "recovery_operation": recovery_operation,
            "retryable": true,
        })
    ));
}

fn workspace_error_code(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest => "invalid_request",
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired => "relaunch_required",
        crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            "execution_binding_mismatch"
        }
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => {
            "workspace_ensure_required"
        }
        crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch => "provenance_mismatch",
        crate::AgentWorkspaceUpdateErrorCode::IdentityConflict => "identity_conflict",
        crate::AgentWorkspaceUpdateErrorCode::TransactionConflict => "transaction_conflict",
        crate::AgentWorkspaceUpdateErrorCode::Internal => "internal",
    }
}

fn recovery_operation(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => "workspace.ensure",
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired
        | crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => "execution.status",
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest
        | crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch
        | crate::AgentWorkspaceUpdateErrorCode::IdentityConflict
        | crate::AgentWorkspaceUpdateErrorCode::TransactionConflict
        | crate::AgentWorkspaceUpdateErrorCode::Internal => "execution.status",
    }
}

fn preflight_rejection_reason(code: crate::AgentWorkspaceUpdateErrorCode) -> &'static str {
    match code {
        crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired => {
            "Session-bound Work is not materialized"
        }
        crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired
        | crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch => {
            "managed Session authority is missing, stale, or no longer current; inspect execution.status, then relaunch the owning Session through Start Work / Launch"
        }
        crate::AgentWorkspaceUpdateErrorCode::InvalidRequest => {
            "managed build.start preflight request or runtime observation is invalid"
        }
        crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch => {
            "runtime provenance does not match the authenticated Session"
        }
        crate::AgentWorkspaceUpdateErrorCode::IdentityConflict => {
            "Session-bound Work identity conflicts with the current authority"
        }
        crate::AgentWorkspaceUpdateErrorCode::TransactionConflict => {
            "Host workspace authority changed during preflight"
        }
        crate::AgentWorkspaceUpdateErrorCode::Internal => {
            "Host could not determine the Session-bound Work"
        }
    }
}

fn managed_build_environment_present() -> bool {
    std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some()
        || std::env::var_os(gwt_agent::GWT_HOOK_FORWARD_URL_ENV).is_some()
        || std::env::var_os(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV).is_some()
}

fn managed_build_context_present<E: CliEnv>(env: &E) -> bool {
    managed_build_environment_present()
        || !matches!(crate::cli::execution_state::load(env.repo_path()), Ok(None))
}

fn run_after_quarantine_precheck<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
    completion_verification_hash: Option<&str>,
    held_trusted_dir: Option<&std::path::Path>,
) -> Result<i32, SpecOpsError> {
    match abort_abandoned_build(env, &action, out) {
        Ok(Some(code)) => return Ok(code),
        Ok(None) => {}
        Err(error) => {
            out.push_str(&format!(
                "{VERB}: abandoned lifecycle recovery failed: {error}\n"
            ));
            return Ok(1);
        }
    }
    let recovered_orphan = missing_build_work_recovery_identity(env, &action);
    if let Err(error) = if recovered_orphan.is_some() {
        Ok(())
    } else {
        record_current_work_terminal_before_finalize_with_lease(env, &action, held_trusted_dir)
    } {
        out.push_str(&format!("{VERB}: Work lifecycle update failed: {error}\n"));
        return Ok(1);
    }
    // SPEC-3248 P8a: a successful build completion also settles the launch's
    // Execution Control Record (best-effort — the build-spec skill flow must
    // not require a second explicit `execution.complete`). Guarded strictly:
    // the settlement fires only when this `build.complete` actually finalized
    // an ACTIVE build state for the same spec — a vacuous "nothing to
    // finalize" exit 0 must not settle the execution — and only when the
    // record names the same owner. Aborting a build never settles.
    let completed_spec = match &action {
        SkillStateAction::Complete { spec, .. } => {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let had_active_matching_state = gwt_core::skill_state::load(&worktree, SKILL_NAME)
                .ok()
                .flatten()
                .is_some_and(|state| {
                    state.active && (state.owner_spec.is_none() || state.owner_spec == Some(*spec))
                });
            had_active_matching_state.then_some(*spec)
        }
        _ => None,
    };
    let code = skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)?;
    if code == 0 {
        if let Some((spec, session_id)) = recovered_orphan {
            let recovered = gwt_core::skill_state::load(env.repo_path(), SKILL_NAME)
                .ok()
                .flatten()
                .is_some_and(|state| {
                    !state.active
                        && state.owner_spec == Some(spec)
                        && state.session_id.trim() == session_id
                });
            if !recovered {
                out.push_str(
                    "build: orphan recovery could not be confirmed by lifecycle state readback\n",
                );
                return Ok(1);
            }
            out.push_str(&format!("build: {}\n", serde_json::json!({
                "ok": true,
                "status": "orphan_recovered",
                "reason": "the exact current Active Session has no materialized Work; no terminal request was sent",
            })));
        }
        if let Some(spec) = completed_spec {
            if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
                // SPEC-3248 P8b (T-111): the execution settlement piggybacked
                // on build.complete also requires fresh verification
                // evidence; a build completion without it finalizes the
                // skill state but leaves the execution active so the Stop
                // gate keeps the session working toward real evidence.
                let has_matching_active_record = crate::cli::execution_state::load(&worktree)
                    .ok()
                    .flatten()
                    .is_some_and(|record| {
                        record.status == crate::cli::execution_state::ExecutionControlStatus::Active
                            && record.primary_session_id == session_id
                            && record.owner_number == spec
                    });
                if has_matching_active_record {
                    let status = crate::cli::verification_record::evaluate_evidence(
                        &worktree,
                        &session_id,
                        Some(spec),
                    );
                    if status.is_delivery_acceptable() {
                        let settled = if let Some(expected_hash) = completion_verification_hash {
                            debug_assert!(held_trusted_dir.is_some());
                            match crate::cli::execution_state::settle_completed_with_evidence_locked(
                                &worktree,
                                &session_id,
                                Some(spec),
                                Some(expected_hash),
                            ) {
                                Ok(Ok(
                                    crate::cli::execution_state::SettleResult::Settled(_)
                                    | crate::cli::execution_state::SettleResult::AlreadySettled(_),
                                )) => true,
                                Ok(Ok(_)) | Ok(Err(_)) | Err(_) => false,
                            }
                        } else {
                            crate::cli::execution_state::settle_completed_best_effort(
                                &worktree,
                                &session_id,
                                spec,
                            );
                            true
                        };
                        if completion_verification_hash.is_some() && !settled {
                            out.push_str(
                                "build: completion refused — typed quarantine verification evidence changed after live PR validation\n",
                            );
                            return Ok(2);
                        }
                    } else {
                        out.push_str(&format!(
                            "{VERB}: execution control not settled — {}\n",
                            status.describe()
                        ));
                    }
                }
            }
        }
    }
    Ok(code)
}

/// An old build is bookkeeping, not the new Session's Work. Abort only that
/// lifecycle, under the old Session lease, once its runtime is known to be gone.
fn abort_abandoned_build<E: CliEnv>(
    env: &mut E,
    action: &SkillStateAction,
    out: &mut String,
) -> Result<Option<i32>, String> {
    let SkillStateAction::Abort { spec, .. } = action else {
        return Ok(None);
    };
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let Some(state) = gwt_core::skill_state::load(env.repo_path(), SKILL_NAME)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if !state.active
        || state.owner_spec != Some(*spec)
        || state.session_id.trim().is_empty()
        || session_id.trim().is_empty()
        || state.session_id.trim() == session_id.trim()
    {
        return Ok(None);
    }
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    gwt_agent::with_session_path_lease(&sessions_dir, &state.session_id, |holder| {
        if !classify_lifecycle_holder(&sessions_dir, holder)?.is_gone() {
            return Ok(None);
        }
        if gwt_core::skill_state::load(env.repo_path(), SKILL_NAME)?.as_ref() != Some(&state) {
            return Err(std::io::Error::other(
                "build lifecycle changed during recovery; retry build.abort",
            ));
        }
        // Preserve the old Session identity and the reason via the ordinary
        // finalizer. No Work terminalization or Execution settlement is sent.
        skill_state_runtime::run(env, action.clone(), SKILL_NAME, SKILL_DISPLAY, VERB, out)
            .map(Some)
            .map_err(std::io::Error::other)
    })
    .map_err(|error| error.to_string())
}

/// Runtime state of the Session recorded as the build lifecycle holder.
enum LifecycleHolderLiveness {
    Gone,
    Alive { disposition: &'static str },
}

impl LifecycleHolderLiveness {
    fn is_gone(&self) -> bool {
        matches!(self, Self::Gone)
    }
}

/// Classify the recorded holder under its Session path lease. Only decisive
/// evidence that no runtime remains counts as gone; ambiguity stays alive.
fn classify_lifecycle_holder(
    sessions_dir: &std::path::Path,
    holder: gwt_agent::SessionPathState,
) -> std::io::Result<LifecycleHolderLiveness> {
    use crate::cli::execution_state::{
        classify_exact_session_runtime, ExactSessionRuntimeDisposition,
    };
    let holder = match holder {
        gwt_agent::SessionPathState::Missing => return Ok(LifecycleHolderLiveness::Gone),
        gwt_agent::SessionPathState::Error(error) => return Err(error),
        gwt_agent::SessionPathState::Present(holder) => holder,
    };
    let disposition = match gwt_agent::SessionExecutionIdentity::from_session(&holder)
        .map_err(std::io::Error::other)?
    {
        Some(identity) => match classify_exact_session_runtime(sessions_dir, &identity)? {
            ExactSessionRuntimeDisposition::Absent
            | ExactSessionRuntimeDisposition::Terminal(_)
            | ExactSessionRuntimeDisposition::Defunct(_)
            | ExactSessionRuntimeDisposition::HostDead
            | ExactSessionRuntimeDisposition::ChildExited => {
                return Ok(LifecycleHolderLiveness::Gone)
            }
            ExactSessionRuntimeDisposition::Live => "live",
            ExactSessionRuntimeDisposition::Unknown => "unknown",
        },
        None => {
            let observed = crate::session_inventory::observe_session(&holder, sessions_dir);
            if observed.sessions.is_empty() && observed.uncertainties.is_empty() {
                return Ok(LifecycleHolderLiveness::Gone);
            }
            "observed"
        }
    };
    Ok(LifecycleHolderLiveness::Alive { disposition })
}

/// Who may drive an active build lifecycle (Issue #5054).
enum BuildLifecycleOwnership {
    /// No active foreign lifecycle: the current Session owns it or may start.
    Current,
    /// The recorded holder is terminal or gone; the current Session inherits.
    Inherited,
    /// The recorded holder still runs; carries the typed refusal evidence.
    HeldByLiveSession(serde_json::Value),
}

/// The single ownership judgment shared by `build.start` and `build.phase`.
fn judge_build_lifecycle_ownership(
    worktree: &std::path::Path,
    state: &gwt_core::skill_state::SkillState,
    current_session_id: &str,
) -> std::io::Result<BuildLifecycleOwnership> {
    let holder_id = state.session_id.trim();
    if !state.active || holder_id == current_session_id.trim() {
        return Ok(BuildLifecycleOwnership::Current);
    }
    if holder_id.is_empty() {
        return Ok(BuildLifecycleOwnership::Inherited);
    }
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let liveness = gwt_agent::with_session_path_lease(&sessions_dir, holder_id, |holder| {
        if let gwt_agent::SessionPathState::Present(session) = &holder {
            if let Some(identity) = gwt_agent::SessionExecutionIdentity::from_session(session)
                .map_err(std::io::Error::other)?
            {
                if Some(identity.execution_binding.owner_number) == state.owner_spec
                    && crate::cli::execution_state::session_generation_is_terminal(
                        worktree, &identity,
                    )?
                {
                    return Ok(LifecycleHolderLiveness::Gone);
                }
            }
        }
        classify_lifecycle_holder(&sessions_dir, holder)
    })?;
    Ok(match liveness {
        LifecycleHolderLiveness::Gone => BuildLifecycleOwnership::Inherited,
        LifecycleHolderLiveness::Alive { disposition } => {
            BuildLifecycleOwnership::HeldByLiveSession(serde_json::json!({
                "session_id": holder_id,
                "runtime": disposition,
                "runtime_sidecars": lifecycle_holder_runtime_evidence(&sessions_dir, holder_id),
            }))
        }
    })
}

/// pid / heartbeat evidence from every runtime sidecar of the holder.
fn lifecycle_holder_runtime_evidence(
    sessions_dir: &std::path::Path,
    session_id: &str,
) -> Vec<serde_json::Value> {
    let Ok(namespaces) = std::fs::read_dir(sessions_dir.join("runtime")) else {
        return Vec::new();
    };
    let mut evidence: Vec<serde_json::Value> = namespaces
        .flatten()
        .filter_map(|namespace| {
            let host_pid = namespace.file_name().to_str()?.parse::<u32>().ok()?;
            let runtime = gwt_agent::SessionRuntimeState::load(
                &namespace.path().join(format!("{session_id}.json")),
            )
            .ok()?;
            Some(serde_json::json!({
                "host_pid": host_pid,
                "child_pid": runtime.child_pid,
                "status": runtime.status,
                "heartbeat_at": runtime.last_activity_at.max(runtime.updated_at),
            }))
        })
        .collect();
    evidence.sort_by_key(|entry| entry["host_pid"].as_u64());
    evidence
}

/// Apply the shared ownership judgment; on refusal writes the typed error.
fn admit_build_lifecycle(
    worktree: &std::path::Path,
    state: &gwt_core::skill_state::SkillState,
    spec: u64,
    current_session_id: &str,
    out: &mut String,
) -> Option<BuildLifecycleOwnership> {
    if state.active && state.owner_spec != Some(spec) {
        push_start_preflight_refusal(
            out,
            "build_state_owner_mismatch",
            "an active build lifecycle belongs to another or unknown owner",
            "build.abort",
        );
        return None;
    }
    match judge_build_lifecycle_ownership(worktree, state, current_session_id) {
        Ok(BuildLifecycleOwnership::HeldByLiveSession(holder)) => {
            out.push_str(&format!(
                "{}\n",
                serde_json::json!({
                    "ok": false,
                    "error_code": "build_state_session_mismatch",
                    "reason": "an active build lifecycle belongs to another Session that is still running",
                    "holder": holder,
                    "recovery_operation": "execution.status",
                    "retryable": true,
                })
            ));
            None
        }
        Ok(ownership) => {
            if state.active
                && build_execution_owner_preflight(worktree, spec, current_session_id, out)
                    .is_none()
            {
                return None;
            }
            Some(ownership)
        }
        Err(error) => {
            push_start_preflight_refusal(
                out,
                "build_state_holder_unverified",
                &format!("the build lifecycle holder could not be classified: {error}"),
                "execution.status",
            );
            None
        }
    }
}

/// Recovery only abandons an orphaned lifecycle. Existing Work, denied Host
/// requests, Blocked executions, and ambiguous authority keep the normal
/// canonical terminalization path and its fail-closed behavior.
fn missing_build_work_recovery_identity<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
) -> Option<(u64, String)> {
    let SkillStateAction::Abort { spec, .. } = action else {
        return None;
    };
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok()?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let state = gwt_core::skill_state::load(env.repo_path(), SKILL_NAME).ok()??;
    if !state.active || state.owner_spec != Some(*spec) || state.session_id.trim() != session_id {
        return None;
    }
    matches!(
        crate::agent_project_state::bound_active_build_work_is_missing(
            env.repo_path(),
            session_id,
            *spec,
        ),
        Ok(true)
    )
    .then(|| (*spec, session_id.to_string()))
}

#[cfg(test)]
fn record_current_work_terminal_before_finalize<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
) -> Result<(), String> {
    record_current_work_terminal_before_finalize_with_lease(env, action, None)
}

fn record_current_work_terminal_before_finalize_with_lease<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
    held_trusted_dir: Option<&std::path::Path>,
) -> Result<(), String> {
    let (spec, close_kind, abort_reason) = match action {
        SkillStateAction::Complete { spec, .. } => (*spec, WorkTerminalKind::Done, None),
        SkillStateAction::Abort { spec, reason } => {
            (*spec, WorkTerminalKind::Discarded, reason.as_deref())
        }
        SkillStateAction::Start { .. } | SkillStateAction::Phase { .. } => return Ok(()),
    };
    let repo = env.repo_path();
    let state = gwt_core::skill_state::load(repo, SKILL_NAME).map_err(|error| error.to_string())?;
    let Some(state) = state else {
        return Ok(());
    };
    if state.owner_spec.is_some() && state.owner_spec != Some(spec) {
        return Ok(());
    }

    if !state.active {
        return Ok(());
    }
    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    if session_id.is_empty() {
        if state.session_id.trim().is_empty() && !managed_build_context_present(env) {
            return Ok(());
        }
        return Err("active build state has no current Session authority; relaunch the owning Session before finalizing".to_string());
    }
    if state.session_id.trim() != session_id {
        return Err("active build state belongs to another Session; only its owning Session may finalize it".to_string());
    }
    if let Some(target) = crate::daemon_runtime::HookForwardTarget::from_env_strict()? {
        // SPEC-3431 (#3425 family): a session that was never bound by a launch
        // has no bound Work for the managed bridge to terminalize, so
        // demanding exact durable Host Work authority is unsatisfiable and
        // left the skill state permanently open (complete needs a receipt,
        // abort needed this authority — both require the missing binding).
        // Skipping only the managed-bridge terminalization is truthful: there
        // is no bound Work to mark. The legacy no-bridge path below still runs
        // for legacy Work state, and the Done path keeps its own receipt gate
        // in `build.complete`, which an unbound session still cannot pass.
        let session_toml = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
        if let Ok(session) = gwt_agent::Session::load(&session_toml) {
            if session.execution_binding.is_none() {
                if matches!(close_kind, WorkTerminalKind::Discarded)
                    && state.owner_spec == Some(spec)
                {
                    return Ok(());
                }
                return Err(
                    "unbound build terminalization requires explicit Abort for the exact lifecycle owner".to_string(),
                );
            }
        }
        let terminal_refusal = |refusal: String| {
            if matches!(close_kind, WorkTerminalKind::Discarded) {
                crate::cli::execution_state::terminal_recovery_refusal(repo, &session_id, &refusal)
            } else {
                let reason = refusal
                    .split_once(
                        "; run workspace.ensure for this Session before retrying workspace.update",
                    )
                    .map_or(refusal.as_str(), |(reason, _)| reason);
                format!("{reason}; build.complete cannot proceed with the current Work authority, and workspace.ensure cannot restore terminal Work; inspect JSON operation `execution.status` and its `available_recoveries` / `recovery_probes`; for nonterminal Work only, follow an available workspace.ensure probe; if no recovery is available, request PM intervention")
            }
        };
        let compatibility_authority =
            crate::agent_project_state::snapshot_bound_terminal_compatibility_authority(
                repo,
                &session_id,
                match close_kind {
                    WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                    WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
                },
            )
            .map_err(|error| terminal_refusal(error.to_string()))?
            .ok_or_else(|| {
                terminal_refusal(
                    "managed build terminalization requires an exact durable Host Work authority"
                        .to_string(),
                )
            })?;
        if compatibility_authority.owner_number() != spec {
            return Err(
                "active build lifecycle owner does not match the current Session-bound Work owner"
                    .to_string(),
            );
        }
        let observation = crate::observe_agent_runtime(repo).map_err(|error| error.to_string())?;
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: session_id.clone(),
            observation,
            terminal_kind: match close_kind {
                WorkTerminalKind::Done => crate::AgentWorkTerminalKind::Done,
                WorkTerminalKind::Discarded => crate::AgentWorkTerminalKind::Discarded,
            },
        };
        if matches!(close_kind, WorkTerminalKind::Done)
            && compatibility_authority.is_confirmation_only()
        {
            // Issue #5070 / SPEC #3590: an exact canonical Done Work needs
            // only read-only confirmation under the current identity lease.
            // Sending another terminal mutation can strand delivered work.
            let confirmation = if let Some(held_trusted_dir) = held_trusted_dir {
                crate::agent_project_state::confirm_bound_terminal_compatibility_authority_held_global_lease(
                    &compatibility_authority,
                    request,
                    held_trusted_dir,
                )
            } else {
                crate::agent_project_state::confirm_bound_terminal_compatibility_authority(
                    &compatibility_authority,
                    request,
                )
            };
            return confirmation.map_err(|error| terminal_refusal(error.to_string()));
        }
        let blocked_build_abort = compatibility_authority
            .requires_blocked_build_abort_bridge()
            .map_err(|error| terminal_refusal(error.to_string()))?;
        let receipt = if blocked_build_abort {
            let reason = abort_reason
                .filter(|reason| !reason.trim().is_empty())
                .ok_or_else(|| {
                    "Blocked build abort requires a non-empty reason before Host terminalization"
                        .to_string()
                })?;
            match crate::daemon_runtime::send_blocked_build_abort_terminalization_via_agent_bridge(
                &target,
                &crate::AgentBuildAbortTerminalizationRequest {
                    schema_version: crate::AGENT_BUILD_ABORT_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: request.claimed_session_id.clone(),
                    owner_number: spec,
                    reason: reason.to_string(),
                    observation: request.observation.clone(),
                },
            ) {
                Ok(receipt) => receipt,
                Err(bridge_error) if bridge_error.is_missing_route_rejection() => {
                    let reconciliation = if let Some(held_trusted_dir) = held_trusted_dir {
                        crate::agent_project_state::continue_bound_terminal_compatibility_held_global_lease(
                            &compatibility_authority,
                            request,
                            held_trusted_dir,
                        )
                    } else {
                        crate::agent_project_state::continue_bound_terminal_compatibility(
                            &compatibility_authority,
                            request,
                        )
                    };
                    return reconciliation
                    .map(|_| ())
                    .map_err(|local_error| {
                        format!(
                            "{bridge_error}; local Blocked build abort reconciliation was also refused: {local_error}"
                        )
                    });
                }
                Err(bridge_error) => return Err(bridge_error.to_string()),
            }
        } else {
            crate::daemon_runtime::send_work_terminalization_via_agent_bridge(&target, &request)?
        };
        return match receipt.outcome {
            crate::AgentWorkTerminalizationOutcome::Emitted => {
                let confirmation = if let Some(held_trusted_dir) = held_trusted_dir {
                    crate::agent_project_state::confirm_bound_terminal_compatibility_authority_held_global_lease(
                        &compatibility_authority,
                        request,
                        held_trusted_dir,
                    )
                } else {
                    crate::agent_project_state::confirm_bound_terminal_compatibility_authority(
                        &compatibility_authority,
                        request,
                    )
                };
                confirmation
                .map_err(|error| {
                    format!(
                        "Host emitted a terminal event outside the canonical Work authority: {error}"
                    )
                })
            }
            crate::AgentWorkTerminalizationOutcome::AlreadyMatching
            | crate::AgentWorkTerminalizationOutcome::WrongTerminal
            | crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing
            | crate::AgentWorkTerminalizationOutcome::NoTarget => if let Some(held_trusted_dir) =
                held_trusted_dir
            {
                crate::agent_project_state::continue_bound_terminal_compatibility_held_global_lease(
                    &compatibility_authority,
                    request,
                    held_trusted_dir,
                )
            } else {
                crate::agent_project_state::continue_bound_terminal_compatibility(
                    &compatibility_authority,
                    request,
                )
            }
            .map(|_| ()),
            crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal => {
                map_agent_terminal_outcome(receipt.outcome, close_kind)
            }
        };
    }
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_some() {
        return Err(
            "managed build terminalization is missing its Host bridge capability; relaunch the Session"
                .to_string(),
        );
    }

    let (project_state_root, work_event_root) =
        crate::agent_project_state::agent_session_roots_or_fallback(repo, &session_id)
            .map_err(|error| error.to_string())?;
    let legacy_work_id = format!("work-session-{session_id}");

    let now = chrono::Utc::now();
    let outcome = match close_kind {
        WorkTerminalKind::Done => {
            gwt_core::workspace_projection::emit_workspace_done_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
        WorkTerminalKind::Discarded => {
            gwt_core::workspace_projection::emit_workspace_discard_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
    }
    .map_err(|error| error.to_string())?;
    match outcome {
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::Emitted
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AlreadyMatching
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::NoTarget => Ok(()),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AssignedWorkMissing(
            work_id,
        ) => Err(format!(
            "assigned Work {work_id} is not materialized; retry workspace.ensure before finalizing the build"
        )),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::WrongTerminal => Err(
            format!(
                "assigned Work has the wrong terminal state for {}",
                close_kind.as_str()
            ),
        ),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AmbiguousTerminal => Err(
            "assigned Work has ambiguous Done and Discarded terminal state".to_string(),
        ),
    }
}

fn map_agent_terminal_outcome(
    outcome: crate::AgentWorkTerminalizationOutcome,
    close_kind: WorkTerminalKind,
) -> Result<(), String> {
    match outcome {
        crate::AgentWorkTerminalizationOutcome::Emitted
        | crate::AgentWorkTerminalizationOutcome::AlreadyMatching
        | crate::AgentWorkTerminalizationOutcome::NoTarget => Ok(()),
        crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing => Err(
            "assigned Work is not materialized; retry workspace.ensure before finalizing the build"
                .to_string(),
        ),
        crate::AgentWorkTerminalizationOutcome::WrongTerminal => Err(format!(
            "assigned Work has the wrong terminal state for {}",
            close_kind.as_str()
        )),
        crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal => {
            Err("assigned Work has ambiguous Done and Discarded terminal state".to_string())
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WorkTerminalKind {
    Done,
    Discarded,
}

impl WorkTerminalKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Discarded => "Discarded",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener as StdTcpListener,
        path::{Path, PathBuf},
        sync::{mpsc, Arc},
        thread::JoinHandle,
        time::Duration,
    };

    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use gwt_core::test_support::ScopedEnvVar;
    use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

    use super::*;

    struct TerminalBridgeServer {
        runtime: Runtime,
        shutdown_tx: Option<oneshot::Sender<()>>,
        rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        abort_rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        redirect_rx: mpsc::Receiver<HeaderMap>,
        forward_url: String,
    }

    struct RawTerminalBridgeServer {
        join_handle: Option<JoinHandle<()>>,
        request_rx: mpsc::Receiver<()>,
        forward_url: String,
    }

    impl RawTerminalBridgeServer {
        fn start(
            status: StatusCode,
            declared_content_length: Option<usize>,
            body: Vec<u8>,
        ) -> Self {
            let listener =
                StdTcpListener::bind(("127.0.0.1", 0)).expect("raw terminal bridge listener");
            let address = listener.local_addr().expect("raw terminal bridge address");
            let (request_tx, request_rx) = mpsc::channel();
            let join_handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("raw terminal bridge request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("raw terminal bridge read timeout");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut chunk).expect("read raw terminal request");
                    assert_ne!(read, 0, "terminal bridge request ended before its body");
                    request.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&request[..header_end])
                        .expect("raw terminal request headers");
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
                request_tx.send(()).expect("capture raw terminal request");
                let reason = status.canonical_reason().unwrap_or("Rejected");
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                    status.as_u16(),
                    reason
                )
                .expect("write raw terminal response status");
                if let Some(content_length) = declared_content_length {
                    write!(stream, "Content-Length: {content_length}\r\n")
                        .expect("write raw terminal response length");
                }
                write!(stream, "\r\n").expect("finish raw terminal response headers");
                // The client may reject an oversized Content-Length before
                // consuming the body and close the socket. That is the
                // behavior this fixture exercises, not a server failure.
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            });
            Self {
                join_handle: Some(join_handle),
                request_rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) {
            self.request_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("raw build abort bridge request");
        }
    }

    impl Drop for RawTerminalBridgeServer {
        fn drop(&mut self) {
            if let Some(join_handle) = self.join_handle.take() {
                join_handle.join().expect("join raw terminal bridge server");
            }
        }
    }

    #[derive(Clone)]
    struct TerminalBridgeState {
        tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        abort_tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        status: StatusCode,
        body: String,
        before_response: Arc<dyn Fn() + Send + Sync>,
        redirect_location: Option<String>,
        redirect_tx: mpsc::Sender<HeaderMap>,
    }

    impl TerminalBridgeServer {
        fn start(status: StatusCode, body: serde_json::Value) -> Self {
            Self::start_with_hook(status, body, || {})
        }

        fn start_with_hook(
            status: StatusCode,
            body: serde_json::Value,
            before_response: impl Fn() + Send + Sync + 'static,
        ) -> Self {
            Self::start_inner(status, body, before_response, None)
        }

        fn start_redirect() -> Self {
            Self::start_inner(
                StatusCode::TEMPORARY_REDIRECT,
                serde_json::Value::Null,
                || {},
                Some("/redirected-terminal".to_string()),
            )
        }

        fn start_inner(
            status: StatusCode,
            body: serde_json::Value,
            before_response: impl Fn() + Send + Sync + 'static,
            redirect_location: Option<String>,
        ) -> Self {
            let runtime = Runtime::new().expect("terminal bridge runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("terminal bridge listener");
            let address = listener.local_addr().expect("terminal bridge address");
            let (tx, rx) = mpsc::channel();
            let (abort_tx, abort_rx) = mpsc::channel();
            let (redirect_tx, redirect_rx) = mpsc::channel();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let app = Router::new()
                .route(
                    "/internal/work-materialization-probe",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture materialization probe request");
                            (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response()
                        },
                    ),
                )
                .route(
                    "/internal/work-terminalization",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture terminal bridge request");
                            (state.before_response)();
                            let mut response = (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response();
                            if let Some(location) = state.redirect_location.as_deref() {
                                response.headers_mut().insert(
                                    axum::http::header::LOCATION,
                                    axum::http::HeaderValue::from_str(location)
                                        .expect("valid redirect location"),
                                );
                            }
                            response
                        },
                    ),
                )
                .route(
                    "/internal/build-abort-terminalization",
                    post(
                        |headers: HeaderMap,
                         State(state): State<TerminalBridgeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .abort_tx
                                .send((headers, body))
                                .expect("capture build abort bridge request");
                            (state.before_response)();
                            let mut response = (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response();
                            if let Some(location) = state.redirect_location.as_deref() {
                                response.headers_mut().insert(
                                    axum::http::header::LOCATION,
                                    axum::http::HeaderValue::from_str(location)
                                        .expect("valid redirect location"),
                                );
                            }
                            response
                        },
                    ),
                )
                .route(
                    "/redirected-terminal",
                    post(
                        |headers: HeaderMap, State(state): State<TerminalBridgeState>| async move {
                            state
                                .redirect_tx
                                .send(headers)
                                .expect("capture redirected terminal request");
                            StatusCode::OK
                        },
                    ),
                )
                .with_state(TerminalBridgeState {
                    tx,
                    abort_tx,
                    status,
                    body: body.to_string(),
                    before_response: Arc::new(before_response),
                    redirect_location,
                    redirect_tx,
                });
            runtime.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("terminal bridge server");
            });
            Self {
                runtime,
                shutdown_tx: Some(shutdown_tx),
                rx,
                abort_rx,
                redirect_rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) -> (HeaderMap, serde_json::Value) {
            self.rx
                .recv_timeout(Duration::from_secs(2))
                .expect("terminal bridge request")
        }

        fn receive_abort(&self) -> (HeaderMap, serde_json::Value) {
            self.abort_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("build abort bridge request")
        }

        fn assert_no_request(&self) {
            assert!(
                matches!(self.rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "terminal bridge must not receive a request"
            );
        }

        fn assert_no_abort_request(&self) {
            assert!(
                matches!(self.abort_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "build abort bridge must not receive a request"
            );
        }

        fn assert_no_redirect(&self) {
            assert!(
                matches!(self.redirect_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "terminal bridge client must not follow redirects"
            );
        }
    }

    impl Drop for TerminalBridgeServer {
        fn drop(&mut self) {
            if let Some(shutdown_tx) = self.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
            self.runtime
                .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
        }
    }

    fn terminal_receipt(outcome: crate::AgentWorkTerminalizationOutcome) -> serde_json::Value {
        serde_json::to_value(crate::AgentWorkTerminalizationReceipt {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            outcome,
        })
        .expect("serialize terminal receipt")
    }

    fn run_active_action(
        action: SkillStateAction,
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        gwt_core::skill_state::save(
            &fixture.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(3327),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: "terminal-bridge-session".to_string(),
            },
        )
        .expect("save active build state");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "terminal-bridge-session");
        let _forward_url = forward_url.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, value),
        );
        let _forward_token = forward_token.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, value),
        );
        let _runtime = if managed {
            ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                fixture.repo.join("managed-runtime.json"),
            )
        } else {
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        };
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(&mut env, action, &mut output).expect("run build action");
        (code, output, fixture)
    }

    fn assert_build_still_active(
        fixture: &crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load build state")
                .expect("build state")
                .active,
            "failed Host terminalization must not finalize build state"
        );
    }

    struct BoundTerminalFixture {
        git: crate::cli::verification_record::tests::WorkEventGitFixture,
        session: gwt_agent::Session,
        binding: gwt_agent::SessionExecutionBinding,
        work_id: String,
    }

    impl BoundTerminalFixture {
        fn new() -> Self {
            const OWNER_NUMBER: u64 = 3327;
            let git = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
            let branch =
                gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&git.repo))
                    .expect("read fixture branch");
            assert!(branch.status.success(), "fixture branch must be readable");
            let branch = String::from_utf8(branch.stdout)
                .expect("fixture branch is UTF-8")
                .trim()
                .to_string();
            let mut session =
                gwt_agent::Session::new(&git.repo, &branch, gwt_agent::AgentId::Codex);
            session.id = "terminal-bridge-session".to_string();
            session.project_state_root = Some(git.repo.clone());
            session.linked_issue_number = Some(OWNER_NUMBER);
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save unbound Session fixture");

            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: OWNER_NUMBER,
            };
            crate::cli::execution_state::materialize_at_launch(
                &git.repo,
                owner.kind,
                owner.number,
                &session.id,
                "gwt-execute",
                false,
            )
            .expect("materialize execution control fixture");
            crate::cli::execution_state::ensure_generation_ledger(
                &git.repo,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize generation ledger fixture");
            let identity = crate::cli::execution_state::current_execution_binding(&git.repo, owner)
                .expect("read fixture execution binding")
                .expect("fixture execution binding");
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: session
                    .repo_hash
                    .clone()
                    .expect("fixture repository identity"),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            session
                .set_execution_binding(Some(binding.clone()))
                .expect("bind fixture Session");
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save bound Session fixture");

            let work_id = "work-terminal-bridge-canonical".to_string();
            let now = chrono::Utc::now();
            let mut current =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&git.repo);
            current
                .agents
                .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                    session_id: session.id.clone(),
                    window_id: Some("project::terminal-bridge".to_string()),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.agent_id.display_name().to_string(),
                    status_category:
                        gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                    current_focus: None,
                    title_summary: None,
                    worktree_path: Some(git.repo.clone()),
                    branch: Some(branch),
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status:
                        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                    workspace_id: Some(work_id.clone()),
                    updated_at: now,
                });
            gwt_core::workspace_projection::save_workspace_projection(&git.repo, &current)
                .expect("save canonical current fixture");

            let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
            let mut start = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Start,
                &work_id,
                now,
            );
            start.title = Some("Canonical terminal bridge Work".to_string());
            start.owner = Some(format!("Issue #{}", owner.number));
            start.status_category =
                Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
            start.agent_session_id = Some(session.id.clone());
            start.agent_id = Some(session.agent_id.command().to_string());
            start.execution_container = Some(
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some(session.branch.clone()),
                    worktree_path: Some(git.repo.clone()),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            );
            work_items.apply_event(start);
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&git.repo);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &works_path,
                &work_items,
            )
            .expect("save canonical WorkItems fixture");

            gwt_core::skill_state::save(
                &git.repo,
                SKILL_NAME,
                &gwt_core::skill_state::SkillState {
                    start_evidence: None,
                    active: true,
                    owner_spec: Some(OWNER_NUMBER),
                    started_at: now,
                    phase: Some("pr".to_string()),
                    session_id: session.id.clone(),
                },
            )
            .expect("save active build fixture");

            Self {
                git,
                session,
                binding,
                work_id,
            }
        }

        fn terminalize_canonical_done(&self) {
            let receipt = crate::apply_bound_authenticated_work_terminalization(
                &self.git.repo,
                &self.session.id,
                &self.binding,
                crate::AgentWorkTerminalizationRequest {
                    schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: self.session.id.clone(),
                    observation: crate::observe_agent_runtime(&self.git.repo)
                        .expect("observe fixture runtime"),
                    terminal_kind: crate::AgentWorkTerminalKind::Done,
                },
            )
            .expect("terminalize canonical fixture");
            assert_eq!(
                receipt.outcome,
                crate::AgentWorkTerminalizationOutcome::Emitted
            );
        }

        fn settle_execution_blocked(&self) {
            assert!(matches!(
                crate::cli::execution_state::settle(
                    &self.git.repo,
                    &self.session.id,
                    crate::cli::execution_state::ExecutionSettlement::Blocked {
                        reason: "canonical verification is externally blocked".to_string(),
                        missing_verification: Some("full matrix".to_string()),
                    },
                )
                .expect("settle fixture execution"),
                crate::cli::execution_state::SettleResult::Settled(_)
            ));
        }

        fn canonical_work(&self) -> gwt_core::workspace_projection::WorkspaceWorkItem {
            gwt_core::workspace_projection::load_workspace_work_items(&self.git.repo)
                .expect("load canonical WorkItems")
                .expect("canonical WorkItems")
                .work_items
                .into_iter()
                .find(|work| work.id == self.work_id)
                .expect("canonical Work")
        }
    }

    struct SplitRootTerminalFixture {
        _temp: tempfile::TempDir,
        project_state_root: PathBuf,
        worktree: PathBuf,
        session: gwt_agent::Session,
        binding: gwt_agent::SessionExecutionBinding,
        work_id: String,
        legacy_work_items_path: PathBuf,
        legacy_close_path: PathBuf,
    }

    impl SplitRootTerminalFixture {
        fn new() -> Self {
            const OWNER_NUMBER: u64 = 3327;
            const BRANCH: &str = "work/20260601-0934";
            const REMOTE: &str = "https://example.invalid/acme/terminal-split-root.git";
            let temp = tempfile::tempdir().expect("split-root terminal fixture");
            let project_state_root = temp.path().join("workspace-home");
            let bootstrap = project_state_root.join("bootstrap");
            let worktree = project_state_root.join("work").join("issue-3327");
            std::fs::create_dir_all(&bootstrap).expect("split-root bootstrap");
            crate::cli::trusted_store::init_git_repo_with_origin(&bootstrap);
            run_terminal_fixture_git(&["checkout", "-b", BRANCH], &bootstrap);
            let bare = project_state_root.join("gwt.git");
            run_terminal_fixture_git(
                &[
                    "clone",
                    "--bare",
                    bootstrap.to_str().expect("bootstrap path"),
                    bare.to_str().expect("bare path"),
                ],
                &project_state_root,
            );
            run_terminal_fixture_git(&["remote", "set-url", "origin", REMOTE], &bare);
            std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
                .expect("worktree parent");
            run_terminal_fixture_git(
                &[
                    "worktree",
                    "add",
                    worktree.to_str().expect("worktree path"),
                    BRANCH,
                ],
                &bare,
            );
            let project_state_root =
                dunce::canonicalize(&project_state_root).expect("canonical Project State root");
            let worktree = dunce::canonicalize(&worktree).expect("canonical linked worktree");

            let session_id = "terminal-split-root-session";
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: OWNER_NUMBER,
            };
            crate::cli::execution_state::materialize_at_launch(
                &worktree,
                owner.kind,
                owner.number,
                session_id,
                "$gwt-execute #3327",
                false,
            )
            .expect("materialize split-root execution");
            crate::cli::execution_state::ensure_generation_ledger(
                &worktree,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize split-root generation ledger");
            let identity = crate::cli::execution_state::current_execution_binding(&worktree, owner)
                .expect("read split-root execution binding")
                .expect("split-root execution binding");
            let mut session = gwt_agent::Session::new(&worktree, BRANCH, gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.project_state_root = Some(project_state_root.clone());
            session.linked_issue_number = Some(OWNER_NUMBER);
            let binding = gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: session.id.clone(),
                repo_hash: session.repo_hash.clone().expect("split-root repo identity"),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            };
            session
                .set_execution_binding(Some(binding.clone()))
                .expect("bind split-root Session");
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save split-root Session");

            let work_id = "work-terminal-split-root-canonical".to_string();
            let now = chrono::Utc::now();
            let mut current =
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                    &project_state_root,
                );
            current
                .agents
                .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                    session_id: session.id.clone(),
                    window_id: Some("project::terminal-split-root".to_string()),
                    agent_id: session.agent_id.command().to_string(),
                    display_name: session.agent_id.display_name().to_string(),
                    status_category:
                        gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                    current_focus: None,
                    title_summary: None,
                    worktree_path: Some(worktree.clone()),
                    branch: Some(BRANCH.to_string()),
                    last_board_entry_id: None,
                    last_board_entry_kind: None,
                    coordination_scope: None,
                    affiliation_status:
                        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                    workspace_id: Some(work_id.clone()),
                    updated_at: now,
                });
            gwt_core::workspace_projection::save_workspace_projection(
                &project_state_root,
                &current,
            )
            .expect("save split-root canonical current");

            let mut start = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Start,
                &work_id,
                now,
            );
            start.title = Some("Split-root canonical terminal Work".to_string());
            start.owner = Some(format!("Issue #{}", owner.number));
            start.status_category =
                Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
            start.agent_session_id = Some(session.id.clone());
            start.agent_id = Some(session.agent_id.command().to_string());
            start.execution_container = Some(
                gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                    branch: Some(BRANCH.to_string()),
                    worktree_path: Some(worktree.clone()),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                },
            );
            let mut canonical = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
            canonical.apply_event(start);
            let canonical_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_state_root);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &canonical_path,
                &canonical,
            )
            .expect("save split-root canonical WorkItems");

            let mut legacy = canonical.clone();
            let mut discard = gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Discard,
                &work_id,
                now + chrono::Duration::seconds(1),
            );
            discard.agent_session_id = Some(session.id.clone());
            legacy.apply_event(discard.clone());
            // #3466: before the project store followed the repository
            // identity, a linked worktree got its own store keyed by the hash
            // of its *path*. Old Hosts wrote terminals there, so that store
            // still exists on disk after upgrading. Address it the way those
            // Hosts did, not through the (now canonical) worktree lookup.
            let legacy_store = gwt_core::paths::gwt_project_dir(
                &gwt_core::repo_hash::compute_path_hash(&worktree),
            )
            .join("project-state");
            let legacy_work_items_path = legacy_store.join("works.json");
            assert_ne!(canonical_path, legacy_work_items_path);
            gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                &legacy_work_items_path,
                &legacy,
            )
            .expect("save stale linked-worktree WorkItems");
            let legacy_close_path = legacy_store.join("work-events-closed.jsonl");
            std::fs::create_dir_all(legacy_close_path.parent().expect("legacy close parent"))
                .expect("legacy close parent");
            std::fs::write(
                &legacy_close_path,
                format!(
                    "{}\n",
                    serde_json::to_string(&discard).expect("serialize legacy discard")
                ),
            )
            .expect("save stale linked-worktree close ledger");

            gwt_core::skill_state::save(
                &worktree,
                SKILL_NAME,
                &gwt_core::skill_state::SkillState {
                    start_evidence: None,
                    active: true,
                    owner_spec: Some(OWNER_NUMBER),
                    started_at: now,
                    phase: Some("pr".to_string()),
                    session_id: session.id.clone(),
                },
            )
            .expect("save split-root build state");

            Self {
                _temp: temp,
                project_state_root,
                worktree,
                session,
                binding,
                work_id,
                legacy_work_items_path,
                legacy_close_path,
            }
        }

        fn terminalize_canonical_done(&self) {
            let receipt = crate::apply_bound_authenticated_work_terminalization(
                &self.project_state_root,
                &self.session.id,
                &self.binding,
                crate::AgentWorkTerminalizationRequest {
                    schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                    claimed_session_id: self.session.id.clone(),
                    observation: crate::observe_agent_runtime(&self.worktree)
                        .expect("observe split-root fixture runtime"),
                    terminal_kind: crate::AgentWorkTerminalKind::Done,
                },
            )
            .expect("terminalize split-root canonical Work");
            assert_eq!(
                receipt.outcome,
                crate::AgentWorkTerminalizationOutcome::Emitted
            );
        }
    }

    fn run_terminal_fixture_git(args: &[&str], cwd: &Path) {
        let output = gwt_core::process::run_git_logged(args, Some(cwd)).expect("run fixture git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn with_terminal_bridge_env<T>(
        fixture: &BoundTerminalFixture,
        server: &TerminalBridgeServer,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        with_terminal_bridge_env_for(&fixture.git.repo, &fixture.session.id, server, operation)
    }

    fn with_terminal_bridge_env_for<T>(
        repo: &Path,
        session_id: &str,
        server: &TerminalBridgeServer,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        with_terminal_bridge_url_env_for(repo, session_id, &server.forward_url, operation)
    }

    fn with_terminal_bridge_url_env_for<T>(
        repo: &Path,
        session_id: &str,
        forward_url: &str,
        operation: impl FnOnce(&crate::cli::TestEnv) -> T,
    ) -> T {
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, forward_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            repo.join("managed-runtime.json"),
        );
        operation(&crate::cli::TestEnv::new(repo.to_path_buf()))
    }

    #[test]
    fn build_complete_does_not_refuse_on_unsettled_work_event_bookkeeping() {
        // SPEC #3590 FR-026: the bookkeeping gate warns; it never refuses the
        // completion of delivered work.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        gwt_core::skill_state::save(
            &fixture.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(3327),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: "unsettled-bookkeeping-session".to_string(),
            },
        )
        .expect("save active build state");
        fixture.append_event("terminal-update-awaiting-delivery");
        let _session = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "unsettled-bookkeeping-session",
        );
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Complete {
                spec: 3327,
                pr: None,
            },
            &mut output,
        )
        .expect("run build.complete");

        assert_eq!(code, 0, "{output}");
        assert!(!output.contains("completion refused"), "{output}");
        assert!(output.contains("warning"), "{output}");
    }

    #[test]
    fn build_complete_rejects_typed_quarantine_without_session_identity() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let repo = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
        crate::cli::verification_record::save_plan(
            repo.path(),
            &crate::cli::verification_record::VerificationPlanRecord::from(
                crate::cli::verification_record::VerificationPlanData {
                    format_version: Some(1),
                    session_id: "session-missing".to_string(),
                    owner_number: None,
                    execution_binding: None,
                    commands: vec!["git --version".to_string()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    quarantines: Vec::new(),
                    created_at: chrono::Utc::now(),
                    content_hash: String::new(),
                },
            ),
        )
        .unwrap();
        let (mut verification, _) = crate::cli::verification_record::run_verification(
            repo.path(),
            "session-missing",
            &["git --version".to_string()],
        )
        .unwrap();
        verification.quarantined_failures =
            vec![crate::cli::verification_record::TypedQuarantinedFailure {
                failed_command: "cargo test -p gwt --lib".to_string(),
                test_identity: "tests::flake".to_string(),
                owner_issue: 3755,
                head_sha: "head".to_string(),
                merge_base_sha: "base".to_string(),
                baseline_command: "cargo test -p gwt --lib tests::flake -- --exact".to_string(),
                baseline_exit_code: 0,
                baseline_result_line: "test tests::flake ... ok".to_string(),
                pr_number: 3854,
                pr_reference: crate::cli::verification_record::PrQuarantineReference {
                    kind: crate::cli::verification_record::PrQuarantineReferenceKind::Body,
                    comment_id: None,
                    marker: crate::cli::verification_record::quarantine_marker(
                        "tests::flake",
                        3755,
                    ),
                },
            }];
        crate::cli::verification_record::save(repo.path(), &verification).unwrap();

        let mut env = crate::cli::TestEnv::new(repo.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Complete {
                spec: 3248,
                pr: None,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("current owning Session"), "{out}");
    }

    #[test]
    fn build_complete_atomically_settles_typed_quarantine_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let session_id = "session-typed-build";
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let repo = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(repo.path());
        crate::cli::execution_state::materialize_at_launch(
            repo.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            3248,
            session_id,
            "launch",
            false,
        )
        .unwrap();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 3248,
        };
        let binding =
            crate::cli::execution_state::current_execution_binding(repo.path(), owner).unwrap();
        let command = "cargo test -p gwt --lib".to_string();
        let test_identity = "tests::flake";
        let baseline_command = format!("{command} {test_identity} -- --exact");
        crate::cli::verification_record::save_plan(
            repo.path(),
            &crate::cli::verification_record::VerificationPlanRecord::from(
                crate::cli::verification_record::VerificationPlanData {
                    format_version: Some(1),
                    session_id: session_id.to_string(),
                    owner_number: Some(3248),
                    execution_binding: binding.clone(),
                    commands: vec![command.clone()],
                    derived: false,
                    worktree_fingerprint: String::new(),
                    surfaces: Vec::new(),
                    generated_outputs: Vec::new(),
                    quarantines: vec![
                        crate::cli::verification_record::VerificationQuarantineRequest {
                            failed_command: command.clone(),
                            test_identity: test_identity.to_string(),
                            baseline_command: baseline_command.clone(),
                            owner_issue: 3755,
                            pr_number: 3854,
                        },
                    ],
                    created_at: chrono::Utc::now(),
                    content_hash: String::new(),
                },
            ),
        )
        .unwrap();
        let plan = crate::cli::verification_record::load_plan(repo.path())
            .unwrap()
            .unwrap();
        let marker = crate::cli::verification_record::quarantine_marker(test_identity, 3755);
        crate::cli::verification_record::save(
            repo.path(),
            &crate::cli::verification_record::VerificationRunRecord::from(crate::cli::verification_record::VerificationRunData {
format_version: Some(1),
                continuation: None,
                lifecycle: None,
                record_id: "vrr-typed-build".to_string(),
                user_verification_result: None,
                session_id: session_id.to_string(),
                owner_number: Some(3248),
                execution_binding: binding,
                lease_id: None,
                worktree_fingerprint: plan.worktree_fingerprint.clone(),
                verified_head: None,
                driver: None,
                commands: vec![crate::cli::verification_record::VerificationCommandResult {
                    admission: None,
                    measurement: None,
                    headed_e2e: None,
                    nextest: None,
                    terminated_by_signal: None,
                    output_streams: Vec::new(),
                    command: command.clone(),
                    exit_code: 101,
                    output_tail: format!(
                        "failures:\n    {test_identity}\n\ntest result: FAILED. 0 passed; 1 failed"
                    ),
                }],
                all_passed: false,
                quarantined_failures: vec![
                    crate::cli::verification_record::TypedQuarantinedFailure {
                        failed_command: command,
                        test_identity: test_identity.to_string(),
                        owner_issue: 3755,
                        head_sha: "head".to_string(),
                        merge_base_sha: "base".to_string(),
                        baseline_command,
                        baseline_exit_code: 0,
                        baseline_result_line: format!("test {test_identity} ... ok"),
                        pr_number: 3854,
                        pr_reference: crate::cli::verification_record::PrQuarantineReference {
                            kind: crate::cli::verification_record::PrQuarantineReferenceKind::Body,
                            comment_id: None,
                            marker: marker.clone(),
                        },
                    },
                ],
                adjudications: Vec::new(),
                started_at: Some(chrono::Utc::now()),
                created_at: chrono::Utc::now(),
                plan_covered: true,
                planned_missing: Vec::new(),
                skipped_after_failure: Vec::new(),
                verification_plan_hash: plan.content_hash.clone(),
                verification_plan_snapshot: None,
                plan_derived: false,
                content_hash: String::new(),
            }),
        )
        .unwrap();

        let mut env = crate::cli::TestEnv::new(repo.path().to_path_buf());
        env.client.seed(gwt_github::IssueSnapshot {
            author_login: None,
            number: gwt_github::IssueNumber(3755),
            title: "owned flake".to_string(),
            body: String::new(),
            labels: Vec::new(),
            state: gwt_github::IssueState::Open,
            updated_at: gwt_github::client::UpdatedAt::new("2026-09-01T00:00:00Z"),
            comments: Vec::new(),
        });
        env.pr_quarantine_contexts.insert(
            3854,
            crate::cli::pr::PrQuarantineContext {
                number: 3854,
                body: marker,
                comments: Vec::new(),
            },
        );
        let mut out = String::new();
        gwt_core::skill_state::save(
            repo.path(),
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(3248),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: session_id.to_string(),
            },
        )
        .expect("seed active lifecycle for typed completion");
        out.clear();
        assert_eq!(
            run(
                &mut env,
                SkillStateAction::Complete {
                    spec: 3248,
                    pr: None
                },
                &mut out,
            )
            .unwrap(),
            0,
            "{out}"
        );
        let completed = crate::cli::execution_state::load(repo.path())
            .unwrap()
            .unwrap();
        assert_eq!(
            completed.status,
            crate::cli::execution_state::ExecutionControlStatus::Completed
        );
        assert!(
            completed
                .completion_evidence
                .is_some_and(|receipt| receipt.used_typed_quarantine),
            "typed quarantine must be pinned in the Completed ECR"
        );
        let completed_bytes = crate::cli::execution_state::record_contents(repo.path()).unwrap();
        let state_path = gwt_core::skill_state::state_path(repo.path(), SKILL_NAME);
        let state_bytes = std::fs::read(&state_path).unwrap();
        env.pr_quarantine_contexts
            .get_mut(&3854)
            .unwrap()
            .body
            .clear();
        out.clear();
        assert_eq!(
            run(
                &mut env,
                SkillStateAction::Complete {
                    spec: 3248,
                    pr: None,
                },
                &mut out,
            )
            .unwrap(),
            0,
            "completed lifecycle replay must not require the old PR marker: {out}"
        );
        assert_eq!(
            crate::cli::execution_state::record_contents(repo.path()).unwrap(),
            completed_bytes
        );
        assert_eq!(std::fs::read(state_path).unwrap(), state_bytes);
    }

    #[test]
    fn managed_build_complete_confirms_canonical_done_without_another_host_request() {
        struct RestoreCwd(PathBuf);
        impl Drop for RestoreCwd {
            fn drop(&mut self) {
                // test-hygiene: allow-unlocked-env the enclosing test holds env_test_lock through this restore
                std::env::set_current_dir(&self.0).expect("restore fixture caller CWD");
            }
        }
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let start = fixture.canonical_work().events[0].clone();
        let start_path =
            gwt_core::paths::gwt_repo_local_work_event_shard_path(&fixture.git.repo, &start.id);
        std::fs::create_dir_all(start_path.parent().unwrap()).unwrap();
        std::fs::write(
            &start_path,
            gwt_core::workspace_projection::canonical_workspace_work_event_bytes(
                &start,
                &fixture.git.repo,
            )
            .unwrap(),
        )
        .expect("persist accepted Start source without applying it again");
        fixture.terminalize_canonical_done();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let terminal_before = fixture.canonical_work();
        let mut old_pr = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Pr,
            &fixture.work_id,
            chrono::Utc::now(),
        );
        old_pr.agent_session_id = Some(fixture.session.id.clone());
        old_pr.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(fixture.session.branch.clone()),
                worktree_path: Some(std::path::PathBuf::from(".")),
                pr_number: Some(6122),
                pr_url: Some("https://github.com/akiojin/gwt/pull/6122".into()),
                pr_state: Some("OPEN".into()),
            },
        );
        gwt_core::workspace_projection::record_workspace_work_event(&fixture.git.repo, old_pr)
            .expect("reproduce accepted old-reader relative PR container");
        assert_eq!(fixture.canonical_work().execution_containers.len(), 2);
        let closed_path =
            gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&fixture.git.repo);
        let closed_before = std::fs::read(&closed_path).expect("accepted terminal source");
        let source_bytes: Vec<_> = fixture
            .canonical_work()
            .events
            .iter()
            .filter_map(|event| {
                let path = gwt_core::paths::gwt_repo_local_work_event_shard_path(
                    &fixture.git.repo,
                    &event.id,
                );
                std::fs::read(&path).ok().map(|bytes| (path, bytes))
            })
            .collect();
        let poisoned = std::fs::read(&works_path).unwrap();
        let caller_cwd = RestoreCwd(std::env::current_dir().unwrap());
        std::env::set_current_dir(&fixture.git.repo).unwrap();
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            assert!(
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete {
                        spec: 3327,
                        pr: None
                    },
                )
                .is_err(),
                "old duplicate containers must remain ambiguous before repair"
            );
            std::env::set_current_dir(&caller_cwd.0).unwrap();
            let legacy = gwt_core::paths::gwt_repo_local_work_events_path(&fixture.git.repo);
            let journal = fixture
                .canonical_work()
                .events
                .iter()
                .map(|event| serde_json::to_string(event).unwrap() + "\n")
                .collect::<String>();
            std::fs::write(&legacy, journal).unwrap();
            let (source_path, original) = &source_bytes[0];
            std::fs::write(source_path, b"{}\n").unwrap();
            let mut damaged_env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            damaged_env.stdin = serde_json::json!({"operation":"workspace.work_repair", "params":{"work_id":fixture.work_id}}).to_string();
            assert_ne!(
                crate::cli::json_envelope::dispatch(&mut damaged_env, "gwtd"),
                0,
                "a valid legacy copy must not hide a tampered immutable shard"
            );
            assert_eq!(std::fs::read(&works_path).unwrap(), poisoned);
            std::fs::write(source_path, original).unwrap();
            for dry_run in [true, false, false] {
                let mut repair_env = crate::cli::TestEnv::new(fixture.git.repo.clone());
                repair_env.stdin = serde_json::json!({
                    "operation": "workspace.work_repair",
                    "params": {"work_id":fixture.work_id, "dry_run":dry_run}
                })
                .to_string();
                let code = crate::cli::json_envelope::dispatch(&mut repair_env, "gwtd");
                assert_eq!(
                    code,
                    0,
                    "repair failed: {} {}",
                    String::from_utf8_lossy(&repair_env.stdout),
                    String::from_utf8_lossy(&repair_env.stderr)
                );
                if dry_run {
                    assert_eq!(std::fs::read(&works_path).unwrap(), poisoned);
                }
            }
            let repaired = fixture.canonical_work();
            assert_eq!(repaired.execution_containers.len(), 1);
            assert_eq!(
                repaired.execution_containers[0].worktree_path.as_deref(),
                Some(fixture.git.repo.as_path())
            );
            assert_eq!(repaired.execution_containers[0].pr_number, Some(6122));
            assert_eq!(repaired.owner, terminal_before.owner);
            assert_eq!(repaired.completed_at, terminal_before.completed_at);
            assert_eq!(repaired.status_category, terminal_before.status_category);
            assert_eq!(std::fs::read(&closed_path).unwrap(), closed_before);
            for (path, bytes) in &source_bytes {
                assert_eq!(&std::fs::read(path).unwrap(), bytes);
            }
            let before = std::fs::read(&works_path).unwrap();
            let result = record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            );
            assert_eq!(
                std::fs::read(&works_path).unwrap(),
                before,
                "confirm-only compatibility must not append another terminal event"
            );
            result
        });

        assert!(result.is_ok(), "canonical Done must win: {result:?}");
        assert!(fixture.canonical_work().is_terminal());
        assert!(!fixture.canonical_work().discarded);
        server.assert_no_request();
        let mut delivered = fixture.canonical_work().execution_containers[0].clone();
        delivered.pr_state = Some("MERGED".to_string());
        gwt_core::workspace_projection::record_workspace_pr_metadata_for_execution(
            &fixture.git.repo,
            "Issue #3327",
            &fixture.session.id,
            &delivered,
        )
        .expect("PR metadata must select the repaired canonical container");
        assert_eq!(fixture.canonical_work().execution_containers.len(), 1);
        assert_eq!(
            fixture.canonical_work().execution_containers[0]
                .pr_state
                .as_deref(),
            Some("MERGED")
        );
    }

    #[test]
    fn managed_build_complete_closes_done_unassigned_work_with_merged_pr() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.terminalize_canonical_done();
        let mut final_event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            &fixture.work_id,
            chrono::Utc::now(),
        );
        final_event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        let mut container = fixture.canonical_work().execution_containers[0].clone();
        container.pr_number = Some(3840);
        container.pr_state = Some("MERGED".to_string());
        final_event.execution_container = Some(container);
        gwt_core::workspace_projection::record_workspace_work_event(&fixture.git.repo, final_event)
            .expect("record final Done event with merged PR");
        run_terminal_fixture_git(&["add", "--", ".gwt/work/events"], &fixture.git.repo);
        fixture
            .git
            .commit("chore(work): record delivered Done event");
        fixture.git.push();
        let mut current =
            gwt_core::workspace_projection::load_workspace_projection(&fixture.git.repo)
                .unwrap()
                .unwrap();
        current.agents[0].affiliation_status =
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
        current.agents[0].workspace_id = None;
        gwt_core::workspace_projection::save_workspace_projection(&fixture.git.repo, &current)
            .unwrap();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let state_paths = [
            works_path,
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&fixture.git.repo),
            gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&fixture.git.repo),
        ];
        let state_bytes = || {
            state_paths
                .each_ref()
                .map(|path| std::fs::read(path).unwrap())
        };
        let before = state_bytes();
        let server = TerminalBridgeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "transaction_conflict",
                "reason": "transaction_conflict",
                "message": "Host workspace transaction failed without committing"
            }),
        );
        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
                &mut output,
            )
            .unwrap();
            (code, output)
        });
        assert_eq!(code, 0, "{output}");
        assert!(!output.contains("run workspace.ensure"), "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .unwrap()
                .unwrap()
                .active
        );
        assert_eq!(state_bytes(), before);
        assert_eq!(
            fixture.canonical_work().execution_containers[0]
                .pr_state
                .as_deref(),
            Some("MERGED")
        );
        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
                &mut output,
            )
            .unwrap();
            (code, output)
        });
        assert_eq!(code, 0, "idempotent retry: {output}");
        assert_eq!(state_bytes(), before);
        server.assert_no_request();
    }

    #[test]
    fn managed_build_complete_preserves_opposite_linked_worktree_legacy_terminal() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = SplitRootTerminalFixture::new();
        fixture.terminalize_canonical_done();
        let canonical_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(
            &fixture.project_state_root,
        );
        let canonical_before =
            std::fs::read(&canonical_path).expect("snapshot repo-global canonical WorkItems");
        let legacy_before = [
            std::fs::read(&fixture.legacy_work_items_path)
                .expect("snapshot linked-worktree legacy WorkItems"),
            std::fs::read(&fixture.legacy_close_path)
                .expect("snapshot linked-worktree legacy close ledger"),
        ];
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::WrongTerminal),
        );

        let result =
            with_terminal_bridge_env_for(&fixture.worktree, &fixture.session.id, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete {
                        spec: 3327,
                        pr: None,
                    },
                )
            });

        assert!(
            result.is_ok(),
            "split-root canonical Done must win: {result:?}"
        );
        assert_eq!(
            std::fs::read(&canonical_path).expect("read repo-global canonical WorkItems"),
            canonical_before,
            "confirm-only must not duplicate the canonical terminal"
        );
        assert_eq!(
            [
                std::fs::read(&fixture.legacy_work_items_path)
                    .expect("read linked-worktree legacy WorkItems"),
                std::fs::read(&fixture.legacy_close_path)
                    .expect("read linked-worktree legacy close ledger"),
            ],
            legacy_before,
            "new-client reconciliation must preserve old-Host legacy terminal bytes"
        );
        let canonical =
            gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_state_root)
                .expect("load canonical split-root WorkItems")
                .expect("canonical split-root WorkItems");
        let work = canonical
            .work_items
            .iter()
            .find(|work| work.id == fixture.work_id)
            .expect("canonical split-root Work");
        assert!(work.is_terminal());
        assert!(!work.discarded);
        server.assert_no_request();
    }

    /// SPEC-3431 (#3425 family): a session without an execution binding can
    /// still abort its own build state.
    ///
    /// A binding exists only when gwt's launch materialization wrote it; a
    /// resumed successor session has none. Such a session also has no bound
    /// Work to terminalize — yet the abort path demanded exact durable Host
    /// Work authority and failed with "durable Session ... has no execution
    /// binding". Combined with `build.complete` requiring a Work event
    /// receipt (which likewise needs the binding), a skill state opened by an
    /// unbound session could be neither settled nor abandoned, so the Stop
    /// hook blocked that session forever. Observed live on work/issue-3431.
    ///
    /// Fail-open is safe here: skipping means "no bound Work exists, so there
    /// is nothing to mark", and the Done path keeps its own receipt gate in
    /// `build.complete`, which an unbound session still cannot pass.
    #[test]
    fn unbound_session_abort_skips_the_terminal_work_update() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let git = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let branch =
            gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&git.repo))
                .expect("read fixture branch");
        let branch = String::from_utf8(branch.stdout)
            .expect("utf8")
            .trim()
            .to_string();
        let mut session = gwt_agent::Session::new(&git.repo, &branch, gwt_agent::AgentId::Codex);
        session.id = "unbound-successor-session".to_string();
        session.project_state_root = Some(git.repo.clone());
        session.linked_issue_number = Some(3431);
        assert!(
            session.execution_binding.is_none(),
            "precondition: this session was never bound by a launch"
        );
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save unbound session");
        gwt_core::skill_state::save(
            &git.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(3431),
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: session.id.clone(),
            },
        )
        .expect("open the build state as this session");

        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let result = with_terminal_bridge_env_for(&git.repo, &session.id, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Abort {
                    spec: 3431,
                    reason: Some("opened only to probe verify authority".to_string()),
                },
            )
        });

        assert!(
            result.is_ok(),
            "an unbound session must be able to abandon its own build state: {result:?}"
        );
    }

    #[test]
    fn managed_build_abort_closes_canonical_active_after_old_host_no_write_outcome() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        assert!(!fixture.canonical_work().is_terminal());
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("cancelled".to_string()),
                },
            )
        });

        assert!(result.is_ok(), "canonical abort must converge: {result:?}");
        let work = fixture.canonical_work();
        assert!(work.is_terminal());
        assert!(
            work.discarded,
            "canonical Work must be discarded exactly once"
        );
        let (_, request) = server.receive();
        assert_eq!(request["terminal_kind"], "discarded");
    }

    #[test]
    fn blocked_execution_allows_build_abort_but_rejects_build_complete() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let abort_fixture = BoundTerminalFixture::new();
        let mut projection =
            gwt_core::workspace_projection::load_workspace_projection(&abort_fixture.git.repo)
                .expect("load abort fixture identity")
                .expect("abort fixture identity exists");
        projection.agents[0].title_summary = Some("blocked build abort authority".to_string());
        projection.agents[0].current_focus = Some("abort the matching blocked build".to_string());
        gwt_core::workspace_projection::save_workspace_projection(
            &abort_fixture.git.repo,
            &projection,
        )
        .expect("open abort fixture identity gate");
        let _session = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            abort_fixture.session.id.clone(),
        );
        // Active executions have an unavailable Blocked-only abort probe too;
        // they must still finish or abort through the ordinary lifecycle.
        assert!(matches!(
            crate::cli::hook::skill_build_spec_stop_check::handle_with_input(
                &abort_fixture.git.repo,
                "{}",
                Some(&abort_fixture.session.id),
            ),
            crate::cli::hook::HookOutput::StopBlock { .. }
        ));
        assert!(matches!(
            crate::cli::execution_state::settle(
                &abort_fixture.git.repo,
                &abort_fixture.session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle abort fixture execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let abort_server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let diagnosis = crate::cli::execution_state::diagnose(
            &abort_fixture.git.repo,
            Some(&abort_fixture.session.id),
        );
        assert!(diagnosis
            .available_recoveries
            .contains(&"build.abort".to_string()));
        assert!(matches!(
            crate::cli::hook::skill_build_spec_stop_check::handle_with_input(
                &abort_fixture.git.repo,
                "{}",
                Some(&abort_fixture.session.id),
            ),
            crate::cli::hook::HookOutput::StopBlock { .. }
        ));
        let abort_repo = abort_fixture.git.repo.clone();
        let (abort_code, abort_output) =
            with_terminal_bridge_env(&abort_fixture, &abort_server, |_| {
                let mut env = crate::cli::TestEnv::new(abort_repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after execution.blocked");
                (code, output)
            });

        assert_eq!(abort_code, 0, "{abort_output}");
        assert!(
            !gwt_core::skill_state::load(&abort_fixture.git.repo, SKILL_NAME)
                .expect("load aborted build state")
                .expect("aborted build state")
                .active,
            "build.abort must close the stranded lifecycle"
        );
        let aborted_work = abort_fixture.canonical_work();
        assert!(aborted_work.is_terminal());
        assert!(aborted_work.discarded);
        let (_, abort_request) = abort_server.receive_abort();
        assert_eq!(abort_request["owner_number"], 3327);
        assert_eq!(
            abort_request["reason"],
            "blocked verification cannot proceed"
        );
        assert!(abort_request.get("terminal_kind").is_none());

        let history_paths = [
            gwt_core::skill_state::state_path(&abort_fixture.git.repo, SKILL_NAME),
            crate::cli::execution_state::state_path(&abort_fixture.git.repo),
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&abort_fixture.git.repo),
        ];
        let history_before = history_paths
            .iter()
            .map(|path| std::fs::read(path).expect("terminal history"))
            .collect::<Vec<_>>();
        let (retry_code, retry_output) =
            with_terminal_bridge_env(&abort_fixture, &abort_server, |_| {
                let mut env = crate::cli::TestEnv::new(abort_fixture.git.repo.clone());
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("retry after the original abort".to_string()),
                    },
                    &mut output,
                )
                .expect("replay build.abort");
                (code, output)
            });
        assert_eq!(retry_code, 0, "{retry_output}");
        for (path, before) in history_paths.iter().zip(history_before) {
            assert_eq!(
                std::fs::read(path).expect("replayed terminal history"),
                before,
                "build.abort replay must preserve {}",
                path.display()
            );
        }
        abort_server.assert_no_abort_request();
        abort_server.assert_no_request();

        let complete_fixture = BoundTerminalFixture::new();
        assert!(matches!(
            crate::cli::execution_state::settle(
                &complete_fixture.git.repo,
                &complete_fixture.session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle complete fixture execution"),
            crate::cli::execution_state::SettleResult::Settled(_)
        ));
        let complete_server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let complete_repo = complete_fixture.git.repo.clone();
        let (complete_code, complete_output) =
            with_terminal_bridge_env(&complete_fixture, &complete_server, |_| {
                let mut env = crate::cli::TestEnv::new(complete_repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Complete {
                        spec: 3327,
                        pr: None,
                    },
                    &mut output,
                )
                .expect("run build.complete after execution.blocked");
                (code, output)
            });

        assert_ne!(complete_code, 0, "{complete_output}");
        assert_build_still_active(&complete_fixture.git);
        assert!(!complete_fixture.canonical_work().is_terminal());
        complete_server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_transport_failure_never_falls_back_locally() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();

        let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve unavailable bridge address");
        let port = unavailable
            .local_addr()
            .expect("unavailable bridge address")
            .port();
        drop(unavailable);
        let unavailable_url = format!("http://127.0.0.1:{port}/internal/hook-live");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &fixture.session.id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &unavailable_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            fixture.git.repo.join("managed-runtime.json"),
        );
        let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
        let mut output = String::new();

        let code = run(
            &mut env,
            SkillStateAction::Abort {
                spec: 3327,
                reason: Some("blocked verification cannot proceed".to_string()),
            },
            &mut output,
        )
        .expect("run build.abort with unavailable Host bridge");

        assert_ne!(code, 0, "{output}");
        assert!(
            output.contains("outcome may be unknown"),
            "transport failure must remain fail-closed: {output}"
        );
        assert_build_still_active(&fixture.git);
        assert!(
            !fixture.canonical_work().is_terminal(),
            "unknown Host outcome must not trigger a local terminal write"
        );
    }

    #[test]
    fn blocked_build_abort_rejects_blank_reason_before_host_or_local_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let server = TerminalBridgeServer::start(StatusCode::NOT_FOUND, serde_json::Value::Null);
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("   ".to_string()),
                },
                &mut output,
            )
            .expect("run build.abort with blank reason");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("non-empty reason"), "{output}");
        assert_build_still_active(&fixture.git);
        assert!(!fixture.canonical_work().is_terminal());
        server.assert_no_abort_request();
    }

    #[test]
    fn blocked_build_abort_unreadable_or_oversized_rejection_body_fails_closed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let limit = 64 * 1024;

        for (label, declared_content_length, body) in [
            ("truncated", Some(64), b"{}".to_vec()),
            ("declared-oversized", Some(limit + 1), vec![b' '; limit + 1]),
            ("streamed-oversized", None, vec![b' '; limit + 1]),
        ] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = RawTerminalBridgeServer::start(
                StatusCode::NOT_FOUND,
                declared_content_length,
                body,
            );
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_url_env_for(
                &fixture.git.repo,
                &fixture.session.id,
                &server.forward_url,
                |_| {
                    let mut env = crate::cli::TestEnv::new(repo);
                    let mut output = String::new();
                    let code = run(
                        &mut env,
                        SkillStateAction::Abort {
                            spec: 3327,
                            reason: Some("blocked verification cannot proceed".to_string()),
                        },
                        &mut output,
                    )
                    .expect("run build.abort after unsafe Host rejection body");
                    (code, output)
                },
            );

            assert_ne!(code, 0, "{label}: {output}");
            assert!(output.contains("transport_failure"), "{label}: {output}");
            assert_build_still_active(&fixture.git);
            assert!(
                !fixture.canonical_work().is_terminal(),
                "{label}: unsafe rejection body must not trigger a local terminal write"
            );
            server.receive();
        }
    }

    #[test]
    fn blocked_build_abort_missing_host_route_falls_back_locally() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        for status in [StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = TerminalBridgeServer::start(status, serde_json::Value::Null);
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
                let mut env = crate::cli::TestEnv::new(repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after explicit Host rejection");
                (code, output)
            });

            assert_eq!(code, 0, "{status}: {output}");
            assert!(
                !gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                    .expect("load reconciled build state")
                    .expect("reconciled build state")
                    .active
            );
            let work = fixture.canonical_work();
            assert!(work.is_terminal());
            assert!(work.discarded);
            server.receive_abort();
        }
    }

    #[test]
    fn blocked_build_abort_host_denials_and_unknown_responses_fail_closed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, status, body) in [
            (
                "unauthorized",
                StatusCode::UNAUTHORIZED,
                serde_json::json!({
                    "code": "invalid_request",
                    "reason": "invalid_request",
                    "message": "agent capability is invalid"
                }),
            ),
            (
                "forbidden",
                StatusCode::FORBIDDEN,
                serde_json::json!({
                    "code": "invalid_request",
                    "reason": "invalid_request",
                    "message": "agent capability is forbidden"
                }),
            ),
            (
                "authority-mismatch",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "code": "execution_binding_mismatch",
                    "reason": "authority_mismatch",
                    "message": "execution binding is stale"
                }),
            ),
            (
                "unknown-conflict",
                StatusCode::CONFLICT,
                serde_json::json!({
                    "code": "future_conflict",
                    "reason": "future_conflict",
                    "message": "future Host rejection"
                }),
            ),
            (
                "internal",
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "code": "internal",
                    "reason": "internal",
                    "message": "Host mutation outcome is unknown"
                }),
            ),
            (
                "unsupported-receipt",
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 2,
                    "outcome": "already_matching"
                }),
            ),
            ("invalid-receipt", StatusCode::OK, serde_json::Value::Null),
        ] {
            let fixture = BoundTerminalFixture::new();
            fixture.settle_execution_blocked();
            let server = TerminalBridgeServer::start(status, body);
            let repo = fixture.git.repo.clone();

            let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
                let mut env = crate::cli::TestEnv::new(repo);
                let mut output = String::new();
                let code = run(
                    &mut env,
                    SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("blocked verification cannot proceed".to_string()),
                    },
                    &mut output,
                )
                .expect("run build.abort after Host denial");
                (code, output)
            });

            assert_ne!(code, 0, "{label}: {output}");
            assert_build_still_active(&fixture.git);
            assert!(
                !fixture.canonical_work().is_terminal(),
                "{label}: Host denial or unknown outcome must not trigger a local terminal write"
            );
            server.receive_abort();
        }
    }

    #[test]
    fn blocked_build_abort_redirect_never_falls_back_or_forwards_its_bearer() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let server = TerminalBridgeServer::start_redirect();
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("blocked verification cannot proceed".to_string()),
                },
                &mut output,
            )
            .expect("run redirected build.abort");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert_build_still_active(&fixture.git);
        assert!(!fixture.canonical_work().is_terminal());
        server.receive_abort();
        server.assert_no_redirect();
    }

    #[test]
    fn managed_build_complete_converges_after_each_schema_valid_host_no_write_outcome() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for outcome in [
            crate::AgentWorkTerminalizationOutcome::AlreadyMatching,
            crate::AgentWorkTerminalizationOutcome::WrongTerminal,
            crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing,
            crate::AgentWorkTerminalizationOutcome::NoTarget,
        ] {
            let fixture = BoundTerminalFixture::new();
            let initial_event_count = fixture.canonical_work().events.len();
            let server = TerminalBridgeServer::start(StatusCode::OK, terminal_receipt(outcome));

            let result = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete {
                        spec: 3327,
                        pr: None,
                    },
                )
            });

            assert!(result.is_ok(), "{outcome:?} must converge: {result:?}");
            let work = fixture.canonical_work();
            assert!(work.is_terminal());
            assert!(!work.discarded);
            assert_eq!(
                work.events.len(),
                initial_event_count + 1,
                "{outcome:?} must append exactly one canonical terminal event"
            );
            server.receive();
        }
    }

    #[test]
    fn managed_build_rejects_host_emitted_without_exact_canonical_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let close_path =
            gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&fixture.git.repo);
        let mut unprojected = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Done,
            &fixture.work_id,
            chrono::Utc::now(),
        );
        unprojected.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done);
        unprojected.agent_session_id = Some(fixture.session.id.clone());
        std::fs::create_dir_all(close_path.parent().expect("canonical close parent"))
            .expect("canonical close parent");
        std::fs::write(
            &close_path,
            format!(
                "{}\n",
                serde_json::to_string(&unprojected).expect("serialize unprojected close")
            ),
        )
        .expect("seed unprojected canonical close");
        let before = std::fs::read(&works_path).expect("snapshot active canonical WorkItems");
        let close_before =
            std::fs::read(&close_path).expect("snapshot unprojected canonical close");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );

        let error = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        })
        .expect_err("Host Emitted without canonical readback must fail closed");

        assert!(
            error.contains("outside the canonical Work authority"),
            "{error}"
        );
        let cause = format!(
            "exact Work terminal confirmation found unprojected machine-local close event {}",
            unprojected.id
        );
        let log = std::fs::read_to_string(gwt_core::logging::current_log_file(
            &gwt_core::paths::gwt_logs_dir(),
        ))
        .expect("checkout compatibility failure must persist its cause");
        assert!(log.contains(&cause), "{log}");
        assert!(
            log.contains("Host workspace transaction failed without committing"),
            "{log}"
        );
        assert!(
            !error.contains(&cause),
            "the original cause must stay in the machine log: {error}"
        );
        assert_eq!(
            std::fs::read(&works_path).expect("read active canonical WorkItems"),
            before,
            "post-Emitted confirmation must never replay a local terminal event"
        );
        assert_eq!(
            std::fs::read(&close_path).expect("read unprojected canonical close"),
            close_before,
            "post-Emitted confirmation must be a pure readback"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_opposite_and_ambiguous_canonical_terminal_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for ambiguous in [false, true] {
            let fixture = BoundTerminalFixture::new();
            fixture.terminalize_canonical_done();
            if ambiguous {
                let works_path =
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
                let mut works =
                    gwt_core::workspace_projection::load_workspace_work_items(&fixture.git.repo)
                        .expect("load canonical WorkItems")
                        .expect("canonical WorkItems");
                works
                    .work_items
                    .iter_mut()
                    .find(|work| work.id == fixture.work_id)
                    .expect("canonical Work")
                    .discarded = true;
                gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                    &works_path,
                    &works,
                )
                .expect("seed ambiguous canonical terminal");
            }
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
            let before = std::fs::read(&works_path).expect("snapshot canonical terminal");
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
            );

            let error = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Abort {
                        spec: 3327,
                        reason: Some("cancelled".to_string()),
                    },
                )
            })
            .expect_err("opposite or ambiguous canonical terminal must fail before bridge");

            assert!(
                error.contains(if ambiguous { "ambiguous" } else { "opposite" }),
                "{error}"
            );
            assert_eq!(
                std::fs::read(&works_path).expect("read canonical terminal"),
                before
            );
            assert_build_still_active(&fixture.git);
            server.assert_no_request();
        }
    }

    #[test]
    fn managed_build_rejects_binding_rotation_between_bridge_and_exact_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot active canonical WorkItems");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session_id = fixture.session.id.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
            move || {
                gwt_agent::rotate_session_execution_capability(&sessions_dir, &session_id)
                    .expect("rotate capability before terminal continuation");
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        });

        assert!(result.is_err(), "rotated binding must fail closed");
        assert_eq!(
            std::fs::read(&works_path).expect("read active canonical WorkItems"),
            before,
            "binding rotation must preserve canonical Work bytes"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_accepts_host_emitted_only_after_exact_canonical_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let initial_event_count = fixture.canonical_work().events.len();
        let repo = fixture.git.repo.clone();
        let session_id = fixture.session.id.clone();
        let binding = fixture.binding.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
            move || {
                let receipt = crate::apply_bound_authenticated_work_terminalization(
                    &repo,
                    &session_id,
                    &binding,
                    crate::AgentWorkTerminalizationRequest {
                        schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
                        claimed_session_id: session_id.clone(),
                        observation: crate::observe_agent_runtime(&repo)
                            .expect("observe exact Host emission"),
                        terminal_kind: crate::AgentWorkTerminalKind::Done,
                    },
                )
                .expect("emit exact canonical Host terminal");
                assert_eq!(
                    receipt.outcome,
                    crate::AgentWorkTerminalizationOutcome::Emitted
                );
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        });

        assert!(
            result.is_ok(),
            "exact Host emission must confirm: {result:?}"
        );
        let work = fixture.canonical_work();
        assert!(work.is_terminal());
        assert!(!work.discarded);
        assert_eq!(work.events.len(), initial_event_count + 1);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_work_reassignment_during_bridge_without_closing_old_work() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let old_work_before = fixture.canonical_work();
        let repo = fixture.git.repo.clone();
        let session_id = fixture.session.id.clone();
        let server = TerminalBridgeServer::start_with_hook(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
            move || {
                let mut current = gwt_core::workspace_projection::load_workspace_projection(&repo)
                    .expect("load current during reassignment")
                    .expect("current during reassignment");
                current
                    .agents
                    .iter_mut()
                    .find(|agent| agent.session_id == session_id)
                    .expect("assigned Session")
                    .workspace_id = Some("work-reassigned-during-bridge".to_string());
                gwt_core::workspace_projection::save_workspace_projection(&repo, &current)
                    .expect("save reassigned current");
            },
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        });

        assert!(result.is_err(), "changed Work id must fail closed");
        assert_eq!(
            fixture.canonical_work(),
            old_work_before,
            "the snapshotted Work must remain byte-equivalent in projection semantics"
        );
        assert_build_still_active(&fixture.git);
        server.receive();
    }

    #[test]
    fn managed_build_rejects_duplicate_canonical_session_authority_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let mut current =
            gwt_core::workspace_projection::load_workspace_projection(&fixture.git.repo)
                .expect("load canonical current")
                .expect("canonical current");
        let duplicate = current
            .agents
            .iter()
            .find(|agent| agent.session_id == fixture.session.id)
            .expect("canonical Session assignment")
            .clone();
        current.agents.push(duplicate);
        gwt_core::workspace_projection::save_workspace_projection(&fixture.git.repo, &current)
            .expect("seed duplicate Session assignment");
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot canonical WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        });

        assert!(result.is_err(), "duplicate authority must fail closed");
        let error = result.unwrap_err();
        assert!(!error.contains("run workspace.ensure"), "{error}");
        assert!(error.contains("execution.status"), "{error}");
        assert!(error.contains("recovery_probes"), "{error}");
        assert_eq!(
            std::fs::read(&works_path).expect("read canonical WorkItems"),
            before
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_duplicate_session_authority_guides_status_recovery_without_ensure_loop()
    {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let mut current =
            gwt_core::workspace_projection::load_workspace_projection(&fixture.git.repo)
                .expect("load canonical current")
                .expect("canonical current");
        let duplicate = current
            .agents
            .iter()
            .find(|agent| agent.session_id == fixture.session.id)
            .expect("canonical Session assignment")
            .clone();
        current.agents.push(duplicate);
        gwt_core::workspace_projection::save_workspace_projection(&fixture.git.repo, &current)
            .expect("seed duplicate Session assignment");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover the stranded lifecycle".to_string()),
                },
                &mut output,
            )
            .expect("run blocked build.abort with ambiguous Work authority");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("execution.status"), "{output}");
        assert!(output.contains("recovery_probes"), "{output}");
        assert!(output.contains("verify.plan"), "{output}");
        assert!(
            !output.contains("run workspace.ensure"),
            "the refusal must not restart the build.abort/workspace.ensure loop: {output}"
        );
        let diagnosis =
            crate::cli::execution_state::diagnose(&fixture.git.repo, Some(&fixture.session.id));
        assert!(
            !diagnosis
                .available_recoveries
                .contains(&"build.abort".to_string()),
            "an operation rejected by the exact Work preflight must not be advertised: {diagnosis:?}"
        );
        let abort_probe = diagnosis
            .recovery_probes
            .iter()
            .find(|probe| probe.operation == "build.abort")
            .expect("build.abort operation-local recovery probe");
        assert_eq!(
            abort_probe.state,
            crate::cli::governance::RecoveryProbeState::Unavailable
        );
        assert_eq!(
            abort_probe.governance.cause,
            Some(crate::cli::governance::GovernanceCause::Authority)
        );
        assert_eq!(abort_probe.governance.retryable, Some(false));
        assert!(
            abort_probe
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("ambiguous")),
            "{abort_probe:?}"
        );
        let state_path = gwt_core::skill_state::state_path(&fixture.git.repo, SKILL_NAME);
        let before_state = std::fs::read(&state_path).unwrap();
        let before_execution = crate::cli::execution_state::load(&fixture.git.repo).unwrap();
        assert_eq!(
            crate::cli::hook::skill_build_spec_stop_check::handle_with_input(
                &fixture.git.repo,
                "{}",
                Some(&fixture.session.id),
            ),
            crate::cli::hook::HookOutput::Silent,
            "non-retryable unavailable abort must not strand Stop"
        );
        assert_eq!(std::fs::read(&state_path).unwrap(), before_state);
        assert_eq!(
            crate::cli::execution_state::load(&fixture.git.repo).unwrap(),
            before_execution,
            "releasing Stop must not pretend the lifecycle was terminalized"
        );
        let log_dir = gwt_core::paths::gwt_project_logs_dir_for_project_path(&fixture.git.repo);
        let log = std::fs::read_to_string(gwt_core::logging::current_log_file(&log_dir)).unwrap();
        let decision = log
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|entry| entry["fields"]["issue"] == 4623)
            .expect("durable stranded-build Stop decision");
        assert_eq!(decision["fields"]["session_id"], fixture.session.id);
        assert_eq!(decision["fields"]["owner_number"], 3327);
        assert_eq!(decision["fields"]["build_active"], true);
        assert_eq!(decision["fields"]["phase"], "pr");
        assert_eq!(decision["fields"]["abort_probe"]["state"], "unavailable");
        assert_eq!(
            decision["fields"]["abort_probe"]["governance"]["retryable"],
            false
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn blocked_build_abort_without_host_authority_guides_status_recovery() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        fixture.settle_execution_blocked();
        let mut docker = fixture.session.clone();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        docker
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Docker Session");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::NoTarget),
        );
        let repo = fixture.git.repo.clone();

        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover without Host authority".to_string()),
                },
                &mut output,
            )
            .expect("run blocked build.abort without Host authority");
            (code, output)
        });

        assert_ne!(code, 0, "{output}");
        assert!(output.contains("execution.status"), "{output}");
        assert!(output.contains("available_recoveries"), "{output}");
        assert!(output.contains("verify.plan"), "{output}");
        let diagnosis =
            crate::cli::execution_state::diagnose(&fixture.git.repo, Some(&fixture.session.id));
        let abort_probe = diagnosis
            .recovery_probes
            .iter()
            .find(|probe| probe.operation == "build.abort")
            .expect("build.abort operation-local recovery probe");
        assert_eq!(
            abort_probe.governance.cause,
            Some(crate::cli::governance::GovernanceCause::Authority)
        );
        assert_eq!(abort_probe.governance.retryable, Some(false));
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn managed_build_rejects_docker_authority_before_bridge() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let mut docker = fixture.session.clone();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        docker
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Docker Session");
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).expect("snapshot canonical WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
        );

        let result = with_terminal_bridge_env(&fixture, &server, |env| {
            record_current_work_terminal_before_finalize(
                env,
                &SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
            )
        });

        assert!(result.is_err(), "Docker authority must fail closed");
        assert_eq!(
            std::fs::read(&works_path).expect("read canonical WorkItems"),
            before
        );
        assert_build_still_active(&fixture.git);
        server.assert_no_request();
    }

    #[test]
    fn bridge_terminalization_requires_exact_durable_host_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for managed in [false, true] {
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::Emitted),
            );
            let (code, output, fixture) = run_active_action(
                SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
                Some(&server.forward_url),
                Some("terminal-secret"),
                managed,
            );
            assert_eq!(code, 1, "managed={managed}: {output}");
            assert!(
                output.contains("exact durable Host Work authority"),
                "{output}"
            );
            assert_build_still_active(&fixture);
            server.assert_no_request();
        }
    }

    #[test]
    fn managed_build_terminalization_failures_never_finalize_or_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, url, token) in [
            (
                "url-only",
                Some("http://127.0.0.1:45123/internal/hook-live"),
                None,
            ),
            ("token-only", None, Some("terminal-secret")),
            ("managed-missing", None, None),
        ] {
            let (code, output, fixture) = run_active_action(
                SkillStateAction::Complete {
                    spec: 3327,
                    pr: None,
                },
                url,
                token,
                true,
            );
            assert_eq!(code, 1, "{label}: {output}");
            assert_build_still_active(&fixture);
        }

        for (label, status, body) in [
            (
                "authentication",
                StatusCode::UNAUTHORIZED,
                serde_json::json!({
                    "code": "invalid_request",
                    "message": "untrusted Host diagnostic terminal-secret"
                }),
            ),
            (
                "invalid-response",
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 2,
                    "outcome": "already_matching"
                }),
            ),
            (
                "ambiguous-terminal",
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AmbiguousTerminal),
            ),
        ] {
            let fixture = BoundTerminalFixture::new();
            let server = TerminalBridgeServer::start(status, body);
            let result = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(
                    env,
                    &SkillStateAction::Complete {
                        spec: 3327,
                        pr: None,
                    },
                )
            });
            let output = result.expect_err("invalid Host result must fail closed");
            assert!(
                !output.contains("terminal-secret"),
                "{label}: Host response must not reflect the bearer into diagnostics: {output}"
            );
            assert_build_still_active(&fixture.git);
            server.receive();
        }

        let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve unavailable bridge address");
        let port = unavailable
            .local_addr()
            .expect("unavailable bridge address")
            .port();
        drop(unavailable);
        let unavailable_url = format!("http://127.0.0.1:{port}/internal/hook-live");
        let fixture = BoundTerminalFixture::new();
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &fixture.session.id);
        let _forward_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &unavailable_url);
        let _forward_token =
            ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            fixture.git.repo.join("managed-runtime.json"),
        );
        let env = crate::cli::TestEnv::new(fixture.git.repo.clone());
        let output = record_current_work_terminal_before_finalize(
            &env,
            &SkillStateAction::Complete {
                spec: 3327,
                pr: None,
            },
        )
        .expect_err("transport failure must fail closed");
        assert!(
            output.contains("unavailable") && output.contains("outcome may be unknown"),
            "{output}"
        );
        assert_build_still_active(&fixture.git);
    }

    #[test]
    fn terminal_bridge_never_follows_redirects_or_forwards_its_bearer() {
        let server = TerminalBridgeServer::start_redirect();
        let target = crate::daemon_runtime::HookForwardTarget {
            url: server.forward_url.clone(),
            token: "redirect-secret".to_string(),
        };
        let request = crate::AgentWorkTerminalizationRequest {
            schema_version: crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION,
            claimed_session_id: "redirect-session".to_string(),
            observation: crate::AgentRuntimeObservation {
                cwd: "/workspace/repo".to_string(),
                git_toplevel: "/workspace/repo".to_string(),
                repo_hash: "redirect-repo".to_string(),
                branch: "work/redirect".to_string(),
            },
            terminal_kind: crate::AgentWorkTerminalKind::Done,
        };

        let error =
            crate::daemon_runtime::send_work_terminalization_via_agent_bridge(&target, &request)
                .expect_err("redirected terminal bridge response must fail closed");

        assert!(!error.contains("redirect-secret"), "{error}");
        let (headers, _) = server.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer redirect-secret")
        );
        server.assert_no_redirect();
    }
    fn run_active_action_with_identity(
        action: SkillStateAction,
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
        owner_spec: Option<u64>,
        state_session_id: &str,
        current_session_id: Option<&str>,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        gwt_core::skill_state::save(
            &fixture.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec,
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: state_session_id.to_string(),
            },
        )
        .expect("save active build state");
        let _session = current_session_id.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, value),
        );
        let _forward_url = forward_url.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, value),
        );
        let _forward_token = forward_token.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, value),
        );
        let _runtime = if managed {
            ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                fixture.repo.join("managed-runtime.json"),
            )
        } else {
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        };
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(&mut env, action, &mut output).expect("run build action");
        (code, output, fixture)
    }

    #[test]
    fn managed_build_start_old_host_records_verified_local_probe() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let state_path = gwt_core::skill_state::state_path(&fixture.git.repo, SKILL_NAME);
        std::fs::remove_file(&state_path).unwrap();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        let before = std::fs::read(&works_path).unwrap();
        let server = TerminalBridgeServer::start(StatusCode::NOT_FOUND, serde_json::Value::Null);
        with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            let mut output = String::new();
            assert_eq!(
                run(
                    &mut env,
                    SkillStateAction::Start { spec: 3327 },
                    &mut output
                )
                .unwrap(),
                0,
                "{output}"
            );
            assert!(output.contains("host_probe_unavailable_404"), "{output}");
            let upgraded = TerminalBridgeServer::start(
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 1, "owner_number": 3327, "work_id": fixture.work_id,
                }),
            );
            {
                let _url =
                    ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &upgraded.forward_url);
                assert_eq!(
                    run(
                        &mut env,
                        SkillStateAction::Start { spec: 3327 },
                        &mut output
                    )
                    .unwrap(),
                    0
                );
            }
            upgraded.receive();
            assert_eq!(
                run(
                    &mut env,
                    SkillStateAction::Phase {
                        spec: 3327,
                        label: "green".into()
                    },
                    &mut output
                )
                .unwrap(),
                0
            );
            // Shared finalization must retain the start evidence as well.
            assert_eq!(
                skill_state_runtime::run(
                    &mut env,
                    SkillStateAction::Complete {
                        spec: 3327,
                        pr: None
                    },
                    SKILL_NAME,
                    SKILL_DISPLAY,
                    VERB,
                    &mut output,
                )
                .unwrap(),
                0
            );
        });
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
        assert_eq!(
            state["start_evidence"]["reason"],
            "host_probe_unavailable_404"
        );
        assert_eq!(state["start_evidence"]["work_id"], fixture.work_id);
        assert_eq!(std::fs::read(works_path).unwrap(), before);
        server.receive();
    }

    #[test]
    fn managed_build_start_old_host_never_bypasses_invalid_work_or_other_http_errors() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        for (status, invalid) in [
            (StatusCode::NOT_FOUND, "missing"),
            (StatusCode::NOT_FOUND, "terminal"),
            (StatusCode::NOT_FOUND, "stale"),
            (StatusCode::NOT_FOUND, "docker"),
            (StatusCode::INTERNAL_SERVER_ERROR, ""),
            (StatusCode::UNAUTHORIZED, ""),
        ] {
            let fixture = BoundTerminalFixture::new();
            let state_path = gwt_core::skill_state::state_path(&fixture.git.repo, SKILL_NAME);
            std::fs::remove_file(&state_path).unwrap();
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
            match invalid {
                "missing" => std::fs::remove_file(&works_path).unwrap(),
                "terminal" => fixture.terminalize_canonical_done(),
                "stale" | "docker" => {
                    let mut session = fixture.session.clone();
                    if invalid == "docker" {
                        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
                    } else {
                        session
                            .execution_binding
                            .as_mut()
                            .unwrap()
                            .identity
                            .generation_id = "stale-generation".into();
                    }
                    session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
                }
                _ => {}
            }
            let before = std::fs::read(&works_path).ok();
            let server = TerminalBridgeServer::start(status, serde_json::Value::Null);
            with_terminal_bridge_env(&fixture, &server, |_| {
                let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
                let mut output = String::new();
                assert_ne!(
                    run(
                        &mut env,
                        SkillStateAction::Start { spec: 3327 },
                        &mut output
                    )
                    .unwrap(),
                    0,
                    "{status}: {output}"
                );
                assert!(
                    !state_path.exists(),
                    "{status}: no orphan build may be created"
                );
            });
            assert_eq!(std::fs::read(&works_path).ok(), before);
            server.receive();
        }
    }

    fn run_start_action(
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        run_start_action_with_execution_record(forward_url, forward_token, managed, None, 3403)
    }

    fn run_start_action_with_execution_record(
        forward_url: Option<&str>,
        forward_token: Option<&str>,
        managed: bool,
        execution_session_id: Option<&str>,
        requested_spec: u64,
    ) -> (
        i32,
        String,
        crate::cli::verification_record::tests::WorkEventGitFixture,
    ) {
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        if let Some(execution_session_id) = execution_session_id {
            crate::cli::execution_state::save(
                &fixture.repo,
                &crate::cli::execution_state::ExecutionControlRecord {
                    owner_kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                    owner_number: 3403,
                    primary_session_id: execution_session_id.to_string(),
                    entrypoint: "$gwt-execute".to_string(),
                    bundled_required_owners: Vec::new(),
                    status: crate::cli::execution_state::ExecutionControlStatus::Active,
                    blocked_reason: None,
                    missing_verification: None,
                    launched_at: chrono::Utc::now(),
                    settled_at: None,
                    completion_evidence: None,
                    permission_decision: None,
                    transfers: Vec::new(),
                    recoveries: Vec::new(),
                    content_hash: String::new(),
                },
            )
            .expect("save active execution record");
        }
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "start-bridge-session");
        let _forward_url = forward_url.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, value),
        );
        let _forward_token = forward_token.map_or_else(
            || ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            |value| ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, value),
        );
        let _runtime = if managed {
            ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
                fixture.repo.join("managed-runtime.json"),
            )
        } else {
            ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        };
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Start {
                spec: requested_spec,
            },
            &mut output,
        )
        .expect("run build start");
        (code, output, fixture)
    }

    #[test]
    fn managed_build_start_requires_materialized_work_before_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let rejected = TerminalBridgeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "workspace_ensure_required",
                "message": "untrusted Host diagnostic terminal-secret",
                "recovery_operations": ["workspace.ensure"]
            }),
        );
        let (code, output, fixture) =
            run_start_action(Some(&rejected.forward_url), Some("terminal-secret"), true);
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed build.start refusal");
        assert_eq!(refusal["ok"], false);
        assert_eq!(refusal["error_code"], "workspace_ensure_required");
        assert_eq!(refusal["recovery_operation"], "workspace.ensure");
        assert!(refusal["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()));
        assert!(
            !output.contains("terminal-secret"),
            "Host response must not reflect the bearer into diagnostics: {output}"
        );
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load rejected build state")
                .is_none(),
            "managed rejection must not create lifecycle state"
        );
        let (headers, request) = rejected.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer terminal-secret")
        );
        assert_eq!(request["claimed_session_id"], "start-bridge-session");
        assert_eq!(request["owner_number"], 3403);
        assert!(request.get("work_id").is_none());

        let accepted = TerminalBridgeServer::start(
            StatusCode::OK,
            serde_json::json!({
                "schema_version": 1,
                "owner_number": 3403,
                "work_id": "work-feature-build-start"
            }),
        );
        let (code, output, fixture) =
            run_start_action(Some(&accepted.forward_url), Some("terminal-secret"), true);
        assert_eq!(code, 0, "{output}");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load accepted build state")
                .expect("accepted build state")
                .active,
            "successful managed preflight may create lifecycle state"
        );
        accepted.receive();
    }

    #[test]
    fn managed_build_start_rejects_incomplete_bridge_without_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, url, token) in [
            (
                "url-only",
                Some("http://127.0.0.1:45123/internal/hook-live"),
                None,
            ),
            ("token-only", None, Some("terminal-secret")),
            ("managed-missing", None, None),
        ] {
            let (code, output, fixture) = run_start_action(url, token, true);
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(
                refusal["error_code"], "relaunch_required",
                "{label}: {output}"
            );
            assert_eq!(
                refusal["recovery_operation"], "execution.status",
                "{label}: {output}"
            );
            assert!(
                gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                    .expect("load rejected build state")
                    .is_none(),
                "{label}: missing managed capability must not create lifecycle state"
            );
        }
    }

    #[test]
    fn managed_build_start_rejects_invalid_probe_receipts_without_creating_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, body) in [
            (
                "invalid-schema",
                serde_json::json!({
                    "schema_version": crate::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION + 1,
                    "owner_number": 3403,
                    "work_id": "work-feature-build-start"
                }),
            ),
            (
                "empty-work-id",
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 3403,
                    "work_id": ""
                }),
            ),
            (
                "wrong-owner",
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 2359,
                    "work_id": "work-feature-build-start"
                }),
            ),
        ] {
            let server = TerminalBridgeServer::start(StatusCode::OK, body);
            let (code, output, fixture) =
                run_start_action(Some(&server.forward_url), Some("terminal-secret"), true);
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(refusal["error_code"], "host_outcome_unknown");
            assert!(
                gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                    .expect("load rejected build state")
                    .is_none(),
                "{label}: invalid receipt must not create lifecycle state"
            );
            server.receive();
        }
    }

    #[test]
    fn fresh_session_start_inherits_build_of_gone_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            serde_json::json!({"schema_version": 1, "owner_number": 4687, "work_id": "fresh-work"}),
        );
        // Issue #5054: the predecessor Session ended (no Session file, no
        // runtime); its successor's build.start inherits the lifecycle.
        let (code, output, fixture) = run_active_action_with_identity(
            SkillStateAction::Start { spec: 4687 },
            Some(&server.forward_url),
            Some("terminal-secret"),
            true,
            Some(4687),
            "missing-owner",
            Some("fresh-session"),
        );
        assert_eq!(code, 0, "{output}");
        assert!(!output.contains("build_state_session_mismatch"), "{output}");
        let inherited = gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
            .unwrap()
            .unwrap();
        assert!(inherited.active);
        assert_eq!(inherited.owner_spec, Some(4687));
        assert_eq!(inherited.session_id, "fresh-session");
        server.receive();
    }

    fn save_live_runtime(fixture: &BoundTerminalFixture) {
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&fixture.session)
            .unwrap()
            .unwrap();
        let started = crate::process::host_process_start_time(std::process::id()).unwrap();
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            1,
            started,
            std::process::id(),
            started,
        )
        .save(&gwt_agent::runtime_state_path(
            &sessions,
            &fixture.session.id,
        ))
        .unwrap();
    }

    fn launch_build_successor(fixture: &BoundTerminalFixture, blocked: bool) -> String {
        use crate::cli::execution_state::*;
        let owner = ExecutionOwnerKey {
            kind: ExecutionOwnerKind::Issue,
            number: fixture.binding.owner_number,
        };
        let request = SuccessorRequest {
            operation_id: "build-successor".into(),
            principal_id: "host".into(),
            work_id: None,
            source: if blocked {
                FRESH_LINKED_OWNER_LAUNCH_SOURCE
            } else {
                CONCURRENT_LINKED_OWNER_LAUNCH_SOURCE
            }
            .into(),
            session_binding_id: "build-successor-binding".into(),
            initial_session_id: "build-successor-session".into(),
            entrypoint: "gwt-execute".into(),
            requested_at: chrono::Utc::now(),
        };
        if blocked {
            prepare_fresh_linked_owner_launch_successor(&fixture.git.repo, owner, &request)
                .unwrap();
        } else {
            prepare_concurrent_linked_owner_launch_successor(&fixture.git.repo, owner, &request)
                .unwrap();
        }
        activate_successor(&fixture.git.repo, owner, &request).unwrap();
        let mut successor = gwt_agent::Session::new(
            &fixture.git.repo,
            &fixture.session.branch,
            fixture.session.agent_id.clone(),
        );
        successor.id = request.initial_session_id;
        successor.project_state_root = fixture.session.project_state_root.clone();
        successor.linked_issue_number = fixture.session.linked_issue_number;
        successor
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                session_id: successor.id.clone(),
                identity: current_execution_binding(&fixture.git.repo, owner)
                    .unwrap()
                    .unwrap(),
                ..fixture.binding.clone()
            }))
            .unwrap();
        successor
            .save(&gwt_core::paths::gwt_sessions_dir())
            .unwrap();
        successor.id
    }

    #[test]
    fn build_start_and_phase_inherit_lifecycle_of_blocked_predecessor_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        // The predecessor settled Blocked, but its process is still alive.
        let predecessor = BoundTerminalFixture::new();
        predecessor.settle_execution_blocked();
        save_live_runtime(&predecessor);
        let predecessor_state = gwt_core::skill_state::load(&predecessor.git.repo, SKILL_NAME)
            .unwrap()
            .unwrap();
        let successor_session = launch_build_successor(&predecessor, true);
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            serde_json::json!({"schema_version": 1, "owner_number": 3327, "work_id": "successor-work"}),
        );
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &successor_session);
        let _url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &server.forward_url);
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let _runtime = ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            predecessor.git.repo.join("managed-runtime.json"),
        );
        let mut env = crate::cli::TestEnv::new(predecessor.git.repo.clone());

        let mut output = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Start { spec: 3327 },
            &mut output,
        )
        .unwrap();
        assert_eq!(code, 0, "{output}");
        assert!(!output.contains("build_state_session_mismatch"), "{output}");
        server.receive();
        let started = gwt_core::skill_state::load(&predecessor.git.repo, SKILL_NAME)
            .unwrap()
            .unwrap();
        assert!(started.active);
        assert_eq!(started.session_id, successor_session);

        // build.phase reaches the same verdict on the same lifecycle.
        gwt_core::skill_state::save(&predecessor.git.repo, SKILL_NAME, &predecessor_state).unwrap();
        let mut output = String::new();
        let code = run(
            &mut env,
            SkillStateAction::Phase {
                spec: 3327,
                label: "verify".to_string(),
            },
            &mut output,
        )
        .unwrap();
        assert_eq!(code, 0, "{output}");
        let phased = gwt_core::skill_state::load(&predecessor.git.repo, SKILL_NAME)
            .unwrap()
            .unwrap();
        assert_eq!(phased.session_id, successor_session);
        assert_eq!(phased.phase.as_deref(), Some("verify"));
    }

    #[test]
    fn build_start_and_phase_refuse_live_holder_with_liveness_evidence() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for action in ["start", "phase"] {
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let fixture = BoundTerminalFixture::new();
            let holder = fixture.session.id.clone();
            save_live_runtime(&fixture);
            let before = gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .unwrap()
                .unwrap();
            // A concurrent successor does not terminalize the live Active holder.
            let successor_session = launch_build_successor(&fixture, false);
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &successor_session);
            let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            let mut output = String::new();
            let request = if action == "start" {
                SkillStateAction::Start { spec: 3327 }
            } else {
                SkillStateAction::Phase {
                    spec: 3327,
                    label: "verify".to_string(),
                }
            };
            let code = run(&mut env, request, &mut output).unwrap();
            assert_eq!(code, 2, "{action}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{action}: typed refusal: {error}: {output}"));
            assert_eq!(refusal["error_code"], "build_state_session_mismatch");
            assert_eq!(refusal["holder"]["session_id"], holder.as_str());
            assert_eq!(refusal["holder"]["runtime"], "live");
            let sidecar = &refusal["holder"]["runtime_sidecars"][0];
            assert_eq!(sidecar["host_pid"], std::process::id());
            assert_eq!(sidecar["child_pid"], std::process::id());
            assert!(sidecar["heartbeat_at"].is_string(), "{output}");
            assert_eq!(
                gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                    .unwrap()
                    .unwrap(),
                before,
                "{action}: lifecycle must be preserved"
            );
        }
    }

    #[test]
    fn abandoned_build_abort_checks_runtime_and_preserves_execution_and_work() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for runtime in [
            "missing-session",
            "absent",
            "live",
            "unknown",
            "unbound-absent",
            "unbound-live",
            "unbound-unknown",
        ] {
            let home = tempfile::tempdir().unwrap();
            let _home = ScopedEnvVar::set("HOME", home.path());
            let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
            let mut fixture = BoundTerminalFixture::new();
            let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "fresh-session");
            let sessions = gwt_core::paths::gwt_sessions_dir();
            let path = gwt_agent::runtime_state_path(&sessions, &fixture.session.id);
            if runtime == "missing-session" {
                std::fs::remove_file(sessions.join(format!("{}.toml", fixture.session.id)))
                    .unwrap();
            } else if !runtime.ends_with("absent") {
                let identity = gwt_agent::SessionExecutionIdentity::from_session(&fixture.session)
                    .unwrap()
                    .unwrap();
                let started = crate::process::host_process_start_time(std::process::id()).unwrap();
                let mut state = if runtime.ends_with("live") {
                    gwt_agent::SessionRuntimeState::for_execution_process(
                        gwt_agent::AgentStatus::Running,
                        &identity,
                        1,
                        started,
                        std::process::id(),
                        started,
                    )
                } else {
                    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
                };
                if runtime.starts_with("unbound") {
                    state.execution_identity = None;
                }
                state.save(&path).unwrap();
            }
            if runtime.starts_with("unbound") {
                fixture.session.set_execution_binding(None).unwrap();
                fixture.session.repo_hash = None;
                fixture.session.worktree_path = fixture.git.repo.join("legacy-worktree");
                std::fs::create_dir_all(&fixture.session.worktree_path).unwrap();
                fixture.session.save(&sessions).unwrap();
                if runtime == "unbound-live" {
                    // Another Session's stale attribution must not deduplicate
                    // away this exact owner's live process evidence.
                    let mut other = fixture.session.clone();
                    other.id = "aaa-other-session".to_string();
                    other.save(&sessions).unwrap();
                    std::fs::copy(&path, gwt_agent::runtime_state_path(&sessions, &other.id))
                        .unwrap();
                }
            }
            let execution = crate::cli::execution_state::load(&fixture.git.repo).unwrap();
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
            let works = std::fs::read(&works_path).unwrap();
            let before = gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .unwrap()
                .unwrap();
            let mut env = crate::cli::TestEnv::new(fixture.git.repo.clone());
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover old build".to_string()),
                },
                &mut output,
            )
            .unwrap();
            let after = gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .unwrap()
                .unwrap();
            if runtime == "missing-session" || runtime.ends_with("absent") {
                assert_eq!(code, 0, "{runtime}: {output}");
                assert!(!after.active);
                assert_eq!(after.session_id, before.session_id);
            } else {
                assert_ne!(code, 0, "{runtime}: {output}");
                assert_eq!(after, before);
            }
            assert_eq!(
                crate::cli::execution_state::load(&fixture.git.repo).unwrap(),
                execution
            );
            assert_eq!(std::fs::read(&works_path).unwrap(), works);
        }
    }

    #[test]
    fn managed_build_start_never_overwrites_active_foreign_state() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        for (label, owner_spec, state_session_id, current_session_id, error_code) in [
            (
                "foreign-owner",
                Some(3327),
                "terminal-bridge-session",
                Some("terminal-bridge-session"),
                "build_state_owner_mismatch",
            ),
            (
                "gone-foreign-session-and-owner",
                Some(3327),
                "other-session",
                Some("terminal-bridge-session"),
                "build_state_owner_mismatch",
            ),
        ] {
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                serde_json::json!({
                    "schema_version": 1,
                    "owner_number": 3403,
                    "work_id": "work-feature-build-start"
                }),
            );
            let (code, output, fixture) = run_active_action_with_identity(
                SkillStateAction::Start { spec: 3403 },
                Some(&server.forward_url),
                Some("terminal-secret"),
                true,
                owner_spec,
                state_session_id,
                current_session_id,
            );
            assert_eq!(code, 2, "{label}: {output}");
            let refusal: serde_json::Value = serde_json::from_str(output.trim())
                .unwrap_or_else(|error| panic!("{label}: typed refusal: {error}: {output}"));
            assert_eq!(refusal["error_code"], error_code, "{label}: {output}");
            let state = gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load preserved build state")
                .expect("preserved build state");
            assert!(state.active, "{label}: active state must remain active");
            assert_eq!(
                state.owner_spec, owner_spec,
                "{label}: owner must be preserved"
            );
            assert_eq!(
                state.session_id, state_session_id,
                "{label}: Session must be preserved"
            );
            server.assert_no_request();
        }
    }

    #[test]
    fn active_execution_record_keeps_missing_bridge_in_managed_preflight() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("start-bridge-session"),
            3403,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed degraded-execution refusal");
        assert_eq!(refusal["error_code"], "relaunch_required");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load rejected build state")
                .is_none(),
            "a degraded managed execution must not become standalone"
        );

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("start-bridge-session"),
            2359,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed owner mismatch refusal");
        assert_eq!(refusal["error_code"], "execution_owner_mismatch");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load mismatched build state")
                .is_none(),
            "the current Execution owner must constrain build.start"
        );

        let (code, output, fixture) = run_start_action_with_execution_record(
            None,
            None,
            false,
            Some("execution-owner-session"),
            3403,
        );
        assert_eq!(code, 2, "{output}");
        let refusal: serde_json::Value =
            serde_json::from_str(output.trim()).expect("typed Session mismatch refusal");
        assert_eq!(refusal["error_code"], "execution_session_mismatch");
        assert!(
            gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
                .expect("load foreign Session build state")
                .is_none(),
            "a foreign Session must not downgrade an existing Execution to standalone"
        );
        let _caller = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "start-bridge-session");
        for holder in ["gone-predecessor", "start-bridge-session"] {
            let state = gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(3403),
                started_at: chrono::Utc::now(),
                phase: Some("green".into()),
                session_id: holder.into(),
            };
            gwt_core::skill_state::save(&fixture.repo, SKILL_NAME, &state).unwrap();
            for action in [
                SkillStateAction::Start { spec: 3403 },
                SkillStateAction::Phase {
                    spec: 3403,
                    label: "verify".into(),
                },
            ] {
                let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
                let mut output = String::new();
                let lease_observed = std::rc::Rc::new(std::cell::Cell::new(false));
                let observed = lease_observed.clone();
                let repo = fixture.repo.clone();
                crate::cli::trusted_store::set_write_lease_acquired_hook(move || {
                    let error = crate::cli::trusted_store::with_write_lease_wait(
                        &repo,
                        std::time::Duration::ZERO,
                        || Ok(()),
                    )
                    .expect_err("a concurrent canonical owner writer must be refused");
                    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                    observed.set(true);
                });
                assert_eq!(run(&mut env, action, &mut output).unwrap(), 2, "{output}");
                assert!(output.contains("execution_session_mismatch"), "{output}");
                assert!(
                    lease_observed.get(),
                    "start/phase must hold the owner lease before judging ownership"
                );
                assert_eq!(
                    gwt_core::skill_state::load(&fixture.repo, SKILL_NAME).unwrap(),
                    Some(state.clone()),
                );
            }
        }
    }

    #[test]
    fn managed_build_abort_recovers_genuine_missing_work_before_any_request() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = BoundTerminalFixture::new();
        let works_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &works_path,
            &gwt_core::workspace_projection::WorkItemsProjection::empty(chrono::Utc::now()),
        )
        .expect("remove materialized Work while preserving exact Session assignment");
        let before = std::fs::read(&works_path).expect("snapshot WorkItems");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AssignedWorkMissing),
        );
        let repo = fixture.git.repo.clone();
        let (code, output) = with_terminal_bridge_env(&fixture, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3327,
                    reason: Some("recover absent canonical Work".to_string()),
                },
                &mut output,
            )
            .expect("run orphan abort");
            (code, output)
        });
        assert_eq!(code, 0, "{output}");
        assert!(output.contains("orphan_recovered"), "{output}");
        assert!(
            !gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .expect("read recovered lifecycle")
                .expect("lifecycle")
                .active
        );
        assert_eq!(
            std::fs::read(&works_path).expect("read unchanged WorkItems"),
            before
        );
        server.assert_no_request();
    }

    #[test]
    fn managed_build_start_rejects_unreadable_execution_before_any_probe() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let record_path = crate::cli::execution_state::state_path(&fixture.repo);
        std::fs::create_dir_all(record_path.parent().expect("record parent"))
            .expect("record directory");
        std::fs::write(&record_path, "invalid execution JSON").expect("seed corrupt execution");
        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            serde_json::json!({
                "schema_version": 1, "owner_number": 3403, "work_id": "work-valid-probe",
            }),
        );
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "start-bridge-session");
        let _url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &server.forward_url);
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "terminal-secret");
        let mut env = crate::cli::TestEnv::new(fixture.repo.clone());
        let mut output = String::new();
        assert_eq!(
            run(
                &mut env,
                SkillStateAction::Start { spec: 3403 },
                &mut output
            )
            .expect("reject invalid execution"),
            2,
            "{output}"
        );
        assert!(output.contains("execution_state_invalid"), "{output}");
        assert!(gwt_core::skill_state::load(&fixture.repo, SKILL_NAME)
            .expect("load build")
            .is_none());
        assert_eq!(
            std::fs::read_to_string(&record_path).expect("preserved execution"),
            "invalid execution JSON"
        );
        server.assert_no_request();
    }
    #[test]
    fn unbound_session_abort_preserves_build_without_exact_owner() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let git = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let branch =
            gwt_core::process::run_git_logged(&["branch", "--show-current"], Some(&git.repo))
                .expect("read fixture branch");
        let branch = String::from_utf8(branch.stdout)
            .expect("utf8")
            .trim()
            .to_string();
        let mut session = gwt_agent::Session::new(&git.repo, &branch, gwt_agent::AgentId::Codex);
        session.id = "unbound-successor-session".to_string();
        session.project_state_root = Some(git.repo.clone());
        session.linked_issue_number = Some(3431);
        assert!(
            session.execution_binding.is_none(),
            "precondition: this session was never bound by a launch"
        );
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save unbound session");
        gwt_core::skill_state::save(
            &git.repo,
            SKILL_NAME,
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: None,
                started_at: chrono::Utc::now(),
                phase: None,
                session_id: session.id.clone(),
            },
        )
        .expect("open the build state as this session");

        let server = TerminalBridgeServer::start(
            StatusCode::OK,
            terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
        );
        let repo = git.repo.clone();
        let (code, output) = with_terminal_bridge_env_for(&git.repo, &session.id, &server, |_| {
            let mut env = crate::cli::TestEnv::new(repo);
            let mut output = String::new();
            let code = run(
                &mut env,
                SkillStateAction::Abort {
                    spec: 3431,
                    reason: Some("unowned lifecycle must remain active".to_string()),
                },
                &mut output,
            )
            .expect("reject unknown owner abort");
            (code, output)
        });
        assert_ne!(code, 0, "{output}");
        assert!(
            gwt_core::skill_state::load(&git.repo, SKILL_NAME)
                .expect("load preserved lifecycle")
                .expect("lifecycle")
                .active
        );
        server.assert_no_request();
    }

    #[test]
    fn managed_build_terminalization_rejects_lifecycle_owner_outside_current_binding() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("trusted store home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        for action in [
            SkillStateAction::Abort {
                spec: 3403,
                reason: Some("stale lifecycle owner".to_string()),
            },
            SkillStateAction::Complete {
                spec: 3403,
                pr: None,
            },
        ] {
            let fixture = BoundTerminalFixture::new();
            let mut state = gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                .expect("load lifecycle")
                .expect("lifecycle");
            state.owner_spec = Some(3403);
            gwt_core::skill_state::save(&fixture.git.repo, SKILL_NAME, &state)
                .expect("seed stale lifecycle owner");
            let works_path =
                gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.git.repo);
            let before = std::fs::read(&works_path).expect("snapshot canonical Work");
            let server = TerminalBridgeServer::start(
                StatusCode::OK,
                terminal_receipt(crate::AgentWorkTerminalizationOutcome::AlreadyMatching),
            );
            let result = with_terminal_bridge_env(&fixture, &server, |env| {
                record_current_work_terminal_before_finalize(env, &action)
            });
            assert!(
                result.is_err(),
                "current binding owns 3327, lifecycle 3403 must not close it: {result:?}"
            );
            assert_eq!(std::fs::read(&works_path).expect("preserved Work"), before);
            assert!(
                gwt_core::skill_state::load(&fixture.git.repo, SKILL_NAME)
                    .expect("preserved lifecycle")
                    .expect("lifecycle")
                    .active
            );
            server.assert_no_request();
        }
    }

    #[test]
    fn managed_build_start_recovery_operations_are_registered() {
        for code in [
            crate::AgentWorkspaceUpdateErrorCode::InvalidRequest,
            crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired,
            crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch,
            crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired,
            crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch,
            crate::AgentWorkspaceUpdateErrorCode::IdentityConflict,
            crate::AgentWorkspaceUpdateErrorCode::TransactionConflict,
            crate::AgentWorkspaceUpdateErrorCode::Internal,
        ] {
            let operation = recovery_operation(code);
            assert!(
                crate::cli::operation_catalog::canonical_names().any(|name| name == operation),
                "build.start must recommend an available JSON operation: {code:?} -> {operation}"
            );
        }
    }
}
