//! Project-owned manual successor preparation and Continue Work coordination.

use crate as gwt;
use crate::project_runtime::launch_finalization::{
    pending_continue_work_session_identity, pending_execution_activation_status,
    reject_continue_work_workspace_commit, resolve_split_workspace_state_external_commit,
    resolve_split_workspace_state_external_commit_with_retry_hint,
};
use crate::project_runtime::launch_payload::PendingContinueWorkExecution;
use crate::session_launch::launch_config_from_persisted_session;
use crate::session_launch::recovery::bind_durable_launch_recovery_session_identity;
use crate::{agent_capability::AgentCapabilityIssuer, launch_runtime::resolve_launch_worktree};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub fn prepare_manual_successor_before_pane(
    project_root: &Path,
    sessions_dir: &Path,
    issuer: Option<&AgentCapabilityIssuer>,
    config: &mut gwt_agent::LaunchConfig,
    require_existing_attempt: bool,
) -> Result<(), String> {
    let gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
        operation_id,
        expected_binding,
        expected_predecessor,
        expected_runtime,
        predecessor_kind,
    } = config.execution_intent.clone()
    else {
        return Ok(());
    };
    resolve_launch_worktree(project_root, config)?;
    let worktree = config
        .working_dir
        .clone()
        .ok_or_else(|| "Manual successor preflight did not resolve a worktree".to_string())?;
    let owner_number = config
        .linked_issue_number
        .ok_or_else(|| "Manual successor preflight requires a linked owner".to_string())?;
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        // This only selects the predecessor key. The exact launch preparation
        // below revalidates the captured Session and durable binding together.
        kind: match expected_predecessor
            .as_deref()
            .map(|identity| identity.execution_binding.owner_kind.as_str())
        {
            Some("spec") => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            Some("issue") => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            Some(_) => return Err("Manual successor predecessor changed before preflight".into()),
            None => gwt::cli::execution_state::detect_owner_kind(&worktree, owner_number),
        },
        number: owner_number,
    };
    if expected_predecessor.as_deref().is_some_and(|identity| {
        identity.execution_binding.owner_kind != owner.kind.as_str()
            || identity.execution_binding.owner_number != owner.number
            || identity.execution_binding.identity != expected_binding
    }) {
        return Err("Manual successor predecessor changed before preflight".to_string());
    }
    let issuer = issuer.ok_or_else(|| {
        "Manual successor preflight is missing its Host capability issuer".to_string()
    })?;
    let reservation =
        if predecessor_kind == gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive {
            let predecessor = expected_predecessor.as_deref().ok_or_else(|| {
                "Active manual successor requires an exact predecessor Session".to_string()
            })?;
            Some(issuer.reserve_manual_execution_handoff(&predecessor.execution_binding)?)
        } else {
            None
        };
    let result = (|| {
        let candidate_session_id =
            manual_successor_stable_component("manual-successor", &operation_id);
        let source = match predecessor_kind {
            gwt_agent::ManualLaunchSuccessorPredecessor::Completed => {
                gwt::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE
            }
            gwt_agent::ManualLaunchSuccessorPredecessor::Blocked
            | gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive => {
                gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE
            }
        };
        let predecessor_status = match predecessor_kind {
            gwt_agent::ManualLaunchSuccessorPredecessor::Completed => {
                gwt::cli::execution_state::SuccessorPredecessorStatus::Completed
            }
            gwt_agent::ManualLaunchSuccessorPredecessor::Blocked => {
                gwt::cli::execution_state::SuccessorPredecessorStatus::Blocked
            }
            gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive => {
                gwt::cli::execution_state::SuccessorPredecessorStatus::Active
            }
        };
        let existing = gwt::cli::execution_state::continuation_attempt_for_operation(
            &worktree,
            owner,
            &operation_id,
        )
        .map_err(|error| error.to_string())?;
        if require_existing_attempt
            && existing.as_ref().is_none_or(|attempt| {
                attempt.status != gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
            })
        {
            return Err(
                "Manual successor retry requires its exact retained Prepared attempt".into(),
            );
        }
        let requested_at = gwt::cli::execution_state::load_generation_ledger(&worktree, owner)
            .map_err(|error| error.to_string())?
            .and_then(|ledger| {
                ledger
                    .current_generation()
                    .map(|generation| generation.identity.activated_at)
            })
            .ok_or_else(|| "Manual successor predecessor generation is unavailable".to_string())?;
        let request = existing.map(|attempt| attempt.request).unwrap_or_else(|| {
            gwt::cli::execution_state::SuccessorRequest {
                operation_id: operation_id.clone(),
                principal_id: "gwt-host-manual-launch".to_string(),
                work_id: None,
                source: source.to_string(),
                session_binding_id: manual_successor_stable_component(
                    "manual-binding",
                    &operation_id,
                ),
                initial_session_id: candidate_session_id.clone(),
                entrypoint: gwt::cli::execution_state::entrypoint_from_launch(
                    config.entrypoint_args(),
                    false,
                ),
                requested_at,
            }
        });
        gwt::session_launch::recovery::persist_durable_launch_recovery(
            sessions_dir,
            gwt::session_launch::recovery::DurableLaunchRecoveryKind::FreshSuccessor {
                operation_id: operation_id.clone(),
            },
            &request.initial_session_id,
            project_root,
            &worktree,
            owner,
            None,
            None,
        )?;
        let attempt = gwt::cli::execution_state::prepare_exact_manual_launch_successor(
            &worktree,
            owner,
            &request,
            gwt::cli::execution_state::ExactManualLaunchPredecessor {
                sessions_dir,
                session: expected_predecessor.as_deref(),
                runtime: expected_runtime,
                binding: &expected_binding,
                status: predecessor_status,
                terminal_reason: "exact producing runtime terminated before manual Launch Agent",
            },
        );
        let attempt = match attempt {
            Ok(attempt) => attempt,
            Err(error) => {
                if gwt::cli::execution_state::continuation_attempt_for_operation(
                    &worktree,
                    owner,
                    &operation_id,
                )
                .is_ok_and(|attempt| attempt.is_none())
                {
                    let _ = gwt::session_launch::recovery::clear_durable_launch_recovery(
                        sessions_dir,
                        &request.initial_session_id,
                    );
                }
                return Err(error.to_string());
            }
        };
        if attempt.status != gwt::cli::execution_state::ContinuationAttemptStatus::Prepared {
            return Err(format!(
                "Manual successor operation is {:?}; reconcile before retrying",
                attempt.status
            ));
        }
        let identity = gwt::cli::execution_state::prepared_successor_execution_binding(
            &worktree,
            owner,
            &attempt.request,
        )
        .map_err(|error| error.to_string())?;
        let repo_hash = gwt_core::repo_hash::detect_repo_hash(&worktree)
            .ok_or_else(|| "Manual successor repository hash is unavailable".to_string())?;
        config.execution_intent = gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(
            gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: attempt.request.initial_session_id,
                repo_hash: repo_hash.as_str().to_string(),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity,
                capability_generation: 1,
            },
        );
        Ok(())
    })();
    if let Some(reservation) = reservation.as_ref() {
        let finalized = if result.is_ok() {
            issuer.commit_manual_execution_handoff(reservation)
        } else {
            issuer.rollback_manual_execution_handoff(reservation)
        };
        if !finalized && result.is_ok() {
            return Err("Manual successor handoff reservation was lost".to_string());
        }
    }
    result
}

pub fn manual_successor_stable_component(prefix: &str, operation_id: &str) -> String {
    let digest = Sha256::digest(operation_id.as_bytes());
    format!("{prefix}-{}", hex::encode(digest))
}

pub fn claim_prepared_manual_successor_launch(
    sessions_dir: &Path,
    project_root: &Path,
    config: &gwt_agent::LaunchConfig,
) -> Result<Option<gwt_agent::SessionActiveLaunchHandshake>, String> {
    let gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(binding) =
        &config.execution_intent
    else {
        return Ok(None);
    };
    let owner_kind = match binding.owner_kind.as_str() {
        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        _ => return Err("Prepared manual successor owner kind is not canonical".to_string()),
    };
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: binding.owner_number,
    };
    let worktree_path = config
        .working_dir
        .as_deref()
        .ok_or_else(|| "Prepared manual successor has no exact worktree".to_string())?;
    let branch = config
        .branch
        .as_deref()
        .filter(|branch| !branch.trim().is_empty())
        .ok_or_else(|| "Prepared manual successor has no canonical branch".to_string())?;
    let mut candidate = gwt_agent::Session::new(worktree_path, branch, config.agent_id.clone());
    candidate.id = binding.session_id.clone();
    candidate.project_state_root = Some(project_root.to_path_buf());
    candidate.repo_hash = Some(binding.repo_hash.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate.set_execution_binding(Some(binding.clone()))?;
    candidate.update_status(gwt_agent::AgentStatus::Running);
    let expected =
        bind_durable_launch_recovery_session_identity(sessions_dir, &candidate, binding)?;
    gwt::cli::execution_state::claim_prepared_session_launch(
        &expected.worktree_path,
        owner,
        sessions_dir,
        &candidate,
    )
    .map_err(|error| error.to_string())?
    .map(Some)
    .ok_or_else(|| {
        "Prepared manual successor is already being materialized by another Host".to_string()
    })
}

use super::launch_finalization::{
    path_matches, pending_fresh_execution_activation_status, pending_fresh_execution_attempt_status,
};
use super::launch_payload::{
    ActiveAgentSession, PendingContinueWork, PendingFreshExecutionLaunch, WorkspaceResumeContext,
};
use super::workspace::{
    apply_workspace_launch_for_current_work, apply_workspace_launch_transition,
    non_empty_workspace_text, WorkspaceLaunchProjectionKind, WorkspaceLaunchTransition,
};
use crate::session_launch::recovery::clear_durable_launch_recovery;

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce() + 'static>>> = std::cell::RefCell::new(None);
}
#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub fn set_fresh_execution_pre_work_commit_hook_for_test(hook: Box<dyn FnOnce() + 'static>) {
    FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}
#[doc(hidden)]
pub fn invoke_fresh_execution_pre_work_commit_hook() {
    #[cfg(any(test, feature = "test-gh-guard"))]
    FRESH_EXECUTION_PRE_WORK_COMMIT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_fresh_execution_launch_session_start(
    pending: &PendingFreshExecutionLaunch,
    active_session: Option<&ActiveAgentSession>,
    token: Option<&str>,
    issuer: Option<&crate::agent_capability::AgentCapabilityIssuer>,
    readiness_nonce: Option<&str>,
    sessions_dir: &Path,
    live_session_ids: &HashSet<String>,
    window_is_current: impl Fn() -> bool,
    admission: &mut Option<gwt_core::operation_deadline::NamedFileLock>,
) -> Result<bool, String> {
    if !window_is_current() {
        return Err("fresh launch window changed before activation".to_string());
    }
    if readiness_nonce != Some(pending.readiness_nonce.as_str()) {
        return Err("the authenticated SessionStart readiness nonce did not match".to_string());
    }
    let Some(active_session) = active_session else {
        return Err("the launched pane has no active Session".to_string());
    };
    if active_session.session_id != pending.binding.session_id
        || !path_matches(&active_session.worktree_path, &pending.worktree_path)
    {
        return Err(
            "the launched Session does not match its Prepared execution binding".to_string(),
        );
    }
    let Some(token) = token else {
        return Err("the Prepared Host capability is missing".to_string());
    };
    let Some(issuer) = issuer else {
        return Err("the Host capability issuer is unavailable".to_string());
    };
    if !issuer.prepared_token_is_current(token, &pending.binding) {
        return Err("the Prepared Host capability is no longer current".to_string());
    }
    let probe = gwt::probe_authenticated_prepared_execution_binding(
        &pending.project_root,
        &pending.binding.session_id,
        &pending.binding,
        "fresh-linked-owner-launch-coordinator",
        gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: pending.operation_id.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    );
    if probe.as_ref().is_err()
        || probe
            .as_ref()
            .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
    {
        return Err("the Host could not prove the exact Prepared execution binding".to_string());
    }
    if issuer.promote_prepared(token, &pending.binding).is_err()
        || !issuer.active_token_is_current(token, &pending.binding)
    {
        return Err("the Prepared Host capability could not be promoted".to_string());
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    super::agent_frontend::run_owner_fresh_work_commit_test_hook(
        &pending.project_root,
        &pending.binding.session_id,
    );
    invoke_fresh_execution_pre_work_commit_hook();
    // Issue #4940: the canonical Work transaction has one Prepared slot.
    // Serialize fresh finalizers before taking any owner/Session lease, across
    // staging, activation and publication. Work recovery remains nonblocking.
    let admission_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&pending.project_root)
            .with_file_name("fresh-launch-admission.lock");
    std::fs::create_dir_all(admission_path.parent().expect("canonical Work directory"))
        .map_err(|error| format!("fresh launch admission directory failed: {error}"))?;
    let acquisition_deadline = gwt_core::operation_deadline::current().is_none().then(|| {
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            gwt_core::operation_deadline::now() + std::time::Duration::from_secs(120),
        )
    });
    *admission = Some(
        gwt_core::operation_deadline::NamedFileLock::acquire_quiet(
            &admission_path,
            "fresh launch admission",
        )
        .map_err(|error| format!("fresh launch admission failed: {error}"))?,
    );
    drop(acquisition_deadline);
    let already_activated = pending_fresh_execution_activation_status(pending) == Some(true);
    let transaction_committed = if already_activated {
        resolve_activated_fresh_execution_commit(
            &pending.project_root,
            &pending.worktree_path,
            pending.owner,
            &pending.operation_id,
            &pending.request,
            sessions_dir,
            &pending.session_identity,
        )
    } else {
        gwt::cli::execution_state::with_prepared_successor_exact_session_activation(
                &pending.worktree_path,
                pending.owner,
                &pending.request,
                sessions_dir,
                &pending.session_identity,
                |activate| {
                    gwt_core::workspace_projection::transact_workspace_state_for_work_event_root_with_supersession_commit(
                        &pending.project_root,
                        &pending.worktree_path,
                        &pending.operation_id,
                        |projection, work_items, _| {
                            let now = chrono::Utc::now();
                            let events = apply_workspace_launch_for_current_work(
                                &pending.project_root,
                                projection,
                                work_items,
                                active_session,
                                WorkspaceLaunchTransition {
                                    work_id: None,
                                    base_branch: pending.base_branch.as_deref(),
                                    linked_issue_number: pending.linked_issue_number,
                                    canonical_owner: Some(pending.owner),
                                    resume_context: pending.resume_context.as_ref(),
                                    kind: if pending.base_branch.is_some() {
                                        WorkspaceLaunchProjectionKind::StartWork
                                    } else {
                                        WorkspaceLaunchProjectionKind::Resume {
                                            created_by_start_work: active_session
                                                .branch_name
                                                .starts_with("work/"),
                                        }
                                    },
                                    live_session_ids,
                                    now,
                                },
                                true,
                            )?;
                            Ok(((), events))
                        },
                        || {
                            if !window_is_current()
                                || !issuer.active_token_is_current(token, &pending.binding)
                            {
                                return Err(gwt_core::error::GwtError::Io(std::io::Error::other(
                                    "fresh launch window or capability changed before activation",
                                )));
                            }
                            activate().map_err(gwt_core::error::GwtError::Io)?;
                            #[cfg(any(test, feature = "test-gh-guard"))]
                            super::agent_frontend::run_owner_fresh_activation_test_hook(
                                &pending.project_root, &pending.binding.session_id,
                            );
                            Ok(())
                        },
                    )
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                    if fresh_execution_commit_readback_matches(
                        &pending.worktree_path,
                        pending.owner,
                        &pending.session_identity,
                    ) {
                        Ok(())
                    } else {
                        Err(std::io::Error::other(
                            "fresh launch commit readback does not match exact authority",
                        ))
                    }
                },
            )
            .map(|result| result.is_some())
    };
    if !matches!(transaction_committed, Ok(true)) {
        if pending_fresh_execution_activation_status(pending) != Some(true) {
            let reason = match transaction_committed {
                Err(error) => error.to_string(),
                Ok(false) => {
                    "exact Prepared successor Session or binding changed before activation"
                        .to_string()
                }
                Ok(true) => unreachable!("successful transaction was excluded"),
            };
            return Err(format!(
                "the fresh generation activation transaction was rejected: {reason}"
            ));
        }
        // A response-loss retry owns any partial generation+Work repair.
        // It must reacquire the exact active Session lease and must never
        // publish Work from this unclassified error path.
        return Ok(false);
    }

    if !issuer.active_token_is_current(token, &pending.binding) {
        return Ok(false);
    }
    let active_probe = gwt::probe_authenticated_execution_binding(
        &pending.project_root,
        &pending.binding.session_id,
        &pending.binding,
        "fresh-linked-owner-launch-coordinator",
        gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: pending.operation_id.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    );
    if active_probe.as_ref().is_err()
        || active_probe
            .as_ref()
            .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        || gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            == Some(pending.predecessor_binding.clone())
    {
        return Ok(false);
    }

    Ok(true)
}

pub fn cleanup_fresh_execution_launch_candidate(
    pending: &PendingFreshExecutionLaunch,
    sessions_dir: &Path,
    status: gwt::cli::execution_state::ContinuationAttemptStatus,
) -> std::io::Result<bool> {
    if status == gwt::cli::execution_state::ContinuationAttemptStatus::Prepared {
        gwt::cli::execution_state::abort_successor_and_remove_exact_session(
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
        )
    } else {
        gwt::cli::execution_state::remove_exact_session_with_owner_lease(
            &pending.worktree_path,
            pending.owner,
            sessions_dir,
            &pending.session_identity,
            || {
                reject_continue_work_workspace_commit(
                    &pending.project_root,
                    &pending.worktree_path,
                    &pending.operation_id,
                )
            },
        )
    }
}

