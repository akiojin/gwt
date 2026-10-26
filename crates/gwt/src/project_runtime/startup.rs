//! Fixed-project durable launch recovery, run by the physical owner before admission.

use super::continuation::{
    invoke_fresh_execution_pre_work_commit_hook, invoke_missing_session_cleanup_hook,
    resolve_activated_fresh_execution_commit,
};
use super::launch_finalization::{
    compensate_genesis_workspace_projection, durable_fresh_execution_candidate,
    genesis_initial_execution_binding_matches, path_matches, reject_continue_work_workspace_commit,
    resolve_split_workspace_state_external_commit, DurableFreshExecutionCandidate,
    GenesisCompensationAuthority,
};
use crate::session_launch::recovery::{
    clear_durable_launch_recovery, durable_launch_recovery_records, DurableLaunchRecoveryKind,
    DurableLaunchRecoveryRecord,
};
use std::path::Path;

/// Reconcile only this owner's existing receipts and return removed Session IDs
/// so read-only caches can discard them without duplicating recovery writes.
pub fn reconcile_durable_launches(project_root: &Path, sessions_dir: &Path) -> Vec<String> {
    let mut recovery = StartupLaunchRecovery {
        sessions_dir,
        project_key: gwt_core::paths::project_scope_hash(project_root).to_string(),
        cleaned_sessions: Vec::new(),
    };
    recovery.reconcile_durable_fresh_execution_launches();
    normalize_project_sessions(project_root, sessions_dir);
    recovery.cleaned_sessions
}

struct StartupLaunchRecovery<'a> {
    sessions_dir: &'a Path,
    project_key: String,
    cleaned_sessions: Vec<String>,
}

fn split_workspace_state_external_commit_resolution(
    project_root: &Path,
    work_event_root: &Path,
    operation_id: &str,
) -> gwt_core::error::Result<gwt_core::workspace_projection::ExternalWorkspaceCommitResolution> {
    let current_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(project_root);
    let canonical_work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(project_root);
    let legacy_work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let canonical = gwt_core::workspace_projection::workspace_state_external_commit_resolution_at(
        &current_path,
        &canonical_work_items_path,
        operation_id,
    )?;
    if canonical != gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Missing
        || legacy_work_items_path == canonical_work_items_path
    {
        return Ok(canonical);
    }
    gwt_core::workspace_projection::workspace_state_external_commit_resolution_at(
        &current_path,
        &legacy_work_items_path,
        operation_id,
    )
}