pub fn prepare_fresh_execution_launch_publication(
    pending: &PendingFreshExecutionLaunch,
    active_session: &ActiveAgentSession,
    sessions_dir: &Path,
    issue_link_cache_dir: &Path,
) -> bool {
    if let Some(issue_number) = pending.linked_issue_number {
        if let Err(error) = super::installation::record_issue_branch_link_with_cache_dir(
            &pending.worktree_path,
            &active_session.branch_name,
            issue_number,
            issue_link_cache_dir,
        ) {
            tracing::warn!(error = %error, "fresh launch issue linkage update was skipped");
        }
    }
    if let Err(error) = clear_durable_launch_recovery(sessions_dir, &pending.binding.session_id) {
        tracing::warn!(
            session_id = %pending.binding.session_id,
            error = %error,
            "settled fresh-launch recovery receipt cleanup remains pending"
        );
        return false;
    }
    true
}

pub fn fresh_execution_commit_readback_matches(
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session: &gwt_agent::SessionExecutionIdentity,
) -> bool {
    gwt::cli::execution_state::current_execution_binding(worktree_path, owner)
        .ok()
        .flatten()
        == Some(session.execution_binding.identity.clone())
}

pub fn resolve_activated_fresh_execution_commit(
    project_root: &Path,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    operation_id: &str,
    request: &gwt::cli::execution_state::SuccessorRequest,
    sessions_dir: &Path,
    session: &gwt_agent::SessionExecutionIdentity,
) -> std::io::Result<bool> {
    gwt::cli::execution_state::with_activated_successor_exact_session_repair(
        worktree_path,
        owner,
        request,
        sessions_dir,
        session,
        |repair| {
            repair()?;
            let resolution = resolve_split_workspace_state_external_commit(
                project_root,
                worktree_path,
                operation_id,
                gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(resolution
                == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                && fresh_execution_commit_readback_matches(worktree_path, owner, session))
        },
    )
    .map(|result| result == Some(true))
}

#[derive(Debug, Clone)]
pub enum FreshExecutionFinalizationOutcome {
    Committed,
    RolledBack(String),
    Retained,
}

/// Finalize one exact authenticated fresh launch while retaining admission through cleanup.
#[allow(clippy::too_many_arguments)]
pub fn finalize_fresh_execution_launch(
    pending: &PendingFreshExecutionLaunch,
    active_session: Option<&ActiveAgentSession>,
    token: Option<&str>,
    issuer: Option<&crate::agent_capability::AgentCapabilityIssuer>,
    readiness_nonce: Option<&str>,
    sessions_dir: &Path,
    issue_link_cache_dir: &Path,
    live_session_ids: &HashSet<String>,
    window_is_current: impl Fn() -> bool,
) -> FreshExecutionFinalizationOutcome {
    // Keep admission through rollback: a failed preparation cannot expose its Prepared marker.
    let mut admission = None;
    let result = prepare_fresh_execution_launch_session_start(
        pending,
        active_session,
        token,
        issuer,
        readiness_nonce,
        sessions_dir,
        live_session_ids,
        window_is_current,
        &mut admission,
    );
    let outcome = match result {
        Ok(true)
            if prepare_fresh_execution_launch_publication(
                pending,
                active_session.expect("committed exact Session"),
                sessions_dir,
                issue_link_cache_dir,
            ) =>
        {
            FreshExecutionFinalizationOutcome::Committed
        }
        Err(detail) => {
            let cleaned = pending_fresh_execution_attempt_status(pending)
                .filter(|status| {
                    *status != gwt::cli::execution_state::ContinuationAttemptStatus::Activated
                })
                .is_some_and(|status| {
                    cleanup_fresh_execution_launch_candidate(pending, sessions_dir, status)
                        .unwrap_or_else(|error| {
                            tracing::warn!(
                                session_id = %pending.binding.session_id,
                                %error,
                                "fresh SessionStart cleanup failed; retained exact candidate"
                            );
                            false
                        })
                });
            if cleaned {
                let monitor_startup_failure =
                    pending
                        .launch_feedback_context
                        .as_ref()
                        .is_some_and(|context| {
                            context
                                .issue_monitor_launch_binding_issue_number()
                                .is_some()
                                && context.issue_monitor_delivery_id.is_some()
                                && context.issue_monitor_autonomous_handoff.is_none()
                        });
                if !monitor_startup_failure {
                    if let Err(error) =
                        clear_durable_launch_recovery(sessions_dir, &pending.binding.session_id)
                    {
                        tracing::warn!(%error, "aborted fresh launch receipt cleanup remains pending");
                    }
                }
                FreshExecutionFinalizationOutcome::RolledBack(detail)
            } else {
                FreshExecutionFinalizationOutcome::Retained
            }
        }
        _ => FreshExecutionFinalizationOutcome::Retained,
    };
    outcome
}

pub fn strict_projection_owner(raw_owner: &str) -> Option<ProjectionOwnerRef> {
    let owner = raw_owner.trim();
    if owner.is_empty() {
        return None;
    }
    let lower = owner.to_ascii_lowercase();
    let (declared_kind, digits) = if let Some(rest) = owner.strip_prefix('#') {
        (None, rest.trim())
    } else if lower.starts_with("issue") || lower.starts_with("spec") {
        let (prefix_len, declared_kind) = if lower.starts_with("issue") {
            (5, gwt::cli::execution_state::ExecutionOwnerKind::Issue)
        } else {
            (4, gwt::cli::execution_state::ExecutionOwnerKind::Spec)
        };
        let rest = &owner[prefix_len..];
        let first = rest.chars().next()?;
        if !(first.is_ascii_whitespace() || first == '#' || first == '-') {
            return None;
        }
        (
            Some(declared_kind),
            rest.trim_start()
                .strip_prefix('#')
                .or_else(|| rest.trim_start().strip_prefix('-'))
                .unwrap_or(rest.trim_start())
                .trim(),
        )
    } else {
        (None, owner)
    };
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .map(|number| ProjectionOwnerRef {
            declared_kind,
            number,
        })
}

#[allow(clippy::too_many_arguments)]
pub fn projection_continue_authority_matches(
    item: &gwt_core::workspace_projection::WorkItem,
    project_root: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    worktree_path: &Path,
    branch: &str,
    agent_id: &gwt_agent::AgentId,
    agent_session_id: Option<&str>,
    require_exact_owner_kind: bool,
) -> bool {
    let Ok(projected_owner) = projection_only_continue_owner(item) else {
        return false;
    };
    if projected_owner.number != owner.number
        || (require_exact_owner_kind
            && projected_owner
                .declared_kind
                .is_some_and(|kind| kind != owner.kind))
    {
        return false;
    }
    let Ok((current_worktree, current_branch)) =
        projection_only_continue_container(item, project_root)
    else {
        return false;
    };
    if !path_matches(&current_worktree, worktree_path) || current_branch != branch {
        return false;
    }
    if let Some(agent_session_id) = agent_session_id {
        let mut matching = item
            .agents
            .iter()
            .filter(|agent| agent.session_id == agent_session_id);
        return matching
            .next()
            .is_some_and(|agent| work_agent_ref_authenticates_agent(agent, agent_id))
            && matching.next().is_none();
    }
    match projection_only_continue_agent(item) {
        Ok((current_agent, _)) => current_agent == *agent_id,
        Err(_) => false,
    }
}

pub fn projection_only_continue_agent(
    item: &gwt_core::workspace_projection::WorkItem,
) -> Result<(gwt_agent::AgentId, Option<String>), ContinueWorkFailure> {
    let latest_at = item
        .agents
        .iter()
        .filter(|agent| {
            agent.agent_id.as_deref().is_some_and(|agent_id| {
                let agent_id = agent_id.trim();
                !agent_id.is_empty()
                    && !agent_id
                        .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
            })
        })
        .map(|agent| agent.updated_at)
        .max()
        .ok_or_else(|| {
            ContinueWorkFailure::failed(
                "agent_identity_missing",
                "This Work has no Agent identity for a new conversation.",
                false,
            )
        })?;
    let mut selected: Option<(gwt_agent::AgentId, Option<String>)> = None;
    for agent in item
        .agents
        .iter()
        .filter(|agent| agent.updated_at == latest_at)
    {
        let Some(raw_agent_id) = agent.agent_id.as_deref().map(str::trim).filter(|agent_id| {
            !agent_id.is_empty()
                && !agent_id
                    .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
        }) else {
            continue;
        };
        let Some(agent_id) = gwt_agent::resolve_agent_id(raw_agent_id) else {
            continue;
        };
        if matches!(agent_id, gwt_agent::AgentId::Custom(_)) {
            return Err(ContinueWorkFailure::failed(
                "agent_identity_unsupported",
                "This Work requires durable Session metadata to relaunch its custom Agent.",
                false,
            ));
        }
        if selected
            .as_ref()
            .is_some_and(|(selected, _)| selected != &agent_id)
        {
            return Err(ContinueWorkFailure::failed(
                "agent_identity_ambiguous",
                "The Work has conflicting latest Agent identities and cannot be continued safely.",
                false,
            ));
        }
        let display_name = agent
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        selected.get_or_insert((agent_id, display_name));
    }
    selected.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "agent_identity_missing",
            "This Work has no Agent identity for a new conversation.",
            false,
        )
    })
}

pub fn fresh_execution_readiness_conflict() -> gwt::AgentWorkspaceUpdateError {
    gwt::AgentWorkspaceUpdateError::new(
        gwt::AgentWorkspaceUpdateErrorCode::TransactionConflict,
        "Host readiness coordinator did not commit the candidate; run execution.status before retrying",
    )
}

pub fn worktree_matches_project_state(worktree_path: &Path, project_root: &Path) -> bool {
    if path_matches(worktree_path, project_root) {
        return true;
    }
    if path_matches(
        &crate::launch_runtime::normalize_recent_project_path(worktree_path),
        project_root,
    ) {
        return true;
    }
    let Ok(project_anchor) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    let Ok(worktree_anchor) = gwt_git::worktree::main_worktree_root(worktree_path) else {
        return false;
    };
    path_matches(&project_anchor, &worktree_anchor)
}

#[derive(Debug)]
pub enum ContinueWorkFinalizationOutcome {
    Committed,
    RolledBack {
        detail: String,
    },
    Retained {
        failure: Option<ContinueWorkFailure>,
    },
}

/// Finalize one exact authenticated continuation without releasing a replaced pane's evidence.
#[allow(clippy::too_many_arguments)]
pub fn finalize_continue_work_launch(
    pending: &PendingContinueWork,
    active_session: Option<&ActiveAgentSession>,
    token: Option<&str>,
    issuer: Option<&AgentCapabilityIssuer>,
    readiness_nonce: Option<&str>,
    sessions_dir: &Path,
    live_session_ids: &HashSet<String>,
    active_session_is_live: bool,
    window_is_current: impl Fn() -> bool,
) -> ContinueWorkFinalizationOutcome {
    let result = (|| -> Result<bool, String> {
        if !window_is_current() {
            return Ok(false);
        }
        if readiness_nonce != Some(pending.readiness_nonce.as_str()) {
            return Err("the authenticated SessionStart readiness nonce did not match".to_string());
        }
        let Some(active_session) = active_session else {
            return Err("the launched pane has no active Session".to_string());
        };
        if active_session.session_id != pending.binding.session_id
            || !path_matches(&active_session.worktree_path, &pending.worktree_path)
        {
            return Err(
                "the launched Session does not match its Prepared execution binding".to_string(),
            );
        }
        let Some(token) = token else {
            return Err("the Prepared Host capability is missing".to_string());
        };
        let Some(issuer) = issuer else {
            return Err("the Host capability issuer is unavailable".to_string());
        };
        if !issuer.prepared_token_is_current(token, &pending.binding) {
            return Err("the Prepared Host capability is no longer current".to_string());
        }
        let probe = gwt::probe_authenticated_prepared_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "continue-work-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if probe.as_ref().is_err()
            || probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        {
            return Err(
                "the Host could not prove the exact Prepared execution binding".to_string(),
            );
        }
        // Finish every fallible process-local authority transition before the
        // durable generation+Work commit point. If the CAS below fails, the
        // normal pre-activation cleanup revokes this bearer; after the CAS,
        // no capability promotion failure can strand a partial generation.
        if !window_is_current() {
            return Ok(false);
        }
        if issuer.promote_prepared(token, &pending.binding).is_err()
            || !issuer.active_token_is_current(token, &pending.binding)
        {
            return Err("the Prepared Host capability could not be promoted".to_string());
        }

        let exact_session_identity = match pending_continue_work_session_identity(pending) {
            Ok(identity) => identity,
            Err(_) => {
                return Err(
                    "the Prepared continuation Session identity is not canonical".to_string(),
                );
            }
        };
        let already_activated = pending_execution_is_activated(pending);
        let transaction_result: Result<(), gwt_core::error::GwtError> = if already_activated {
            match gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                sessions_dir,
                &exact_session_identity,
                || {
                    if !window_is_current() {
                        return Err(gwt_core::error::GwtError::Other(
                            "the continuation window changed before Work repair".to_string(),
                        ));
                    }
                    resolve_split_workspace_state_external_commit(
                        &pending.project_root,
                        &pending.worktree_path,
                        &pending.operation_id,
                        gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                    )
                    .and_then(|resolution| {
                        if resolution
                            == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed
                            && continue_work_commit_readback_matches(pending)
                        {
                            Ok(())
                        } else {
                            Err(gwt_core::error::GwtError::Other(format!(
                                "unexpected continuation Work resolution or readback: {resolution:?}"
                            )))
                        }
                    })
                },
            ) {
                Ok(Some(result)) => result,
                Ok(None) => Err(gwt_core::error::GwtError::Other(
                    "the active continuation Session changed before Work commit".to_string(),
                )),
                Err(error) => Err(gwt_core::error::GwtError::Io(error)),
            }
        } else {
            let leased = match &pending.execution {
                PendingContinueWorkExecution::Successor(request) => {
                    gwt::cli::execution_state::with_prepared_successor_exact_session_activation(
                        &pending.worktree_path,
                        pending.owner,
                        request,
                        sessions_dir,
                        &exact_session_identity,
                        |activate| {
                            let mut activate_unit = || {
                                if !window_is_current()
                                    || !issuer.active_token_is_current(token, &pending.binding)
                                {
                                    return Err(std::io::Error::other(
                                        "continuation window or capability changed before activation",
                                    ));
                                }
                                activate().map(|_| ())
                            };
                            transact_pending_continue_work_with_activation(
                                pending,
                                active_session,
                                live_session_ids,
                                &mut activate_unit,
                            )
                        },
                    )
                }
                PendingContinueWorkExecution::Takeover(request) => {
                    gwt::cli::execution_state::with_prepared_generation_takeover_exact_session_activation(
                        &pending.worktree_path,
                        pending.owner,
                        request,
                        sessions_dir,
                        &exact_session_identity,
                        |activate| {
                            let mut activate_unit = || {
                                if !window_is_current()
                                    || !issuer.active_token_is_current(token, &pending.binding)
                                {
                                    return Err(std::io::Error::other(
                                        "continuation window or capability changed before activation",
                                    ));
                                }
                                activate().map(|_| ())
                            };
                            transact_pending_continue_work_with_activation(
                                pending,
                                active_session,
                                live_session_ids,
                                &mut activate_unit,
                            )
                        },
                    )
                }
            };
            match leased {
                Ok(Some(())) => Ok(()),
                Ok(None) => Err(gwt_core::error::GwtError::Other(
                    "the Prepared continuation Session changed before activation".to_string(),
                )),
                Err(error) => Err(gwt_core::error::GwtError::Io(error)),
            }
        };
        if transaction_result.is_err() {
            let activated = pending_execution_is_activated(pending);
            if !activated {
                return Err("the generation activation transaction was rejected".to_string());
            }
            // The generation may have committed before a Work publication
            // error, but recovery must repair that partial commit under the
            // exact active Session lease. Never resolve it from this
            // unclassified error path after a lease mismatch.
            return Ok(false);
        }

        if gwt::cli::execution_state::current_execution_binding(
            &pending.worktree_path,
            pending.owner,
        )
        .ok()
        .flatten()
            != Some(pending.binding.identity.clone())
        {
            return Ok(false);
        }
        if !issuer.active_token_is_current(token, &pending.binding) {
            return Ok(false);
        }
        let active_probe = gwt::probe_authenticated_execution_binding(
            &pending.project_root,
            &pending.binding.session_id,
            &pending.binding,
            "continue-work-coordinator",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: pending.operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if active_probe.as_ref().is_err()
            || active_probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != pending.binding.identity)
        {
            return Ok(false);
        }
        if gwt::cli::execution_state::current_active_execution_binding_matches(
            &pending.worktree_path,
            pending.owner,
            &pending.predecessor_session_id,
            &pending.predecessor_binding,
        )
        .unwrap_or(true)
        {
            return Ok(false);
        }
        let work_readback =
            gwt_core::workspace_projection::load_workspace_work_items(&pending.project_root)
                .ok()
                .flatten()
                .is_some_and(|projection| {
                    projection.work_items.iter().any(|item| {
                        item.id == pending.work_id
                            && item.status_category
                                == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                            && item
                                .agents
                                .iter()
                                .any(|agent| agent.session_id == pending.binding.session_id)
                    })
                });
        if !work_readback {
            return Ok(false);
        }

        Ok(window_is_current())
    })();
    let detail = match result {
        Ok(true) => return ContinueWorkFinalizationOutcome::Committed,
        Ok(false) => return ContinueWorkFinalizationOutcome::Retained { failure: None },
        Err(detail) => detail,
    };
    let reconcile_activated = || {
        if window_is_current()
            && active_session_is_live
            && active_session
                .zip(token)
                .zip(issuer)
                .is_some_and(|((active, token), issuer)| {
                    continue_work_active_readback_matches(
                        pending,
                        active,
                        token,
                        issuer,
                        sessions_dir,
                    )
                })
            && window_is_current()
        {
            ContinueWorkFinalizationOutcome::Committed
        } else {
            ContinueWorkFinalizationOutcome::Retained { failure: None }
        }
    };
    // A captured close owns cleanup once this exact window is cancelled. An
    // Activated or unreadable attempt always retains its evidence for repair.
    if !window_is_current() {
        return ContinueWorkFinalizationOutcome::Retained { failure: None };
    }
    match pending_execution_activation_status(pending) {
        Some(true) => return reconcile_activated(),
        Some(false) => {}
        None => return ContinueWorkFinalizationOutcome::Retained { failure: None },
    }
    if let Err(error) = pending_continue_work_session_identity(pending) {
        tracing::warn!(%error, "retained continuation after invalid expected Session identity");
        return ContinueWorkFinalizationOutcome::Retained { failure: None };
    }
    if !window_is_current() {
        return ContinueWorkFinalizationOutcome::Retained { failure: None };
    }
    match crate::project_runtime::launch_finalization::rollback_worker_continue_work(
        sessions_dir,
        pending,
    ) {
        Ok(true) => ContinueWorkFinalizationOutcome::RolledBack { detail },
        Ok(false) => reconcile_activated(),
        Err(_) if pending_execution_activation_status(pending) == Some(true) => {
            reconcile_activated()
        }
        Err(error) => {
            let failure = if error == "continuation candidate Session changed before exact rollback"
            {
                ContinueWorkFailure::conflict(
                    "The failed continuation candidate no longer matches its exact Agent identity.",
                )
            } else {
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The failed continuation could not commit its exact cleanup; retry reconciliation.",
                    true,
                )
            };
            ContinueWorkFinalizationOutcome::Retained {
                failure: Some(failure),
            }
        }
    }
}