impl StartupLaunchRecovery<'_> {
    fn reconcile_durable_fresh_execution_launches(&mut self) {
        for (_receipt_path, receipt) in durable_launch_recovery_records(self.sessions_dir) {
            if receipt.repo_hash != self.project_key
                || gwt_core::paths::project_scope_hash(&receipt.project_root).as_str()
                    != self.project_key
            {
                continue;
            }
            let owner = match receipt.owner() {
                Ok(owner) => owner,
                Err(error) => {
                    tracing::warn!(session_id = %receipt.session_id, error = %error, "retained invalid launch recovery receipt");
                    continue;
                }
            };
            let operation_id = match &receipt.kind {
                DurableLaunchRecoveryKind::Genesis => {
                    self.reconcile_durable_genesis_launch(&receipt, owner);
                    continue;
                }
                DurableLaunchRecoveryKind::FreshSuccessor { operation_id } => operation_id,
            };
            let path = self
                .sessions_dir
                .join(format!("{}.toml", receipt.session_id));
            let session = match gwt_agent::inspect_session_path(&path) {
                gwt_agent::SessionPathState::Present(session) => session,
                gwt_agent::SessionPathState::Missing => {
                    self.reconcile_fresh_launch_without_session(&receipt, owner, operation_id);
                    continue;
                }
                gwt_agent::SessionPathState::Error(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "retained unreadable pending launch Session");
                    continue;
                }
            };
            let candidate = match durable_fresh_execution_candidate(&session) {
                Ok(Some(candidate)) => candidate,
                Ok(None) => {
                    tracing::warn!(session_id = %session.id, "retained pending launch receipt without a durable candidate");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "retained ambiguous fresh-launch recovery evidence"
                    );
                    continue;
                }
            };
            if candidate.owner != owner
                || candidate.binding.session_id != receipt.session_id
                || receipt.expected_binding.as_ref() != Some(&candidate.binding)
                || receipt.expected_agent_id.as_ref() != Some(&candidate.agent_id)
                || receipt.expected_session_identity.as_ref() != Some(&candidate.session_identity)
                || candidate.attempt.request.operation_id != *operation_id
                || !path_matches(&candidate.worktree_path, &receipt.worktree_path)
                || !path_matches(&candidate.project_root, &receipt.project_root)
            {
                tracing::warn!(session_id = %receipt.session_id, "retained launch recovery receipt after exact identity mismatch");
                continue;
            }

            match candidate.attempt.status {
                crate::cli::execution_state::ContinuationAttemptStatus::Activated => {
                    let activated_matches = candidate
                        .attempt
                        .activated_generation
                        .as_ref()
                        .is_some_and(|identity| {
                            identity.generation_id == candidate.binding.identity.generation_id
                                && identity.session_binding_id
                                    == candidate.binding.identity.binding_id
                        });
                    if !activated_matches {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            "retained Activated fresh-launch evidence with a mismatched Session binding"
                        );
                        continue;
                    }
                    invoke_fresh_execution_pre_work_commit_hook();
                    let recovery_owner = crate::cli::execution_state::recovery_generation_owner(
                        &candidate.worktree_path,
                    )
                    .ok()
                    .flatten();
                    let superseding_authority_is_readable = match recovery_owner {
                        Some(current_owner) if current_owner != candidate.owner => {
                            crate::cli::execution_state::current_execution_binding(
                                &candidate.worktree_path,
                                current_owner,
                            )
                            .is_ok_and(|binding| binding.is_some())
                        }
                        Some(_) => crate::cli::execution_state::current_execution_binding(
                            &candidate.worktree_path,
                            candidate.owner,
                        )
                        .is_ok_and(|binding| {
                            binding.is_some_and(|binding| binding != candidate.binding.identity)
                        }),
                        None => false,
                    };
                    if superseding_authority_is_readable {
                        let terminal_resolution = split_workspace_state_external_commit_resolution(
                            &candidate.project_root,
                            &candidate.worktree_path,
                            &candidate.attempt.request.operation_id,
                        );
                        let resolution_is_terminal = terminal_resolution.is_ok_and(|resolution| {
                            matches!(
                                resolution,
                                gwt_core::workspace_projection::
                                    ExternalWorkspaceCommitResolution::Committed
                                    | gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Rejected
                            )
                        }) || resolve_split_workspace_state_external_commit(
                                &candidate.project_root,
                                &candidate.worktree_path,
                                &candidate.attempt.request.operation_id,
                                gwt_core::workspace_projection::
                                    ExternalWorkspaceCommitDecision::Reject,
                            )
                            .is_ok_and(|resolution| {
                                resolution
                                    == gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Rejected
                            });
                        if resolution_is_terminal {
                            if let Err(error) = clear_durable_launch_recovery(
                                self.sessions_dir,
                                &candidate.binding.session_id,
                            ) {
                                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "superseded launch recovery receipt cleanup remains pending");
                            }
                        } else {
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                "superseded fresh-launch Work rejection remains pending"
                            );
                        }
                        continue;
                    }
                    match resolve_activated_fresh_execution_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        candidate.owner,
                        &candidate.attempt.request.operation_id,
                        &candidate.attempt.request,
                        self.sessions_dir,
                        &candidate.session_identity,
                    ) {
                        Ok(true) => {
                            if let Err(error) = clear_durable_launch_recovery(
                                self.sessions_dir,
                                &candidate.binding.session_id,
                            ) {
                                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "settled launch recovery receipt cleanup remains pending");
                                continue;
                            }
                            tracing::info!(
                                session_id = %candidate.binding.session_id,
                                operation_id = %candidate.attempt.request.operation_id,
                                "recovered an Activated fresh launch after Host restart"
                            );
                        }
                        Ok(false) => {
                            let terminal_resolution =
                                split_workspace_state_external_commit_resolution(
                                    &candidate.project_root,
                                    &candidate.worktree_path,
                                    &candidate.attempt.request.operation_id,
                                );
                            if terminal_resolution.is_ok_and(|resolution| {
                                matches!(
                                    resolution,
                                    gwt_core::workspace_projection::
                                        ExternalWorkspaceCommitResolution::Committed
                                        | gwt_core::workspace_projection::
                                            ExternalWorkspaceCommitResolution::Rejected
                                )
                            }) {
                                if let Err(error) = clear_durable_launch_recovery(
                                    self.sessions_dir,
                                    &candidate.binding.session_id,
                                ) {
                                    tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "settled launch recovery receipt cleanup remains pending");
                                }
                                continue;
                            }
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                "Activated fresh-launch exact Session lease or Work readback remains pending"
                            );
                        }
                        Err(error) => {
                            tracing::warn!(
                                session_id = %candidate.binding.session_id,
                                error = %error,
                                "Activated fresh-launch Workspace commit could not be recovered"
                            );
                        }
                    }
                }
                crate::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                    let prepared_matches =
                        crate::cli::execution_state::prepared_execution_binding_matches(
                            &candidate.worktree_path,
                            candidate.owner,
                            &candidate.binding.session_id,
                            &candidate.binding.identity,
                        )
                        .unwrap_or(false);
                    if !prepared_matches {
                        tracing::warn!(
                            session_id = %candidate.binding.session_id,
                            "retained a Prepared fresh launch whose exact binding is unreadable"
                        );
                        continue;
                    }
                    if self.finish_durable_aborted_fresh_execution_cleanup(
                        &candidate,
                        receipt
                            .expected_binding
                            .as_ref()
                            .expect("validated fresh recovery binding"),
                    ) {
                        let _ = clear_durable_launch_recovery(
                            self.sessions_dir,
                            &candidate.binding.session_id,
                        );
                    }
                }
                crate::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                    if self.finish_durable_aborted_fresh_execution_cleanup(
                        &candidate,
                        receipt
                            .expected_binding
                            .as_ref()
                            .expect("validated fresh recovery binding"),
                    ) {
                        let _ = clear_durable_launch_recovery(
                            self.sessions_dir,
                            &candidate.binding.session_id,
                        );
                    }
                }
            }
        }
    }

    fn reconcile_fresh_launch_without_session(
        &mut self,
        receipt: &DurableLaunchRecoveryRecord,
        owner: crate::cli::execution_state::ExecutionOwnerKey,
        operation_id: &str,
    ) {
        let attempt = match crate::cli::execution_state::continuation_attempt_for_operation(
            &receipt.worktree_path,
            owner,
            operation_id,
        ) {
            Ok(attempt) => attempt,
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "pending launch attempt remains unreadable");
                return;
            }
        };
        let Some(attempt) = attempt else {
            if receipt.expected_binding.is_none() {
                let _ = clear_durable_launch_recovery(self.sessions_dir, &receipt.session_id);
            } else {
                tracing::warn!(session_id = %receipt.session_id, "retained bound fresh-launch receipt whose owner attempt is missing");
            }
            return;
        };
        if !crate::cli::execution_state::is_owner_launch_successor_attempt(&attempt)
            || attempt.request.initial_session_id != receipt.session_id
            || attempt.request.operation_id != operation_id
        {
            tracing::warn!(session_id = %receipt.session_id, "retained launch recovery receipt that does not identify an exact fresh successor");
            return;
        }
        if let Some(expected_binding) = receipt.expected_binding.as_ref() {
            let binding_envelope_matches = expected_binding.schema_version
                == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                && expected_binding.session_id == receipt.session_id
                && expected_binding.repo_hash == receipt.repo_hash
                && expected_binding.owner_kind == owner.kind.as_str()
                && expected_binding.owner_number == owner.number
                && expected_binding.capability_generation > 0;
            let exact_attempt_matches = binding_envelope_matches
                && crate::cli::execution_state::continuation_attempt_execution_binding_matches(
                    &receipt.worktree_path,
                    owner,
                    &attempt,
                    &receipt.session_id,
                    &expected_binding.identity,
                )
                .unwrap_or(false);
            if !exact_attempt_matches {
                tracing::warn!(session_id = %receipt.session_id, "retained missing-Session fresh-launch receipt after exact binding mismatch");
                return;
            }
        }
        invoke_missing_session_cleanup_hook(&receipt.session_id);
        let cleanup = match attempt.status {
            crate::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                if let Some(expected_session_identity) = receipt.expected_session_identity.as_ref()
                {
                    crate::cli::execution_state::abort_abandoned_successor_and_remove_exact_session(
                        &receipt.worktree_path,
                        owner,
                        &attempt.request,
                        "Host restarted before fresh launch Session persistence",
                        self.sessions_dir,
                        expected_session_identity,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                } else {
                    crate::cli::execution_state::abort_successor_if_session_missing_with(
                        &receipt.worktree_path,
                        owner,
                        &attempt.request,
                        "Host restarted before fresh launch Session persistence",
                        self.sessions_dir,
                        &receipt.session_id,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                }
            }
            crate::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                if let Some(expected_session_identity) = receipt.expected_session_identity.as_ref()
                {
                    crate::cli::execution_state::remove_exact_session_with_owner_lease(
                        &receipt.worktree_path,
                        owner,
                        self.sessions_dir,
                        expected_session_identity,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                } else {
                    crate::cli::execution_state::commit_if_session_missing_with_owner_lease(
                        &receipt.worktree_path,
                        owner,
                        self.sessions_dir,
                        &receipt.session_id,
                        || {
                            reject_continue_work_workspace_commit(
                                &receipt.project_root,
                                &receipt.worktree_path,
                                operation_id,
                            )
                        },
                    )
                }
            }
            crate::cli::execution_state::ContinuationAttemptStatus::Activated => {
                tracing::warn!(session_id = %receipt.session_id, "retained Activated launch receipt whose exact Session is missing");
                return;
            }
        };
        match cleanup {
            Ok(true) => {
                let _ = clear_durable_launch_recovery(self.sessions_dir, &receipt.session_id);
            }
            Ok(false) => {
                tracing::warn!(session_id = %receipt.session_id, "retained missing-Session fresh-launch recovery after same-id Session materialization");
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "missing-Session fresh-launch atomic cleanup remains pending");
            }
        }
    }

    fn reconcile_durable_genesis_launch(
        &mut self,
        receipt: &DurableLaunchRecoveryRecord,
        owner: crate::cli::execution_state::ExecutionOwnerKey,
    ) {
        let ledger = match crate::cli::execution_state::load_owner_generation_ledger(
            &receipt.worktree_path,
            owner,
        ) {
            Ok(Some(ledger)) => ledger,
            Ok(None) => {
                if receipt.expected_binding.is_some() {
                    tracing::warn!(session_id = %receipt.session_id, "retained bound genesis receipt whose owner ledger is missing");
                    return;
                }
                let flat = match crate::cli::execution_state::load(&receipt.worktree_path) {
                    Ok(flat) => flat,
                    Err(_) => return,
                };
                let Some(flat) = flat else {
                    if receipt.expected_binding.is_none() {
                        let _ =
                            clear_durable_launch_recovery(self.sessions_dir, &receipt.session_id);
                    } else {
                        tracing::warn!(session_id = %receipt.session_id, "retained bound genesis receipt whose execution authority is missing");
                    }
                    return;
                };
                if flat.owner_kind != owner.kind
                    || flat.owner_number != owner.number
                    || flat.primary_session_id != receipt.session_id
                    || flat.status != crate::cli::execution_state::ExecutionControlStatus::Active
                {
                    return;
                }
                if crate::cli::execution_state::ensure_generation_ledger(
                    &receipt.worktree_path,
                    owner,
                    crate::cli::execution_state::LegacyActiveDisposition::Live,
                )
                .is_err()
                {
                    return;
                }
                match crate::cli::execution_state::load_owner_generation_ledger(
                    &receipt.worktree_path,
                    owner,
                ) {
                    Ok(Some(ledger)) => ledger,
                    _ => return,
                }
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "genesis launch recovery authority remains unreadable");
                return;
            }
        };
        let expected_binding = receipt.expected_binding.as_ref();
        let target = ledger.generations.iter().find(|generation| {
            generation.identity.predecessor_generation_id.is_none()
                && generation.identity.initial_session_id == receipt.session_id
                && expected_binding.is_none_or(|binding| {
                    generation.identity.generation_id == binding.identity.generation_id
                        && generation.identity.session_binding_id == binding.identity.binding_id
                })
        });
        let Some(target) = target.cloned() else {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after generation identity mismatch");
            return;
        };
        if expected_binding.is_some_and(|binding| {
            !genesis_initial_execution_binding_matches(
                &receipt.worktree_path,
                owner,
                &receipt.session_id,
                binding,
            )
        }) {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after full initial binding mismatch");
            return;
        }
        let binding = if let Some(binding) = expected_binding {
            binding.identity.clone()
        } else {
            if ledger.current_generation_id != target.identity.generation_id {
                return;
            }
            match crate::cli::execution_state::current_owner_execution_binding(
                &receipt.worktree_path,
                owner,
            ) {
                Ok(Some(binding)) => binding,
                _ => return,
            }
        };
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", receipt.session_id));
        let persisted_session = match std::fs::symlink_metadata(&session_path) {
            Ok(_) => match gwt_agent::Session::load(&session_path) {
                Ok(session) => Some(session),
                Err(_) => return,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return,
        };
        let persisted_binding = persisted_session
            .as_ref()
            .and_then(|session| session.execution_binding.clone());
        if persisted_session.is_some()
            && (expected_binding.is_none() || persisted_binding.as_ref() != expected_binding)
        {
            tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after full Session binding mismatch");
            return;
        }
        if let Some(session) = persisted_session.as_ref() {
            let actual_identity = match gwt_agent::SessionExecutionIdentity::from_session(session) {
                Ok(Some(identity)) => identity,
                _ => return,
            };
            if receipt.expected_session_identity.as_ref() != Some(&actual_identity) {
                tracing::warn!(session_id = %receipt.session_id, "retained genesis receipt after exact Session anchor mismatch");
                return;
            }
        }
        let superseded_by_different_owner = matches!(
            crate::cli::execution_state::recovery_generation_owner(&receipt.worktree_path),
            Ok(Some(current_owner)) if current_owner != owner
        );
        let foreign_owner_write_incomplete = !superseded_by_different_owner
            && matches!(
                crate::cli::execution_state::recovery_projection_owner_hint(&receipt.worktree_path),
                Ok(Some(projection_owner)) if projection_owner != owner
            );
        if foreign_owner_write_incomplete {
            tracing::warn!(
                "genesis launch recovery retained behind an incomplete foreign-owner authority write"
            );
            return;
        }
        let terminal_event = ledger
            .lifecycle_events
            .iter()
            .filter(|event| event.generation_id == target.identity.generation_id)
            .max_by_key(|event| event.sequence);
        let target_status = terminal_event.map_or(target.status, |event| event.to_status);
        let terminal_reason = match target_status {
            crate::cli::execution_state::ExecutionControlStatus::Active => {
                if ledger.current_generation_id != target.identity.generation_id {
                    return;
                }
                if expected_binding
                    .is_some_and(|session_binding| session_binding.identity != binding)
                {
                    return;
                }
                (!superseded_by_different_owner)
                    .then(|| "Host restarted before genesis launch readiness".to_string())
            }
            crate::cli::execution_state::ExecutionControlStatus::Blocked => {
                let terminalization_operation_id =
                    crate::cli::execution_state::genesis_terminalization_operation_id(
                        &target.identity.generation_id,
                        &target.identity.session_binding_id,
                    );
                let terminal_reason = terminal_event.and_then(|event| {
                    (event.from_status
                        == crate::cli::execution_state::ExecutionControlStatus::Active
                        && event.to_status
                            == crate::cli::execution_state::ExecutionControlStatus::Blocked
                        && event.session_id == receipt.session_id
                        && event.operation_id.as_deref()
                            == Some(terminalization_operation_id.as_str()))
                    .then(|| event.reason.clone())
                });
                let Some(terminal_reason) = terminal_reason else {
                    return;
                };
                (!superseded_by_different_owner
                    && ledger.current_generation_id == target.identity.generation_id)
                    .then_some(terminal_reason)
            }
            _ => return,
        };
        if persisted_session.is_some() {
            let (Some(session_binding), Some(_expected_session_identity)) =
                (expected_binding, receipt.expected_session_identity.as_ref())
            else {
                return;
            };
            if session_binding.identity.generation_id != target.identity.generation_id
                || session_binding.identity.binding_id != target.identity.session_binding_id
            {
                return;
            }
        }
        let compensation_authority = if superseded_by_different_owner {
            GenesisCompensationAuthority::SupersededByDifferentOwner
        } else {
            GenesisCompensationAuthority::Terminalized
        };
        let persisted_branch = persisted_session
            .as_ref()
            .map(|session| session.branch.clone());
        let compensate = || -> std::io::Result<()> {
            compensate_genesis_workspace_projection(
                &receipt.project_root,
                &receipt.worktree_path,
                owner,
                &receipt.session_id,
                receipt.expected_binding.as_ref(),
                persisted_branch.as_deref(),
                compensation_authority,
            )
            .map_err(std::io::Error::other)
        };
        if persisted_session.is_none() {
            invoke_missing_session_cleanup_hook(&receipt.session_id);
        }
        let cleanup = if expected_binding.is_some() {
            let Some(expected_session_identity) = receipt.expected_session_identity.as_ref() else {
                return;
            };
            if let Some(reason) = terminal_reason.as_deref() {
                crate::cli::execution_state::block_genesis_and_remove_exact_session(
                    &receipt.worktree_path,
                    owner,
                    &receipt.session_id,
                    &binding,
                    reason,
                    self.sessions_dir,
                    expected_session_identity,
                    compensate,
                )
            } else {
                crate::cli::execution_state::remove_exact_session_with_owner_lease(
                    &receipt.worktree_path,
                    owner,
                    self.sessions_dir,
                    expected_session_identity,
                    compensate,
                )
            }
        } else {
            if let Some(reason) = terminal_reason.as_deref() {
                crate::cli::execution_state::block_genesis_if_session_missing_with(
                    &receipt.worktree_path,
                    owner,
                    &receipt.session_id,
                    &binding,
                    reason,
                    self.sessions_dir,
                    compensate,
                )
            } else {
                crate::cli::execution_state::commit_if_session_missing_with_owner_lease(
                    &receipt.worktree_path,
                    owner,
                    self.sessions_dir,
                    &receipt.session_id,
                    compensate,
                )
            }
        };
        match cleanup {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(session_id = %receipt.session_id, "retained genesis recovery after same-id Session materialization");
                return;
            }
            Err(error) => {
                tracing::warn!(session_id = %receipt.session_id, error = %error, "genesis atomic cleanup remains pending");
                return;
            }
        }
        self.cleaned_sessions.push(receipt.session_id.clone());
        let _ = clear_durable_launch_recovery(self.sessions_dir, &receipt.session_id);
    }

    fn finish_durable_aborted_fresh_execution_cleanup(
        &mut self,
        candidate: &DurableFreshExecutionCandidate,
        _expected_binding: &gwt_agent::SessionExecutionBinding,
    ) -> bool {
        let cleanup = if candidate.attempt.status
            == crate::cli::execution_state::ContinuationAttemptStatus::Prepared
        {
            crate::cli::execution_state::abort_abandoned_successor_and_remove_exact_session(
                &candidate.worktree_path,
                candidate.owner,
                &candidate.attempt.request,
                "Host restarted before fresh launch activation",
                self.sessions_dir,
                &candidate.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        &candidate.attempt.request.operation_id,
                    )
                },
            )
        } else {
            crate::cli::execution_state::remove_exact_session_with_owner_lease(
                &candidate.worktree_path,
                candidate.owner,
                self.sessions_dir,
                &candidate.session_identity,
                || {
                    reject_continue_work_workspace_commit(
                        &candidate.project_root,
                        &candidate.worktree_path,
                        &candidate.attempt.request.operation_id,
                    )
                },
            )
        };
        match cleanup {
            Ok(true) => {
                self.cleaned_sessions
                    .push(candidate.binding.session_id.clone());
                tracing::info!(session_id = %candidate.binding.session_id, "completed durable Aborted fresh-launch Session cleanup");
                true
            }
            Ok(false) => {
                tracing::warn!(session_id = %candidate.binding.session_id, "retained fresh-launch cleanup evidence while the exact Session is live or its binding changed");
                false
            }
            Err(error) => {
                tracing::warn!(session_id = %candidate.binding.session_id, error = %error, "Aborted fresh-launch Session cleanup remains retryable");
                false
            }
        }
    }
}