fn continue_work_active_readback_matches(
    pending: &PendingContinueWork,
    active: &ActiveAgentSession,
    token: &str,
    issuer: &AgentCapabilityIssuer,
    sessions_dir: &Path,
) -> bool {
    if active.session_id != pending.binding.session_id
        || !path_matches(&active.worktree_path, &pending.worktree_path)
        || gwt::launch_runtime::normalize_branch_name(active.branch_name.trim())
            != pending.work_branch
        || gwt_agent::resolve_agent_id(active.agent_id.trim()).as_ref()
            != Some(&pending.work_agent_id)
    {
        return false;
    }
    let Ok(exact_session_identity) = pending_continue_work_session_identity(pending) else {
        return false;
    };
    if gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
        sessions_dir,
        &exact_session_identity,
        || continue_work_commit_readback_matches(pending),
    )
    .ok()
    .flatten()
        != Some(true)
    {
        return false;
    }
    if !issuer.active_token_is_current(token, &pending.binding) {
        return false;
    }
    let Ok(probe) = gwt::probe_authenticated_execution_binding(
        &pending.project_root,
        &pending.binding.session_id,
        &pending.binding,
        "continue-work-coordinator",
        gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: pending.operation_id.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
        },
    ) else {
        return false;
    };
    if probe.execution_binding != pending.binding.identity
        || gwt::cli::execution_state::current_active_execution_binding_matches(
            &pending.worktree_path,
            pending.owner,
            &pending.predecessor_session_id,
            &pending.predecessor_binding,
        )
        .unwrap_or(true)
    {
        return false;
    }
    true
}

pub fn continue_work_commit_readback_matches(pending: &PendingContinueWork) -> bool {
    if gwt::cli::execution_state::current_execution_binding(&pending.worktree_path, pending.owner)
        .ok()
        .flatten()
        != Some(pending.binding.identity.clone())
    {
        return false;
    }
    gwt_core::workspace_projection::load_workspace_work_items(&pending.project_root)
        .ok()
        .flatten()
        .is_some_and(|projection| {
            let matching = projection
                .work_items
                .iter()
                .filter(|item| item.id == pending.work_id)
                .collect::<Vec<_>>();
            matching.len() == 1
                && !matching[0].discarded
                && matching[0].status_category
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                && projection_continue_authority_matches(
                    matching[0],
                    &pending.project_root,
                    pending.owner,
                    &pending.worktree_path,
                    &pending.work_branch,
                    &pending.work_agent_id,
                    Some(&pending.binding.session_id),
                    true,
                )
        })
}

pub fn transact_pending_continue_work_with_activation(
    pending: &PendingContinueWork,
    active_session: &ActiveAgentSession,
    live_session_ids: &HashSet<String>,
    activate: &mut dyn FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    gwt_core::workspace_projection::transact_workspace_state_for_work_event_root_with_commit(
        &pending.project_root,
        &pending.worktree_path,
        &pending.operation_id,
        |projection, work_items, _| {
            let matching = work_items
                .work_items
                .iter()
                .filter(|item| item.id == pending.work_id)
                .collect::<Vec<_>>();
            if matching.len() != 1 || matching[0].discarded {
                return Err(gwt_core::error::GwtError::Other(
                    "Continue work target changed before activation".to_string(),
                ));
            }
            if !projection_continue_authority_matches(
                matching[0],
                &pending.project_root,
                pending.owner,
                &pending.worktree_path,
                &pending.work_branch,
                &pending.work_agent_id,
                pending.work_agent_session_id.as_deref(),
                false,
            ) {
                return Err(gwt_core::error::GwtError::Other(
                    "Continue work authority changed before activation".to_string(),
                ));
            }
            let event = apply_workspace_launch_transition(
                projection,
                active_session,
                WorkspaceLaunchTransition {
                    work_id: Some(pending.work_id.clone()),
                    base_branch: None,
                    linked_issue_number: Some(pending.owner.number),
                    canonical_owner: Some(pending.owner),
                    resume_context: Some(&pending.resume_context),
                    kind: WorkspaceLaunchProjectionKind::Resume {
                        created_by_start_work: active_session.branch_name.starts_with("work/"),
                    },
                    live_session_ids,
                    now: chrono::Utc::now(),
                },
            );
            Ok(((), vec![event]))
        },
        || activate().map_err(gwt_core::error::GwtError::Io),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    if !continue_work_commit_readback_matches(pending) {
        return Err(std::io::Error::other(
            "Continue work commit readback does not match exact authority",
        ));
    }
    Ok(())
}

pub fn projection_only_continue_container(
    item: &gwt_core::workspace_projection::WorkItem,
    project_root: &Path,
) -> Result<(PathBuf, String), ContinueWorkFailure> {
    let mut selected_path: Option<PathBuf> = None;
    let mut selected_branch: Option<String> = None;
    for container in item
        .events
        .iter()
        .filter_map(|event| event.execution_container.as_ref())
        .chain(item.execution_containers.iter())
    {
        if let Some(path) = container.worktree_path.as_deref() {
            let canonical = dunce::canonicalize(path).map_err(|_| {
                ContinueWorkFailure::failed(
                    "worktree_unavailable",
                    "This Work cannot be continued because its worktree is unavailable.",
                    false,
                )
            })?;
            if selected_path
                .as_deref()
                .is_some_and(|selected| !path_matches(selected, &canonical))
            {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_ambiguous",
                    "The Work has multiple execution containers and cannot be continued safely.",
                    false,
                ));
            }
            if !worktree_matches_project_state(&canonical, project_root) {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_foreign",
                    "The Work execution container does not belong to the active project.",
                    false,
                ));
            }
            selected_path.get_or_insert(canonical);
        }

        let branch = container
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(crate::launch_runtime::normalize_branch_name);
        if let Some(branch) = branch {
            if selected_branch
                .as_deref()
                .is_some_and(|selected| selected != branch.as_str())
            {
                return Err(ContinueWorkFailure::failed(
                    "execution_container_ambiguous",
                    "The Work has conflicting execution branches and cannot be continued safely.",
                    false,
                ));
            }
            selected_branch.get_or_insert(branch);
        }
    }
    let worktree_path = selected_path.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_container_missing",
            "This Work has no execution container to continue.",
            false,
        )
    })?;
    let branch = selected_branch.ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_branch_missing",
            "This Work has no execution branch to continue.",
            false,
        )
    })?;
    Ok((worktree_path, branch))
}

pub fn fresh_execution_readiness_receipt(
    pending: &PendingFreshExecutionLaunch,
    operation_id: String,
) -> Result<gwt::AgentExecutionContinuationReceipt, gwt::AgentWorkspaceUpdateError> {
    if !path_matches(
        &pending.worktree_path,
        &pending.session_identity.worktree_path,
    ) || !worktree_matches_project_state(&pending.worktree_path, &pending.project_root)
    {
        return Err(fresh_execution_readiness_conflict());
    }
    let work_id = gwt_core::workspace_projection::load_workspace_work_items(&pending.project_root)
        .ok()
        .flatten()
        .and_then(|works| {
            let work_id = gwt_core::workspace_projection::current_work_id(
                &works,
                &pending.project_root,
                Some(&pending.session_identity.branch),
                Some(&pending.worktree_path),
            )?;
            works
                .work_items
                .iter()
                .find(|work| {
                    if work.id != work_id || work.is_terminal() {
                        return false;
                    }
                    let owner_matches = projection_only_continue_owner(work).is_ok_and(|owner| {
                        owner.number == pending.owner.number
                            && owner
                                .declared_kind
                                .is_none_or(|kind| kind == pending.owner.kind)
                    });
                    // Fresh activation already proved this local target. Other
                    // hosts' historical containers need not exist on this host.
                    let target_matches = work.execution_containers.iter().any(|container| {
                        container
                            .worktree_path
                            .as_deref()
                            .is_some_and(|path| path_matches(path, &pending.worktree_path))
                            && container.branch.as_deref().is_some_and(|branch| {
                                crate::launch_runtime::normalize_branch_name(branch)
                                    == crate::launch_runtime::normalize_branch_name(
                                        &pending.session_identity.branch,
                                    )
                            })
                    });
                    let mut sessions = work
                        .agents
                        .iter()
                        .filter(|agent| agent.session_id == pending.session_identity.session_id);
                    owner_matches
                        && target_matches
                        && sessions.next().is_some_and(|agent| {
                            work_agent_ref_authenticates_agent(
                                agent,
                                &pending.session_identity.agent_id,
                            )
                        })
                        && sessions.next().is_none()
                })
                .map(|work| work.id.clone())
        })
        .ok_or_else(fresh_execution_readiness_conflict)?;
    Ok(gwt::AgentExecutionContinuationReceipt {
        schema_version: gwt::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
        operation_id,
        outcome: gwt::AgentExecutionContinuationOutcome::SuccessorCreated,
        work_id: Some(work_id),
        predecessor_generation_id: Some(pending.predecessor_binding.generation_id.clone()),
        generation_id: pending.binding.identity.generation_id.clone(),
        execution_binding: pending.binding.identity.clone(),
        capability_generation: pending.binding.capability_generation,
        superseded_execution_binding: None,
        takeover_audit_id: None,
        validated: true,
    })
}

pub fn work_agent_ref_authenticates_agent(
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    agent_id: &gwt_agent::AgentId,
) -> bool {
    match agent.agent_id.as_deref() {
        None => false,
        Some(raw_agent_id) => {
            let raw_agent_id = raw_agent_id.trim();
            !raw_agent_id.is_empty()
                && !raw_agent_id
                    .eq_ignore_ascii_case(gwt_core::workspace_projection::SHELL_WORK_AGENT_ID)
                && gwt_agent::resolve_agent_id(raw_agent_id).as_ref() == Some(agent_id)
        }
    }
}

pub fn projection_only_continue_owner(
    item: &gwt_core::workspace_projection::WorkItem,
) -> Result<ProjectionOwnerRef, ContinueWorkFailure> {
    let owner = item
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty())
        .ok_or_else(|| {
            ContinueWorkFailure::failed(
                "execution_owner_missing",
                "This Work has no linked Issue or SPEC execution owner.",
                false,
            )
        })?;
    strict_projection_owner(owner).ok_or_else(|| {
        ContinueWorkFailure::failed(
            "execution_owner_ambiguous",
            "The Work has an ambiguous execution owner and cannot be continued safely.",
            false,
        )
    })
}

#[derive(Debug)]
pub struct ContinueWorkFailure {
    pub outcome: gwt::ContinueWorkOutcomeKind,
    pub message: String,
    pub code: &'static str,
    pub retryable: bool,
}

impl ContinueWorkFailure {
    pub fn failed(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            message: message.into(),
            code,
            retryable,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
            message: message.into(),
            code: "execution_owner_unknown",
            retryable: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionOwnerRef {
    pub declared_kind: Option<gwt::cli::execution_state::ExecutionOwnerKind>,
    pub number: u64,
}

#[derive(Debug, Clone)]
pub struct CachedContinueWorkOutcome {
    pub work_id: String,
    pub outcome: gwt::ContinueWorkOutcomeKind,
    pub message: Option<String>,
    pub error_code: Option<String>,
    pub retryable: bool,
}

pub struct ContinueWorkInput<'a> {
    pub project_root: &'a Path,
    pub sessions_dir: &'a Path,
    pub profile_config_path: Option<&'a Path>,
    pub is_git: bool,
    pub migration_pending: bool,
    pub client_id: &'a str,
    pub operation_id: &'a str,
    pub work_id: &'a str,
}

pub struct ContinueWorkPane<'a> {
    pub id: &'a str,
    pub active: &'a ActiveAgentSession,
    pub token: Option<&'a str>,
    pub live: bool,
}

pub struct ContinueWorkPending<'a> {
    pub id: &'a str,
    pub pending: &'a PendingContinueWork,
}

pub struct ContinueWorkView<'a> {
    pub panes: &'a [ContinueWorkPane<'a>],
    pub pending: &'a [ContinueWorkPending<'a>],
    pub outcomes: &'a HashMap<String, CachedContinueWorkOutcome>,
    pub issuer: Option<&'a AgentCapabilityIssuer>,
}
impl<'a> ContinueWorkView<'a> {
    fn pane(&self, id: &str) -> Option<&ContinueWorkPane<'a>> {
        self.panes.iter().find(|pane| pane.id == id)
    }
    fn pane_is_live(&self, id: &str) -> bool {
        self.pane(id).is_some_and(|pane| pane.live)
    }
}

#[derive(Debug)]
pub enum ContinueWorkAction {
    FocusExisting {
        window_id: String,
        outcome: CachedContinueWorkOutcome,
    },
    PreparedLaunch {
        config: Box<gwt_agent::LaunchConfig>,
        pending: Box<PendingContinueWork>,
    },
    Pending {
        window_id: String,
    },
    Replay {
        outcome: CachedContinueWorkOutcome,
        focus: Option<String>,
        completed_window: Option<String>,
        clear_operation: bool,
        refresh_projection: bool,
    },
    Failure {
        failure: ContinueWorkFailure,
        cacheable: bool,
    },
}

pub fn coordinate_continue_work(
    input: ContinueWorkInput<'_>,
    view: ContinueWorkView<'_>,
) -> ContinueWorkAction {
    let client_id = input.client_id;
    let operation_id = input.operation_id.to_string();
    let work_id = input.work_id.to_string();
    ContinueWorkCoordinator { input, view }.continue_work_action(client_id, operation_id, work_id)
}

pub fn reconcile_activated_pending_continue_work(
    input: ContinueWorkInput<'_>,
    view: ContinueWorkView<'_>,
    window_id: &str,
    pending: &PendingContinueWork,
) -> Option<ContinueWorkAction> {
    ContinueWorkCoordinator { input, view }
        .reconcile_activated_pending_continue_work(window_id, pending)
}

pub fn continue_work_dispatch_failed(
    pending: &PendingContinueWork,
    error: String,
) -> ContinueWorkAction {
    if let Err(abort_error) = abort_prepared_execution(
        &pending.worktree_path,
        pending.owner,
        &pending.execution,
        "continuation window dispatch failed",
    ) {
        tracing::warn!(error_kind = ?abort_error.kind(), "window dispatch failure could not durably abort continuation");
        return ContinueWorkAction::Failure {
            failure: ContinueWorkFailure::failed(
                "continuation_reconciliation_required",
                "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                true,
            ),
            cacheable: false,
        };
    }
    ContinueWorkAction::Failure {
        failure: ContinueWorkFailure::failed("launch_dispatch_failed", error, true),
        cacheable: true,
    }
}

struct ContinueWorkCoordinator<'a> {
    input: ContinueWorkInput<'a>,
    view: ContinueWorkView<'a>,
}
impl ContinueWorkCoordinator<'_> {
    fn failure_action(
        &self,
        _context: &ContinueWorkInput<'_>,
        _client: &str,
        _operation: String,
        _work: String,
        failure: ContinueWorkFailure,
    ) -> ContinueWorkAction {
        ContinueWorkAction::Failure {
            failure,
            cacheable: true,
        }
    }
    fn uncached_failure_action(
        &self,
        _client: &str,
        _operation: String,
        _work: String,
        failure: ContinueWorkFailure,
    ) -> ContinueWorkAction {
        ContinueWorkAction::Failure {
            failure,
            cacheable: false,
        }
    }
    fn replay_action(
        &self,
        _context: &ContinueWorkInput<'_>,
        _client: &str,
        _operation: String,
        outcome: CachedContinueWorkOutcome,
    ) -> ContinueWorkAction {
        ContinueWorkAction::Replay {
            outcome,
            focus: None,
            completed_window: None,
            clear_operation: false,
            refresh_projection: false,
        }
    }
    fn classify_nonlocal_active_owner_liveness(&self, id: &str) -> ActiveOwnerLiveness {
        classify_nonlocal_active_owner_liveness_at(self.input.sessions_dir, id)
    }
    fn resolve_continue_work_target(
        &self,
        _context: &ContinueWorkInput<'_>,
        work_id: &str,
    ) -> Result<ContinueWorkTarget, ContinueWorkFailure> {
        if !self.input.is_git {
            return Err(ContinueWorkFailure::failed(
                "git_project_required",
                "Continue work requires a Git project.",
                false,
            ));
        }
        if self.input.migration_pending {
            return Err(ContinueWorkFailure::failed(
                "project_migration_pending",
                "Complete the project migration before continuing Work.",
                true,
            ));
        }
        let work_state_root = self.input.project_root.to_path_buf();
        let project_root = gwt::launch_runtime::normalize_recent_project_path(&work_state_root);
        let works = gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(
            &work_state_root,
        )
        .map_err(|_| {
            ContinueWorkFailure::failed(
                "work_state_unavailable",
                "The Work state could not be read safely.",
                true,
            )
        })?;
        let mut matching = works.work_items.iter().filter(|item| item.id == work_id);
        let item = matching.next().ok_or_else(|| {
            ContinueWorkFailure::failed(
                "work_not_found",
                "The selected Work no longer exists.",
                false,
            )
        })?;
        if matching.next().is_some() {
            return Err(ContinueWorkFailure::conflict(
                "The selected Work identity is ambiguous.",
            ));
        }
        if item.discarded {
            return Err(ContinueWorkFailure::failed(
                "work_discarded",
                "Discarded Work cannot be continued.",
                false,
            ));
        }

        let (worktree_path, branch) = projection_only_continue_container(item, &work_state_root)?;
        let actual_branch = canonical_continue_work_branch(&worktree_path)?;
        if actual_branch != branch {
            return Err(ContinueWorkFailure::conflict(
                "The Work branch no longer matches its current worktree.",
            ));
        }
        let projected_owner = projection_only_continue_owner(item)?;
        let owner = canonical_continue_work_owner(&project_root, &worktree_path, projected_owner)?;
        let source_session = item
            .agents
            .iter()
            .filter_map(|agent| {
                if gwt_agent::validate_session_id_path_component(&agent.session_id).is_err() {
                    return None;
                }
                let path = self
                    .input
                    .sessions_dir
                    .join(format!("{}.toml", agent.session_id));
                gwt_agent::Session::load_and_migrate(&path)
                    .ok()
                    .filter(|session| session.id == agent.session_id)
                    .map(|session| (agent, session))
            })
            .filter(|(agent, session)| {
                work_agent_ref_authenticates_session(agent, session)
                    && session_matches_project_state(session, &project_root)
                    && path_matches(&session.worktree_path, &worktree_path)
                    && gwt::launch_runtime::normalize_branch_name(session.branch.trim()) == branch
                    && session.linked_issue_number == Some(owner.number)
                    && session.execution_binding.as_ref().is_none_or(|binding| {
                        binding.schema_version
                            == gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                            && binding.session_id == session.id
                            && gwt_core::repo_hash::detect_repo_hash(&worktree_path)
                                .is_some_and(|repo_hash| repo_hash.as_str() == binding.repo_hash)
                            && binding.owner_number == owner.number
                            && binding.owner_kind == owner.kind.as_str()
                    })
            })
            .max_by_key(|(agent, _)| agent.updated_at)
            .map(|(_, session)| session);
        let (projected_agent_id, work_agent_session_id, launch_seed) =
            if let Some(source_session) = source_session {
                let projected_agent_id = source_session.agent_id.clone();
                let work_agent_session_id = Some(source_session.id.clone());
                (
                    projected_agent_id,
                    work_agent_session_id,
                    ContinueWorkLaunchSeed::DurableSession(Box::new(source_session)),
                )
            } else {
                let (projected_agent_id, projected_display_name) =
                    projection_only_continue_agent(item)?;
                (
                    projected_agent_id.clone(),
                    None,
                    ContinueWorkLaunchSeed::WorkProjection {
                        agent_id: projected_agent_id,
                        display_name: projected_display_name,
                        branch: branch.clone(),
                    },
                )
            };
        let resume_context = WorkspaceResumeContext {
            title: non_empty_workspace_text(Some(&item.title)),
            owner: non_empty_workspace_text(item.owner.as_deref())
                .or_else(|| Some(format!("Issue #{}", owner.number))),
            summary: non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        };
        Ok(ContinueWorkTarget {
            project_root,
            worktree_path,
            work_branch: Some(branch),
            work_agent_id: Some(projected_agent_id),
            work_agent_session_id,
            launch_seed,
            owner,
            resume_context,
        })
    }

    fn resolve_continue_work_target_for_durable_operation(
        &self,
        _context: &ContinueWorkInput<'_>,
        operation_id: &str,
    ) -> Result<Option<ContinueWorkTarget>, ContinueWorkFailure> {
        if !self.input.is_git || self.input.migration_pending {
            return Ok(None);
        }
        let project_root =
            gwt::launch_runtime::normalize_recent_project_path(self.input.project_root);
        let sessions =
            gwt_agent::session_ledger::load_sessions(self.input.sessions_dir).map_err(|_| {
                ContinueWorkFailure::failed(
                    "session_state_unavailable",
                    "Durable Session metadata could not be read for reconciliation.",
                    true,
                )
            })?;
        let mut matches = Vec::new();
        for source_session in sessions {
            if gwt_agent::validate_session_id_path_component(&source_session.id).is_err() {
                continue;
            }
            if !session_matches_project_state(&source_session, &project_root) {
                continue;
            }
            let Some(owner_number) = source_session.linked_issue_number else {
                continue;
            };
            let Ok(worktree_path) = dunce::canonicalize(&source_session.worktree_path) else {
                continue;
            };
            if !worktree_matches_project_state(&worktree_path, &project_root) {
                continue;
            }
            let binding_owner = source_session
                .execution_binding
                .as_ref()
                .and_then(|binding| {
                    if binding.schema_version
                        != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                        || binding.session_id != source_session.id
                        || binding.owner_number != owner_number
                    {
                        return None;
                    }
                    let kind = match binding.owner_kind.as_str() {
                        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
                        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                        _ => return None,
                    };
                    Some(gwt::cli::execution_state::ExecutionOwnerKey {
                        kind,
                        number: owner_number,
                    })
                });
            let authority_owner =
                match gwt::cli::execution_state::current_generation_owner(&worktree_path) {
                    Ok(Some(owner)) => Some(owner),
                    Ok(None) => match gwt::cli::execution_state::recovery_projection_owner_hint(
                        &worktree_path,
                    ) {
                        Ok(owner) => owner,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
            let owner = match (binding_owner, authority_owner) {
                (Some(binding), Some(authority)) if binding == authority => binding,
                (Some(_), Some(_)) => continue,
                (Some(binding), None) => binding,
                (None, Some(authority)) if authority.number == owner_number => authority,
                (None, Some(_)) => continue,
                (None, None) => gwt::cli::execution_state::ExecutionOwnerKey {
                    kind: gwt::cli::execution_state::detect_owner_kind(&project_root, owner_number),
                    number: owner_number,
                },
            };
            let successor = gwt::cli::execution_state::continuation_attempt_for_operation(
                &worktree_path,
                owner,
                operation_id,
            )
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The continuation operation could not be read safely: {error}"
                ))
            })?;
            let takeover = gwt::cli::execution_state::generation_takeover_attempt_for_operation(
                &worktree_path,
                owner,
                operation_id,
            )
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The takeover operation could not be read safely: {error}"
                ))
            })?;
            let (correlated_public_id, is_predecessor, is_candidate) = match (&successor, &takeover)
            {
                (Some(attempt), None) => (
                    attempt.predecessor.initial_session_id == source_session.id
                        || attempt.request.initial_session_id == source_session.id,
                    attempt.predecessor.initial_session_id == source_session.id,
                    attempt.request.initial_session_id == source_session.id,
                ),
                (None, Some(attempt)) => (
                    attempt.request.from_session_id == source_session.id
                        || attempt.request.to_session_id == source_session.id,
                    attempt.request.from_session_id == source_session.id,
                    attempt.request.to_session_id == source_session.id,
                ),
                (None, None) => (false, false, false),
                (Some(_), Some(_)) => {
                    return Err(ContinueWorkFailure::conflict(
                        "The operation id is bound to conflicting durable continuation attempts.",
                    ));
                }
            };
            if !correlated_public_id {
                continue;
            }
            let current_branch = canonical_continue_work_branch(&worktree_path)?;
            let session_branch =
                gwt::launch_runtime::normalize_branch_name(source_session.branch.trim());
            if current_branch != session_branch {
                return Err(ContinueWorkFailure::conflict(
                    "A correlated continuation Session belongs to a different branch.",
                ));
            }
            let live_candidate_agent_id = if is_candidate {
                let mut live_agent_id = None;
                for active in self
                    .view
                    .panes
                    .iter()
                    .map(|pane| pane.active)
                    .filter(|active| {
                        active.session_id == source_session.id
                            && path_matches(&active.worktree_path, &worktree_path)
                            && self.view.pane_is_live(&active.window_id)
                    })
                {
                    let Some(agent_id) = gwt_agent::resolve_agent_id(active.agent_id.trim()) else {
                        return Err(ContinueWorkFailure::conflict(
                            "A live continuation candidate has no canonical Agent identity.",
                        ));
                    };
                    if live_agent_id
                        .as_ref()
                        .is_some_and(|expected| expected != &agent_id)
                    {
                        return Err(ContinueWorkFailure::conflict(
                            "Live continuation candidates disagree on Agent identity.",
                        ));
                    }
                    live_agent_id = Some(agent_id);
                }
                live_agent_id
            } else {
                None
            };
            let trusted_agent_id = if is_predecessor {
                Some(source_session.agent_id.clone())
            } else {
                live_candidate_agent_id
            };
            matches.push((
                ContinueWorkTarget {
                    project_root: project_root.clone(),
                    worktree_path,
                    work_branch: Some(current_branch),
                    work_agent_id: Some(source_session.agent_id.clone()),
                    work_agent_session_id: Some(source_session.id.clone()),
                    launch_seed: ContinueWorkLaunchSeed::DurableSession(Box::new(source_session)),
                    owner,
                    resume_context: WorkspaceResumeContext {
                        title: None,
                        owner: Some(format!("{} #{owner_number}", owner.kind.as_str())),
                        summary: None,
                        next_action: None,
                    },
                },
                trusted_agent_id,
                is_predecessor,
            ));
        }
        if matches.is_empty() {
            return Ok(None);
        }
        let mut trusted_agent_ids = matches
            .iter()
            .filter_map(|(_, trusted_agent_id, _)| trusted_agent_id.as_ref());
        let Some(expected_agent_id) = trusted_agent_ids.next().cloned() else {
            return Err(ContinueWorkFailure::conflict(
                "The durable continuation candidate has no independent Agent identity evidence.",
            ));
        };
        if trusted_agent_ids.any(|agent_id| agent_id != &expected_agent_id)
            || matches.iter().any(|(candidate, _, _)| {
                candidate.work_agent_id.as_ref() != Some(&expected_agent_id)
            })
        {
            return Err(ContinueWorkFailure::conflict(
                "The durable continuation Sessions disagree on Agent identity.",
            ));
        }
        for (candidate, _, _) in &mut matches {
            candidate.work_agent_id = Some(expected_agent_id.clone());
        }
        let target_index = matches
            .iter()
            .position(|(_, _, is_predecessor)| *is_predecessor)
            .or_else(|| {
                matches
                    .iter()
                    .position(|(_, trusted_agent_id, _)| trusted_agent_id.is_some())
            })
            .expect("trusted durable continuation target");
        let (target, _, _) = matches.swap_remove(target_index);
        // A successor attempt intentionally names both its predecessor and candidate Sessions.
        // They are one recovery target when they converge on the same durable authority.
        if matches.iter().all(|(candidate, _, _)| {
            candidate.owner == target.owner
                && path_matches(&candidate.project_root, &target.project_root)
                && path_matches(&candidate.worktree_path, &target.worktree_path)
                && candidate.work_branch == target.work_branch
                && candidate.work_agent_id == target.work_agent_id
        }) {
            Ok(Some(target))
        } else {
            Err(ContinueWorkFailure::conflict(
                "The durable continuation operation resolves to multiple execution authorities.",
            ))
        }
    }

    fn durable_continue_work_attempt(
        &self,
        target: &ContinueWorkTarget,
        operation_id: &str,
    ) -> Result<Option<DurableContinueWorkAttempt>, ContinueWorkFailure> {
        let successor = gwt::cli::execution_state::continuation_attempt_for_operation(
            &target.worktree_path,
            target.owner,
            operation_id,
        )
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The continuation operation could not be read safely: {error}"
            ))
        })?;
        let takeover = gwt::cli::execution_state::generation_takeover_attempt_for_operation(
            &target.worktree_path,
            target.owner,
            operation_id,
        )
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The takeover operation could not be read safely: {error}"
            ))
        })?;
        match (successor, takeover) {
            (Some(_), Some(_)) => Err(ContinueWorkFailure::conflict(
                "The operation id is bound to conflicting durable continuation attempts.",
            )),
            (Some(attempt), None) => Ok(Some(DurableContinueWorkAttempt::Successor(Box::new(
                attempt,
            )))),
            (None, Some(attempt)) => Ok(Some(DurableContinueWorkAttempt::Takeover(Box::new(
                attempt,
            )))),
            (None, None) => Ok(None),
        }
    }

    fn exact_durable_continue_work_candidate(
        &self,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Result<gwt_agent::Session, ContinueWorkFailure> {
        let candidate_session_id = attempt.candidate_session_id();
        gwt_agent::validate_session_id_path_component(candidate_session_id).map_err(|_| {
            ContinueWorkFailure::conflict(
                "The committed continuation candidate Session id is not canonical.",
            )
        })?;
        let candidate_path = self
            .input
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        if candidate_path.file_stem().and_then(|value| value.to_str()) != Some(candidate_session_id)
        {
            return Err(ContinueWorkFailure::conflict(
                "The committed continuation candidate filename is not canonical.",
            ));
        }
        let session = gwt_agent::Session::load_and_migrate(&candidate_path).map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The committed continuation candidate Session could not be read exactly: {error}"
            ))
        })?;
        let binding = session.execution_binding.as_ref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation candidate Session has no execution binding.",
            )
        })?;
        let expected_agent_id = target.work_agent_id.as_ref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation target has no exact Agent identity.",
            )
        })?;
        let current_branch = canonical_continue_work_branch(&target.worktree_path)?;
        let target_branch = target.work_branch.as_deref().ok_or_else(|| {
            ContinueWorkFailure::conflict(
                "The committed continuation target has no canonical branch.",
            )
        })?;
        let session_branch = gwt::launch_runtime::normalize_branch_name(session.branch.trim());
        let repo_hash = gwt_core::repo_hash::detect_repo_hash(&target.worktree_path)
            .map(|value| value.to_string());
        let binding_matches = attempt
            .candidate_binding_matches(&target.worktree_path, target.owner, binding)
            .map_err(|error| {
                ContinueWorkFailure::conflict(format!(
                    "The committed continuation candidate binding could not be verified: {error}"
                ))
            })?;
        let live_agent_matches = self
            .view
            .panes
            .iter()
            .map(|pane| pane.active)
            .filter(|active| {
                active.session_id == candidate_session_id
                    && path_matches(&active.worktree_path, &target.worktree_path)
                    && self.view.pane_is_live(&active.window_id)
            })
            .all(|active| {
                gwt_agent::resolve_agent_id(active.agent_id.trim()).as_ref()
                    == Some(expected_agent_id)
            });
        if session.id != candidate_session_id
            || !session_matches_project_state(&session, &target.project_root)
            || !path_matches(&session.worktree_path, &target.worktree_path)
            || session.linked_issue_number != Some(target.owner.number)
            || repo_hash.as_deref() != session.repo_hash.as_deref()
            || repo_hash.as_deref() != Some(binding.repo_hash.as_str())
            || current_branch != target_branch
            || session_branch != current_branch
            || &session.agent_id != expected_agent_id
            || !live_agent_matches
            || binding.owner_kind != target.owner.kind.as_str()
            || binding.owner_number != target.owner.number
            || !binding_matches
        {
            return Err(ContinueWorkFailure::conflict(
                "The committed continuation candidate no longer matches its exact authority.",
            ));
        }
        Ok(session)
    }

    fn validate_durable_continue_work_candidate_if_present(
        &self,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> Result<(), ContinueWorkFailure> {
        let candidate_session_id = attempt.candidate_session_id();
        gwt_agent::validate_session_id_path_component(candidate_session_id).map_err(|_| {
            ContinueWorkFailure::conflict(
                "The durable continuation candidate Session id is not canonical.",
            )
        })?;
        let candidate_path = self
            .input
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        match gwt_agent::inspect_session_path(&candidate_path) {
            gwt_agent::SessionPathState::Missing => Ok(()),
            gwt_agent::SessionPathState::Present(_) => self
                .exact_durable_continue_work_candidate(target, attempt)
                .map(|_| ()),
            gwt_agent::SessionPathState::Error(error) => {
                Err(ContinueWorkFailure::conflict(format!(
            "The durable continuation candidate Session could not be inspected safely: {error}"
        )))
            }
        }
    }

    fn reconcile_aborted_continue_work_attempt(
        &self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        target: &ContinueWorkTarget,
        attempt: &DurableContinueWorkAttempt,
    ) -> ContinueWorkAction {
        let context = &self.input;
        if let Err(failure) =
            self.validate_durable_continue_work_candidate_if_present(target, attempt)
        {
            return self.uncached_failure_action(client_id, operation_id, work_id, failure);
        }
        let candidate_session_id = attempt.candidate_session_id();
        let candidate_path = self
            .input
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"));
        match gwt_agent::inspect_session_path(&candidate_path) {
            gwt_agent::SessionPathState::Present(_) => {
                let session = match self.exact_durable_continue_work_candidate(target, attempt) {
                    Ok(session) => session,
                    Err(failure) => {
                        return self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            failure,
                        )
                    }
                };
                let session_identity =
                    match gwt_agent::SessionExecutionIdentity::from_session(&session) {
                        Ok(Some(identity)) => identity,
                        _ => {
                            return self.uncached_failure_action(
                                client_id,
                                operation_id,
                                work_id,
                                ContinueWorkFailure::conflict(
                                    "The aborted candidate Session identity is not canonical.",
                                ),
                            )
                        }
                    };
                let cleanup = gwt::cli::execution_state::remove_exact_session_with_owner_lease(
                    &target.worktree_path,
                    target.owner,
                    self.input.sessions_dir,
                    &session_identity,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                match cleanup {
                    Ok(true) => {}
                    Ok(false) => {
                        return self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::conflict(
                                "The aborted candidate Session changed before exact cleanup.",
                            ),
                        )
                    }
                    Err(error) => {
                        return self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                format!(
                                "The aborted candidate Session could not be removed safely: {error}"
                            ),
                                true,
                            ),
                        )
                    }
                }
            }
            gwt_agent::SessionPathState::Missing => {
                invoke_missing_session_cleanup_hook(candidate_session_id);
                let cleanup = gwt::cli::execution_state::commit_if_session_missing_with_owner_lease(
                    &target.worktree_path,
                    target.owner,
                    self.input.sessions_dir,
                    candidate_session_id,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                match cleanup {
                    Ok(true) => {}
                    Ok(false) => {
                        return self.uncached_failure_action(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The aborted candidate Session materialized before missing-state cleanup.",
                        ),
                    );
                    }
                    Err(error) => {
                        return self.uncached_failure_action(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::failed(
                            "continuation_reconciliation_required",
                            format!(
                                "The aborted continuation Work transaction could not be rejected safely: {error}"
                            ),
                            true,
                        ),
                    );
                    }
                }
            }
            gwt_agent::SessionPathState::Error(error) => {
                return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        format!("The aborted candidate Session could not be read safely: {error}"),
                        true,
                    ),
                );
            }
        }

        self.failure_action(
            context,
            client_id,
            operation_id,
            work_id,
            ContinueWorkFailure::failed(
                "continuation_aborted",
                "This continuation attempt was aborted. Start Continue work again.",
                false,
            ),
        )
    }

    fn reconcile_durable_continue_work_attempt(
        &self,
        client_id: &str,
        operation_id: String,
        work_id: String,
        target: &ContinueWorkTarget,
        attempt: DurableContinueWorkAttempt,
    ) -> ContinueWorkAction {
        let context = &self.input;
        if attempt.work_id() != Some(work_id.as_str()) {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "operation_conflict",
                    "This operation id is already bound to another Work.",
                    false,
                ),
            );
        }
        if attempt.source().is_none_or(|source| {
            !matches!(source, "continue-work:resume" | "continue-work:handoff")
        }) {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(
                    "The durable continuation source is missing or unsupported.",
                ),
            );
        }

        match attempt.status() {
            DurableContinueWorkAttemptStatus::Aborted => {
                return self.reconcile_aborted_continue_work_attempt(
                    client_id,
                    operation_id,
                    work_id,
                    target,
                    &attempt,
                );
            }
            DurableContinueWorkAttemptStatus::Prepared => {
                if let Err(failure) =
                    self.validate_durable_continue_work_candidate_if_present(target, &attempt)
                {
                    return self.uncached_failure_action(client_id, operation_id, work_id, failure);
                }
                let candidate_session_id = attempt.candidate_session_id().to_string();
                match self.classify_nonlocal_active_owner_liveness(&candidate_session_id) {
                    ActiveOwnerLiveness::Unknown => {
                        return self.uncached_failure_action(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::failed(
                            "continuation_reconciliation_required",
                            "A prior continuation candidate may still be live; retry after its launch is reconciled.",
                            true,
                        ),
                    );
                    }
                    ActiveOwnerLiveness::Stale(_) => {}
                }

                let exact_candidate =
                    match self.exact_durable_continue_work_candidate(target, &attempt) {
                        Ok(session) => session,
                        Err(failure) => {
                            return self.uncached_failure_action(
                                client_id,
                                operation_id,
                                work_id,
                                failure,
                            )
                        }
                    };
                let session_identity =
                    match gwt_agent::SessionExecutionIdentity::from_session(&exact_candidate) {
                        Ok(Some(identity)) => identity,
                        _ => {
                            return self.uncached_failure_action(
                                client_id,
                                operation_id,
                                work_id,
                                ContinueWorkFailure::conflict(
                                    "The Prepared continuation Session identity is not canonical.",
                                ),
                            )
                        }
                    };
                let cleanup = attempt.abort_and_remove_exact_session(
                    &target.worktree_path,
                    target.owner,
                    "owning Host crashed before authenticated Ready",
                    self.input.sessions_dir,
                    &session_identity,
                    || {
                        reject_continue_work_workspace_commit(
                            &target.project_root,
                            &target.worktree_path,
                            &operation_id,
                        )
                    },
                );
                return match cleanup {
                Ok(true) => self.failure_action(context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_aborted",
                        "This continuation attempt was aborted. Start Continue work again.",
                        false,
                    ),
                ),
                Ok(false) => self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The Prepared continuation candidate changed before exact cleanup.",
                    ),
                ),
                Err(error) => {
                    match self.durable_continue_work_attempt(target, &operation_id) {
                        Ok(Some(latest))
                            if latest.status()
                                == DurableContinueWorkAttemptStatus::Activated =>
                        {
                            self.reconcile_durable_continue_work_attempt(
                                client_id,
                                operation_id,
                                work_id,
                                target,
                                latest,
                            )
                        }
                        _ => self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            ContinueWorkFailure::failed(
                                "continuation_reconciliation_required",
                                format!(
                                    "The stale Prepared continuation could not commit exact cleanup: {error}"
                                ),
                                true,
                            ),
                        ),
                    }
                }
            };
            }
            DurableContinueWorkAttemptStatus::Activated => {}
        }

        if let Err(failure) = self.exact_durable_continue_work_candidate(target, &attempt) {
            return self.uncached_failure_action(client_id, operation_id, work_id, failure);
        }
        if let Err(error) = attempt.repair_activation(&target.worktree_path, target.owner) {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The committed continuation generation could not be repaired: {error}"
                )),
            );
        }
        invoke_durable_continue_work_post_repair_hook();
        let exact_candidate = match self.exact_durable_continue_work_candidate(target, &attempt) {
            Ok(session) => session,
            Err(failure) => {
                return self.uncached_failure_action(client_id, operation_id, work_id, failure)
            }
        };
        let exact_session_identity =
            match gwt_agent::SessionExecutionIdentity::from_session(&exact_candidate) {
                Ok(Some(identity)) => identity,
                _ => {
                    return self.uncached_failure_action(
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The committed continuation Session identity is not canonical.",
                        ),
                    )
                }
            };
        invoke_durable_continue_work_pre_work_commit_hook();
        let candidate_session_id = attempt.candidate_session_id().to_string();
        let leased_readback =
            gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                self.input.sessions_dir,
                &exact_session_identity,
                || {
                    match resolve_split_workspace_state_external_commit_with_retry_hint(
                    &target.project_root,
                    &target.worktree_path,
                    &operation_id,
                    gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
                ) {
                    Ok((
                        gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
                        _,
                    )) => {}
                    Ok((
                        gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Busy,
                        Some(retry_hint),
                    )) => {
                        return Err(ContinueWorkFailure::failed(
                            "continuation_reconciliation_required",
                            retry_hint,
                            true,
                        ));
                    }
                    Ok((resolution, _)) => {
                        return Err(ContinueWorkFailure::conflict(format!(
                            "The committed continuation has no matching Work transaction: {resolution:?}"
                        )));
                    }
                    Err(error) => {
                        return Err(ContinueWorkFailure::conflict(format!(
                            "The committed continuation Work transaction could not be repaired: {error}"
                        )));
                    }
                }

                    let current = match gwt::cli::execution_state::current_execution_binding(
                        &target.worktree_path,
                        target.owner,
                    ) {
                        Ok(Some(binding)) if attempt.activated_identity_matches(&binding) => {
                            binding
                        }
                        _ => {
                            return Err(ContinueWorkFailure::conflict(
                            "The committed continuation is no longer the current execution binding.",
                        ));
                        }
                    };
                    if !exact_candidate
                        .execution_binding
                        .as_ref()
                        .is_some_and(|binding| binding.identity == current)
                    {
                        return Err(ContinueWorkFailure::conflict(
                            "The committed continuation Session binding could not be read back.",
                        ));
                    }
                    let projection_matches = gwt::cli::execution_state::load(&target.worktree_path)
                        .ok()
                        .flatten()
                        .is_some_and(|record| {
                            record.owner_kind == target.owner.kind
                                && record.owner_number == target.owner.number
                                && record.status
                                    == gwt::cli::execution_state::ExecutionControlStatus::Active
                                && record.primary_session_id == candidate_session_id
                        });
                    let work_matches = gwt_core::workspace_projection::load_workspace_work_items(
                        &target.project_root,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|projection| {
                        let matching = projection
                            .work_items
                            .iter()
                            .filter(|item| item.id == work_id)
                            .collect::<Vec<_>>();
                        matching.len() == 1
                            && !matching[0].discarded
                            && matching[0].status_category
                                == gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                            && projection_continue_authority_matches(
                                matching[0],
                                &target.project_root,
                                target.owner,
                                &target.worktree_path,
                                &exact_candidate.branch,
                                &exact_candidate.agent_id,
                                Some(&candidate_session_id),
                                true,
                            )
                    });
                    if !projection_matches || !work_matches {
                        return Err(ContinueWorkFailure::conflict(
                        "The committed continuation generation and Work projection do not agree.",
                    ));
                    }
                    Ok((current, exact_candidate))
                },
            );
        let (current, durable) = match leased_readback {
            Ok(Some(Ok(readback))) => readback,
            Ok(Some(Err(failure))) => {
                return self.uncached_failure_action(client_id, operation_id, work_id, failure)
            }
            Ok(None) => {
                return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The committed continuation Session changed before Work publication.",
                    ),
                )
            }
            Err(error) => {
                return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The committed continuation lease could not be acquired: {error}"
                    )),
                )
            }
        };
        let outcome = attempt
            .outcome()
            .expect("validated continuation source has one typed outcome");
        let local = self
            .view
            .panes
            .iter()
            .map(|pane| (pane.id, pane.active))
            .find(|(window_id, active)| {
                active.session_id == candidate_session_id && self.view.pane_is_live(window_id)
            })
            .map(|(window_id, active)| (window_id.to_string(), active.clone()));
        let Some((window_id, active)) = local else {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The committed continuation has no live exact pane on this Host.",
                    true,
                ),
            );
        };
        let durable_binding = durable.execution_binding.clone();
        if !self.ensure_local_active_execution_authority(
            &window_id,
            &active,
            target.owner,
            &current,
            durable,
        ) {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The live continuation pane could not restore exact active Host authority.",
                    true,
                ),
            );
        }
        let Some(durable_binding) = durable_binding else {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The live continuation pane has no durable execution binding.",
                    true,
                ),
            );
        };
        let predecessor_evidence =
            attempt.predecessor_evidence(&target.worktree_path, target.owner);
        let Ok((predecessor_session_id, predecessor_binding)) = predecessor_evidence else {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The predecessor fence evidence could not be reconstructed.",
                    true,
                ),
            );
        };
        let active_probe = gwt::probe_authenticated_execution_binding(
            &target.project_root,
            &candidate_session_id,
            &durable_binding,
            "continue-work-reconciler",
            gwt::AgentExecutionBindingProbeRequest {
                schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                operation_id: operation_id.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
            },
        );
        if active_probe.as_ref().is_err()
            || active_probe
                .as_ref()
                .is_ok_and(|receipt| receipt.execution_binding != current)
            || gwt::cli::execution_state::current_active_execution_binding_matches(
                &target.worktree_path,
                target.owner,
                &predecessor_session_id,
                &predecessor_binding,
            )
            .unwrap_or(true)
        {
            return self.uncached_failure_action(
            client_id,
            operation_id,
            work_id,
            ContinueWorkFailure::failed(
                "continuation_reconciliation_required",
                "The live continuation pane could not prove active authority and predecessor fencing.",
                true,
            ),
        );
        }
        ContinueWorkAction::Replay {
            outcome: CachedContinueWorkOutcome {
                work_id,
                outcome,
                message: None,
                error_code: None,
                retryable: false,
            },
            focus: Some(window_id),
            completed_window: None,
            clear_operation: true,
            refresh_projection: true,
        }
    }

    fn ensure_local_active_execution_authority(
        &self,
        window_id: &str,
        active: &ActiveAgentSession,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        current: &gwt_agent::ExecutionBindingIdentity,
        mut durable: gwt_agent::Session,
    ) -> bool {
        let Some(issuer) = self.view.issuer.cloned() else {
            return false;
        };
        let Some(token) = self
            .view
            .pane(window_id)
            .and_then(|pane| pane.token)
            .map(str::to_owned)
        else {
            return false;
        };
        let (binding, repaired) = match durable.execution_binding.clone() {
            Some(binding) if binding.identity == *current => (binding, false),
            Some(_) => return false,
            None => {
                let unbound_snapshot = durable.clone();
                let repo_hash = durable
                    .repo_hash
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        gwt_core::repo_hash::detect_repo_hash(&active.worktree_path)
                            .map(|value| value.to_string())
                    });
                let Some(repo_hash) = repo_hash else {
                    return false;
                };
                let binding = gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: active.session_id.clone(),
                    repo_hash,
                    owner_kind: owner.kind.as_str().to_string(),
                    owner_number: owner.number,
                    identity: current.clone(),
                    capability_generation: 1,
                };
                if durable
                    .set_execution_binding(Some(binding.clone()))
                    .is_err()
                    || durable
                        .save_if_unchanged(self.input.sessions_dir, &unbound_snapshot)
                        .ok()
                        != Some(true)
                {
                    record_binding_repair_outcome(
                        &active.worktree_path,
                        new_binding_repair_outcome_record(
                            &binding,
                            owner,
                            BindingRepairOutcome::Failed {
                                cause: BindingRepairFailureCause::SessionPersistenceFailed,
                                observed_generation_id: None,
                            },
                        ),
                    );
                    return false;
                }
                (binding, true)
            }
        };
        if !issuer.active_token_is_current(&token, &binding)
            && issuer.promote_inspection(&token, &binding).is_err()
        {
            if repaired {
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::CapabilityPromotionFailed,
                            observed_generation_id: None,
                        },
                    ),
                );
            }
            return false;
        }
        if !repaired {
            return issuer.active_token_is_current(&token, &binding)
                && gwt::cli::execution_state::current_active_execution_binding_matches(
                    &active.worktree_path,
                    owner,
                    &active.session_id,
                    &binding.identity,
                )
                .unwrap_or(false);
        }

        let project_root = match gwt::validated_project_state_root_for_session_recovery(&durable) {
            Ok(project_root) => project_root,
            Err(error) => {
                tracing::warn!(
                    session_id = %active.session_id,
                    error = %error,
                    "repaired execution binding has no valid repository Project State anchor"
                );
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::ProjectStateAnchorInvalid,
                            observed_generation_id: None,
                        },
                    ),
                );
                return false;
            }
        };
        let request = gwt::AgentExecutionBindingProbeRequest {
            schema_version: gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: BINDING_REPAIR_OPERATION_ID.to_string(),
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        let receipt = match gwt::probe_authenticated_execution_binding(
            &project_root,
            &active.session_id,
            &binding,
            BINDING_REPAIR_OPERATION_ID,
            request.clone(),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                tracing::warn!(
                    session_id = %active.session_id,
                    error = %error,
                    "repaired execution binding Host probe failed"
                );
                record_binding_repair_outcome(
                    &active.worktree_path,
                    new_binding_repair_outcome_record(
                        &binding,
                        owner,
                        BindingRepairOutcome::Failed {
                            cause: BindingRepairFailureCause::ProbeTransportFailed,
                            observed_generation_id: None,
                        },
                    ),
                );
                return false;
            }
        };
        if !repaired_binding_probe_receipt_matches(
            &active.session_id,
            &binding,
            &request,
            Some(&receipt),
        ) {
            tracing::warn!(
                session_id = %active.session_id,
                "repaired execution binding failed exact read-after-write authority probe"
            );
            record_binding_repair_outcome(
                &active.worktree_path,
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Failed {
                        cause: BindingRepairFailureCause::ProbeReceiptMismatch,
                        observed_generation_id: Some(
                            receipt.execution_binding.generation_id.clone(),
                        ),
                    },
                ),
            );
            return false;
        }
        let authority_matches = issuer.active_token_is_current(&token, &binding)
            && gwt::cli::execution_state::current_active_execution_binding_matches(
                &active.worktree_path,
                owner,
                &active.session_id,
                &binding.identity,
            )
            .unwrap_or(false);
        if !authority_matches {
            return record_binding_repair_outcome(
                &active.worktree_path,
                new_binding_repair_outcome_record(
                    &binding,
                    owner,
                    BindingRepairOutcome::Failed {
                        cause: BindingRepairFailureCause::ActiveAuthorityMismatch,
                        observed_generation_id: Some(
                            receipt.execution_binding.generation_id.clone(),
                        ),
                    },
                ),
            );
        }
        record_binding_repair_outcome(
            &active.worktree_path,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Succeeded {
                    host_instance_id: receipt.host_instance_id,
                    receipt_generation_id: receipt.execution_binding.generation_id,
                },
            ),
        )
    }

    fn reconcile_activated_pending_continue_work(
        &self,
        window_id: &str,
        pending: &PendingContinueWork,
    ) -> Option<ContinueWorkAction> {
        if !self.view.pane_is_live(window_id) {
            return None;
        }
        let active = self.view.pane(window_id)?.active;
        if !continue_work_active_readback_matches(
            pending,
            active,
            self.view.pane(window_id)?.token?,
            self.view.issuer?,
            self.input.sessions_dir,
        ) {
            return None;
        }
        Some(ContinueWorkAction::Replay {
            outcome: CachedContinueWorkOutcome {
                work_id: pending.work_id.clone(),
                outcome: pending.outcome,
                message: None,
                error_code: None,
                retryable: false,
            },
            focus: None,
            completed_window: Some(window_id.to_string()),
            clear_operation: false,
            refresh_projection: true,
        })
    }

    fn continue_work_action(
        &self,
        client_id: &str,
        operation_id: String,
        work_id: String,
    ) -> ContinueWorkAction {
        let context = &self.input;
        if !canonical_public_id(&operation_id, 256) || !canonical_public_id(&work_id, 512) {
            return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "invalid_request",
                    "Continue work request identity is invalid.",
                    false,
                ),
            );
        }
        if let Some(cached) = self.view.outcomes.get(&operation_id).cloned() {
            if cached.work_id != work_id {
                return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "operation_conflict",
                        "This operation id is already bound to another Work.",
                        false,
                    ),
                );
            }
            return self.replay_action(context, client_id, operation_id, cached);
        }
        if let Some((window_id, pending)) = self
            .view
            .pending
            .iter()
            .map(|entry| (entry.id, entry.pending))
            .find(|(_, pending)| pending.operation_id == operation_id)
            .map(|(window_id, pending)| (window_id.to_string(), pending.clone()))
        {
            if pending.work_id != work_id {
                return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "operation_conflict",
                        "This operation id is already bound to another Work.",
                        false,
                    ),
                );
            }
            if !pending_execution_is_activated(&pending) {
                return ContinueWorkAction::Pending { window_id };
            }
        }
        if self
            .view
            .pending
            .iter()
            .map(|entry| (entry.id, entry.pending))
            .any(|(_, pending)| pending.work_id == work_id && pending.operation_id != operation_id)
        {
            return self.failure_action(
                context,
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continue_work_in_progress",
                    "Continue work is already in progress for this Work.",
                    true,
                ),
            );
        }

        let target = match self.resolve_continue_work_target(context, &work_id) {
            Ok(target) => target,
            Err(failure) if failure.code == "work_state_unavailable" => {
                match self
                    .resolve_continue_work_target_for_durable_operation(context, &operation_id)
                {
                    Ok(Some(target)) => target,
                    Ok(None) => {
                        return self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            failure,
                        )
                    }
                    Err(reconciliation_failure) => {
                        return self.uncached_failure_action(
                            client_id,
                            operation_id,
                            work_id,
                            reconciliation_failure,
                        )
                    }
                }
            }
            Err(failure) => {
                return self.failure_action(context, client_id, operation_id, work_id, failure)
            }
        };
        // Issue #3759: Continue work is the GUI recovery route for a
        // stranded owner. Rebuild a lost worktree pointer/projection from
        // the owner ledger before the strict read so the route does not
        // refuse the exact state it exists to recover from.
        gwt::project_runtime::launch_finalization::heal_lost_generation_publication_best_effort(
            &target.worktree_path,
            target.owner,
        );
        let generation_was_missing = match gwt::cli::execution_state::load_generation_ledger(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(_)) => false,
            Ok(None) => true,
            Err(error) => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The execution generation could not be verified: {error}"
                    )),
                )
            }
        };
        let legacy_disposition = if generation_was_missing {
            match gwt::cli::execution_state::load(&target.worktree_path) {
                Ok(Some(record))
                    if record.status
                        == gwt::cli::execution_state::ExecutionControlStatus::Active =>
                {
                    let local_live = self
                        .view
                        .panes
                        .iter()
                        .map(|pane| (pane.id, pane.active))
                        .any(|(window_id, active)| {
                            active.session_id == record.primary_session_id
                                && self.view.pane_is_live(window_id)
                        });
                    if local_live
                        || matches!(
                            self.classify_nonlocal_active_owner_liveness(
                                &record.primary_session_id,
                            ),
                            ActiveOwnerLiveness::Stale(_)
                        )
                    {
                        gwt::cli::execution_state::LegacyActiveDisposition::Live
                    } else {
                        gwt::cli::execution_state::LegacyActiveDisposition::Unknown
                    }
                }
                _ => gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
            }
        } else {
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown
        };
        if let Err(error) = gwt::cli::execution_state::ensure_generation_ledger(
            &target.worktree_path,
            target.owner,
            legacy_disposition,
        ) {
            return self.failure_action(
                context,
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The current execution owner could not be classified safely: {error}"
                )),
            );
        }
        let ledger = match gwt::cli::execution_state::load_generation_ledger(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(ledger)) => ledger,
            Ok(None) => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "execution_owner_missing",
                        "No execution generation exists for this Work.",
                        false,
                    ),
                )
            }
            Err(error) => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(format!(
                        "The execution generation could not be verified: {error}"
                    )),
                )
            }
        };
        let durable_attempt = match self.durable_continue_work_attempt(&target, &operation_id) {
            Ok(attempt) => attempt,
            Err(failure) => {
                return self.failure_action(context, client_id, operation_id, work_id, failure);
            }
        };
        if let Some(attempt) = durable_attempt {
            return self.reconcile_durable_continue_work_attempt(
                client_id,
                operation_id,
                work_id,
                &target,
                attempt,
            );
        }
        let takeover_owner = match ledger.current_effective_status() {
            Some(gwt::cli::execution_state::ExecutionControlStatus::Active) => {
                let Some(record) = gwt::cli::execution_state::load(&target.worktree_path)
                    .ok()
                    .flatten()
                else {
                    return self.failure_action(
                        context,
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "The active execution projection could not be verified.",
                        ),
                    );
                };
                if let Some((window_id, active)) = self
                    .view
                    .panes
                    .iter()
                    .map(|pane| (pane.id, pane.active))
                    .find(|(window_id, active)| {
                        active.session_id == record.primary_session_id
                            && self.view.pane_is_live(window_id)
                    })
                    .map(|(window_id, active)| (window_id.to_string(), active.clone()))
                {
                    let current = gwt::cli::execution_state::current_execution_binding(
                        &target.worktree_path,
                        target.owner,
                    )
                    .ok()
                    .flatten();
                    let durable = gwt_agent::Session::load_and_migrate(
                        &self
                            .input
                            .sessions_dir
                            .join(format!("{}.toml", active.session_id)),
                    )
                    .ok();
                    if let (Some(current), Some(durable)) = (current, durable) {
                        if self.ensure_local_active_execution_authority(
                            &window_id,
                            &active,
                            target.owner,
                            &current,
                            durable,
                        ) {
                            return ContinueWorkAction::FocusExisting {
                                window_id,
                                outcome: CachedContinueWorkOutcome {
                                    work_id: work_id.clone(),
                                    outcome: gwt::ContinueWorkOutcomeKind::FocusedExisting,
                                    message: None,
                                    error_code: None,
                                    retryable: false,
                                },
                            };
                        }
                    }
                }
                match self.classify_nonlocal_active_owner_liveness(
                &record.primary_session_id,
            ) {
                ActiveOwnerLiveness::Stale(reason) => {
                    Some((record.primary_session_id, reason))
                }
                ActiveOwnerLiveness::Unknown => {
                    return self.failure_action(context,
                        client_id,
                        operation_id,
                        work_id,
                        ContinueWorkFailure::conflict(
                            "Another Host may own the active execution; its liveness could not be determined.",
                        ),
                    )
                }
            }
            }
            Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked) => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "execution_blocked",
                        "Resolve and reopen the blocked execution before continuing Work.",
                        false,
                    ),
                );
            }
            Some(gwt::cli::execution_state::ExecutionControlStatus::Completed) => None,
            None => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The current execution generation is incomplete.",
                    ),
                );
            }
        };
        let predecessor_session_id = match (
            takeover_owner.as_ref(),
            gwt::cli::execution_state::load(&target.worktree_path),
        ) {
            (Some((session_id, _)), _) => session_id.clone(),
            (None, Ok(Some(record)))
                if record.owner_kind == target.owner.kind
                    && record.owner_number == target.owner.number
                    && record.status
                        == gwt::cli::execution_state::ExecutionControlStatus::Completed =>
            {
                record.primary_session_id
            }
            _ => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The current execution projection changed before continuation.",
                    ),
                )
            }
        };
        let grok_home = match &target.launch_seed {
            ContinueWorkLaunchSeed::DurableSession(source_session)
                if source_session.agent_id == gwt_agent::AgentId::GrokBuild =>
            {
                self.active_profile_grok_home_for_continuation(
                    source_session,
                    &target.worktree_path,
                )
            }
            _ => None,
        };
        let (mut config, outcome) = continuation_launch_config(
            &target.launch_seed,
            &target.worktree_path,
            target.owner,
            grok_home.as_deref(),
        );
        let continuation_session_id = uuid::Uuid::new_v4().to_string();
        let predecessor_binding = match gwt::cli::execution_state::current_execution_binding(
            &target.worktree_path,
            target.owner,
        ) {
            Ok(Some(binding)) => binding,
            _ => {
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::conflict(
                        "The predecessor execution binding disappeared before launch.",
                    ),
                );
            }
        };
        if outcome == gwt::ContinueWorkOutcomeKind::StartedWithHandoff {
            config.args.push(handoff_context(
                &target.resume_context,
                &work_id,
                &predecessor_binding,
            ));
        }
        let requested_at = chrono::Utc::now();
        let replaces_active_generation = takeover_owner.is_some();
        let execution =
            PendingContinueWorkExecution::Successor(gwt::cli::execution_state::SuccessorRequest {
                operation_id: operation_id.clone(),
                principal_id: "gwt-host-continuation".to_string(),
                work_id: Some(work_id.clone()),
                source: match outcome {
                    gwt::ContinueWorkOutcomeKind::ContinuedConversation => {
                        "continue-work:resume".to_string()
                    }
                    _ => "continue-work:handoff".to_string(),
                },
                session_binding_id: uuid::Uuid::new_v4().to_string(),
                initial_session_id: continuation_session_id.clone(),
                entrypoint: gwt::cli::execution_state::entrypoint_from_launch(
                    config.entrypoint_args(),
                    config.session_mode == gwt_agent::SessionMode::Resume,
                ),
                requested_at,
            });
        let prepared = match &execution {
            PendingContinueWorkExecution::Successor(request) => {
                if replaces_active_generation {
                    gwt::cli::execution_state::prepare_active_continuation_successor(
                        &target.worktree_path,
                        target.owner,
                        request,
                    )
                    .map(|_| ())
                } else {
                    gwt::cli::execution_state::prepare_successor(
                        &target.worktree_path,
                        target.owner,
                        request,
                    )
                    .map(|_| ())
                }
            }
            PendingContinueWorkExecution::Takeover(request) => {
                gwt::cli::execution_state::prepare_generation_takeover(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
                .map(|_| ())
            }
        };
        if let Err(error) = prepared {
            return self.failure_action(
                context,
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(format!(
                    "The execution continuation could not be prepared: {error}"
                )),
            );
        }
        let planned_identity = match &execution {
            PendingContinueWorkExecution::Successor(request) => {
                gwt::cli::execution_state::prepared_successor_execution_binding(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
            }
            PendingContinueWorkExecution::Takeover(request) => {
                gwt::cli::execution_state::prepared_generation_takeover_execution_binding(
                    &target.worktree_path,
                    target.owner,
                    request,
                )
            }
        };
        let planned_identity = match planned_identity {
            Ok(identity) => identity,
            Err(error) => {
                if let Err(abort_error) = abort_prepared_execution(
                    &target.worktree_path,
                    target.owner,
                    &execution,
                    "prepared binding derivation failed",
                ) {
                    tracing::warn!(
                        error_kind = ?abort_error.kind(),
                        "prepared continuation binding failure could not be durably aborted"
                    );
                    return self.uncached_failure_action(
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "continuation_reconciliation_required",
                        "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                        true,
                    ),
                );
                }
                return self.failure_action(
                    context,
                    client_id,
                    operation_id,
                    work_id,
                    ContinueWorkFailure::failed(
                        "prepared_binding_failed",
                        format!("The Prepared execution binding could not be verified: {error}"),
                        true,
                    ),
                );
            }
        };
        let repo_hash = match &target.launch_seed {
            ContinueWorkLaunchSeed::DurableSession(source_session) => {
                source_session.repo_hash.clone()
            }
            ContinueWorkLaunchSeed::WorkProjection { .. } => None,
        }
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            gwt_core::repo_hash::detect_repo_hash(&target.worktree_path)
                .map(|repo_hash| repo_hash.to_string())
        });
        let Some(repo_hash) = repo_hash else {
            if let Err(abort_error) = abort_prepared_execution(
                &target.worktree_path,
                target.owner,
                &execution,
                "repository identity unavailable",
            ) {
                tracing::warn!(
                    error_kind = ?abort_error.kind(),
                    "repository identity failure could not durably abort continuation"
                );
                return self.uncached_failure_action(
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "continuation_reconciliation_required",
                    "The Prepared continuation could not prove a durable abort; retry reconciliation.",
                    true,
                ),
            );
            }
            return self.failure_action(
                context,
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::failed(
                    "repository_identity_unavailable",
                    "The repository identity could not be verified for continuation.",
                    false,
                ),
            );
        };
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: continuation_session_id,
            repo_hash,
            owner_kind: target.owner.kind.as_str().to_string(),
            owner_number: target.owner.number,
            identity: planned_identity,
            capability_generation: 1,
        };
        let readiness_nonce = uuid::Uuid::new_v4().to_string();
        config.env_vars.insert(
            gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
            readiness_nonce.clone(),
        );
        config.execution_intent =
            gwt_agent::ExecutionLaunchIntent::PreparedContinuation(binding.clone());
        let (Some(work_branch), Some(work_agent_id)) =
            (target.work_branch.clone(), target.work_agent_id.clone())
        else {
            let _ = abort_prepared_execution(
                &target.worktree_path,
                target.owner,
                &execution,
                "Work authority disappeared before continuation dispatch",
            );
            return self.failure_action(
                context,
                client_id,
                operation_id,
                work_id,
                ContinueWorkFailure::conflict(
                    "The Work authority disappeared before continuation dispatch.",
                ),
            );
        };
        let pending = PendingContinueWork {
            client_id: client_id.to_string(),
            operation_id: operation_id.clone(),
            work_id: work_id.clone(),
            project_root: target.project_root.clone(),
            worktree_path: target.worktree_path.clone(),
            owner: target.owner,
            work_branch,
            work_agent_id,
            work_agent_session_id: target.work_agent_session_id.clone(),
            execution,
            binding,
            readiness_nonce,
            outcome,
            resume_context: target.resume_context.clone(),
            predecessor_session_id,
            predecessor_binding,
        };
        ContinueWorkAction::PreparedLaunch {
            config: Box::new(config),
            pending: Box::new(pending),
        }
    }

    fn active_profile_grok_home_for_continuation(
        &self,
        source_session: &gwt_agent::Session,
        launch_cwd: &Path,
    ) -> Option<PathBuf> {
        active_profile_grok_home_for_continuation(
            self.input.profile_config_path?,
            source_session,
            launch_cwd,
        )
    }
}