fn normalize_project_sessions(project_root: &Path, sessions_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|id| id.to_str()) else {
            continue;
        };
        let Some(session) = gwt_agent::session_ledger::load_session_candidate(sessions_dir, id)
        else {
            continue;
        };
        if !super::continuation::session_matches_project_state(&session, project_root) {
            continue;
        }
        let needs_interruption = session.status != gwt_agent::AgentStatus::Interrupted
            && session.should_mark_interrupted_from_lifecycle()
            && session.worktree_path.exists();
        if session.schema_version >= gwt_agent::Session::CURRENT_SCHEMA_VERSION
            && !needs_interruption
        {
            continue;
        }
        if let Err(error) = gwt_agent::update_session_if_changed(sessions_dir, id, |current| {
            if !super::continuation::session_matches_project_state(current, project_root) {
                return Err(std::io::Error::other(
                    "Session project changed before startup normalization",
                ));
            }
            if current.status != gwt_agent::AgentStatus::Interrupted
                && current.worktree_path.exists()
                && current.should_mark_interrupted_from_lifecycle()
            {
                current.update_status(gwt_agent::AgentStatus::Interrupted);
            }
            Ok(())
        }) {
            tracing::warn!(%error, session_id = id, "owner startup Session normalization retained for retry");
        }
    }
}