/// Resolve provider state from the active profile without mutating a Session.
pub fn active_profile_grok_home_for_continuation(
    profile_config_path: &Path,
    source_session: &gwt_agent::Session,
    launch_cwd: &Path,
) -> Option<PathBuf> {
    let (effective_env, _) = gwt_agent::LaunchEnvironment::from_active_profile(
        profile_config_path,
        source_session.runtime_target,
    )
    .ok()?
    .with_project_root(launch_cwd)
    .into_parts();
    gwt_core::usage::grok::grok_home_from_env(&effective_env, launch_cwd)
}

#[derive(Debug)]
struct ContinueWorkTarget {
    project_root: PathBuf,
    worktree_path: PathBuf,
    work_branch: Option<String>,
    work_agent_id: Option<gwt_agent::AgentId>,
    work_agent_session_id: Option<String>,
    launch_seed: ContinueWorkLaunchSeed,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    resume_context: WorkspaceResumeContext,
}

#[derive(Debug)]
pub enum ContinueWorkLaunchSeed {
    DurableSession(Box<gwt_agent::Session>),
    WorkProjection {
        agent_id: gwt_agent::AgentId,
        display_name: Option<String>,
        branch: String,
    },
}

#[derive(Debug, Clone)]
enum DurableContinueWorkAttempt {
    Successor(Box<gwt::cli::execution_state::ContinuationAttempt>),
    Takeover(Box<gwt::cli::execution_state::GenerationTakeoverAttempt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurableContinueWorkAttemptStatus {
    Prepared,
    Aborted,
    Activated,
}

impl DurableContinueWorkAttempt {
    fn work_id(&self) -> Option<&str> {
        match self {
            Self::Successor(attempt) => attempt.request.work_id.as_deref(),
            Self::Takeover(attempt) => attempt.request.work_id.as_deref(),
        }
    }

    fn source(&self) -> Option<&str> {
        match self {
            Self::Successor(attempt) => Some(attempt.request.source.as_str()),
            Self::Takeover(attempt) => attempt.request.source.as_deref(),
        }
    }

    fn candidate_session_id(&self) -> &str {
        match self {
            Self::Successor(attempt) => &attempt.request.initial_session_id,
            Self::Takeover(attempt) => &attempt.request.to_session_id,
        }
    }

    fn status(&self) -> DurableContinueWorkAttemptStatus {
        match self {
            Self::Successor(attempt) => match attempt.status {
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared => {
                    DurableContinueWorkAttemptStatus::Prepared
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Aborted => {
                    DurableContinueWorkAttemptStatus::Aborted
                }
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated => {
                    DurableContinueWorkAttemptStatus::Activated
                }
            },
            Self::Takeover(attempt) => match attempt.status {
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Prepared => {
                    DurableContinueWorkAttemptStatus::Prepared
                }
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Aborted => {
                    DurableContinueWorkAttemptStatus::Aborted
                }
                gwt::cli::execution_state::GenerationTakeoverAttemptStatus::Activated => {
                    DurableContinueWorkAttemptStatus::Activated
                }
            },
        }
    }

    fn outcome(&self) -> Option<gwt::ContinueWorkOutcomeKind> {
        match self.source()? {
            "continue-work:resume" => Some(gwt::ContinueWorkOutcomeKind::ContinuedConversation),
            "continue-work:handoff" => Some(gwt::ContinueWorkOutcomeKind::StartedWithHandoff),
            _ => None,
        }
    }

    fn activated_identity_matches(&self, current: &gwt_agent::ExecutionBindingIdentity) -> bool {
        match self {
            Self::Successor(attempt) => {
                attempt
                    .activated_generation
                    .as_ref()
                    .is_some_and(|generation| {
                        generation.generation_id == current.generation_id
                            && generation.session_binding_id == current.binding_id
                    })
            }
            Self::Takeover(attempt) => attempt.activated_binding.as_ref() == Some(current),
        }
    }

    fn candidate_binding_matches(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        binding: &gwt_agent::SessionExecutionBinding,
    ) -> std::io::Result<bool> {
        let repo_hash =
            gwt_core::repo_hash::detect_repo_hash(worktree).map(|value| value.to_string());
        if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
            || binding.session_id != self.candidate_session_id()
            || binding.owner_kind != owner.kind.as_str()
            || binding.owner_number != owner.number
            || binding.capability_generation != 1
            || repo_hash.as_deref() != Some(binding.repo_hash.as_str())
        {
            return Ok(false);
        }
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::continuation_attempt_execution_binding_matches(
                    worktree,
                    owner,
                    attempt,
                    &binding.session_id,
                    &binding.identity,
                )
            }
            Self::Takeover(attempt) => {
                gwt::cli::execution_state::generation_takeover_attempt_execution_binding_matches(
                    worktree,
                    owner,
                    attempt,
                    &binding.session_id,
                    &binding.identity,
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn abort_and_remove_exact_session<F>(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
        reason: &str,
        sessions_dir: &Path,
        session_identity: &gwt_agent::SessionExecutionIdentity,
        after_abort: F,
    ) -> std::io::Result<bool>
    where
        F: FnOnce() -> std::io::Result<()>,
    {
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::abort_successor_and_remove_exact_session(
                    worktree,
                    owner,
                    &attempt.request,
                    reason,
                    sessions_dir,
                    session_identity,
                    after_abort,
                )
            }
            Self::Takeover(attempt) => {
                gwt::cli::execution_state::abort_generation_takeover_and_remove_exact_session(
                    worktree,
                    owner,
                    &attempt.request,
                    reason,
                    sessions_dir,
                    session_identity,
                    after_abort,
                )
            }
        }
    }

    fn repair_activation(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
    ) -> std::io::Result<()> {
        match self {
            Self::Successor(attempt) => {
                gwt::cli::execution_state::activate_successor(worktree, owner, &attempt.request)
                    .map(|_| ())
            }
            Self::Takeover(attempt) => gwt::cli::execution_state::activate_generation_takeover(
                worktree,
                owner,
                &attempt.request,
            )
            .map(|_| ()),
        }
    }

    fn predecessor_evidence(
        &self,
        worktree: &Path,
        owner: gwt::cli::execution_state::ExecutionOwnerKey,
    ) -> std::io::Result<(String, gwt_agent::ExecutionBindingIdentity)> {
        match self {
            Self::Successor(attempt) => Ok((
                attempt.predecessor.initial_session_id.clone(),
                gwt_agent::ExecutionBindingIdentity {
                    generation_id: attempt.predecessor.generation_id.clone(),
                    binding_id: attempt.predecessor.session_binding_id.clone(),
                    ledger_head_hash: attempt.predecessor_generation_content_hash.clone(),
                },
            )),
            Self::Takeover(attempt) => {
                let ledger = gwt::cli::execution_state::load_generation_ledger(worktree, owner)?
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "takeover generation ledger is missing",
                        )
                    })?;
                let generation = ledger
                    .generations
                    .iter()
                    .find(|generation| generation.identity.generation_id == attempt.generation_id)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "takeover predecessor generation is missing",
                        )
                    })?;
                Ok((
                    attempt.request.from_session_id.clone(),
                    gwt_agent::ExecutionBindingIdentity {
                        generation_id: attempt.generation_id.clone(),
                        binding_id: generation.identity.session_binding_id.clone(),
                        ledger_head_hash: attempt.predecessor_head_hash.clone(),
                    },
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveOwnerLiveness {
    Stale(&'static str),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderConversationAvailability {
    Present,
    Missing,
    Foreign,
    Unknown,
}

fn canonical_public_id(value: &str, max_len: usize) -> bool {
    value.trim() == value
        && !value.is_empty()
        && value.len() <= max_len
        && !value.chars().any(char::is_control)
}

pub fn sanitize_handoff_value(value: &str) -> Option<String> {
    static SECRET: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    static ABSOLUTE_PATH: OnceLock<Regex> = OnceLock::new();
    static RELATIVE_PATH: OnceLock<Regex> = OnceLock::new();

    let flattened = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut sanitized = gwt_core::process_console::redact_line(&flattened);
    sanitized = SECRET
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:authorization|token|secret|password|api[_-]?key)\s*[:=]\s*\S+")
                .expect("handoff secret regex")
        })
        .replace_all(&sanitized, "[redacted-secret]")
        .into_owned();
    sanitized = URL
        .get_or_init(|| Regex::new(r"(?i)\bhttps?://\S+").expect("handoff URL regex"))
        .replace_all(&sanitized, "[redacted-url]")
        .into_owned();
    sanitized = ABSOLUTE_PATH
        .get_or_init(|| {
            Regex::new(r"(?:[A-Za-z]:[\\/]|/)(?:[^\s/\\]+[\\/])+[^\s]*")
                .expect("handoff absolute path regex")
        })
        .replace_all(&sanitized, "[redacted-path]")
        .into_owned();
    sanitized = RELATIVE_PATH
        .get_or_init(|| {
            Regex::new(r"\b(?:[A-Za-z0-9._-]+[\\/]){2,}[A-Za-z0-9._-]*")
                .expect("handoff relative path regex")
        })
        .replace_all(&sanitized, "[redacted-path]")
        .into_owned();
    let sanitized = sanitized.trim();
    (!sanitized.is_empty()).then(|| sanitized.chars().take(600).collect())
}

pub fn handoff_context(
    context: &WorkspaceResumeContext,
    work_id: &str,
    predecessor: &gwt_agent::ExecutionBindingIdentity,
) -> String {
    fn clean(value: Option<&str>) -> Option<String> {
        value.and_then(sanitize_handoff_value)
    }

    let mut lines = vec![
        "Handoff Context".to_string(),
        "Continue the existing Work in a new conversation.".to_string(),
        format!("Work: {work_id}"),
        format!("Predecessor generation: {}", predecessor.generation_id),
        format!("Predecessor binding: {}", predecessor.binding_id),
    ];
    for (label, value) in [
        ("Title", clean(context.title.as_deref())),
        ("Owner", clean(context.owner.as_deref())),
        ("Summary", clean(context.summary.as_deref())),
        ("Next action", clean(context.next_action.as_deref())),
    ] {
        if let Some(value) = value {
            lines.push(format!("{label}: {value}"));
        }
    }
    lines.join("\n")
}

fn json_string_for_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(values) => values
            .get(key)
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                values
                    .values()
                    .find_map(|value| json_string_for_key(value, key))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| json_string_for_key(value, key)),
        _ => None,
    }
}

fn provider_conversation_cwd(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let file = std::fs::File::open(path)?;
    for line in std::io::BufReader::new(file).lines().take(64) {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(cwd) = json_string_for_key(&value, "cwd")
            .map(str::trim)
            .filter(|cwd| !cwd.is_empty())
        {
            return Ok(Some(PathBuf::from(cwd)));
        }
    }
    Ok(None)
}

pub fn provider_conversation_availability(
    session: &gwt_agent::Session,
) -> ProviderConversationAvailability {
    provider_conversation_availability_with_grok_home(
        session,
        gwt_core::usage::grok::grok_home().as_deref(),
    )
}

pub fn provider_conversation_availability_with_grok_home(
    session: &gwt_agent::Session,
    grok_home: Option<&Path>,
) -> ProviderConversationAvailability {
    let Some(conversation_id) = session.exact_resume_session_id() else {
        return ProviderConversationAvailability::Missing;
    };
    if session.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return ProviderConversationAvailability::Unknown;
    }
    let path = match session.agent_id {
        gwt_agent::AgentId::ClaudeCode => {
            let Some(home) = gwt_core::usage::claude::claude_home() else {
                return ProviderConversationAvailability::Unknown;
            };
            gwt_core::usage::claude::transcript_for_session(&home, conversation_id)
        }
        gwt_agent::AgentId::Codex => {
            let Some(home) = gwt_core::usage::codex::codex_home() else {
                return ProviderConversationAvailability::Unknown;
            };
            gwt_core::usage::codex::rollout_for_session(&home, conversation_id)
        }
        gwt_agent::AgentId::GrokBuild => {
            let Some(home) = grok_home else {
                return ProviderConversationAvailability::Unknown;
            };
            return match gwt_core::usage::grok::resumable_session_summary(
                home,
                conversation_id,
                &session.worktree_path,
            ) {
                Ok(gwt_core::usage::grok::GrokSessionStoreAvailability::Present(_)) => {
                    ProviderConversationAvailability::Present
                }
                Ok(gwt_core::usage::grok::GrokSessionStoreAvailability::Missing) => {
                    ProviderConversationAvailability::Missing
                }
                Ok(gwt_core::usage::grok::GrokSessionStoreAvailability::Foreign) => {
                    ProviderConversationAvailability::Foreign
                }
                Err(_) => ProviderConversationAvailability::Unknown,
            };
        }
        _ => return ProviderConversationAvailability::Unknown,
    };
    let Some(path) = path else {
        return ProviderConversationAvailability::Missing;
    };
    match provider_conversation_cwd(&path) {
        Ok(Some(cwd)) if path_matches(&cwd, &session.worktree_path) => {
            ProviderConversationAvailability::Present
        }
        Ok(Some(_)) => ProviderConversationAvailability::Foreign,
        Ok(None) => ProviderConversationAvailability::Foreign,
        Err(_) => ProviderConversationAvailability::Unknown,
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn configure_provider_continuation(
    config: &mut gwt_agent::LaunchConfig,
    source_session: &gwt_agent::Session,
) -> gwt::ContinueWorkOutcomeKind {
    configure_provider_continuation_with_grok_home(config, source_session, None)
}

pub fn configure_provider_continuation_with_grok_home(
    config: &mut gwt_agent::LaunchConfig,
    source_session: &gwt_agent::Session,
    grok_home: Option<&Path>,
) -> gwt::ContinueWorkOutcomeKind {
    let availability = if source_session.agent_id == gwt_agent::AgentId::GrokBuild {
        provider_conversation_availability_with_grok_home(source_session, grok_home)
    } else {
        provider_conversation_availability(source_session)
    };
    let exact_resume_is_safe = source_session.exact_resume_session_id().is_some()
        && !matches!(
            availability,
            ProviderConversationAvailability::Missing | ProviderConversationAvailability::Foreign
        );
    if exact_resume_is_safe {
        return gwt::ContinueWorkOutcomeKind::ContinuedConversation;
    }
    config.session_mode = gwt_agent::SessionMode::Normal;
    config.resume_session_id = None;
    gwt::ContinueWorkOutcomeKind::StartedWithHandoff
}

/// Build the launch config a Continue work dispatch hands to the launch worker.
///
/// Issue #3489: the continuation binding is always minted from the Work owner,
/// and `Session::set_execution_binding` refuses a binding whose owner differs
/// from the launch config's `linked_issue_number` — before the PTY starts. A
/// durable Session seed carries whatever owner linkage it happened to persist
/// (`None` after a relaunch that dropped it, or another owner entirely), so the
/// owner linkage is re-resolved from `owner` here for every seed. `owner` is the
/// single source of truth for the launch's owner linkage.
pub fn continuation_launch_config(
    seed: &ContinueWorkLaunchSeed,
    worktree_path: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    grok_home: Option<&Path>,
) -> (gwt_agent::LaunchConfig, gwt::ContinueWorkOutcomeKind) {
    let (mut config, outcome) = match seed {
        ContinueWorkLaunchSeed::DurableSession(source_session) => {
            let mut config = launch_config_from_persisted_session(source_session);
            config.working_dir = Some(worktree_path.to_path_buf());
            config.branch = Some(source_session.branch.clone());
            let outcome = configure_provider_continuation_with_grok_home(
                &mut config,
                source_session,
                grok_home,
            );
            (config, outcome)
        }
        ContinueWorkLaunchSeed::WorkProjection {
            agent_id,
            display_name,
            branch,
        } => {
            let mut config = gwt_agent::AgentLaunchBuilder::new(agent_id.clone())
                .working_dir(worktree_path.to_path_buf())
                .branch(branch.clone())
                .linked_issue_number(owner.number)
                .session_mode(gwt_agent::SessionMode::Normal)
                .build();
            if let Some(display_name) = display_name {
                config.display_name = display_name.clone();
            }
            (config, gwt::ContinueWorkOutcomeKind::StartedWithHandoff)
        }
    };
    config.linked_issue_number = Some(owner.number);
    config.launch_route = gwt_agent::LaunchRoute::Manual;
    (config, outcome)
}

pub fn session_matches_project_state(session: &gwt_agent::Session, project_root: &Path) -> bool {
    if let Some(root) = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
    {
        return path_matches(root, project_root);
    }
    let Some(project_repo_hash) = gwt_core::repo_hash::detect_repo_hash(project_root) else {
        return false;
    };
    if session.repo_hash.as_deref() != Some(project_repo_hash.as_str()) {
        return false;
    }
    let Ok(project_anchor) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    let Ok(session_anchor) = gwt_git::worktree::main_worktree_root(&session.worktree_path) else {
        return false;
    };
    path_matches(&project_anchor, &session_anchor)
}

fn canonical_continue_work_branch(worktree_path: &Path) -> Result<String, ContinueWorkFailure> {
    gwt::launch_runtime::current_git_branch(worktree_path)
        .map(|branch| gwt::launch_runtime::normalize_branch_name(branch.trim()))
        .map_err(|error| {
            ContinueWorkFailure::conflict(format!(
                "The Work branch could not be verified against its current worktree: {error}"
            ))
        })
        .and_then(|branch| {
            if branch.is_empty() {
                Err(ContinueWorkFailure::conflict(
                    "The Work branch is detached or empty and cannot be continued safely.",
                ))
            } else {
                Ok(branch)
            }
        })
}

pub fn canonical_continue_work_owner(
    project_root: &Path,
    worktree_path: &Path,
    projected: ProjectionOwnerRef,
) -> Result<gwt::cli::execution_state::ExecutionOwnerKey, ContinueWorkFailure> {
    // #3426: a same-number kind-only disagreement self-heals toward the
    // trusted authority (generation ledger, ECR hint, or cache label
    // evidence); the corrected owner is committed back to the Work
    // projection by the continuation transaction. Number mismatches stay
    // fail-closed.
    let validate = |owner: gwt::cli::execution_state::ExecutionOwnerKey| {
        if owner.number != projected.number {
            Err(ContinueWorkFailure::failed(
                "execution_owner_ambiguous",
                "The Work owner does not match its current execution authority.",
                false,
            ))
        } else {
            Ok(owner)
        }
    };
    match gwt::cli::execution_state::current_generation_owner(worktree_path) {
        Ok(Some(owner)) => validate(owner),
        Ok(None) => {
            match gwt::cli::execution_state::recovery_projection_owner_hint(worktree_path) {
                Ok(Some(owner)) => validate(owner),
                Ok(None) => {
                    // No trusted execution authority exists yet. Prefer cache
                    // label evidence; absent evidence, retain the declared
                    // kind rather than silently downgrading to Issue.
                    let kind = gwt::cli::execution_state::detect_owner_kind_evidence(
                        project_root,
                        projected.number,
                    )
                    .or(projected.declared_kind)
                    .unwrap_or(gwt::cli::execution_state::ExecutionOwnerKind::Issue);
                    validate(gwt::cli::execution_state::ExecutionOwnerKey {
                        kind,
                        number: projected.number,
                    })
                }
                Err(_) => Err(ContinueWorkFailure::conflict(
                    "The Work execution authority could not be read safely.",
                )),
            }
        }
        Err(_) => Err(ContinueWorkFailure::conflict(
            "The Work execution authority could not be read safely.",
        )),
    }
}

fn work_agent_ref_authenticates_session(
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    session: &gwt_agent::Session,
) -> bool {
    agent.session_id == session.id && work_agent_ref_authenticates_agent(agent, &session.agent_id)
}

pub fn pending_execution_is_activated(pending: &PendingContinueWork) -> bool {
    pending_execution_activation_status(pending) == Some(true)
}

pub fn abort_prepared_execution(
    worktree: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    execution: &PendingContinueWorkExecution,
    reason: &str,
) -> std::io::Result<()> {
    match execution {
        PendingContinueWorkExecution::Successor(request) => {
            gwt::cli::execution_state::abort_successor(worktree, owner, request, reason).map(|_| ())
        }
        PendingContinueWorkExecution::Takeover(request) => {
            gwt::cli::execution_state::abort_generation_takeover(worktree, owner, request, reason)
                .map(|_| ())
        }
    }
}
#[cfg(any(test, feature = "test-gh-guard"))]
type MissingSessionCleanupHook = Box<dyn FnOnce(&str) + 'static>;

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static MISSING_SESSION_CLEANUP_HOOK:
        std::cell::RefCell<Option<MissingSessionCleanupHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn set_missing_session_cleanup_hook_for_test(hook: MissingSessionCleanupHook) {
    MISSING_SESSION_CLEANUP_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

pub fn invoke_missing_session_cleanup_hook(session_id: &str) {
    #[cfg(any(test, feature = "test-gh-guard"))]
    MISSING_SESSION_CLEANUP_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(session_id);
        }
    });

    #[cfg(not(any(test, feature = "test-gh-guard")))]
    let _ = session_id;
}

#[cfg(any(test, feature = "test-gh-guard"))]
type DurableContinueWorkPostRepairHook = Box<dyn FnOnce() + 'static>;

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK:
        std::cell::RefCell<Option<DurableContinueWorkPostRepairHook>> =
            std::cell::RefCell::new(None);
    static DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK:
        std::cell::RefCell<Option<DurableContinueWorkPostRepairHook>> =
            std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn set_durable_continue_work_post_repair_hook_for_test(
    hook: DurableContinueWorkPostRepairHook,
) {
    DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn set_durable_continue_work_pre_work_commit_hook_for_test(
    hook: DurableContinueWorkPostRepairHook,
) {
    DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(any(test, feature = "test-gh-guard"))]
fn invoke_durable_continue_work_post_repair_hook() {
    DURABLE_CONTINUE_WORK_POST_REPAIR_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn invoke_durable_continue_work_post_repair_hook() {}

#[cfg(any(test, feature = "test-gh-guard"))]
fn invoke_durable_continue_work_pre_work_commit_hook() {
    DURABLE_CONTINUE_WORK_PRE_WORK_COMMIT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn invoke_durable_continue_work_pre_work_commit_hook() {}

pub use gwt::cli::execution_state::{
    BindingRepairFailureCause, BindingRepairOutcome, BindingRepairOutcomeRecord,
};
#[cfg(any(test, feature = "test-gh-guard"))]
pub const BINDING_REPAIR_OUTCOME_FILE: &str =
    gwt::cli::execution_state::BINDING_REPAIR_OUTCOME_FILE;
const BINDING_REPAIR_OPERATION_ID: &str = "continue-work-local-repair";

fn new_binding_repair_outcome_record(
    binding: &gwt_agent::SessionExecutionBinding,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    outcome: BindingRepairOutcome,
) -> BindingRepairOutcomeRecord {
    gwt::cli::execution_state::new_binding_repair_outcome_record(binding, owner, outcome)
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn load_binding_repair_outcome(
    worktree: &Path,
) -> std::io::Result<Option<BindingRepairOutcomeRecord>> {
    gwt::cli::execution_state::load_binding_repair_outcome(worktree)
}

#[cfg(test)]
fn persist_binding_repair_outcome(
    worktree: &Path,
    record: BindingRepairOutcomeRecord,
) -> std::io::Result<BindingRepairOutcomeRecord> {
    gwt::cli::execution_state::persist_binding_repair_outcome(worktree, record)
}

fn record_binding_repair_outcome(worktree: &Path, record: BindingRepairOutcomeRecord) -> bool {
    gwt::cli::execution_state::record_binding_repair_outcome(worktree, record)
}

pub fn repaired_binding_probe_receipt_matches(
    expected_session_id: &str,
    binding: &gwt_agent::SessionExecutionBinding,
    request: &gwt::AgentExecutionBindingProbeRequest,
    receipt: Option<&gwt::AgentExecutionBindingProbeReceipt>,
) -> bool {
    let Some(receipt) = receipt else {
        return false;
    };
    binding.session_id == expected_session_id
        && receipt.schema_version == gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION
        && receipt.operation_id == request.operation_id
        && receipt.nonce == request.nonce
        && !receipt.host_instance_id.trim().is_empty()
        && receipt.execution_binding == binding.identity
        && receipt.capability_generation == binding.capability_generation
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // Kept beside the private D3-R codec it exercises.
mod repaired_binding_probe_tests {
    use super::*;
    use crate::project_runtime::continuation::repaired_binding_probe_receipt_matches;

    fn init_repo(path: &Path) {
        std::fs::create_dir_all(path).expect("create repository directory");
        let status = gwt_core::process::hidden_command("git")
            .args(["init", "--quiet"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        let status = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://example.com/owner/binding-repair.git",
            ])
            .current_dir(path)
            .status()
            .expect("git remote add origin");
        assert!(status.success(), "git remote add origin failed");
    }

    fn binding() -> gwt_agent::SessionExecutionBinding {
        gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "session-current".to_string(),
            repo_hash: "repo-current".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3394,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-current".to_string(),
                binding_id: "binding-current".to_string(),
                ledger_head_hash: "head-current".to_string(),
            },
            capability_generation: 3,
        }
    }

    fn request() -> crate::AgentExecutionBindingProbeRequest {
        crate::AgentExecutionBindingProbeRequest {
            schema_version: crate::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: "operation-current".to_string(),
            nonce: "nonce-current".to_string(),
        }
    }

    fn exact_receipt(
        binding: &gwt_agent::SessionExecutionBinding,
        request: &crate::AgentExecutionBindingProbeRequest,
    ) -> crate::AgentExecutionBindingProbeReceipt {
        crate::AgentExecutionBindingProbeReceipt {
            schema_version: crate::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            operation_id: request.operation_id.clone(),
            nonce: request.nonce.clone(),
            host_instance_id: "host-current".to_string(),
            execution_binding: binding.identity.clone(),
            capability_generation: binding.capability_generation,
        }
    }

    #[test]
    fn repaired_authority_requires_an_exact_read_after_write_receipt() {
        let binding = binding();
        let request = request();
        let exact = exact_receipt(&binding, &request);
        assert!(repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            Some(&exact),
        ));
        assert!(!repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            None,
        ));

        let mut old_generation = exact.clone();
        old_generation.execution_binding.generation_id = "generation-old".to_string();
        assert!(!repaired_binding_probe_receipt_matches(
            "session-current",
            &binding,
            &request,
            Some(&old_generation),
        ));

        assert!(!repaired_binding_probe_receipt_matches(
            "session-foreign",
            &binding,
            &request,
            Some(&exact),
        ));
    }

    #[test]
    fn binding_repair_outcome_roundtrips_success_and_typed_failure() {
        let _env_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let binding = binding();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        };

        let stored_success = persist_binding_repair_outcome(
            &repo,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Succeeded {
                    host_instance_id: "host-current".to_string(),
                    receipt_generation_id: binding.identity.generation_id.clone(),
                },
            ),
        )
        .expect("persist successful repair outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect("load successful repair outcome")
                .expect("successful repair outcome"),
            stored_success
        );

        let stored_failure = persist_binding_repair_outcome(
            &repo,
            new_binding_repair_outcome_record(
                &binding,
                owner,
                BindingRepairOutcome::Failed {
                    cause: BindingRepairFailureCause::ProbeReceiptMismatch,
                    observed_generation_id: Some("generation-stale".to_string()),
                },
            ),
        )
        .expect("persist failed repair outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect("load failed repair outcome")
                .expect("failed repair outcome"),
            stored_failure
        );
    }

    #[test]
    fn binding_repair_outcome_rejects_tampering_and_store_failure_cannot_report_success() {
        let _env_guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let binding = binding();
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: binding.owner_number,
        };
        let success = new_binding_repair_outcome_record(
            &binding,
            owner,
            BindingRepairOutcome::Succeeded {
                host_instance_id: "host-current".to_string(),
                receipt_generation_id: binding.identity.generation_id.clone(),
            },
        );
        persist_binding_repair_outcome(&repo, success.clone()).expect("persist outcome");

        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(&repo)
            .expect("trusted worktree directory");
        let path = trusted_dir.join(BINDING_REPAIR_OUTCOME_FILE);
        let mut stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read outcome"))
                .expect("parse outcome");
        stored["outcome"]["receipt_generation_id"] =
            serde_json::Value::String("generation-forged".to_string());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&stored).expect("serialize forged outcome"),
        )
        .expect("forge outcome");
        assert_eq!(
            load_binding_repair_outcome(&repo)
                .expect_err("tampered outcome must fail closed")
                .kind(),
            std::io::ErrorKind::InvalidData
        );

        std::fs::remove_file(&path).expect("remove forged outcome");
        std::fs::create_dir(&path).expect("block outcome writer with directory");
        assert!(
            !record_binding_repair_outcome(&repo, success),
            "a verified repair must not report success when its outcome cannot be persisted"
        );
    }
}

#[derive(Debug)]
struct NonlocalActiveOwnerLivenessState {
    durable_status: Option<gwt_agent::AgentStatus>,
    durable_unknown: bool,
    saw_dead_runtime: bool,
    saw_stopped_runtime: bool,
    saw_unknown_runtime: bool,
}

impl NonlocalActiveOwnerLivenessState {
    fn finish(self) -> ActiveOwnerLiveness {
        if self.durable_unknown || self.saw_unknown_runtime {
            return ActiveOwnerLiveness::Unknown;
        }
        if self.saw_stopped_runtime {
            return ActiveOwnerLiveness::Stale("all owning Host runtimes are stopped");
        }
        if self.saw_dead_runtime {
            return ActiveOwnerLiveness::Stale("all owning Host runtimes are dead");
        }
        match self.durable_status {
            None => ActiveOwnerLiveness::Stale("durable Session is missing"),
            Some(gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted) => {
                ActiveOwnerLiveness::Stale("durable Session is stopped")
            }
            Some(_) => ActiveOwnerLiveness::Unknown,
        }
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NonlocalRuntimeIndexScanMetrics {
    pub runtime_root_enumerations: usize,
    pub runtime_namespace_enumerations: usize,
    pub host_process_probes: usize,
}

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static NONLOCAL_RUNTIME_INDEX_SCAN_METRICS:
        std::cell::Cell<NonlocalRuntimeIndexScanMetrics> =
        const { std::cell::Cell::new(NonlocalRuntimeIndexScanMetrics {
            runtime_root_enumerations: 0,
            runtime_namespace_enumerations: 0,
            host_process_probes: 0,
        }) };
}

#[cfg(any(test, feature = "test-gh-guard"))]
fn record_nonlocal_runtime_root_enumeration() {
    NONLOCAL_RUNTIME_INDEX_SCAN_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.runtime_root_enumerations += 1;
        metrics.set(current);
    });
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn record_nonlocal_runtime_root_enumeration() {}

#[cfg(any(test, feature = "test-gh-guard"))]
fn record_nonlocal_runtime_namespace_enumeration() {
    NONLOCAL_RUNTIME_INDEX_SCAN_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.runtime_namespace_enumerations += 1;
        metrics.set(current);
    });
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn record_nonlocal_runtime_namespace_enumeration() {}

#[cfg(any(test, feature = "test-gh-guard"))]
fn record_nonlocal_host_process_probe() {
    NONLOCAL_RUNTIME_INDEX_SCAN_METRICS.with(|metrics| {
        let mut current = metrics.get();
        current.host_process_probes += 1;
        metrics.set(current);
    });
}

#[cfg(not(any(test, feature = "test-gh-guard")))]
fn record_nonlocal_host_process_probe() {}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn reset_nonlocal_runtime_index_scan_metrics() {
    NONLOCAL_RUNTIME_INDEX_SCAN_METRICS.with(|metrics| {
        metrics.set(NonlocalRuntimeIndexScanMetrics::default());
    });
}

#[cfg(any(test, feature = "test-gh-guard"))]
pub fn nonlocal_runtime_index_scan_metrics() -> NonlocalRuntimeIndexScanMetrics {
    NONLOCAL_RUNTIME_INDEX_SCAN_METRICS.with(std::cell::Cell::get)
}

pub fn classify_nonlocal_active_owner_liveness_batch_at<'a>(
    sessions_dir: &Path,
    session_ids: impl IntoIterator<Item = &'a str>,
) -> HashMap<String, ActiveOwnerLiveness> {
    let mut states = session_ids
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|session_id| {
            let durable_path = sessions_dir.join(format!("{session_id}.toml"));
            let (durable_status, durable_unknown) =
                match gwt_agent::inspect_session_path(&durable_path) {
                    gwt_agent::SessionPathState::Present(session) => (Some(session.status), false),
                    gwt_agent::SessionPathState::Missing => (None, false),
                    gwt_agent::SessionPathState::Error(_) => (None, true),
                };
            (
                session_id,
                NonlocalActiveOwnerLivenessState {
                    durable_status,
                    durable_unknown,
                    saw_dead_runtime: false,
                    saw_stopped_runtime: false,
                    saw_unknown_runtime: false,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    if states.is_empty() {
        return HashMap::new();
    }

    let sidecar_owners = states
        .keys()
        .map(|session_id| (format!("{session_id}.json"), session_id.clone()))
        .collect::<HashMap<_, _>>();
    let runtime_root = sessions_dir.join("runtime");
    record_nonlocal_runtime_root_enumeration();
    let namespaces = match std::fs::read_dir(&runtime_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return states
                .into_iter()
                .map(|(session_id, state)| (session_id, state.finish()))
                .collect();
        }
        Err(_) => {
            for state in states.values_mut() {
                state.saw_unknown_runtime = true;
            }
            return states
                .into_iter()
                .map(|(session_id, state)| (session_id, state.finish()))
                .collect();
        }
    };

    let mut global_runtime_error = false;
    for namespace in namespaces {
        let namespace = match namespace {
            Ok(namespace) => namespace,
            Err(_) => {
                global_runtime_error = true;
                break;
            }
        };
        let Some(host_pid) = namespace
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        record_nonlocal_runtime_namespace_enumeration();
        let sidecars = match std::fs::read_dir(namespace.path()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                global_runtime_error = true;
                break;
            }
        };
        let mut host_is_alive = None;
        for sidecar in sidecars {
            let sidecar = match sidecar {
                Ok(sidecar) => sidecar,
                Err(_) => {
                    global_runtime_error = true;
                    break;
                }
            };
            let Some(session_id) = sidecar
                .file_name()
                .to_str()
                .and_then(|filename| sidecar_owners.get(filename))
            else {
                continue;
            };
            let Some(state) = states.get_mut(session_id) else {
                continue;
            };
            let host_is_alive = *host_is_alive.get_or_insert_with(|| {
                record_nonlocal_host_process_probe();
                gwt::process::is_host_process_alive(host_pid)
            });
            if !host_is_alive {
                state.saw_dead_runtime = true;
                continue;
            }
            match gwt_agent::SessionRuntimeState::load(&sidecar.path()) {
                Ok(runtime)
                    if matches!(
                        runtime.status,
                        gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                    ) =>
                {
                    state.saw_stopped_runtime = true;
                }
                Ok(_) | Err(_) => state.saw_unknown_runtime = true,
            }
        }
        if global_runtime_error {
            break;
        }
    }
    if global_runtime_error {
        for state in states.values_mut() {
            state.saw_unknown_runtime = true;
        }
    }
    states
        .into_iter()
        .map(|(session_id, state)| (session_id, state.finish()))
        .collect()
}

pub fn classify_nonlocal_active_owner_liveness_at(
    sessions_dir: &Path,
    session_id: &str,
) -> ActiveOwnerLiveness {
    classify_nonlocal_active_owner_liveness_batch_at(sessions_dir, [session_id])
        .remove(session_id)
        .unwrap_or(ActiveOwnerLiveness::Unknown)
}
